//! Admit idle sleep through the same retained lifecycle as an explicit Sleep.
//! Provider effects still require guest quiescence and the durable worker lease.
use super::*;

fn eligible(row: &Value) -> bool {
    let config = &row["request_config"];
    row["state"] == "ready"
        && row["desired_state"] == "ready"
        && row["runtime_state"] == "online"
        && task_commands::command_mode(row)
        && native_commands_supported(row)
        && storage_enrolled(row)
        && config["rustStorageEnrollment"]["capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|cap| cap == CAPABILITY))
        && config["runtimeReleaseChangeRequested"] != true
        && !matches!(
            config["runtimeActivation"]["phase"].as_str(),
            Some("preparing" | "launching" | "confirming" | "rolling-back")
        )
        && config["rustLifecycleOperation"]["phase"]
            .as_str()
            .is_none_or(|phase| phase == "ready")
        && !super::super::recreation::leases_blocked(row)
}

/// When the worker should next run idle-sleep admission: the earliest known
/// deadline, but never later than one sweep so a computer that became
/// eligible without a notification is still admitted on time (its deadline is
/// at least a full idle timeout away, and the sweep is at most half of it).
pub fn next_check(now: i64, due: Option<i64>, sweep_ms: i64) -> i64 {
    let sweep = now.saturating_add(sweep_ms);
    match due {
        // Already due but not admitted this round (for example a concurrent
        // command): look again after a short floor instead of spinning.
        Some(at) if at <= now => now.saturating_add(1_000).min(sweep),
        Some(at) => at.saturating_add(5).min(sweep),
        None => sweep,
    }
}

/// Locked recheck of idle_candidates.sql's busy predicate. A live command or
/// any command write inside the idle window keeps the computer awake.
/// `outcome-unknown` is not live: it is terminal (a runtime report can never
/// move it, see task_commands::progress), so it holds the computer for one
/// idle window from when it was recorded, like a completed turn. Counting it
/// as live kept computers whose turn was cut off awake until someone paused
/// them by hand.
const BUSY: &str = "SELECT EXISTS(SELECT 1 FROM api_cloud_workspace_launch_intents WHERE workspace_id=$1) OR EXISTS(SELECT 1 FROM zuse_rust_task_command_execution WHERE workspace_id=$1 AND (state IN('queued','starting','running','waiting-input') OR updated_at>$2))";

/// Longest idle-sleep sweep interval.
pub const MAX_SWEEP_MS: i64 = 300_000;

impl RuntimeControl {
    /// Bound on the idle-sleep sweep: at most half the idle timeout.
    pub fn idle_sweep_ms(&self) -> i64 {
        (self.idle_ms / 2).clamp(500, MAX_SWEEP_MS)
    }
    /// The earliest idle-sleep or shared-credit sleep deadline, in one or two
    /// statements, using the same eligibility as admission.
    pub async fn idle_sleep_due(&self, now: i64) -> Result<Option<i64>, ApiError> {
        if self.creation_accounts.is_empty() || !self.task_command_delivery_available().await? {
            return Ok(None);
        }
        let quiet = self
            .idle_ms
            .saturating_add(crate::runtime::activity::COALESCE_MS);
        let idle: Option<i64> = sqlx::query_scalar(include_str!("idle_deadline.sql"))
            .bind(self.creation_accounts.filter())
            .bind(quiet)
            .bind(zuse_protocol::cloud_task_commands::MODE)
            .fetch_one(&self.pool)
            .await?;
        let installed: bool = crate::db::schema::present(
            &self.pool,
            "SELECT to_regclass('zuse_brokeboy_compute_charges') IS NOT NULL",
        )
        .await?;
        let credit: Option<i64> = if installed {
            sqlx::query_scalar(include_str!("credit_deadline.sql"))
                .bind(self.creation_accounts.filter())
                .bind(now)
                .bind(zuse_protocol::cloud_task_commands::MODE)
                .fetch_one(&self.pool)
                .await?
        } else {
            None
        };
        let backstop = self.backstop_due(now).await?;
        Ok(crate::db::wake::earliest([idle, credit, backstop]))
    }
    /// Admit at most one due computer per worker round. Empty scans are paced
    /// by the worker; an open event subscription is not user activity.
    pub async fn idle_sleep_once(&self, now: i64) -> Result<bool, ApiError> {
        if self.creation_accounts.is_empty() || !self.task_command_delivery_available().await? {
            return Ok(false);
        }
        if self.credit_sleep_once(now).await? {
            return Ok(true);
        }
        // Coalesced terminal writes must each receive at least the full idle
        // interval. Older guests already have durable command timestamps, so
        // upgrading the API cannot mistake a recent completed turn for idle.
        let cutoff = now
            .saturating_sub(self.idle_ms)
            .saturating_sub(crate::runtime::activity::COALESCE_MS);
        let candidate: Option<String> = sqlx::query_scalar(include_str!("idle_candidates.sql"))
            .bind(self.creation_accounts.filter())
            .bind(cutoff)
            .bind(now)
            .bind(zuse_protocol::cloud_task_commands::MODE)
            .fetch_optional(&self.pool)
            .await?;
        let Some(id) = candidate else {
            // Nothing passed the normal eligibility: let the independent
            // max-quiet-awake backstop look for computers it missed.
            return self.backstop_sleep_once(now).await;
        };
        let mut tx = self.pool.begin().await?;
        let (row, generation, _) = locking::workspace(&mut tx, &id, true).await?;
        if !eligible(&row)
            || !row["account_id"]
                .as_str()
                .is_some_and(|owner| self.creation_accounts.contains(owner))
            || row["last_activity_at"]
                .as_i64()
                .is_none_or(|at| at > cutoff)
            || row["next_action_at"].as_i64().is_none_or(|at| at > now)
        {
            return Ok(false);
        }
        let busy: bool = sqlx::query_scalar(BUSY)
            .bind(&id)
            .bind(cutoff)
            .fetch_one(&mut *tx)
            .await?;
        if busy {
            return Ok(false);
        }
        self.admit_credit_or_idle_sleep(tx, row, generation, now, "idle-timeout")
            .await
    }
    async fn credit_sleep_once(&self, now: i64) -> Result<bool, ApiError> {
        let installed: bool = crate::db::schema::present(
            &self.pool,
            "SELECT to_regclass('zuse_brokeboy_compute_charges') IS NOT NULL",
        )
        .await?;
        if !installed {
            return Ok(false);
        }
        let candidate: Option<String> = sqlx::query_scalar(include_str!("credit_candidates.sql"))
            .bind(self.creation_accounts.filter())
            .bind(now)
            .bind(now)
            .bind(zuse_protocol::cloud_task_commands::MODE)
            .fetch_optional(&self.pool)
            .await?;
        let Some(id) = candidate else {
            return Ok(false);
        };
        let mut tx = self.pool.begin().await?;
        let (row, generation, _) = locking::workspace(&mut tx, &id, true).await?;
        if !eligible(&row) {
            return Ok(false);
        }
        let held:Option<bool>=sqlx::query_scalar("SELECT shared_credit_hold FROM api_cloud_billing_periods WHERE account_id=$1 AND shared_credit_account AND period_start<=$2 AND period_end>$2 ORDER BY period_start DESC LIMIT 1 FOR SHARE").bind(required(&row,"account_id")?).bind(now).fetch_optional(&mut *tx).await?;
        if held != Some(true) {
            return Ok(false);
        }
        self.admit_credit_or_idle_sleep(tx, row, generation, now, "credits-exhausted")
            .await
    }
    async fn admit_credit_or_idle_sleep(
        &self,
        mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
        row: Value,
        generation: i64,
        now: i64,
        reason: &str,
    ) -> Result<bool, ApiError> {
        let id = required(&row, "workspace_id")?;
        let operation = uuid::Uuid::new_v4().to_string();
        let mut config = row["request_config"].clone();
        progress(&mut config, &operation, Action::Pause, "quiescing");
        config["rustLifecycleOperation"]["reason"] = json!(reason);
        let command = format!(
            "{}:{generation}:{}",
            if reason == "idle-timeout" {
                "idle"
            } else {
                "credit"
            },
            row["next_action_at"].as_i64().unwrap_or(0)
        );
        sqlx::query("INSERT INTO zuse_rust_lifecycle_operations(operation_id,workspace_id,command_id,generation,source_machine_id,storage_incarnation_id,action,phase,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,'pause','quiescing',$7,$7)")
            .bind(&operation).bind(&id).bind(command).bind(generation).bind(required(&row,"provider_sandbox_id")?)
            .bind(required(&config["rustRuntimeStorage"],"storageIncarnationId")?).bind(now).execute(&mut *tx).await?;
        sqlx::query("UPDATE api_cloud_workspaces SET state='pausing',desired_state='paused',status_code='pause-quiescing',request_config=$2,revision=revision+1,updated_at=GREATEST(updated_at+1,$3),next_action_at=$3 WHERE workspace_id=$1")
            .bind(&id).bind(config).bind(now).execute(&mut *tx).await?;
        let job = jobs::enqueue(
            &mut tx,
            &format!("lifecycle:{operation}"),
            JOB_KIND,
            &id,
            generation,
            &json!({"operationId":operation}),
            now,
        )
        .await?;
        sqlx::query("UPDATE zuse_rust_jobs SET max_attempts=120 WHERE job_id=$1")
            .bind(job)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        wakeup::send();
        Ok(true)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod schedule_tests {
    use super::*;

    #[test]
    fn next_check_follows_deadlines_bounded_by_the_sweep() {
        assert_eq!(next_check(1_000, None, 300_000), 301_000);
        assert_eq!(next_check(1_000, Some(61_000), 300_000), 61_005);
        assert_eq!(next_check(1_000, Some(900_000), 300_000), 301_000);
        // Past due but not admitted: a short floor, never a spin.
        assert_eq!(next_check(1_000, Some(1_000), 300_000), 2_000);
        assert_eq!(next_check(1_000, Some(0), 500), 1_500);
        assert_eq!(next_check(i64::MAX, Some(i64::MAX), 300_000), i64::MAX);
    }
}
