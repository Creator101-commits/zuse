//! When the billing worker wakes. Pure arithmetic, so the schedule is testable
//! without a clock or a database.
//!
//! The worker runs settlement (snapshot storage and provider usage) and meter
//! export on an hourly tick, plus at the instants that make new work
//! settleable or due:
//! - the moment the next whole UTC day becomes settleable (see `windows`),
//! - a short retry while the newest closed day is still incomplete,
//! - the earliest known due time of outbox rows, engine work, reservation
//!   expiries and credit grant expiries (queried by the caller).
//!
//! Between those instants it runs no statements at all.
use std::time::Duration;

pub const HOUR_MS: i64 = 3_600_000;
pub(super) const DAY_MS: i64 = 86_400_000;
/// A day is settled once providers had this long to report its last minutes.
pub(super) const SETTLE_DELAY_MS: i64 = 30 * 60_000;
/// Meter reconciliation compares provider totals five minutes after delivery.
pub(super) const RECONCILE_DELAY_MS: i64 = 300_000;
/// Shortest sleep when work stays due after a full pass (the old fixed cadence),
/// so unclaimable or failing work never spins.
pub const BUSY_FLOOR: Duration = Duration::from_secs(60);

/// The first instant after `now` at which `windows` includes one more closed day.
pub fn next_day_settleable(now: i64) -> i64 {
    let shifted = now.saturating_sub(SETTLE_DELAY_MS);
    (shifted.div_euclid(DAY_MS) + 1)
        .saturating_mul(DAY_MS)
        .saturating_add(SETTLE_DELAY_MS)
}

/// Retry delay for the newest closed day while its provider report is
/// unavailable or incomplete: 1, 2, 4, ... minutes, never more than an hour.
pub fn settlement_retry_ms(attempt: u32) -> i64 {
    60_000_i64
        .saturating_mul(1_i64 << attempt.min(16))
        .min(HOUR_MS)
}

/// When settlement should run next, after a pass that finished at `now`.
/// `retry` is the number of consecutive passes that left the newest closed day
/// incomplete (`None` when it was clean).
pub fn next_settlement(now: i64, retry: Option<u32>) -> i64 {
    let mut next = now.saturating_add(HOUR_MS).min(next_day_settleable(now));
    if let Some(attempt) = retry {
        next = next.min(now.saturating_add(settlement_retry_ms(attempt)));
    }
    next
}

/// When meter reconciliation can next compare totals, if this pass delivered
/// meter events.
pub fn next_reconciliation(now: i64, delivered: usize) -> Option<i64> {
    (delivered > 0).then(|| now.saturating_add(RECONCILE_DELAY_MS + 1_000))
}

/// How long to sleep until `next_due`, bounded by the hourly tick. Work that is
/// still due right after a pass waits [`BUSY_FLOOR`].
pub fn wait_until(now: i64, next_due: Option<i64>) -> Duration {
    let cadence = crate::db::wake::Cadence {
        idle: Duration::from_millis(HOUR_MS as u64),
        busy: BUSY_FLOOR,
    };
    crate::db::wake::next_wait(now, next_due, cadence).0
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_START: i64 = 1_700_006_400_000; // a UTC midnight

    #[test]
    fn next_day_becomes_settleable_thirty_minutes_after_midnight() {
        assert_eq!(DAY_START % DAY_MS, 0);
        let close = DAY_START + SETTLE_DELAY_MS;
        assert_eq!(next_day_settleable(DAY_START), close);
        assert_eq!(next_day_settleable(close - 1), close);
        // At the instant itself the day is already included; the next one follows.
        assert_eq!(next_day_settleable(close), close + DAY_MS);
        assert_eq!(
            next_day_settleable(DAY_START + 12 * HOUR_MS),
            close + DAY_MS
        );
        // Consistent with `windows`: one instant before, the day is excluded.
        let before = super::super::settlement::windows(DAY_START - 3 * DAY_MS, close - 1, 0)
            .unwrap()
            .len();
        let after = super::super::settlement::windows(DAY_START - 3 * DAY_MS, close, 0)
            .unwrap()
            .len();
        assert_eq!(after, before + 1);
    }

    #[test]
    fn settlement_runs_hourly_at_day_close_and_retries_incomplete_days() {
        let now = DAY_START + 2 * HOUR_MS;
        assert_eq!(next_settlement(now, None), now + HOUR_MS);
        // Shortly before a day closes, run at the close rather than up to an hour later.
        let late = DAY_START + DAY_MS - 10 * 60_000;
        assert_eq!(
            next_settlement(late, None),
            DAY_START + DAY_MS + SETTLE_DELAY_MS
        );
        assert_eq!(next_settlement(now, Some(0)), now + 60_000);
        assert_eq!(next_settlement(now, Some(3)), now + 8 * 60_000);
        assert_eq!(next_settlement(now, Some(10)), now + HOUR_MS);
        assert_eq!(next_settlement(now, Some(u32::MAX)), now + HOUR_MS);
        assert_eq!(settlement_retry_ms(0), 60_000);
        assert_eq!(settlement_retry_ms(5), 32 * 60_000);
        assert_eq!(settlement_retry_ms(6), HOUR_MS);
        assert_eq!(next_settlement(i64::MAX, Some(0)), i64::MAX);
    }

    #[test]
    fn reconciliation_follows_delivery_and_waits_are_bounded() {
        assert_eq!(next_reconciliation(1_000, 0), None);
        assert_eq!(next_reconciliation(1_000, 2), Some(302_000));
        assert_eq!(wait_until(0, None), Duration::from_millis(HOUR_MS as u64));
        assert_eq!(
            wait_until(0, Some(10 * HOUR_MS)),
            Duration::from_millis(HOUR_MS as u64)
        );
        assert_eq!(wait_until(0, Some(1_000)), Duration::from_millis(1_005));
        assert_eq!(wait_until(5_000, Some(5_000)), BUSY_FLOOR);
        assert_eq!(wait_until(5_000, Some(-1)), BUSY_FLOOR);
    }
}
