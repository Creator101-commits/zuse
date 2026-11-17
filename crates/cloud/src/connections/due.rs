//! When the credential worker next has something to do.
//!
//! One statement mirrors every action of [`ConnectionControl::work_once`],
//! so the worker can sleep until NOTIFY (migration `0045_auth_wakeups.sql`)
//! or the earliest deadline below instead of polling:
//! - a claimable sign-in or credential proposal: now;
//! - an open operation's expiry, or an authorizing lease running out
//!   (interrupted worker): that instant;
//! - a refresh dispatch lease running out (becomes uncertain): that instant;
//! - a finished refresh journal row reaching its retention: that instant;
//! - a checkpointed refresh result whose connection moved on, or a pending
//!   connection whose sign-in ended without an outcome: now.
//!
//! A checkpointed result whose connection still matches waits for serve to
//! apply it on next use; it never makes the worker due. Credential refresh
//! itself is lazy (serve refreshes when a credential is needed), so there is
//! no refresh schedule here.
use super::ConnectionControl;
use crate::error::ApiError;

/// How long finished refresh journal rows are kept (see `cleanup_rotations`).
pub const ROTATION_RETENTION_MS: i64 = 7 * 86_400_000;

/// Raw facts read by [`ConnectionControl::auth_due`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthDue {
    /// A sign-in or proposal could be claimed right now.
    pub claimable: bool,
    /// Earliest expiry or lease end of an open operation.
    pub operation_deadline: Option<i64>,
    /// Earliest dispatch lease end of an in-flight refresh.
    pub dispatch_deadline: Option<i64>,
    /// Oldest `updated_at` of a finished refresh journal row.
    pub oldest_finished_rotation: Option<i64>,
    /// A checkpointed refresh result no longer matches its connection.
    pub stale_result: bool,
    /// A pending connection whose last sign-in ended without success.
    pub abandoned_pending: bool,
}

impl AuthDue {
    /// The earliest instant (epoch ms) the worker should run, `now` when work
    /// is ready, `None` when nothing is known.
    pub fn next(&self, now: i64) -> Option<i64> {
        let ready = self.claimable || self.stale_result || self.abandoned_pending;
        crate::db::wake::earliest([
            ready.then_some(now),
            self.operation_deadline,
            self.dispatch_deadline,
            // Deletion is strict (`updated_at < now - retention`).
            self.oldest_finished_rotation
                .map(|at| at.saturating_add(ROTATION_RETENTION_MS).saturating_add(1)),
        ])
    }
}

/// `$1` = now (epoch ms), `$2` = account filter (`NULL` = every account).
/// Each part repeats the predicate of the `work_once` statement it stands for.
const STATEMENT: &str = "SELECT \
 EXISTS(SELECT 1 FROM cloud_auth_operations o JOIN cloud_agent_connections c ON c.connection_id=o.connection_id AND c.owner_id=o.owner_id WHERE (o.state='queued' OR (o.state='authorizing' AND o.lease_id='credential-proposal-v1')) AND o.expires_at>$1 AND c.state<>'revoked' AND o.owner_id<>'' AND ($2::text[] IS NULL OR o.owner_id=ANY($2))), \
 (SELECT min(LEAST(o.expires_at, CASE WHEN o.state='authorizing' THEN o.lease_expires_at END)) FROM cloud_auth_operations o WHERE o.state IN ('queued','authorizing') AND ($2::text[] IS NULL OR o.owner_id=ANY($2)))::bigint, \
 (SELECT min(lease_expires_at) FROM zuse_rust_credential_rotations WHERE phase='dispatching' AND ($2::text[] IS NULL OR owner_id=ANY($2)))::bigint, \
 (SELECT min(updated_at) FROM zuse_rust_credential_rotations WHERE phase IN ('applied','superseded','uncertain') AND ($2::text[] IS NULL OR owner_id=ANY($2)))::bigint, \
 EXISTS(SELECT 1 FROM zuse_rust_credential_rotations r WHERE r.phase='result' AND ($2::text[] IS NULL OR r.owner_id=ANY($2)) AND NOT EXISTS(SELECT 1 FROM cloud_agent_connections c WHERE c.owner_id=r.owner_id AND c.connection_id=r.connection_id AND c.version=r.source_version AND c.authority_epoch=r.authority_epoch AND c.state='connected')), \
 EXISTS(SELECT 1 FROM cloud_agent_connections c WHERE c.state='pending' AND ($2::text[] IS NULL OR c.owner_id=ANY($2)) AND NOT EXISTS(SELECT 1 FROM cloud_auth_operations o WHERE o.connection_id=c.connection_id AND o.state IN ('queued','authorizing')) AND EXISTS(SELECT 1 FROM cloud_auth_operations o WHERE o.connection_id=c.connection_id AND o.state IN ('error','expired','cancelled')))";

impl ConnectionControl {
    /// Whether migration 0045 installed the sign-in wakeup trigger. Without
    /// it a NOTIFY never arrives, so the worker must keep polling.
    pub async fn auth_wakeups_installed(&self) -> Result<bool, ApiError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_trigger WHERE tgrelid=to_regclass('cloud_auth_operations') AND tgname='zuse_cloud_auth_operations_wake')")
            .fetch_one(&self.pool)
            .await?)
    }
    /// One statement: what the credential worker has to do and when.
    pub async fn auth_due(&self, now: i64) -> Result<AuthDue, ApiError> {
        let row: (bool, Option<i64>, Option<i64>, Option<i64>, bool, bool) =
            sqlx::query_as(STATEMENT)
                .bind(now)
                .bind(self.accounts.filter())
                .fetch_one(&self.pool)
                .await?;
        Ok(AuthDue {
            claimable: row.0,
            operation_deadline: row.1,
            dispatch_deadline: row.2,
            oldest_finished_rotation: row.3,
            stale_result: row.4,
            abandoned_pending: row.5,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_known_is_never_due() {
        assert_eq!(AuthDue::default().next(1_000), None);
    }

    #[test]
    fn ready_work_is_due_now() {
        for due in [
            AuthDue {
                claimable: true,
                ..AuthDue::default()
            },
            AuthDue {
                stale_result: true,
                operation_deadline: Some(9_000),
                ..AuthDue::default()
            },
            AuthDue {
                abandoned_pending: true,
                ..AuthDue::default()
            },
        ] {
            assert_eq!(due.next(1_000), Some(1_000));
        }
    }

    #[test]
    fn deadlines_pick_the_earliest_instant() {
        let due = AuthDue {
            operation_deadline: Some(50_000),
            dispatch_deadline: Some(20_000),
            oldest_finished_rotation: Some(0),
            ..AuthDue::default()
        };
        assert_eq!(due.next(1_000), Some(20_000));
        // A deadline already behind us is due (the caller treats it as now).
        let past = AuthDue {
            operation_deadline: Some(500),
            ..AuthDue::default()
        };
        assert_eq!(past.next(1_000), Some(500));
        // Claimable work wins over a later deadline but not an earlier one.
        let both = AuthDue {
            claimable: true,
            ..past
        };
        assert_eq!(both.next(1_000), Some(500));
    }

    #[test]
    fn finished_rotations_are_due_just_after_retention() {
        let due = AuthDue {
            oldest_finished_rotation: Some(10),
            ..AuthDue::default()
        };
        assert_eq!(due.next(0), Some(10 + ROTATION_RETENTION_MS + 1));
        let edge = AuthDue {
            oldest_finished_rotation: Some(i64::MAX - 1),
            ..AuthDue::default()
        };
        assert_eq!(edge.next(0), Some(i64::MAX));
    }
}
