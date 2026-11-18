//! When the mailbox asks the server for commands.
//!
//! The lease is the source of truth and a gateway hint is only a reason to ask
//! now. With a gateway that pushes hints, an idle runtime sleeps until a hint
//! or a long safety timer. Without one (older servers, a dropped socket, an
//! organization computer), it long-polls exactly as before, only longer.
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use zuse_rpc::cloud::CommandPush;

/// The server's longest lease wait. Every server accepts it, and it answers
/// at once when a command lands, so a longer wait costs no latency.
pub(super) const LONG_POLL_MS: u32 = zuse_protocol::cloud_commands::WATCH_MAX_WAIT_MS;
/// With push, an idle runtime still asks this often in case a hint was lost.
pub(super) const SAFETY_TIMER: Duration = Duration::from_secs(60);
const BACKOFF_START: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(10);

/// What to do before the next lease request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Plan {
    /// Wait this long first; a hint (or losing push) ends the wait early.
    pub sleep: Option<Duration>,
    /// The `waitMs` sent with the lease.
    pub wait_ms: u32,
}

pub(super) fn plan(push: bool, drain: bool, failures: u32) -> Plan {
    let wait_ms = if push { 0 } else { LONG_POLL_MS };
    let sleep = if failures > 0 {
        Some(
            BACKOFF_START
                .saturating_mul(1 << (failures - 1).min(8))
                .min(BACKOFF_MAX),
        )
    } else if push && !drain {
        Some(SAFETY_TIMER)
    } else {
        None
    };
    Plan { sleep, wait_ms }
}

pub(super) struct Pacer {
    push: Arc<CommandPush>,
    active: watch::Receiver<bool>,
    /// Lease again at once: startup, a hint, or the last page had commands.
    drain: bool,
    failures: u32,
}
impl Pacer {
    pub fn new(push: Arc<CommandPush>) -> Self {
        let active = push.active();
        Self {
            push,
            active,
            drain: true,
            failures: 0,
        }
    }
    pub fn plan(&self) -> Plan {
        plan(*self.active.borrow(), self.drain, self.failures)
    }
    /// A lease page arrived. Keep leasing until one comes back empty.
    pub fn leased(&mut self, commands: usize) {
        self.failures = 0;
        self.drain = commands > 0;
    }
    pub fn failed(&mut self) {
        self.failures = self.failures.saturating_add(1);
    }
    /// Something other than a hint means there may be more to lease, such
    /// as a finished command freeing a slot or its session's next command.
    pub fn wake(&mut self) {
        self.drain = true;
    }
    /// Sleeps as planned. Returns the `waitMs` for the lease that follows,
    /// recomputed because push may have changed while sleeping.
    pub async fn sleep(&mut self, cancel: &CancellationToken, duration: Duration) -> u32 {
        let was_push = *self.active.borrow_and_update();
        let active = &mut self.active;
        let hinted = tokio::select! {
            _ = cancel.cancelled() => false,
            _ = self.push.hinted() => true,
            // Losing the socket ends a push-mode sleep: long-poll instead.
            _ = async {
                while active.changed().await.is_ok() {
                    if *active.borrow_and_update() != was_push {
                        return;
                    }
                }
                std::future::pending::<()>().await
            } => false,
            _ = tokio::time::sleep(duration) => false,
        };
        if hinted {
            self.drain = true;
        }
        self.plan().wait_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_follow_push_drain_and_failures() {
        // No push: back-to-back long-polls at the server's maximum wait.
        assert_eq!(
            plan(false, false, 0),
            Plan {
                sleep: None,
                wait_ms: 25_000
            }
        );
        assert_eq!(plan(false, true, 0).sleep, None);
        // Push: an idle runtime sleeps for a hint, then asks without waiting.
        assert_eq!(
            plan(true, false, 0),
            Plan {
                sleep: Some(SAFETY_TIMER),
                wait_ms: 0
            }
        );
        // Push while draining: ask again at once.
        assert_eq!(
            plan(true, true, 0),
            Plan {
                sleep: None,
                wait_ms: 0
            }
        );
        // Failures back off from 1s to at most 10s in either mode.
        assert_eq!(plan(false, true, 1).sleep, Some(Duration::from_secs(1)));
        assert_eq!(plan(true, false, 2).sleep, Some(Duration::from_secs(2)));
        assert_eq!(plan(false, false, 4).sleep, Some(Duration::from_secs(8)));
        assert_eq!(plan(false, false, 5).sleep, Some(BACKOFF_MAX));
        assert_eq!(plan(true, false, u32::MAX).sleep, Some(BACKOFF_MAX));
    }

    #[tokio::test]
    async fn startup_and_full_pages_lease_at_once_until_the_mailbox_is_empty() {
        let push = Arc::new(CommandPush::default());
        push.received();
        let mut pacer = Pacer::new(push.clone());
        assert_eq!(pacer.plan().sleep, None, "startup leases at once");
        pacer.leased(2);
        assert_eq!(pacer.plan().sleep, None, "more may be waiting");
        pacer.leased(1);
        assert_eq!(pacer.plan().sleep, None);
        pacer.leased(0);
        assert_eq!(pacer.plan().sleep, Some(SAFETY_TIMER), "empty: idle");
        pacer.wake();
        assert_eq!(pacer.plan().sleep, None, "a finished command re-leases");
    }

    #[tokio::test(start_paused = true)]
    async fn a_hint_ends_the_idle_sleep_at_once() {
        let push = Arc::new(CommandPush::default());
        push.received();
        let mut pacer = Pacer::new(push.clone());
        pacer.leased(0);
        // The connect/received hint is already pending; consume it first.
        pacer.sleep(&CancellationToken::new(), SAFETY_TIMER).await;
        pacer.leased(0);
        let started = tokio::time::Instant::now();
        let hinter = tokio::spawn({
            let push = push.clone();
            async move {
                tokio::time::sleep(Duration::from_secs(3)).await;
                push.hint();
            }
        });
        let wait = pacer.sleep(&CancellationToken::new(), SAFETY_TIMER).await;
        assert_eq!(started.elapsed(), Duration::from_secs(3));
        assert_eq!(wait, 0);
        assert_eq!(pacer.plan().sleep, None, "the hint drains the mailbox");
        hinter.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_hint_during_a_lease_is_not_lost() {
        let push = Arc::new(CommandPush::default());
        push.received();
        let mut pacer = Pacer::new(push.clone());
        pacer.sleep(&CancellationToken::new(), SAFETY_TIMER).await;
        // A lease is in flight when the hint arrives; it then returns empty.
        push.hint();
        pacer.leased(0);
        let started = tokio::time::Instant::now();
        pacer.sleep(&CancellationToken::new(), SAFETY_TIMER).await;
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn without_hints_the_safety_timer_still_leases() {
        let push = Arc::new(CommandPush::default());
        push.received();
        let mut pacer = Pacer::new(push.clone());
        pacer.sleep(&CancellationToken::new(), SAFETY_TIMER).await;
        pacer.leased(0);
        let started = tokio::time::Instant::now();
        let wait = pacer.sleep(&CancellationToken::new(), SAFETY_TIMER).await;
        assert_eq!(started.elapsed(), SAFETY_TIMER);
        assert_eq!(wait, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn losing_the_gateway_falls_back_to_long_polling_at_once() {
        let push = Arc::new(CommandPush::default());
        push.received();
        let mut pacer = Pacer::new(push.clone());
        pacer.sleep(&CancellationToken::new(), SAFETY_TIMER).await;
        pacer.leased(0);
        let started = tokio::time::Instant::now();
        let dropper = tokio::spawn({
            let push = push.clone();
            async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                push.disconnected();
            }
        });
        let wait = pacer.sleep(&CancellationToken::new(), SAFETY_TIMER).await;
        assert_eq!(started.elapsed(), Duration::from_secs(5));
        assert_eq!(wait, LONG_POLL_MS, "no socket: long-poll");
        assert_eq!(pacer.plan().sleep, None, "and never sleep between polls");
        dropper.await.unwrap();
        // A reconnect wakes at once but stays in long-poll until a hint.
        push.connected();
        pacer.sleep(&CancellationToken::new(), SAFETY_TIMER).await;
        assert_eq!(pacer.plan().wait_ms, LONG_POLL_MS);
    }

    #[tokio::test(start_paused = true)]
    async fn failures_back_off_but_a_hint_retries_early() {
        let push = Arc::new(CommandPush::default());
        let mut pacer = Pacer::new(push.clone());
        pacer.failed();
        pacer.failed();
        pacer.failed();
        let plan = pacer.plan();
        assert_eq!(plan.sleep, Some(Duration::from_secs(4)));
        let started = tokio::time::Instant::now();
        pacer
            .sleep(&CancellationToken::new(), plan.sleep.unwrap())
            .await;
        assert_eq!(started.elapsed(), Duration::from_secs(4));
        push.hint();
        let started = tokio::time::Instant::now();
        pacer
            .sleep(&CancellationToken::new(), plan.sleep.unwrap())
            .await;
        assert_eq!(started.elapsed(), Duration::ZERO);
        pacer.leased(0);
        assert_eq!(pacer.plan().sleep, None, "success resets the backoff");
    }
}
