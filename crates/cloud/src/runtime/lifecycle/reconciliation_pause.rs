//! Pause a computer that the provider reconciliation pass found awake long
//! past its quiet bound, through the same retained lifecycle as an explicit
//! Sleep (guest quiescence first, then the provider effect under the durable
//! worker lease), so the database stays the source of truth.
use super::*;

/// What the pass decided about one stale computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::runtime) enum PauseAdmission {
    Admitted,
    /// Something changed since the pass looked (activity, a lease, a turn,
    /// another operation, a different machine): leave it alone.
    Busy,
    /// The lifecycle cannot pause it (for example its runtime is offline and
    /// cannot quiesce): an operator has to look.
    Unpausable,
}

impl RuntimeControl {
    /// Re-checks everything under the workspace lock before admitting.
    /// `cutoff`: activity at or after it means the computer is in use.
    pub(in crate::runtime) async fn admit_reconciliation_pause(
        &self,
        id: &str,
        machine: &str,
        cutoff: i64,
        now: i64,
    ) -> Result<PauseAdmission, ApiError> {
        let mut tx = self.pool.begin().await?;
        let (row, generation, _) = locking::workspace(&mut tx, id, true).await?;
        if row["state"] != "ready"
            || row["desired_state"] != "ready"
            || row["provider_sandbox_id"] != machine
            || deleted(&row)
            || row["request_config"]["rustLifecycleOperation"]["phase"]
                .as_str()
                .is_some_and(|phase| phase != "ready")
            || row["last_activity_at"]
                .as_i64()
                .is_some_and(|at| at > cutoff)
        {
            return Ok(PauseAdmission::Busy);
        }
        let busy: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM api_cloud_workspace_launch_intents WHERE workspace_id=$1) OR EXISTS(SELECT 1 FROM zuse_rust_gateway_leases WHERE workspace_id=$1 AND expires_at>$3) OR EXISTS(SELECT 1 FROM zuse_rust_task_command_execution WHERE workspace_id=$1 AND (state IN('queued','starting','running','waiting-input') OR updated_at>$2))")
            .bind(id).bind(cutoff).bind(now).fetch_one(&mut *tx).await?;
        if busy {
            return Ok(PauseAdmission::Busy);
        }
        // The same conditions idle sleep requires: the guest must be able to
        // quiesce, or the pause would only wedge the workspace in `pausing`.
        let config = &row["request_config"];
        let pausable = row["runtime_state"] == "online"
            && task_commands::command_mode(&row)
            && native_commands_supported(&row)
            && storage_enrolled(&row)
            && config["rustStorageEnrollment"]["capabilities"]
                .as_array()
                .is_some_and(|caps| caps.iter().any(|cap| cap == CAPABILITY))
            && config["runtimeReleaseChangeRequested"] != true
            && !matches!(
                config["runtimeActivation"]["phase"].as_str(),
                Some("preparing" | "launching" | "confirming" | "rolling-back")
            )
            && !super::super::recreation::leases_blocked(&row)
            && !super::super::repair::leases_blocked(&row);
        if !pausable {
            return Ok(PauseAdmission::Unpausable);
        }
        let operation = uuid::Uuid::new_v4().to_string();
        let mut config = config.clone();
        progress(&mut config, &operation, Action::Pause, "quiescing");
        config["rustLifecycleOperation"]["reason"] = json!("provider-reconciliation");
        let command = format!(
            "reconcile:{generation}:{}",
            row["next_action_at"].as_i64().unwrap_or(0)
        );
        sqlx::query("INSERT INTO zuse_rust_lifecycle_operations(operation_id,workspace_id,command_id,generation,source_machine_id,storage_incarnation_id,action,phase,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,'pause','quiescing',$7,$7)")
            .bind(&operation).bind(id).bind(command).bind(generation).bind(machine)
            .bind(required(&config["rustRuntimeStorage"],"storageIncarnationId")?).bind(now).execute(&mut *tx).await?;
        sqlx::query("UPDATE api_cloud_workspaces SET state='pausing',desired_state='paused',status_code='pause-quiescing',request_config=$2,revision=revision+1,updated_at=GREATEST(updated_at+1,$3),next_action_at=$3 WHERE workspace_id=$1")
            .bind(id).bind(config).bind(now).execute(&mut *tx).await?;
        let job = jobs::enqueue(
            &mut tx,
            &format!("lifecycle:{operation}"),
            JOB_KIND,
            id,
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
        Ok(PauseAdmission::Admitted)
    }
}
