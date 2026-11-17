# Standalone cloud database bootstrap

`database.py` initializes an empty private application schema with no customer or
fixture rows. It runs all reviewed SQL in one transaction, records migration hashes,
serializes concurrent migration attempts, rejects checksum drift and rejects taking
over an existing unmanaged schema. It never modifies the public schema or installs
a database server.

The initial application baseline preserves the current Rust SQL contract's `api_*`
names and storage formats. It was exported from the source application's canonical
schema at `a0155d2dd11f07495a99d3737c9bd40a5c8b4508` with drizzle-orm 0.44.3 and
drizzle-kit 0.31.4. Foreign keys resolve within the selected private schema; fresh
index creation is transactional. This is a compatibility baseline, not the proposed
Postgres hot-history redesign or a legacy data migration.

```sh
# No DB connection or mutation; inspect migration identities/checksums.
python3 infra/cloud/database.py --environment staging plan

# Read ZUSE_CLOUD_DATABASE_URL privately from the deployment secret environment.
python3 infra/cloud/database.py --environment staging apply
python3 infra/cloud/database.py --environment staging check
```

Staging uses `zuse_staging`; production uses `zuse_production`. Remote URLs require
`sslmode=verify-full`; include the verified `sslrootcert` file for Supabase. The URL
is never printed, passed as a command argument, or inferred from DATABASE_URL.
Inherited PG service/options are removed so a shell cannot redirect the operation.
The tool needs the `psql` client and a migration owner authorized to create its schema.

Provision separate restricted application roles before deploying services. Each role
needs its own default search_path/URL options, USAGE only on its environment schema,
and appropriate table/sequence privileges. Keep migration ledgers writable only by
the migration owner. Do not use the Supabase administrator credential in a deployed
service. The single-host deployment helper accepts a shared database only when both URLs
explicitly name the distinct expected environment schema search paths and restricted
login roles; arbitrary different usernames do not satisfy its isolation check.

This tool does not migrate legacy rows, transfer runtime ownership, register GitHub
or model accounts, seed billing entitlements, or configure auth keys. Importing
encrypted legacy records also requires the original keys or a reviewed re-encryption
migration. It deliberately does not apply old ownership records or enable writes to
legacy provider resources.

Validation:

```sh
python3 infra/cloud/tests/database.py
```

Six tests run against a fresh temporary localhost Postgres cluster, which is stopped
and removed afterwards. No deployment database credentials are used. These tests
cover empty staging/production bootstrap, idempotence, drift rejection, transaction
rollback, unmanaged-schema protection and TLS/environment handling.

The Hetzner deployment subsequently applied both private Supabase schemas and
restricted environment roles. No customer rows were imported and public tables
were not modified. See the single-host runbook for deployment state.

Migration rehearsal and the optional private-file read-only legacy inventory are
documented in [MIGRATION_REHEARSAL.md](MIGRATION_REHEARSAL.md). Its temporary-cluster
fixture proves row/ciphertext/reference preservation and writer/rollback mechanics;
it does not migrate real retained customer data or authorize execution from legacy
images.

## Connection budget and wakeups

Each long-running process opens one capped set of connections at startup and
logs it (`Database connection budget`). Every subsystem in the process shares
those pools; none opens its own.

| Process | Pool connections | `LISTEN` connection | Total |
| --- | ---: | ---: | ---: |
| `serve` | 6 (3 shared, 2 read-only catalog, 1 billing receipts) | 1 | 7 |
| plugin executor (child of `serve`, plugins enabled) | 1 | 0 | 1 |
| `worker` | 3 | 1 | 4 |
| `billing-worker` | 2 (1 control, 1 delivery receipts) | 0 | 2 |
| `auth-worker` | 1 | 1 | 2 |
| **One environment** | | | **14** |

Staging and production on one database server need twice that, plus headroom
for migrations and operator commands (one connection each). Size the server's
connection limit for that.

Settings (per process environment):

- `ZUSE_CLOUD_DB_MAX_CONNECTIONS`: pool connections for this process, replacing
  the default above (1–64; `serve` needs at least 3, `billing-worker` at least
  2). `serve` with Slack enabled also needs at least 2 shared connections.
- `ZUSE_CLOUD_EXECUTOR_DB_MAX_CONNECTIONS` (1–16, default 1): connections
  of the plugin executor that `serve` starts when plugins are enabled. It
  runs every SDK and sealed-state statement on one long-lived pool of this
  size; `serve` logs it as `executor` beside its own budget.
- `ZUSE_CLOUD_DB_LISTEN=false`: drop the `LISTEN` connection. Workers then rely
  on due-time scheduling and a shorter idle poll.
- `ZUSE_CLOUD_DB_ACQUIRE_TIMEOUT_MS` (default 5000): how long a request waits
  for a free connection before failing with `503 cloud_database_unavailable`
  (logged as `database_pool_exhausted`). Background loops back off after such
  failures instead of retrying right away.
- `ZUSE_CLOUD_WORKER_IDLE_POLL_MS`: idle fallback for sleeping loops (default
  20 s with `LISTEN`, 5 s without). `ZUSE_CLOUD_WORKER_POLL_MS` (default 1 s) is
  the shortest wait while work stays due.
- `ZUSE_CLOUD_AUTH_WORKER_POLL_MS` (default 2 s): only without `LISTEN`
  (`ZUSE_CLOUD_DB_LISTEN=false`, the listener failed to start, or migration
  `0045_auth_wakeups.sql` is not installed yet), how often
  the credential worker checks for due work with one query. With `LISTEN` it
  sleeps until a notification or its next deadline (operation expiry, lease
  end, refresh dispatch deadline, journal retention), with a 5-minute safety
  check, and runs no statements while idle.

Connections close after 60 s idle and are recycled after 30 minutes.

**Connect directly, not through a transaction-pooling proxy.** A pooler in
transaction mode (for example a managed PgBouncer) cannot deliver `LISTEN`
notifications and breaks the prepared statements the app uses. Point
`ZUSE_CLOUD_DATABASE_URL` at the server's direct endpoint. The small fixed pools
above keep the connection count bounded.

Migration `0043_work_wakeups.sql` adds triggers that send `NOTIFY` on channel
`zuse_cloud_work:<schema>` when jobs, task drains, recreations, machine images,
project cache work or account-event outbox rows are created or rescheduled.
Migration `0045_auth_wakeups.sql` adds payload `auth` for the credential worker:
a sign-in or credential proposal becomes claimable, a refresh starts, or a
connection holding a refresh lease is disconnected or needs reconnecting. The
worker's own claims and heartbeats stay silent. Payloads: `jobs`, `task_drain`,
`recreation`, `machine_images`, `project_cache`, `account_events`, `mailbox`,
`auth`.
The schema in the channel name keeps staging and production from waking each
other. `NOTIFY` only shortens waits: workers also sleep until the next known due
time and poll on the idle timer, so a lost notification, a reconnecting
listener or `ZUSE_CLOUD_DB_LISTEN=false` delays work but never loses it.
Set `RUST_LOG=zuse_cloud_api=debug,zuse_cloud::events=debug` to see why each
loop woke (`notify`, `due`, `idle` or `hint`).

## Billing schedule and spending caps

`billing-worker` keeps its two connections and no `LISTEN` connection. It
does not poll on a fixed interval. It runs a pass at startup, then sleeps until
the earliest of these:

- **Hourly tick.** Snapshot storage settlement, provider usage settlement for
  closed UTC days (every closed day per pass, up to 420; beyond that,
  consecutive passes take consecutive batches, so no day is skipped), and meter
  export and reconciliation.
- **A new closed day.** Each day becomes settleable at 00:30 UTC. The worker
  wakes then, not up to an hour later. While the newest day's provider report
  is unavailable or incomplete, it retries after 1, 2, 4 … minutes, at most
  hourly.
- **Known due times.** The earliest `next_attempt_at` of undelivered outbox
  rows (deliveries keep their own backoff), five minutes after a delivery for
  meter reconciliation, and for embedded billing accounts: engine work
  `availableAt`, held model reservation expiry, credit grant expiry and
  subscription period ends. One indexed query per source after each pass
  reads these.

Work that is still due after a pass waits at least 60 s, the old fixed
interval. Between due times the worker runs no statements.

Events run without waiting for the worker:

- A verified payment notification (subscription change, top-up, payment) for
  an embedded billing account is projected in the API process right after it
  commits: engine work, subscription and period projection, compute debits
  and the hold. Engine retries and expiries caused in the API process
  (automatic top-ups, model reservations) run there at their due time. Each
  account gets at most one such task. A restarted API process only loses
  latency: the worker's next pass does the same work.
- Model request reserve, increase, settle and release recompute the hold in
  the same request.
- Each settlement pass is followed at once by compute debits and a hold
  recompute.

**Why caps stay exact.** Spending only changes on events: a closed day's
provider usage is settled, a storage interval is settled, a payment
notification is projected, a model request changes state, an engine retry
runs, or a reservation or grant expires. Legacy overage accounts are checked
at admission against the settled ledger and unexpired reservations, so nothing
there depends on elapsed time. For shared-credit accounts each of those events
recomputes `api_cloud_billing_periods.shared_credit_hold`. Expiries are known
in advance and scheduled exactly. Unchanged holds are not rewritten. When a
hold turns on, the trigger from optional migration `0047_billing_wakeups.sql`
(installed with the embedded billing migrations by `apply-brokeboy`) sends
`NOTIFY idle_sleep`, so the runtime worker pauses that account's computers
right away. Its idle scan remains the fallback.

**What a projected-deadline cap would need.** Workspace cost is known only
once provider usage for a closed day is settled, so a running machine's spend
is not tracked live. Stopping machines before the cap is reached would need:

- a per-running-machine price (`api_provider_price_schedule` rate × the
  machine's vCPU and memory, plus markup),
- a projected time-to-cap per account (remaining credit or cap ÷ summed hourly
  rate of its running machines), scheduled like the due times above,
- a product decision to stop machines on projected rather than settled spend,
  and to reconcile any difference once usage settles.

That would be new user-visible behavior, so it is not implemented.
