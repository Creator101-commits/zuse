//! One capped PostgreSQL pool per process.
//!
//! Every long-running process (`serve`, `worker`, `billing-worker`,
//! `auth-worker`, ...) opens its connections here, once, and hands clones of
//! the same pool to every subsystem. The connection count is therefore fixed
//! by the process role (overridable with `ZUSE_CLOUD_DB_MAX_CONNECTIONS`),
//! not by how many subsystems happen to be enabled.
//!
//! A worker process may also hold one dedicated `LISTEN` connection (see
//! [`wake`]). It is counted separately in [`Budget::total`] and can be turned
//! off with `ZUSE_CLOUD_DB_LISTEN=false`.
//!
//! The app must connect to PostgreSQL directly: a transaction-pooling proxy
//! cannot deliver `LISTEN` notifications and breaks prepared statements.
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{str::FromStr, time::Duration};

pub mod schema;
pub mod wake;

/// Connections kept open while idle are closed after this long.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Connections are recycled after this long, so server-side state never grows unbounded.
pub const MAX_LIFETIME: Duration = Duration::from_secs(30 * 60);
/// An idle connection is pinged before reuse only after this long.
const PING_AFTER_IDLE: Duration = Duration::from_secs(15);
const MAX_CONFIGURED_CONNECTIONS: u32 = 64;

/// The long-running process a pool belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Serve,
    Gateway,
    Worker,
    BillingWorker,
    AuthWorker,
    ImageWorker,
    ProjectPreparationWorker,
    /// One-shot operator commands.
    Operator,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Serve => "serve",
            Self::Gateway => "gateway",
            Self::Worker => "worker",
            Self::BillingWorker => "billing-worker",
            Self::AuthWorker => "auth-worker",
            Self::ImageWorker => "image-worker",
            Self::ProjectPreparationWorker => "project-preparation-worker",
            Self::Operator => "operator",
        }
    }
    /// Pool connections when `ZUSE_CLOUD_DB_MAX_CONNECTIONS` is unset.
    ///
    /// The production set (`serve` + `worker` + `billing-worker` +
    /// `auth-worker`) is 6 + 3 + 2 + 1 = 12 pool connections, plus one
    /// `LISTEN` connection each for `serve` and `worker`: 14 in total.
    pub const fn default_connections(self) -> u32 {
        match self {
            Self::Serve | Self::Gateway => 6,
            Self::Worker => 3,
            // One control connection plus one for delivery receipts, which
            // commit while the control transaction still holds its fence.
            Self::BillingWorker => 2,
            Self::ImageWorker | Self::ProjectPreparationWorker => 2,
            Self::AuthWorker | Self::Operator => 1,
        }
    }
    /// The fewest connections this role can run with.
    const fn minimum_connections(self) -> u32 {
        match self {
            // Serve keeps a separate read-only connection for the catalog.
            Self::Serve | Self::Gateway | Self::BillingWorker => 2,
            _ => 1,
        }
    }
    /// Roles with background loops that sleep until woken.
    pub const fn listens(self) -> bool {
        matches!(self, Self::Serve | Self::Worker)
    }
    fn application_name(self) -> String {
        format!("zuse-cloud-{}", self.as_str())
    }
}

/// How many connections one process may hold, and how it waits for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub role: Role,
    /// Pool connections, across every pool in the process.
    pub connections: u32,
    /// Whether the process holds one extra dedicated `LISTEN` connection.
    pub listen: bool,
    pub acquire_timeout: Duration,
}

impl Budget {
    pub fn from_env(role: Role) -> anyhow::Result<Self> {
        Self::from_lookup(role, |name| std::env::var(name).ok())
    }
    /// Pure parser behind [`Self::from_env`].
    pub fn from_lookup(
        role: Role,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> anyhow::Result<Self> {
        let value = |name: &str| {
            lookup(name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let connections = match value("ZUSE_CLOUD_DB_MAX_CONNECTIONS") {
            None => role.default_connections(),
            Some(raw) => raw
                .parse::<u32>()
                .ok()
                .filter(|n| (role.minimum_connections()..=MAX_CONFIGURED_CONNECTIONS).contains(n))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Invalid ZUSE_CLOUD_DB_MAX_CONNECTIONS for {} (expected {}..={})",
                        role.as_str(),
                        role.minimum_connections(),
                        MAX_CONFIGURED_CONNECTIONS
                    )
                })?,
        };
        let listen = match value("ZUSE_CLOUD_DB_LISTEN").as_deref() {
            None => role.listens(),
            Some("true") => role.listens(),
            Some("false") => false,
            Some(_) => anyhow::bail!("Invalid ZUSE_CLOUD_DB_LISTEN (expected true or false)"),
        };
        let acquire_timeout = match value("ZUSE_CLOUD_DB_ACQUIRE_TIMEOUT_MS") {
            None => Duration::from_secs(5),
            Some(raw) => raw
                .parse::<u64>()
                .ok()
                .filter(|ms| (100..=60_000).contains(ms))
                .map(Duration::from_millis)
                .ok_or_else(|| anyhow::anyhow!("Invalid ZUSE_CLOUD_DB_ACQUIRE_TIMEOUT_MS"))?,
        };
        Ok(Self {
            role,
            connections,
            listen,
            acquire_timeout,
        })
    }
    /// Connections reserved for the read-only catalog pool (serve and gateway only).
    pub fn read_only_connections(&self) -> u32 {
        match self.role {
            Role::Serve | Role::Gateway => (self.connections / 3).max(1),
            _ => 0,
        }
    }
    /// Connections reserved for billing delivery receipts (billing worker only).
    pub fn receipt_connections(&self) -> u32 {
        match self.role {
            Role::BillingWorker => 1,
            _ => 0,
        }
    }
    /// Connections in the shared writable pool.
    pub fn shared_connections(&self) -> u32 {
        self.connections - self.read_only_connections() - self.receipt_connections()
    }
    /// Every connection this process can open, including `LISTEN`.
    pub fn total(&self) -> u32 {
        self.connections + u32::from(self.listen)
    }
}

/// The pools of one process. Subsystems receive clones of these.
#[derive(Clone)]
pub struct ProcessDb {
    pub budget: Budget,
    /// Shared writable pool used by every subsystem.
    pub pool: PgPool,
    /// `default_transaction_read_only` catalog pool (serve and gateway).
    pub read_only: Option<PgPool>,
    /// Billing delivery receipts (billing worker).
    pub receipts: Option<PgPool>,
    url: String,
}

impl ProcessDb {
    pub async fn connect(url: &str, budget: Budget) -> anyhow::Result<Self> {
        let pool = connect_pool(url, &budget, budget.shared_connections(), Kind::Shared).await?;
        let read_only = match budget.read_only_connections() {
            0 => None,
            n => Some(connect_pool(url, &budget, n, Kind::ReadOnly).await?),
        };
        let receipts = match budget.receipt_connections() {
            0 => None,
            n => Some(connect_pool(url, &budget, n, Kind::Shared).await?),
        };
        tracing::info!(
            role = budget.role.as_str(),
            shared = budget.shared_connections(),
            read_only = budget.read_only_connections(),
            receipts = budget.receipt_connections(),
            listen = budget.listen,
            total = budget.total(),
            acquire_timeout_ms = budget.acquire_timeout.as_millis() as u64,
            "Database connection budget"
        );
        Ok(Self {
            budget,
            pool,
            read_only,
            receipts,
            url: url.to_owned(),
        })
    }
    /// Starts the dedicated `LISTEN` connection, or a disabled waker.
    pub async fn wakeups(&self) -> std::sync::Arc<wake::Wakeups> {
        if !self.budget.listen {
            return wake::Wakeups::disabled();
        }
        match wake::Wakeups::listen(&self.url, &self.budget).await {
            Ok(wakeups) => wakeups,
            Err(error) => {
                tracing::warn!(
                    code = "database_listen_unavailable",
                    error = %error,
                    "Database wakeups unavailable; using due-time scheduling and idle polling"
                );
                wake::Wakeups::disabled()
            }
        }
    }
    pub fn receipts_pool(&self) -> PgPool {
        self.receipts.clone().unwrap_or_else(|| self.pool.clone())
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn read_only_pool(&self) -> PgPool {
        self.read_only.clone().unwrap_or_else(|| self.pool.clone())
    }
}

/// A single-connection read-only pool for one-shot operator commands.
pub async fn read_only_pool(url: &str) -> anyhow::Result<PgPool> {
    let budget = Budget::from_lookup(Role::Operator, |_| None)?;
    connect_pool(url, &budget, 1, Kind::ReadOnly).await
}

/// A small writable pool for a different database URL than the process
/// database (operator commands, or a separately configured device database).
pub async fn dedicated_pool(url: &str, role: Role, max: u32) -> anyhow::Result<PgPool> {
    let budget = Budget::from_lookup(role, |_| None)?;
    connect_pool(url, &budget, max, Kind::Shared).await
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Shared,
    ReadOnly,
}

pub(crate) fn connect_options(
    url: &str,
    application_name: &str,
) -> anyhow::Result<PgConnectOptions> {
    Ok(PgConnectOptions::from_str(url)
        .map_err(|_| anyhow::anyhow!("Invalid cloud PostgreSQL configuration"))?
        .application_name(application_name))
}

async fn connect_pool(url: &str, budget: &Budget, max: u32, kind: Kind) -> anyhow::Result<PgPool> {
    let name = match kind {
        Kind::Shared => budget.role.application_name(),
        Kind::ReadOnly => format!("{}-read", budget.role.application_name()),
    };
    let options = connect_options(url, &name)?;
    pool_options(budget, max, kind)
        .connect_with(options)
        .await
        .map_err(|_| anyhow::anyhow!("Cannot connect to cloud PostgreSQL"))
}

fn pool_options(budget: &Budget, max: u32, kind: Kind) -> PgPoolOptions {
    let read_only = kind == Kind::ReadOnly;
    // The billing writer always bounded its lock waits; keep that.
    let lock_timeout = budget.role == Role::BillingWorker;
    PgPoolOptions::new()
        .max_connections(max.max(1))
        .min_connections(u32::from(budget.role == Role::Serve && !read_only))
        .acquire_timeout(budget.acquire_timeout)
        .idle_timeout(IDLE_TIMEOUT)
        .max_lifetime(MAX_LIFETIME)
        .test_before_acquire(false)
        .before_acquire(|connection, metadata| {
            Box::pin(async move {
                // A recently used connection does not pay another round trip
                // before every query; an idle one is checked first.
                if metadata.idle_for >= PING_AFTER_IDLE {
                    use sqlx::Connection;
                    connection.ping().await?;
                }
                Ok(true)
            })
        })
        .after_connect(move |connection, _| {
            Box::pin(async move {
                if read_only {
                    sqlx::query("SET default_transaction_read_only = on")
                        .execute(&mut *connection)
                        .await?;
                }
                sqlx::query("SET statement_timeout = '10s'")
                    .execute(&mut *connection)
                    .await?;
                if lock_timeout {
                    sqlx::query("SET lock_timeout = '5s'")
                        .execute(&mut *connection)
                        .await?;
                }
                Ok(())
            })
        })
}

/// Backoff for a loop whose last database round failed (for example an
/// exhausted pool): doubles from `base` up to `cap`, never a tight retry.
pub fn failure_backoff(consecutive_failures: u32, base: Duration, cap: Duration) -> Duration {
    let shift = consecutive_failures.saturating_sub(1).min(16);
    base.saturating_mul(1u32 << shift).min(cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn budget(role: Role, vars: &[(&str, &str)]) -> anyhow::Result<Budget> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect();
        Budget::from_lookup(role, |name| vars.get(name).cloned())
    }

    #[test]
    fn role_defaults_fit_the_production_budget() {
        let roles = [
            Role::Serve,
            Role::Worker,
            Role::BillingWorker,
            Role::AuthWorker,
        ];
        let budgets: Vec<_> = roles.iter().map(|r| budget(*r, &[]).unwrap()).collect();
        assert_eq!(
            budgets.iter().map(|b| b.connections).collect::<Vec<_>>(),
            [6, 3, 2, 1]
        );
        assert_eq!(
            budgets.iter().map(|b| b.listen).collect::<Vec<_>>(),
            [true, true, false, false]
        );
        assert_eq!(budgets.iter().map(Budget::total).sum::<u32>(), 14);
        for b in &budgets {
            assert_eq!(b.acquire_timeout, Duration::from_secs(5));
            assert_eq!(
                b.shared_connections() + b.read_only_connections() + b.receipt_connections(),
                b.connections
            );
        }
    }

    #[test]
    fn serve_splits_read_only_and_shared_connections() {
        let serve = budget(Role::Serve, &[]).unwrap();
        assert_eq!(
            (serve.shared_connections(), serve.read_only_connections()),
            (4, 2)
        );
        let small = budget(Role::Serve, &[("ZUSE_CLOUD_DB_MAX_CONNECTIONS", "2")]).unwrap();
        assert_eq!(
            (small.shared_connections(), small.read_only_connections()),
            (1, 1)
        );
        let billing = budget(Role::BillingWorker, &[]).unwrap();
        assert_eq!(
            (billing.shared_connections(), billing.receipt_connections()),
            (1, 1)
        );
        let worker = budget(Role::Worker, &[]).unwrap();
        assert_eq!(worker.shared_connections(), 3);
    }

    #[test]
    fn environment_overrides_are_validated() {
        let worker = budget(
            Role::Worker,
            &[
                ("ZUSE_CLOUD_DB_MAX_CONNECTIONS", " 5 "),
                ("ZUSE_CLOUD_DB_LISTEN", "false"),
                ("ZUSE_CLOUD_DB_ACQUIRE_TIMEOUT_MS", "2500"),
            ],
        )
        .unwrap();
        assert_eq!(worker.connections, 5);
        assert!(!worker.listen);
        assert_eq!(worker.total(), 5);
        assert_eq!(worker.acquire_timeout, Duration::from_millis(2500));
        // A role without background loops never listens, even when asked to.
        assert!(
            !budget(Role::AuthWorker, &[("ZUSE_CLOUD_DB_LISTEN", "true")])
                .unwrap()
                .listen
        );
        for bad in ["0", "65", "-1", "many"] {
            assert!(budget(Role::Worker, &[("ZUSE_CLOUD_DB_MAX_CONNECTIONS", bad)]).is_err());
        }
        assert!(budget(Role::Serve, &[("ZUSE_CLOUD_DB_MAX_CONNECTIONS", "1")]).is_err());
        assert!(
            budget(
                Role::BillingWorker,
                &[("ZUSE_CLOUD_DB_MAX_CONNECTIONS", "1")]
            )
            .is_err()
        );
        assert!(budget(Role::Worker, &[("ZUSE_CLOUD_DB_LISTEN", "yes")]).is_err());
        assert!(budget(Role::Worker, &[("ZUSE_CLOUD_DB_ACQUIRE_TIMEOUT_MS", "10")]).is_err());
        // Empty values mean "use the default".
        assert_eq!(
            budget(Role::Worker, &[("ZUSE_CLOUD_DB_MAX_CONNECTIONS", "")])
                .unwrap()
                .connections,
            3
        );
    }

    #[test]
    fn failure_backoff_doubles_up_to_its_cap() {
        let base = Duration::from_secs(1);
        let cap = Duration::from_secs(30);
        let waits: Vec<_> = (1..=7)
            .map(|n| failure_backoff(n, base, cap).as_secs())
            .collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30]);
        assert_eq!(failure_backoff(0, base, cap), base);
        assert_eq!(failure_backoff(u32::MAX, base, cap), cap);
    }
}
