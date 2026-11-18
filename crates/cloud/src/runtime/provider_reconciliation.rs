//! A second line of defence against paying for machines nobody expects to run.
//!
//! The lifecycle (idle sleep, archive, delete) is driven from the database.
//! This pass looks from the other side: it lists what each provider says
//! exists, compares every machine with this environment's records, and puts to
//! sleep (or, after a safety window, deletes) machines the database does not
//! expect to be awake. It runs at a modest interval on the worker's due-time
//! schedule (`zuse_rust_provider_reconciliation_schedule`), never as a poll.
//!
//! Ownership. Staging and production share one provider organization and the
//! provider has no machine labels. A machine is this environment's only when
//! this database recorded its id or name (allocation names are random per
//! operation and written before the machine is created; migration 0053 keeps
//! them in `zuse_rust_machine_ledger` after a workspace is deleted). Anything
//! else is *foreign*: counted, never touched.
//!
//! Safety. A verdict must hold for a confirmation window before anything is
//! done, and is re-derived from a fresh database read and a fresh provider
//! inspection right before acting. Sleeping a computer the database says is
//! awake goes through the retained lifecycle (guest quiescence first). Only a
//! machine whose workspace the database confirms deleted is ever deleted, only
//! once it is asleep, and only after a longer safety window. `dry-run` (the
//! default) logs every decision at WARN and records alerts without acting.
use super::RuntimeControl;
use crate::{
    error::ApiError,
    providers::{ListedMachine, MachineState, contract::Provider, registry::Registry},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::atomic::Ordering, time::Duration};
use zuse_protocol::cloud_computers::ProviderId;

mod cost;
#[cfg(test)]
mod tests;

pub use cost::CostReport;

const FACTS: &str = include_str!("provider_reconciliation/facts.sql");
const LEDGER: &str = "UNION ALL SELECT l.workspace_id, 'ledger', 3 FROM zuse_rust_machine_ledger l WHERE l.machine_key = 'id:' || m.id OR (m.name <> '' AND l.machine_key = 'name:' || m.name)";
const SCHEDULE_PRESENT: &str =
    "SELECT to_regclass('zuse_rust_provider_reconciliation_schedule') IS NOT NULL";
const LEDGER_PRESENT: &str = "SELECT to_regclass('zuse_rust_machine_ledger') IS NOT NULL";
/// The worker due-time source (`runtime::due`).
pub(super) const DUE_SOURCE: &str = "SELECT 'provider_reconciliation', min(GREATEST(next_run_at, COALESCE(lease_expires_at, next_run_at)))::bigint FROM zuse_rust_provider_reconciliation_schedule";
const MACHINES_TASK: &str = "provider-machines";
pub(super) const COST_TASK: &str = "cost-anomalies";
/// One pass holds its lease at most this long; a crashed pass is retried then.
const LEASE_MS: i64 = 15 * 60_000;
/// After a failed pass, try again sooner than the full interval.
const RETRY_MS: i64 = 5 * 60_000;
const LIST_TIMEOUT: Duration = Duration::from_secs(120);
const ACTION_TIMEOUT: Duration = Duration::from_secs(60);
/// Machines classified per facts query.
const BATCH: usize = 200;

/// What the pass may do. Unset means `dry-run`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Off,
    DryRun,
    Sleep,
    SleepAndDelete,
}
impl Mode {
    fn parse(raw: Option<&str>) -> anyhow::Result<Self> {
        Ok(match raw.map(str::trim) {
            None | Some("") | Some("dry-run") => Self::DryRun,
            Some("off") => Self::Off,
            Some("sleep") => Self::Sleep,
            Some("sleep-and-delete") => Self::SleepAndDelete,
            Some(_) => anyhow::bail!("invalid_provider_reconciliation_mode"),
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::DryRun => "dry-run",
            Self::Sleep => "sleep",
            Self::SleepAndDelete => "sleep-and-delete",
        }
    }
    fn acts(self) -> bool {
        matches!(self, Self::Sleep | Self::SleepAndDelete)
    }
    fn deletes(self) -> bool {
        self == Self::SleepAndDelete
    }
}

/// Worker configuration (environment variables named in [`Config::from_lookup`]).
#[derive(Clone, Debug)]
pub struct Config {
    pub mode: Mode,
    /// How often the machine pass runs.
    pub interval_ms: i64,
    /// A computer the database thinks is awake, with no activity, turn, launch
    /// or runtime lease for this long, should be asleep.
    pub quiet_ms: i64,
    /// A verdict must hold this long before the pass acts on it.
    pub confirm_ms: i64,
    /// A machine of a deleted workspace is deleted only after this long.
    pub delete_after_ms: i64,
    /// Cost check: running this long in a day without activity is flagged.
    pub awake_alert_ms: i64,
    /// Cost check: an account's day costing more than this many times its
    /// trailing seven-day daily average is flagged ...
    pub spike_factor: i64,
    /// ... once it is at least this much (micro-dollars).
    pub spike_floor_micros: i64,
}
impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }
    pub fn from_lookup(env: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let number = |name: &str, default: i64, min: i64, max: i64| -> anyhow::Result<i64> {
            match env(name).filter(|raw| !raw.trim().is_empty()) {
                None => Ok(default),
                Some(raw) => {
                    let value: i64 = raw
                        .trim()
                        .parse()
                        .map_err(|_| anyhow::anyhow!("invalid {name}"))?;
                    anyhow::ensure!((min..=max).contains(&value), "{name} out of range");
                    Ok(value)
                }
            }
        };
        const MINUTE: i64 = 60_000;
        const HOUR: i64 = 60 * MINUTE;
        Ok(Self {
            mode: Mode::parse(env("ZUSE_CLOUD_PROVIDER_RECONCILIATION").as_deref())?,
            interval_ms: number(
                "ZUSE_CLOUD_PROVIDER_RECONCILIATION_INTERVAL_MS",
                20 * MINUTE,
                5 * MINUTE,
                6 * HOUR,
            )?,
            quiet_ms: number(
                "ZUSE_CLOUD_PROVIDER_RECONCILIATION_QUIET_MS",
                4 * HOUR,
                30 * MINUTE,
                7 * 24 * HOUR,
            )?,
            confirm_ms: number(
                "ZUSE_CLOUD_PROVIDER_RECONCILIATION_CONFIRM_MS",
                10 * MINUTE,
                0,
                24 * HOUR,
            )?,
            delete_after_ms: number(
                "ZUSE_CLOUD_PROVIDER_RECONCILIATION_DELETE_AFTER_MS",
                24 * HOUR,
                HOUR,
                30 * 24 * HOUR,
            )?,
            awake_alert_ms: number("ZUSE_CLOUD_COST_ALERT_AWAKE_MS", 6 * HOUR, HOUR, 24 * HOUR)?,
            spike_factor: number("ZUSE_CLOUD_COST_ALERT_SPIKE_FACTOR", 3, 2, 100)?,
            spike_floor_micros: number(
                "ZUSE_CLOUD_COST_ALERT_SPIKE_FLOOR_MICROS",
                5_000_000,
                0,
                1_000_000_000_000,
            )?,
        })
    }
}

/// Database records about one listed machine (see `facts.sql`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Facts {
    /// The live workspace whose current computer this is.
    pub workspace: Option<CurrentWorkspace>,
    /// Recent work that may legitimately have this machine awake.
    pub in_flight: Option<String>,
    /// Never-touched infrastructure that references the machine.
    pub infrastructure: Option<String>,
    /// The workspace that created or last used it, from history or the ledger.
    pub owner: Option<Owner>,
    /// A machine image or project build that referenced it.
    pub builder: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CurrentWorkspace {
    pub id: String,
    pub account: Option<String>,
    pub state: String,
    pub desired: String,
    pub runtime: Option<String>,
    pub last_activity_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub lifecycle_phase: Option<String>,
    #[serde(default)]
    pub live_lease: bool,
    #[serde(default)]
    pub launch_intent: bool,
    #[serde(default)]
    pub active_turn: bool,
    pub last_turn_at: Option<i64>,
    #[serde(default)]
    pub pending_job: bool,
}
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Owner {
    pub workspace_id: Option<String>,
    pub source: String,
    /// `alive`, `deleted`, `missing` (no workspace row) or `none` (not a workspace machine).
    pub status: String,
    pub account: Option<String>,
    pub last_activity_at: Option<i64>,
    pub current_machine: Option<String>,
}
impl Facts {
    /// The workspace and account the machine's cost belongs to, if any.
    pub fn attribution(&self) -> Option<(String, Option<String>, Option<i64>)> {
        if let Some(ws) = &self.workspace {
            let activity = ws.last_activity_at.max(ws.last_turn_at);
            return Some((ws.id.clone(), ws.account.clone(), activity));
        }
        let owner = self.owner.as_ref()?;
        Some((
            owner.workspace_id.clone()?,
            owner.account.clone(),
            owner.last_activity_at,
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Class {
    /// Not recorded in this environment: never touched.
    Foreign,
    /// The database expects this machine in its current state.
    Expected,
    /// Ours and awake, but the database does not expect it awake.
    ShouldSleep,
    /// Ours, but nothing in the database still needs it.
    ShouldNotExist,
}
impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Foreign => "foreign",
            Self::Expected => "expected",
            Self::ShouldSleep => "should-sleep",
            Self::ShouldNotExist => "should-not-exist",
        }
    }
}

/// What the pass would do about a machine, before confirmation windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Planned {
    Nothing,
    /// Provider sleep: the database already records the computer asleep, or
    /// nothing in it needs the machine.
    Sleep,
    /// Retained-lifecycle pause: the database thinks the computer is awake.
    Pause,
    /// Delete an asleep machine whose workspace the database confirms deleted.
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    pub class: Class,
    pub reason: &'static str,
    pub workspace: Option<String>,
    pub planned: Planned,
    /// Expected, but stuck long enough that an operator should look.
    pub alert: bool,
}
impl Verdict {
    fn of(class: Class, reason: &'static str, workspace: Option<&str>, planned: Planned) -> Self {
        Self {
            class,
            reason,
            workspace: workspace.map(str::to_owned),
            planned,
            alert: !matches!(class, Class::Foreign | Class::Expected),
        }
    }
    /// The identity that must stay the same across passes for the
    /// confirmation and deletion windows to keep running.
    fn key(&self) -> String {
        format!("{}:{}", self.class.as_str(), self.reason)
    }
}

fn awake(state: &MachineState) -> bool {
    *state == MachineState::Running
}
fn asleep(state: &MachineState) -> bool {
    matches!(
        state,
        MachineState::Standby | MachineState::Paused | MachineState::Stopped
    )
}

/// The whole decision for one machine. Pure, so every rule is unit-tested.
pub fn classify(machine: &ListedMachine, facts: &Facts, now: i64, config: &Config) -> Verdict {
    let cutoff = now.saturating_sub(config.quiet_ms);
    let workspace = facts
        .workspace
        .as_ref()
        .map(|ws| ws.id.as_str())
        .or_else(|| facts.owner.as_ref().and_then(|o| o.workspace_id.as_deref()));
    if facts.infrastructure.is_some() {
        return Verdict::of(
            Class::Expected,
            "infrastructure",
            workspace,
            Planned::Nothing,
        );
    }
    if facts.in_flight.is_some() || facts.workspace.as_ref().is_some_and(|ws| ws.pending_job) {
        return Verdict::of(
            Class::Expected,
            "operation-in-flight",
            workspace,
            Planned::Nothing,
        );
    }
    let is_awake = awake(&machine.state);
    let is_asleep = asleep(&machine.state);
    if let Some(ws) = &facts.workspace {
        if !is_awake {
            return Verdict::of(Class::Expected, "not-awake", workspace, Planned::Nothing);
        }
        let recent = |at: Option<i64>| at.is_some_and(|at| at >= cutoff);
        let state = (ws.state.as_str(), ws.desired.as_str());
        return match state {
            ("ready", "ready") => {
                if ws.launch_intent
                    || ws.active_turn
                    || ws.live_lease
                    || recent(ws.last_activity_at)
                    || recent(ws.last_turn_at)
                {
                    Verdict::of(Class::Expected, "in-use", workspace, Planned::Nothing)
                } else {
                    Verdict::of(
                        Class::ShouldSleep,
                        "quiet-past-bound",
                        workspace,
                        Planned::Pause,
                    )
                }
            }
            ("paused" | "archived", "paused" | "archived")
                if ws
                    .lifecycle_phase
                    .as_deref()
                    .is_none_or(|phase| matches!(phase, "paused" | "ready")) =>
            {
                Verdict::of(
                    Class::ShouldSleep,
                    "asleep-in-database",
                    workspace,
                    Planned::Sleep,
                )
            }
            ("failed", _) => Verdict::of(
                Class::ShouldSleep,
                "workspace-failed",
                workspace,
                Planned::Sleep,
            ),
            _ => {
                let mut verdict = Verdict::of(
                    Class::Expected,
                    "in-transition",
                    workspace,
                    Planned::Nothing,
                );
                verdict.alert = ws.updated_at.is_none_or(|at| at < cutoff);
                verdict
            }
        };
    }
    let leftover = |reason, deletable: bool| {
        let planned = if is_awake {
            Planned::Sleep
        } else if deletable && is_asleep {
            Planned::Delete
        } else {
            Planned::Nothing
        };
        Verdict::of(Class::ShouldNotExist, reason, workspace, planned)
    };
    if let Some(owner) = &facts.owner {
        match owner.status.as_str() {
            "deleted" | "missing" => return leftover("workspace-deleted", true),
            "alive" => return leftover("replaced", false),
            _ => {}
        }
    }
    if facts.builder.is_some() {
        return leftover("builder-leftover", false);
    }
    Verdict::of(
        Class::Foreign,
        "not-recorded-here",
        workspace,
        Planned::Nothing,
    )
}

/// One pass's outcome, saved on the schedule row and logged.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub mode: Option<Mode>,
    pub listed: usize,
    pub foreign: usize,
    pub expected: usize,
    pub should_sleep: usize,
    pub should_not_exist: usize,
    pub slept: usize,
    pub paused: usize,
    pub deleted: usize,
    /// Actions skipped by dry-run or still inside their confirmation window.
    pub deferred: usize,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug)]
struct Observation {
    key: String,
    first_seen_at: i64,
    last_action: Option<String>,
    last_action_at: Option<i64>,
}

impl RuntimeControl {
    /// Called once by the worker: its due time joins the worker schedule.
    pub fn enable_provider_reconciliation(&self, enabled: bool) {
        self.provider_reconciliation
            .store(enabled, Ordering::Relaxed);
    }
    pub(super) async fn provider_reconciliation_scheduled(&self) -> Result<bool, ApiError> {
        Ok(self.provider_reconciliation.load(Ordering::Relaxed)
            && crate::db::schema::present(&self.pool, SCHEDULE_PRESENT).await?)
    }

    /// Claims a due schedule row for this pass; `None` when not due or held.
    pub(super) async fn claim_reconciliation(
        &self,
        task: &str,
        now: i64,
    ) -> Result<Option<String>, ApiError> {
        let token = uuid::Uuid::new_v4().to_string();
        let claimed = sqlx::query("UPDATE zuse_rust_provider_reconciliation_schedule SET lease_token=$2,lease_expires_at=$3+$4,updated_at=$3 WHERE task=$1 AND next_run_at<=$3 AND (lease_expires_at IS NULL OR lease_expires_at<=$3)")
            .bind(task).bind(&token).bind(now).bind(LEASE_MS).execute(&self.pool).await?;
        Ok((claimed.rows_affected() == 1).then_some(token))
    }
    pub(super) async fn finish_reconciliation(
        &self,
        task: &str,
        token: &str,
        next_run_at: i64,
        summary: Option<Value>,
        now: i64,
    ) -> Result<(), ApiError> {
        sqlx::query("UPDATE zuse_rust_provider_reconciliation_schedule SET next_run_at=$3,lease_token=NULL,lease_expires_at=NULL,last_completed_at=CASE WHEN $4::jsonb IS NULL THEN last_completed_at ELSE $5 END,last_summary=COALESCE($4,last_summary),updated_at=$5 WHERE task=$1 AND lease_token=$2")
            .bind(task).bind(token).bind(next_run_at).bind(summary).bind(now).execute(&self.pool).await?;
        Ok(())
    }

    /// One machine pass when due. `Ok(None)`: disabled, not installed, not due
    /// or another worker holds it.
    pub async fn provider_reconciliation_once(
        &self,
        registry: &Registry,
        config: &Config,
        now: i64,
    ) -> Result<Option<Summary>, ApiError> {
        if config.mode == Mode::Off
            || !crate::db::schema::present(&self.pool, SCHEDULE_PRESENT).await?
        {
            return Ok(None);
        }
        let Some(token) = self.claim_reconciliation(MACHINES_TASK, now).await? else {
            return Ok(None);
        };
        match self.reconcile_machines(registry, config, now).await {
            Ok(summary) => {
                let next = clock().max(now).saturating_add(config.interval_ms);
                self.finish_reconciliation(
                    MACHINES_TASK,
                    &token,
                    next,
                    serde_json::to_value(&summary).ok(),
                    clock(),
                )
                .await?;
                Ok(Some(summary))
            }
            Err(error) => {
                let next = clock()
                    .max(now)
                    .saturating_add(RETRY_MS.min(config.interval_ms));
                self.finish_reconciliation(MACHINES_TASK, &token, next, None, clock())
                    .await?;
                Err(error)
            }
        }
    }

    async fn reconcile_machines(
        &self,
        registry: &Registry,
        config: &Config,
        now: i64,
    ) -> Result<Summary, ApiError> {
        let mut summary = Summary {
            mode: Some(config.mode),
            ..Summary::default()
        };
        for id in registry.configured_ids() {
            let Some(provider) = registry.get(id) else {
                continue;
            };
            let listing = match tokio::time::timeout(LIST_TIMEOUT, provider.list_machines()).await {
                Ok(Ok(Some(listing))) => listing,
                Ok(Ok(None)) => continue,
                Ok(Err(error)) => {
                    tracing::warn!(
                        provider = id.as_str(),
                        ?error,
                        "Provider machine listing deferred"
                    );
                    summary.errors.push(format!("{}:list", id.as_str()));
                    continue;
                }
                Err(_) => {
                    tracing::warn!(provider = id.as_str(), "Provider machine listing timed out");
                    summary.errors.push(format!("{}:list-timeout", id.as_str()));
                    continue;
                }
            };
            self.reconcile_listing(&*provider, id, &listing, config, now, &mut summary)
                .await?;
        }
        tracing::info!(
            mode = config.mode.as_str(),
            listed = summary.listed,
            foreign = summary.foreign,
            expected = summary.expected,
            should_sleep = summary.should_sleep,
            should_not_exist = summary.should_not_exist,
            slept = summary.slept,
            paused = summary.paused,
            deleted = summary.deleted,
            deferred = summary.deferred,
            "Provider machine reconciliation finished"
        );
        Ok(summary)
    }

    /// Database facts for machines of one provider, in listing order.
    pub(super) async fn machine_facts(
        &self,
        provider: ProviderId,
        machines: &[(String, String)],
        config: &Config,
        now: i64,
    ) -> Result<Vec<Facts>, ApiError> {
        static WITH_LEDGER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        static WITHOUT_LEDGER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let sql = if crate::db::schema::present(&self.pool, LEDGER_PRESENT).await? {
            WITH_LEDGER.get_or_init(|| FACTS.replace("/*LEDGER*/", LEDGER))
        } else {
            WITHOUT_LEDGER.get_or_init(|| FACTS.replace("/*LEDGER*/", ""))
        };
        let mut facts = Vec::with_capacity(machines.len());
        for chunk in machines.chunks(BATCH) {
            let ids: Vec<&str> = chunk.iter().map(|(id, _)| id.as_str()).collect();
            let names: Vec<&str> = chunk.iter().map(|(_, name)| name.as_str()).collect();
            let rows: Vec<(i64, Value)> = sqlx::query_as(sql)
                .bind(&ids)
                .bind(&names)
                .bind(provider.as_str())
                .bind(now)
                .bind(now.saturating_sub(config.quiet_ms))
                .fetch_all(&self.pool)
                .await?;
            if rows.len() != chunk.len() {
                return Err(ApiError::unavailable(
                    "provider_reconciliation_facts_invalid",
                ));
            }
            for (_, value) in rows {
                facts.push(
                    serde_json::from_value(value).map_err(|_| {
                        ApiError::unavailable("provider_reconciliation_facts_invalid")
                    })?,
                );
            }
        }
        Ok(facts)
    }

    async fn reconcile_listing(
        &self,
        provider: &dyn Provider,
        id: ProviderId,
        listing: &[ListedMachine],
        config: &Config,
        now: i64,
        summary: &mut Summary,
    ) -> Result<(), ApiError> {
        let pairs: Vec<(String, String)> = listing
            .iter()
            .map(|m| (m.id.clone(), m.name.clone()))
            .collect();
        let facts = self.machine_facts(id, &pairs, config, now).await?;
        let prior: HashMap<String, Observation> = sqlx::query_as::<_, (String, String, String, i64, Option<String>, Option<i64>)>(
            "SELECT machine_id,verdict,reason,first_seen_at,last_action,last_action_at FROM zuse_rust_provider_machine_observations WHERE provider=$1",
        )
        .bind(id.as_str())
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|(machine, verdict, reason, first_seen_at, last_action, last_action_at)| {
            (
                machine,
                Observation {
                    key: format!("{verdict}:{reason}"),
                    first_seen_at,
                    last_action,
                    last_action_at,
                },
            )
        })
        .collect();
        let mut observed = Vec::new();
        for (machine, facts) in listing.iter().zip(&facts) {
            summary.listed += 1;
            let verdict = classify(machine, facts, now, config);
            match verdict.class {
                Class::Foreign => summary.foreign += 1,
                Class::Expected => summary.expected += 1,
                Class::ShouldSleep => summary.should_sleep += 1,
                Class::ShouldNotExist => summary.should_not_exist += 1,
            }
            if !verdict.alert {
                continue;
            }
            let previous = prior.get(&machine.id).filter(|p| p.key == verdict.key());
            let first_seen_at = previous.map_or(now, |p| p.first_seen_at);
            let mut last_action = previous.and_then(|p| p.last_action.clone());
            let mut last_action_at = previous.and_then(|p| p.last_action_at);
            if let Some(action) = self
                .act(
                    provider,
                    id,
                    machine,
                    &verdict,
                    first_seen_at,
                    config,
                    now,
                    summary,
                )
                .await?
            {
                last_action = Some(action.into());
                last_action_at = Some(now);
            }
            self.record_alert(
                &format!("provider-machine:{}", verdict.reason),
                &format!("{}:{}", id.as_str(), machine.id),
                &format!(
                    "{} machine {} is {} ({})",
                    id.as_str(),
                    machine.id,
                    verdict.class.as_str(),
                    verdict.reason
                ),
                json!({"machineName":machine.name,"machineState":format!("{:?}",machine.state),
                    "workspaceId":verdict.workspace,"planned":verdict.planned,"mode":config.mode,
                    "firstSeenAt":first_seen_at,"lastAction":last_action}),
                now,
            )
            .await?;
            observed.push((machine, verdict, first_seen_at, last_action, last_action_at));
        }
        // Rewrite this provider's observations: only machines that still
        // disagree keep a row (and so their confirmation windows).
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM zuse_rust_provider_machine_observations WHERE provider=$1")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await?;
        let mut subjects = Vec::new();
        for (machine, verdict, first_seen_at, last_action, last_action_at) in &observed {
            subjects.push(format!("{}:{}", id.as_str(), machine.id));
            sqlx::query("INSERT INTO zuse_rust_provider_machine_observations(provider,machine_id,machine_name,verdict,reason,workspace_id,first_seen_at,last_seen_at,last_action,last_action_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
                .bind(id.as_str()).bind(&machine.id).bind(&machine.name).bind(verdict.class.as_str()).bind(verdict.reason)
                .bind(&verdict.workspace).bind(first_seen_at).bind(now).bind(last_action).bind(last_action_at).execute(&mut *tx).await?;
        }
        // Alerts for machines that no longer disagree (or are gone) resolve.
        sqlx::query("UPDATE zuse_rust_operator_alerts SET resolved_at=$3 WHERE kind LIKE 'provider-machine:%' AND resolved_at IS NULL AND subject LIKE $1 || ':%' AND NOT (subject = ANY($2))")
            .bind(id.as_str()).bind(&subjects).bind(now).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Acts on one disagreeing machine when the mode and windows allow, after
    /// re-deriving the verdict from fresh facts and a fresh inspection.
    /// Returns the action taken.
    #[allow(clippy::too_many_arguments)]
    async fn act(
        &self,
        provider: &dyn Provider,
        id: ProviderId,
        machine: &ListedMachine,
        verdict: &Verdict,
        first_seen_at: i64,
        config: &Config,
        now: i64,
        summary: &mut Summary,
    ) -> Result<Option<&'static str>, ApiError> {
        let held = now.saturating_sub(first_seen_at);
        let (allowed, window) = match verdict.planned {
            Planned::Nothing => (false, true),
            Planned::Sleep | Planned::Pause => (config.mode.acts(), held >= config.confirm_ms),
            Planned::Delete => (config.mode.deletes(), held >= config.delete_after_ms),
        };
        let action = match verdict.planned {
            Planned::Nothing => "none",
            Planned::Sleep => "sleep",
            Planned::Pause => "pause",
            Planned::Delete => "delete",
        };
        tracing::warn!(
            provider = id.as_str(),
            machine = %machine.id,
            name = %machine.name,
            state = ?machine.state,
            verdict = verdict.class.as_str(),
            reason = verdict.reason,
            workspace = verdict.workspace.as_deref().unwrap_or(""),
            action,
            dry_run = !allowed,
            held_ms = held,
            "Provider machine disagrees with the database"
        );
        if verdict.planned == Planned::Nothing {
            return Ok(None);
        }
        if !allowed || !window {
            summary.deferred += 1;
            return Ok(None);
        }
        // Fresh facts and a fresh provider view, right before acting.
        let facts = self
            .machine_facts(
                id,
                &[(machine.id.clone(), machine.name.clone())],
                config,
                now,
            )
            .await?;
        let Some(facts) = facts.first() else {
            return Ok(None);
        };
        let inspected =
            match tokio::time::timeout(ACTION_TIMEOUT, provider.inspect(&machine.id)).await {
                Ok(Ok(Some(found))) if found.id == machine.id => found,
                _ => {
                    summary.deferred += 1;
                    return Ok(None);
                }
            };
        let current = ListedMachine {
            id: machine.id.clone(),
            name: machine.name.clone(),
            state: inspected.state,
            created_at_ms: machine.created_at_ms,
        };
        let fresh = classify(&current, facts, clock().max(now), config);
        if fresh.key() != verdict.key() || fresh.planned != verdict.planned {
            tracing::warn!(provider = id.as_str(), machine = %machine.id, reason = verdict.reason,
                now_reason = fresh.reason, "Provider machine verdict changed before acting; skipped");
            summary.deferred += 1;
            return Ok(None);
        }
        let outcome = match verdict.planned {
            Planned::Sleep => tokio::time::timeout(ACTION_TIMEOUT, provider.sleep(&machine.id))
                .await
                .map(|r| r.map(|_| ())),
            Planned::Delete => tokio::time::timeout(ACTION_TIMEOUT, provider.delete(&machine.id))
                .await
                .map(|r| r.map(|_| ())),
            Planned::Pause => {
                let workspace = verdict.workspace.as_deref().unwrap_or_default();
                let admission = match self
                    .admit_reconciliation_pause(
                        workspace,
                        &machine.id,
                        now.saturating_sub(config.quiet_ms),
                        clock().max(now),
                    )
                    .await
                {
                    Ok(admission) => admission,
                    // Fenced or not owned by this backend: one computer never
                    // stops the rest of the pass.
                    Err(error) => {
                        tracing::warn!(provider = id.as_str(), machine = %machine.id, workspace,
                            code = error.code, "Lifecycle pause admission refused");
                        summary
                            .errors
                            .push(format!("{}:pause:{}", id.as_str(), error.code));
                        super::lifecycle::PauseAdmission::Unpausable
                    }
                };
                return Ok(match admission {
                    super::lifecycle::PauseAdmission::Admitted => {
                        tracing::warn!(provider = id.as_str(), machine = %machine.id, workspace,
                            reason = verdict.reason, "Provider reconciliation admitted a lifecycle pause");
                        summary.paused += 1;
                        Some("pause-admitted")
                    }
                    super::lifecycle::PauseAdmission::Busy => {
                        summary.deferred += 1;
                        None
                    }
                    super::lifecycle::PauseAdmission::Unpausable => {
                        tracing::warn!(provider = id.as_str(), machine = %machine.id, workspace,
                            "Awake computer cannot be paused by its lifecycle; operator attention needed");
                        summary.deferred += 1;
                        Some("pause-unavailable")
                    }
                });
            }
            Planned::Nothing => return Ok(None),
        };
        match outcome {
            Ok(Ok(())) => {
                tracing::warn!(provider = id.as_str(), machine = %machine.id, name = %machine.name,
                    reason = verdict.reason, action, "Provider reconciliation acted on a machine");
                if verdict.planned == Planned::Delete {
                    summary.deleted += 1;
                } else {
                    summary.slept += 1;
                }
                Ok(Some(action))
            }
            Ok(Err(error)) => {
                tracing::warn!(provider = id.as_str(), machine = %machine.id, action, ?error,
                    "Provider reconciliation action failed; retried next pass");
                summary.errors.push(format!("{}:{action}", id.as_str()));
                Ok(None)
            }
            Err(_) => {
                tracing::warn!(provider = id.as_str(), machine = %machine.id, action,
                    "Provider reconciliation action timed out; retried next pass");
                summary
                    .errors
                    .push(format!("{}:{action}-timeout", id.as_str()));
                Ok(None)
            }
        }
    }

    /// Upserts an operator-visible alert and logs it at WARN.
    pub(super) async fn record_alert(
        &self,
        kind: &str,
        subject: &str,
        summary: &str,
        details: Value,
        now: i64,
    ) -> Result<(), ApiError> {
        tracing::warn!(kind, subject, summary, "Operator alert");
        sqlx::query("INSERT INTO zuse_rust_operator_alerts(kind,subject,summary,details,first_seen_at,last_seen_at) VALUES($1,$2,$3,$4,$5,$5) ON CONFLICT (kind,subject) DO UPDATE SET summary=EXCLUDED.summary,details=EXCLUDED.details,last_seen_at=EXCLUDED.last_seen_at,occurrences=zuse_rust_operator_alerts.occurrences+1,first_seen_at=CASE WHEN zuse_rust_operator_alerts.resolved_at IS NULL THEN zuse_rust_operator_alerts.first_seen_at ELSE EXCLUDED.first_seen_at END,resolved_at=NULL")
            .bind(kind).bind(subject).bind(summary).bind(details).bind(now).execute(&self.pool).await?;
        Ok(())
    }
}

fn clock() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
