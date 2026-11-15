//! Account feed dispatches compact durable invalidations. Execution and tool output
//! stay on task channels; neither subscription nor replay calls a provider.
use super::{cursor::Cursors, store::EventStore, tickets::Tickets};
use crate::{crypto::MasterKey, error::ApiError};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::sync::{Mutex, Notify, Semaphore, watch};

pub const PROTOCOL: &str = "zuse.account-events.v1";
/// Global outstanding tickets and live sockets on this process.
pub(crate) const GLOBAL_CAPACITY: usize = 4096;
/// One account cannot consume more than this share of either global pool.
pub(crate) const PER_ACTOR_CAPACITY: usize = 16;
pub struct Hub {
    pub(crate) events: EventStore,
    pub(crate) cursors: Cursors,
    pub(crate) tickets: Tickets,
    pub(crate) capacity: Arc<Semaphore>,
    rooms: Mutex<HashMap<String, Weak<Room>>>,
    pub(crate) devices: DeviceLeases,
    dispatch: Notify,
}
pub(crate) struct Room {
    pub(crate) changed: watch::Sender<i64>,
}
impl Hub {
    pub async fn start(
        pool: sqlx::PgPool,
        key: MasterKey,
        issuer: String,
    ) -> Result<Arc<Self>, ApiError> {
        let events = EventStore::new(pool);
        events.ready().await?;
        let hub = Arc::new(Self {
            events,
            cursors: Cursors::new(key, issuer),
            tickets: Tickets::new(GLOBAL_CAPACITY, PER_ACTOR_CAPACITY),
            capacity: Arc::new(Semaphore::new(GLOBAL_CAPACITY)),
            rooms: Mutex::new(HashMap::new()),
            devices: DeviceLeases::new(PER_ACTOR_CAPACITY),
            dispatch: Notify::new(),
        });
        let signal = match std::env::var_os("ZUSE_CLOUD_EVENT_SIGNAL_DIRECTORY") {
            #[cfg(unix)]
            Some(path) => Some(
                super::hint::Listener::bind(std::path::Path::new(&path))
                    .map_err(|_| ApiError::unavailable("account_event_signal_unavailable"))?,
            ),
            _ => None,
        };
        let weak = Arc::downgrade(&hub);
        tokio::spawn(async move {
            let mut recovery = tokio::time::interval(Duration::from_secs(1));
            recovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut last_prune = std::time::Instant::now();
            loop {
                let Some(hub) = weak.upgrade() else { break };
                tokio::select! {
                    _ = hub.dispatch.notified() => {},
                    _ = recovery.tick() => {},
                    _ = async {
                        #[cfg(unix)]
                        if let Some(signal) = &signal { let _ = signal.receive().await; return; }
                        std::future::pending::<()>().await;
                    } => {},
                }
                // A dispatch failure is uncertain, so wake current subscribers to
                // replay the authoritative log even if the commit reply was lost.
                for _ in 0..16 {
                    match hub.events.dispatch().await {
                        Ok(changes) if changes.is_empty() => break,
                        Ok(changes) => {
                            let mut rooms = hub.rooms.lock().await;
                            rooms.retain(|_, room| room.strong_count() > 0);
                            for (owner, sequence) in changes {
                                if let Some(room) = rooms.get(&owner).and_then(Weak::upgrade) {
                                    room.changed.send_replace(sequence);
                                }
                            }
                        }
                        Err(error) => {
                            tracing::warn!(code = error.code, "Account event dispatch deferred");
                            for room in hub.rooms.lock().await.values().filter_map(Weak::upgrade) {
                                room.changed.send_modify(|n| *n = n.wrapping_add(1));
                            }
                            break;
                        }
                    }
                    tokio::task::yield_now().await;
                }
                if last_prune.elapsed() >= Duration::from_secs(3600) {
                    let _ = hub
                        .events
                        .prune(chrono::Utc::now().timestamp_millis() - 7 * 86400_000)
                        .await;
                    last_prune = std::time::Instant::now();
                }
                // Do not retain the authority across an idle wait after shutdown.
                if Arc::strong_count(&hub) == 1 {
                    break;
                }
            }
        });
        Ok(hub)
    }
    pub fn wake(&self) {
        self.dispatch.notify_one();
    }
    pub(crate) async fn room(&self, owner: &str) -> Arc<Room> {
        let mut rooms = self.rooms.lock().await;
        if let Some(room) = rooms.get(owner).and_then(Weak::upgrade) {
            return room;
        }
        rooms.retain(|_, room| room.strong_count() > 0);
        let room = Arc::new(Room {
            changed: watch::channel(0).0,
        });
        rooms.insert(owner.to_owned(), Arc::downgrade(&room));
        room
    }
}

type DeviceMap = HashMap<(String, uuid::Uuid), (uuid::Uuid, watch::Sender<bool>)>;
/// Live account-event sockets keyed by actor and device. A device reconnect
/// replaces its previous socket; distinct devices per actor are capped.
pub(crate) struct DeviceLeases {
    live: Arc<std::sync::Mutex<DeviceMap>>,
    per_actor: usize,
}
/// Releases its slot when dropped, including when an upgrade never completes.
pub(crate) struct DeviceLease {
    live: Arc<std::sync::Mutex<DeviceMap>>,
    key: (String, uuid::Uuid),
    id: uuid::Uuid,
    pub(crate) replaced: watch::Receiver<bool>,
}
impl DeviceLeases {
    pub(crate) fn new(per_actor: usize) -> Self {
        Self {
            live: Arc::new(std::sync::Mutex::new(HashMap::new())),
            per_actor,
        }
    }
    pub(crate) fn claim(&self, actor: &str, device: uuid::Uuid) -> Result<DeviceLease, ApiError> {
        let key = (actor.to_owned(), device);
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        let others = live
            .keys()
            .filter(|(a, d)| a == actor && *d != device)
            .count();
        if others >= self.per_actor {
            return Err(ApiError::new(
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                "account_event_actor_capacity",
            ));
        }
        let id = uuid::Uuid::new_v4();
        let (sender, replaced) = watch::channel(false);
        if let Some((_, old)) = live.insert(key.clone(), (id, sender)) {
            old.send_replace(true);
        }
        Ok(DeviceLease {
            live: self.live.clone(),
            key,
            id,
            replaced,
        })
    }
    #[cfg(test)]
    pub(crate) fn live(&self) -> usize {
        self.live.lock().unwrap().len()
    }
}
impl Drop for DeviceLease {
    fn drop(&mut self) {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if live.get(&self.key).is_some_and(|(id, _)| *id == self.id) {
            live.remove(&self.key);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_leases_cap_distinct_devices_per_actor_and_release_on_drop() {
        let leases = DeviceLeases::new(2);
        let (a, b, c) = (
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
        );
        let first = leases.claim("actor", a).unwrap();
        let second = leases.claim("actor", b).unwrap();
        let error = leases.claim("actor", c).err().unwrap();
        assert_eq!(error.status, axum::http::StatusCode::TOO_MANY_REQUESTS);
        // Another account is unaffected by this actor's usage.
        let other = leases.claim("other", c).unwrap();
        // The same device may reconnect at the cap; the previous socket is told to leave.
        let again = leases.claim("actor", a).unwrap();
        assert!(*first.replaced.borrow());
        drop(first);
        assert_eq!(
            leases.live(),
            3,
            "a replaced lease must not release its successor"
        );
        drop(second);
        let third = leases.claim("actor", c).unwrap();
        drop((again, third, other));
        assert_eq!(leases.live(), 0);
    }
}
