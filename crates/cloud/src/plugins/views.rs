//! Per-tenant plugin state kept in memory by `serve`, and the per-tenant fence.
//!
//! A tool call needs the tenant's connection rows and custom server
//! definitions. [`Views`] keeps them per tenant so a call does not re-read
//! them. It is only used while database notifications can invalidate it
//! (migration `0046_plugin_wakeups.sql`): every write to those rows sends
//! `NOTIFY plugins`, every process drops its views, and the writing process
//! also drops its own synchronously. Entries also expire after
//! [`VIEW_TTL`], which bounds staleness if a notification is ever lost
//! without the listener noticing.
use super::{ConnectionRow, CustomServer};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Longest time a view is served without being re-read.
pub(super) const VIEW_TTL: Duration = Duration::from_secs(60);
const MAX_TENANTS: usize = 4096;

/// One tenant's plugin rows, as read from the database.
#[derive(Clone)]
pub(super) struct View {
    /// Every connection row of the tenant, ordered by creation.
    pub rows: Arc<Vec<ConnectionRow>>,
    /// Decrypted custom server definitions by server id, ordered by creation.
    pub servers: Arc<Vec<(String, CustomServer)>>,
}

struct Entries {
    /// Bumped by every invalidation; a read that raced one is not kept.
    generation: u64,
    views: HashMap<String, (Instant, View)>,
}

pub(super) struct Views {
    enabled: AtomicBool,
    ttl: Duration,
    entries: Mutex<Entries>,
}

impl Views {
    /// Disabled until [`Self::enable`]: every lookup misses.
    pub fn new(ttl: Duration) -> Self {
        Self {
            enabled: AtomicBool::new(false),
            ttl,
            entries: Mutex::new(Entries {
                generation: 0,
                views: HashMap::new(),
            }),
        }
    }
    pub fn enable(&self) {
        self.clear();
        self.enabled.store(true, Ordering::SeqCst);
    }
    /// Stop serving views (notifications can no longer reach this process).
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::SeqCst);
        self.clear();
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Entries> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn get(&self, tenant: &str) -> Option<View> {
        if !self.enabled() {
            return None;
        }
        let entries = self.lock();
        entries
            .views
            .get(tenant)
            .filter(|(at, _)| at.elapsed() < self.ttl)
            .map(|(_, view)| view.clone())
    }
    /// Call before reading the rows a view is built from.
    pub fn generation(&self) -> u64 {
        self.lock().generation
    }
    /// Keeps a view read after [`Self::generation`] returned `generation`,
    /// unless an invalidation happened since.
    pub fn store(&self, tenant: &str, generation: u64, view: View) {
        if !self.enabled() {
            return;
        }
        let mut entries = self.lock();
        if entries.generation != generation {
            return;
        }
        if entries.views.len() >= MAX_TENANTS && !entries.views.contains_key(tenant) {
            let ttl = self.ttl;
            entries.views.retain(|_, (at, _)| at.elapsed() < ttl);
            if entries.views.len() >= MAX_TENANTS {
                entries.views.clear();
            }
        }
        entries
            .views
            .insert(tenant.to_owned(), (Instant::now(), view));
    }
    /// Drops the tenant's view when the returned guard drops. Hold it inside
    /// the tenant fence around writes, so this process's next operation for
    /// the tenant re-reads the rows whether the writes succeeded or not.
    pub fn changes(&self, tenant: &str) -> Changes<'_> {
        Changes {
            views: self,
            tenant: tenant.to_owned(),
        }
    }
    /// This process changed the tenant's rows.
    pub fn invalidate(&self, tenant: &str) {
        let mut entries = self.lock();
        entries.generation = entries.generation.wrapping_add(1);
        entries.views.remove(tenant);
    }
    /// Some tenant's rows changed, or notifications may have been missed.
    pub fn clear(&self) {
        let mut entries = self.lock();
        entries.generation = entries.generation.wrapping_add(1);
        entries.views.clear();
    }
}

pub(super) struct Changes<'a> {
    views: &'a Views,
    tenant: String,
}
impl Drop for Changes<'_> {
    fn drop(&mut self) {
        self.views.invalidate(&self.tenant);
    }
}

/// Serializes one tenant's plugin operations within this process. Management
/// operations also take a database advisory lock across processes.
#[derive(Default)]
pub(super) struct Fences {
    locks: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
}

impl Fences {
    pub async fn lock(&self, tenant: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let fence = {
            let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());
            match locks.get(tenant).and_then(Weak::upgrade) {
                Some(fence) => fence,
                None => {
                    if locks.len() >= 1024 {
                        locks.retain(|_, fence| fence.strong_count() > 0);
                    }
                    let fence = Arc::new(tokio::sync::Mutex::new(()));
                    locks.insert(tenant.to_owned(), Arc::downgrade(&fence));
                    fence
                }
            }
        };
        fence.lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(tenant: &str, id: &str, subject: &str) -> ConnectionRow {
        ConnectionRow {
            id: id.into(),
            plugin_id: format!("custom-{tenant}"),
            label: id.into(),
            owner: "user".into(),
            subject: subject.into(),
            state: "connected".into(),
            enabled: true,
            auth: Some("header".into()),
            executor_ref: None,
            definition_hash: None,
            created_at: 0,
        }
    }
    fn view(tenant: &str, ids: &[&str]) -> View {
        View {
            rows: Arc::new(ids.iter().map(|id| row(tenant, id, "s")).collect()),
            servers: Arc::new(vec![(
                format!("custom-{tenant}"),
                CustomServer {
                    name: tenant.into(),
                    endpoint: format!("https://{tenant}.example/mcp"),
                    header: None,
                },
            )]),
        }
    }
    fn ids(view: &View) -> Vec<String> {
        view.rows.iter().map(|row| row.id.clone()).collect()
    }

    #[test]
    fn views_are_disabled_until_notifications_can_reach_them() {
        let views = Views::new(VIEW_TTL);
        let generation = views.generation();
        views.store("personal:a", generation, view("a", &["c1"]));
        assert!(views.get("personal:a").is_none());
        views.enable();
        let generation = views.generation();
        views.store("personal:a", generation, view("a", &["c1"]));
        assert_eq!(ids(&views.get("personal:a").unwrap()), ["c1"]);
        views.disable();
        assert!(views.get("personal:a").is_none());
        views.enable();
        assert!(
            views.get("personal:a").is_none(),
            "re-enabling starts empty"
        );
    }

    #[test]
    fn tenants_never_see_each_others_views() {
        let views = Views::new(VIEW_TTL);
        views.enable();
        let generation = views.generation();
        views.store("personal:a", generation, view("a", &["c1"]));
        views.store("organization:b", generation, view("b", &["c2", "c3"]));
        let a = views.get("personal:a").unwrap();
        assert_eq!(ids(&a), ["c1"]);
        let servers: Vec<&str> = a.servers.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(servers, ["custom-a"]);
        assert_eq!(ids(&views.get("organization:b").unwrap()), ["c2", "c3"]);
        assert!(views.get("personal:b").is_none());
        // A local write drops only the writer's tenant.
        views.invalidate("personal:a");
        assert!(views.get("personal:a").is_none());
        assert!(views.get("organization:b").is_some());
    }

    #[test]
    fn a_notification_drops_every_view_and_stale_reads_are_not_kept() {
        let views = Views::new(VIEW_TTL);
        views.enable();
        let generation = views.generation();
        views.store("personal:a", generation, view("a", &["c1"]));
        views.store("personal:b", generation, view("b", &["c2"]));
        // A read that started before a change must not repopulate the cache.
        let racing = views.generation();
        views.clear();
        assert!(views.get("personal:a").is_none() && views.get("personal:b").is_none());
        views.store("personal:a", racing, view("a", &["stale"]));
        assert!(views.get("personal:a").is_none());
        let racing = views.generation();
        views.invalidate("personal:b");
        views.store("personal:a", racing, view("a", &["stale"]));
        assert!(views.get("personal:a").is_none());
        let fresh = views.generation();
        views.store("personal:a", fresh, view("a", &["c9"]));
        assert_eq!(ids(&views.get("personal:a").unwrap()), ["c9"]);
    }

    #[test]
    fn a_local_write_drops_the_view_when_its_guard_ends() {
        let views = Views::new(VIEW_TTL);
        views.enable();
        let generation = views.generation();
        views.store("personal:a", generation, view("a", &["c1"]));
        {
            let _changes = views.changes("personal:a");
            // A read racing the write is not kept either.
            let racing = views.generation();
            views.store("personal:b", racing, view("b", &["c2"]));
        }
        assert!(views.get("personal:a").is_none());
        assert!(views.get("personal:b").is_some(), "stored before the drop");
        let racing = views.generation();
        drop(views.changes("personal:a"));
        views.store("personal:a", racing, view("a", &["stale"]));
        assert!(views.get("personal:a").is_none());
    }

    #[test]
    fn views_expire() {
        let views = Views::new(Duration::from_millis(20));
        views.enable();
        let generation = views.generation();
        views.store("personal:a", generation, view("a", &["c1"]));
        assert!(views.get("personal:a").is_some());
        std::thread::sleep(Duration::from_millis(30));
        assert!(views.get("personal:a").is_none());
    }

    #[tokio::test]
    async fn fences_serialize_one_tenant_only() {
        let fences = Arc::new(Fences::default());
        let held = fences.lock("personal:a").await;
        // Another tenant is not blocked.
        tokio::time::timeout(Duration::from_millis(100), fences.lock("personal:b"))
            .await
            .unwrap();
        let waiting = {
            let fences = fences.clone();
            tokio::spawn(async move { fences.lock("personal:a").await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished());
        drop(held);
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap();
    }
}
