//! One statement tells the worker which stages have due work and when the
//! next known work becomes due, so an idle worker neither runs every stage
//! query each tick nor polls at a fixed one-second cadence.
//!
//! Every time here is conservative: it may be earlier than the stage's real
//! claim predicate (the stage then finds nothing and the worker backs off to
//! its busy floor), but it is never later. Work whose readiness depends on
//! time-based rules that are not expressed here (retirement sweeps, blocked
//! requests, recovery admission) runs on the worker's idle timer instead.
use super::{RuntimeControl, allocation, deletion, lifecycle, recovery, repair, restore};
use crate::error::ApiError;

/// Earliest due time (epoch ms) per worker stage; `None` means nothing pending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkerDue {
    pub allocation: Option<i64>,
    pub lifecycle: Option<i64>,
    pub repair: Option<i64>,
    pub deletion: Option<i64>,
    pub recovery: Option<i64>,
    pub restore: Option<i64>,
    pub task_drain: Option<i64>,
    pub recreation: Option<i64>,
    pub user_images: Option<i64>,
    pub project_cache: Option<i64>,
}

impl WorkerDue {
    /// The earliest known due time across every stage.
    pub fn earliest(&self) -> Option<i64> {
        crate::db::wake::earliest([
            self.allocation,
            self.lifecycle,
            self.repair,
            self.deletion,
            self.recovery,
            self.restore,
            self.task_drain,
            self.recreation,
            self.user_images,
            self.project_cache,
        ])
    }
    fn set(&mut self, source: &str, at: Option<i64>) {
        let slot = match source {
            allocation::JOB_KIND => &mut self.allocation,
            lifecycle::JOB_KIND => &mut self.lifecycle,
            repair::JOB_KIND => &mut self.repair,
            deletion::JOB_KIND => &mut self.deletion,
            recovery::JOB_KIND => &mut self.recovery,
            restore::JOB_KIND => &mut self.restore,
            "task_drain" => &mut self.task_drain,
            "recreation" => &mut self.recreation,
            "user_images" => &mut self.user_images,
            "project_cache" => &mut self.project_cache,
            _ => return,
        };
        *slot = crate::db::wake::earliest([*slot, at]);
    }
}

/// Whether a stage should run this round: its work is due, or the due query
/// failed (then every stage runs, as before this schedule existed).
pub fn should_run(
    due: Option<&WorkerDue>,
    at: impl Fn(&WorkerDue) -> Option<i64>,
    now: i64,
) -> bool {
    match due {
        None => true,
        Some(due) => at(due).is_some_and(|t| t <= now),
    }
}

/// Jobs: claimable when due and unleased (or the lease expired) at the
/// owner's exact generation; exhausted running jobs become due when their
/// lease expires (the claim marks them failed without an owner check).
const JOBS: &str = "SELECT j.kind AS source, min(CASE WHEN j.attempts >= j.max_attempts THEN j.lease_expires_at ELSE GREATEST(j.due_at, COALESCE(j.lease_expires_at, j.due_at)) END)::bigint AS due FROM zuse_rust_jobs j WHERE j.kind = ANY($1) AND j.state IN ('pending','running') AND ((j.state = 'running' AND j.attempts >= j.max_attempts) OR (j.attempts < j.max_attempts AND EXISTS (SELECT 1 FROM zuse_rust_control_owners o WHERE o.resource_kind = 'workspace' AND o.resource_id = j.resource_id AND o.backend = 'rust' AND o.generation = j.resource_generation))) GROUP BY j.kind";
const TASK_DRAINS: &str =
    "SELECT 'task_drain', min(next_attempt_at)::bigint FROM zuse_rust_task_command_drains";
const RECREATIONS: &str = "SELECT 'recreation', min(due_at)::bigint FROM zuse_rust_computer_recreations WHERE phase NOT IN ('ready','cancelled','attention-required')";
const USER_IMAGES: &str = "SELECT 'user_images', min(GREATEST(next_attempt_at, COALESCE(lease_expires_at, next_attempt_at)))::bigint FROM cloud_machine_images WHERE state IN ('queued','building','cleaning','retiring')";
/// Optional migration 0028. Blocked requests wait for the idle timer.
const PROJECT_CACHE: &str = "SELECT 'project_cache', LEAST((SELECT min(GREATEST(next_attempt_at, COALESCE(lease_expires_at, next_attempt_at))) FROM zuse_rust_project_preparation_requests WHERE state IN ('queued','resolving')), (SELECT min(GREATEST(next_attempt_at, COALESCE(lease_expires_at, next_attempt_at))) FROM zuse_rust_project_cache_operations WHERE state IN ('queued','running','uncertain','cleaning')))::bigint";

const WORKER_JOB_KINDS: [&str; 6] = [
    allocation::JOB_KIND,
    lifecycle::JOB_KIND,
    repair::JOB_KIND,
    deletion::JOB_KIND,
    recovery::JOB_KIND,
    restore::JOB_KIND,
];

fn statement(project_cache: bool) -> String {
    let mut parts = vec![JOBS, TASK_DRAINS, RECREATIONS, USER_IMAGES];
    if project_cache {
        parts.push(PROJECT_CACHE);
    }
    parts.join(" UNION ALL ")
}

impl RuntimeControl {
    /// One round trip: the earliest due time of every worker stage.
    pub async fn worker_due(&self) -> Result<WorkerDue, ApiError> {
        let project_cache = self.project_cache_control.available().await?;
        static WITH_CACHE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        static WITHOUT_CACHE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let sql = if project_cache {
            WITH_CACHE.get_or_init(|| statement(true))
        } else {
            WITHOUT_CACHE.get_or_init(|| statement(false))
        };
        let rows: Vec<(String, Option<i64>)> = sqlx::query_as(sql)
            .bind(&WORKER_JOB_KINDS[..])
            .fetch_all(&self.pool)
            .await?;
        let mut due = WorkerDue::default();
        for (source, at) in rows {
            due.set(&source, at);
        }
        Ok(due)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earliest_and_should_run_follow_due_times() {
        let mut due = WorkerDue::default();
        assert_eq!(due.earliest(), None);
        due.set(lifecycle::JOB_KIND, Some(50));
        due.set(lifecycle::JOB_KIND, Some(70));
        due.set("task_drain", Some(20));
        due.set("unknown", Some(1));
        assert_eq!(due.lifecycle, Some(50));
        assert_eq!(due.earliest(), Some(20));
        assert!(should_run(Some(&due), |d| d.task_drain, 20));
        assert!(!should_run(Some(&due), |d| d.lifecycle, 49));
        assert!(!should_run(Some(&due), |d| d.deletion, i64::MAX));
        // Unknown state (the due query failed): run every stage.
        assert!(should_run(None, |d| d.deletion, 0));
    }

    #[test]
    fn statement_includes_optional_sources_only_when_present() {
        assert!(!statement(false).contains("zuse_rust_project_cache_operations"));
        assert!(statement(true).contains("zuse_rust_project_cache_operations"));
        assert_eq!(statement(false).matches(" UNION ALL ").count(), 3);
    }
}

#[cfg(test)]
#[path = "due_tests.rs"]
mod pg_tests;
