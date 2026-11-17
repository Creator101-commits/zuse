//! Embedded billing follow-up inside the API process.
//!
//! A verified payment notification is projected right after it commits
//! instead of waiting for the billing worker, and engine work or expiries the
//! API caused (a retried provider call, an automatic top-up, a model
//! reservation) run at their due time. At most one task runs per account; it
//! ends once nothing is due within the hour. The billing worker still runs the
//! same maintenance hourly and at the due times it knows, so a restarted API
//! process loses latency only, never work.
use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Notify;

#[derive(Default)]
pub struct FollowUps {
    owners: Mutex<std::collections::HashMap<String, Arc<Entry>>>,
}

#[derive(Default)]
struct Entry {
    /// Wakes a sleeping task early.
    wake: Notify,
    /// Run maintenance on the next turn.
    run: AtomicBool,
    /// Something changed since the task last read its next due time.
    dirty: AtomicBool,
}

/// Runs closer together than this are coalesced, so work that stays due
/// cannot spin.
const MIN_GAP_MS: i64 = 1_000;

/// What a follow-up task does after reading the account's next due time.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Step {
    Run,
    Sleep(std::time::Duration),
    Exit,
}

/// `last_run` is when maintenance last started in this task (if ever).
pub(super) fn step(now: i64, due: Option<i64>, last_run: Option<i64>) -> Step {
    let Some(due) = due else {
        return Step::Exit;
    };
    if due <= now {
        return match last_run {
            Some(last) if now - last < MIN_GAP_MS => Step::Sleep(std::time::Duration::from_millis(
                (MIN_GAP_MS - (now - last)) as u64,
            )),
            _ => Step::Run,
        };
    }
    if due - now > crate::billing::schedule::HOUR_MS {
        // The billing worker's hourly pass covers anything later.
        return Step::Exit;
    }
    Step::Sleep(std::time::Duration::from_millis((due - now) as u64 + 5))
}

impl BillingControl {
    /// Enables in-process follow-up for embedded billing accounts (API only).
    pub fn with_native_follow_ups(mut self) -> Self {
        if self.native.is_some() {
            self.follow_ups = Some(Arc::default());
        }
        self
    }
    /// `run_now` projects immediately (a payment notification committed);
    /// otherwise the task only re-reads the account's next due time.
    pub(in crate::billing) fn follow_up(&self, owner: &str, run_now: bool) {
        let (Some(follow_ups), Some(native)) = (&self.follow_ups, &self.native) else {
            return;
        };
        if !native.accounts.contains(owner) {
            return;
        }
        let mut owners = follow_ups.owners.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = owners.get(owner) {
            entry.dirty.store(true, Ordering::SeqCst);
            if run_now {
                entry.run.store(true, Ordering::SeqCst);
            }
            entry.wake.notify_one();
            return;
        }
        let entry = Arc::new(Entry::default());
        entry.run.store(run_now, Ordering::SeqCst);
        owners.insert(owner.to_owned(), entry.clone());
        drop(owners);
        let control = self.clone();
        let owner = owner.to_owned();
        tokio::spawn(async move { control.follow_up_task(owner, entry).await });
    }
    async fn follow_up_task(self, owner: String, entry: Arc<Entry>) {
        let mut last_run: Option<i64> = None;
        loop {
            entry.dirty.store(false, Ordering::SeqCst);
            if entry.run.swap(false, Ordering::SeqCst) {
                let started = chrono::Utc::now().timestamp_millis();
                last_run = Some(started);
                match self.maintain_native_account(&owner).await {
                    Ok(Some(report)) if report["more"] == true => {
                        entry.run.store(true, Ordering::SeqCst);
                        continue;
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!(
                        code = error.code,
                        "Embedded billing follow-up deferred to the billing worker"
                    ),
                }
            }
            // Anything due at or before the last run was already attempted.
            let floor = last_run.unwrap_or(i64::MIN);
            let due = match self
                .native_due_for(std::slice::from_ref(&owner), floor)
                .await
            {
                Ok(due) => due,
                Err(error) => {
                    tracing::warn!(code = error.code, "Embedded billing due time unavailable");
                    None
                }
            };
            match step(chrono::Utc::now().timestamp_millis(), due, last_run) {
                Step::Run => entry.run.store(true, Ordering::SeqCst),
                Step::Sleep(wait) => {
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {}
                        _ = entry.wake.notified() => {}
                    }
                }
                Step::Exit => {
                    let Some(follow_ups) = &self.follow_ups else {
                        return;
                    };
                    let mut owners = follow_ups.owners.lock().unwrap_or_else(|e| e.into_inner());
                    if entry.dirty.load(Ordering::SeqCst) || entry.run.load(Ordering::SeqCst) {
                        continue;
                    }
                    owners.remove(&owner);
                    return;
                }
            }
        }
    }
    /// Test hook: whether a follow-up task is active for `owner`.
    #[cfg(test)]
    pub(in crate::billing) fn follow_up_active(&self, owner: &str) -> bool {
        self.follow_ups.as_ref().is_some_and(|f| {
            f.owners
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(owner)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn follow_up_steps_run_due_work_sleep_until_due_and_exit_beyond_the_hour() {
        assert_eq!(step(1_000, None, None), Step::Exit);
        assert_eq!(step(1_000, Some(1_000), None), Step::Run);
        assert_eq!(step(1_000, Some(10), Some(-5_000)), Step::Run);
        // Still due right after a run: wait out the minimum gap, never spin.
        assert_eq!(
            step(1_000, Some(900), Some(800)),
            Step::Sleep(Duration::from_millis(800))
        );
        assert_eq!(
            step(1_000, Some(4_000), Some(900)),
            Step::Sleep(Duration::from_millis(3_005))
        );
        let hour = crate::billing::schedule::HOUR_MS;
        assert_eq!(
            step(0, Some(hour), None),
            Step::Sleep(Duration::from_millis(hour as u64 + 5))
        );
        assert_eq!(step(0, Some(hour + 1), None), Step::Exit);
        assert_eq!(step(0, Some(i64::MAX), None), Step::Exit);
    }
}
