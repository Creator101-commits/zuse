//! Classification rules (pure) and the whole pass against a real PostgreSQL
//! schema built from the release manifest, with a fake provider listing.
use super::cost::{Finding, MachineDay, evaluate, next_report_at, report_day};
use super::*;
use crate::providers::{ExecResult, Machine, MachineUsage, ProviderError, contract::*};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const HOUR: i64 = 3_600_000;
const NOW: i64 = 1_800_000_000_000;

fn config(mode: Mode) -> Config {
    let mut config = Config::from_lookup(|_| None).unwrap();
    config.mode = mode;
    config
}
fn machine(id: &str, state: MachineState) -> ListedMachine {
    ListedMachine {
        id: id.into(),
        name: format!("zr-{id}"),
        state,
        created_at_ms: None,
    }
}
fn ready(last_activity_at: i64) -> CurrentWorkspace {
    CurrentWorkspace {
        id: "ws-1".into(),
        account: Some("user-1".into()),
        state: "ready".into(),
        desired: "ready".into(),
        runtime: Some("online".into()),
        last_activity_at: Some(last_activity_at),
        updated_at: Some(last_activity_at),
        lifecycle_phase: Some("ready".into()),
        ..CurrentWorkspace::default()
    }
}
fn current(ws: CurrentWorkspace) -> Facts {
    Facts {
        workspace: Some(ws),
        ..Facts::default()
    }
}
fn owner(status: &str) -> Facts {
    Facts {
        owner: Some(Owner {
            workspace_id: Some("ws-gone".into()),
            source: "ledger".into(),
            status: status.into(),
            ..Owner::default()
        }),
        ..Facts::default()
    }
}

#[test]
fn a_machine_unknown_to_this_environment_is_never_touched() {
    let config = config(Mode::SleepAndDelete);
    for state in [MachineState::Running, MachineState::Paused] {
        let verdict = classify(
            &machine("other-env", state),
            &Facts::default(),
            NOW,
            &config,
        );
        assert_eq!(verdict.class, Class::Foreign);
        assert_eq!(verdict.planned, Planned::Nothing);
        assert!(!verdict.alert);
    }
}

#[test]
fn a_genuinely_active_computer_is_expected_awake() {
    let config = config(Mode::Sleep);
    let awake = machine("m", MachineState::Running);
    let stale = NOW - 10 * HOUR;
    let cases = [
        ready(NOW - HOUR),
        CurrentWorkspace {
            live_lease: true,
            ..ready(stale)
        },
        CurrentWorkspace {
            active_turn: true,
            ..ready(stale)
        },
        CurrentWorkspace {
            launch_intent: true,
            ..ready(stale)
        },
        CurrentWorkspace {
            last_turn_at: Some(NOW - 60_000),
            ..ready(stale)
        },
    ];
    for ws in cases {
        let verdict = classify(&awake, &current(ws.clone()), NOW, &config);
        assert_eq!(
            (verdict.class, verdict.planned),
            (Class::Expected, Planned::Nothing),
            "{ws:?}"
        );
    }
    // Recent work on the machine (allocation, lifecycle, a pending job) too.
    let mut facts = current(ready(stale));
    facts.in_flight = Some("lifecycle".into());
    assert_eq!(
        classify(&awake, &facts, NOW, &config).class,
        Class::Expected
    );
    let mut facts = current(CurrentWorkspace {
        pending_job: true,
        ..ready(stale)
    });
    facts.in_flight = None;
    assert_eq!(
        classify(&awake, &facts, NOW, &config).class,
        Class::Expected
    );
    // Infrastructure is never touched, whatever else is true.
    let facts = Facts {
        infrastructure: Some("auth-authority".into()),
        ..owner("deleted")
    };
    assert_eq!(
        classify(&awake, &facts, NOW, &config).class,
        Class::Expected
    );
}

#[test]
fn a_quiet_ready_computer_past_the_bound_is_paused_through_the_lifecycle() {
    let config = config(Mode::Sleep);
    let verdict = classify(
        &machine("m", MachineState::Running),
        &current(ready(NOW - config.quiet_ms - 1)),
        NOW,
        &config,
    );
    assert_eq!(verdict.class, Class::ShouldSleep);
    assert_eq!(verdict.reason, "quiet-past-bound");
    assert_eq!(verdict.planned, Planned::Pause);
    assert_eq!(verdict.workspace.as_deref(), Some("ws-1"));
    // Asleep already: nothing to do.
    let asleep = classify(
        &machine("m", MachineState::Paused),
        &current(ready(NOW - config.quiet_ms - 1)),
        NOW,
        &config,
    );
    assert_eq!(asleep.class, Class::Expected);
}

#[test]
fn an_awake_machine_the_database_records_asleep_is_slept() {
    let config = config(Mode::Sleep);
    for (state, desired) in [("paused", "paused"), ("archived", "archived")] {
        let ws = CurrentWorkspace {
            state: state.into(),
            desired: desired.into(),
            lifecycle_phase: Some("paused".into()),
            ..ready(NOW - 30 * HOUR)
        };
        let verdict = classify(
            &machine("m", MachineState::Running),
            &current(ws),
            NOW,
            &config,
        );
        assert_eq!(
            (verdict.class, verdict.reason, verdict.planned),
            (Class::ShouldSleep, "asleep-in-database", Planned::Sleep)
        );
    }
    // A resume in flight (desired ready) is a transition, never slept.
    let resuming = CurrentWorkspace {
        state: "resuming".into(),
        desired: "ready".into(),
        lifecycle_phase: Some("waking".into()),
        updated_at: Some(NOW - 60_000),
        ..ready(NOW - 30 * HOUR)
    };
    let verdict = classify(
        &machine("m", MachineState::Running),
        &current(resuming.clone()),
        NOW,
        &config,
    );
    assert_eq!((verdict.class, verdict.alert), (Class::Expected, false));
    // ... unless it has been stuck past the bound: alert, still no action.
    let stuck = CurrentWorkspace {
        updated_at: Some(NOW - 30 * HOUR),
        ..resuming
    };
    let verdict = classify(
        &machine("m", MachineState::Running),
        &current(stuck),
        NOW,
        &config,
    );
    assert_eq!(
        (verdict.class, verdict.planned, verdict.alert),
        (Class::Expected, Planned::Nothing, true)
    );
}

#[test]
fn leftovers_are_slept_and_only_a_deleted_workspaces_machine_is_deletable() {
    let config = config(Mode::SleepAndDelete);
    let awake = machine("m", MachineState::Running);
    let asleep = machine("m", MachineState::Paused);
    for status in ["deleted", "missing"] {
        let verdict = classify(&awake, &owner(status), NOW, &config);
        assert_eq!(
            (verdict.class, verdict.reason, verdict.planned),
            (Class::ShouldNotExist, "workspace-deleted", Planned::Sleep)
        );
        let verdict = classify(&asleep, &owner(status), NOW, &config);
        assert_eq!(verdict.planned, Planned::Delete);
        // Sleeping first keeps the same verdict, so the deletion window keeps running.
        assert_eq!(
            verdict.key(),
            classify(&awake, &owner(status), NOW, &config).key()
        );
    }
    // A live workspace's replaced machine, or a builder leftover: sleep only.
    let replaced = classify(&awake, &owner("alive"), NOW, &config);
    assert_eq!(
        (replaced.reason, replaced.planned),
        ("replaced", Planned::Sleep)
    );
    assert_eq!(
        classify(&asleep, &owner("alive"), NOW, &config).planned,
        Planned::Nothing
    );
    let builder = Facts {
        builder: Some("machine-image".into()),
        ..Facts::default()
    };
    assert_eq!(
        classify(&awake, &builder, NOW, &config).reason,
        "builder-leftover"
    );
    assert_eq!(
        classify(&asleep, &builder, NOW, &config).planned,
        Planned::Nothing
    );
    // A provider state in transition is never acted on.
    let starting = machine("m", MachineState::Starting);
    assert_eq!(
        classify(&starting, &owner("deleted"), NOW, &config).planned,
        Planned::Nothing
    );
}

#[test]
fn configuration_defaults_to_dry_run_and_rejects_unsafe_values() {
    let config = Config::from_lookup(|_| None).unwrap();
    assert_eq!(config.mode, Mode::DryRun);
    assert_eq!(config.interval_ms, 20 * 60_000);
    assert_eq!(config.delete_after_ms, 24 * HOUR);
    for (raw, mode) in [
        ("off", Mode::Off),
        ("dry-run", Mode::DryRun),
        ("sleep", Mode::Sleep),
        ("sleep-and-delete", Mode::SleepAndDelete),
    ] {
        let config = Config::from_lookup(|name| {
            (name == "ZUSE_CLOUD_PROVIDER_RECONCILIATION").then(|| raw.to_owned())
        })
        .unwrap();
        assert_eq!(config.mode, mode);
    }
    for (name, raw) in [
        ("ZUSE_CLOUD_PROVIDER_RECONCILIATION", "delete"),
        ("ZUSE_CLOUD_PROVIDER_RECONCILIATION_INTERVAL_MS", "1000"),
        (
            "ZUSE_CLOUD_PROVIDER_RECONCILIATION_DELETE_AFTER_MS",
            "60000",
        ),
        ("ZUSE_CLOUD_PROVIDER_RECONCILIATION_QUIET_MS", "soon"),
    ] {
        assert!(
            Config::from_lookup(|key| (key == name).then(|| raw.to_owned())).is_err(),
            "{name}={raw}"
        );
    }
}

#[test]
fn cost_check_flags_quiet_awake_machines_and_account_spikes() {
    let config = config(Mode::DryRun);
    let (start, end) = report_day(NOW);
    assert_eq!(end - start, 86_400_000);
    assert!(end <= NOW - 2 * HOUR && next_report_at(NOW) > NOW);
    let day = |id: &str, hours: u64, cost: i64, account: &str, activity: Option<i64>| MachineDay {
        machine_id: id.into(),
        running_seconds: hours * 3600,
        cost_micros: cost,
        account: Some(account.into()),
        last_activity_at: activity,
    };
    let today = vec![
        // Seven hours awake, last touched the day before: flagged.
        day("stale", 7, 1_000_000, "a", Some(start - HOUR)),
        // Seven hours awake with activity during the day: fine.
        day("used", 7, 1_000_000, "a", Some(start + HOUR)),
        // Awake all day: flagged whatever the activity.
        day("always", 23, 2_000_000, "b", Some(start + HOUR)),
        // Spike: $40 against a $2/day trailing average.
        day("big", 2, 40_000_000, "c", Some(start + HOUR)),
    ];
    let trailing = BTreeMap::from([("a".to_owned(), 14_000_000), ("c".to_owned(), 14_000_000)]);
    let findings = evaluate(start, &today, &trailing, &config);
    assert!(findings.contains(&Finding::AwakeWithoutActivity {
        machine_id: "stale".into(),
        running_seconds: 7 * 3600
    }));
    assert!(findings.contains(&Finding::AwakeAllDay {
        machine_id: "always".into(),
        running_seconds: 23 * 3600
    }));
    assert!(findings.contains(&Finding::Spike {
        account: "c".into(),
        cost_micros: 40_000_000,
        trailing_average_micros: 2_000_000
    }));
    // Account a ($2 vs $2/day) and b (below the $5 floor) are not spikes.
    assert_eq!(findings.len(), 3, "{findings:?}");
}

// ---- PostgreSQL ------------------------------------------------------------

/// A provider whose listing the test controls; records every mutation.
#[derive(Default)]
struct Listing {
    machines: Mutex<Vec<ListedMachine>>,
    usage: Mutex<Vec<MachineUsage>>,
    slept: Mutex<Vec<String>>,
    deleted: Mutex<Vec<String>>,
}
impl Listing {
    fn set(&self, machines: Vec<ListedMachine>) {
        *self.machines.lock().unwrap() = machines;
    }
    fn slept(&self) -> Vec<String> {
        self.slept.lock().unwrap().clone()
    }
    fn deleted(&self) -> Vec<String> {
        self.deleted.lock().unwrap().clone()
    }
}
#[async_trait::async_trait]
impl Provider for Listing {
    fn id(&self) -> ProviderId {
        ProviderId::Boxd
    }
    async fn list_machines(&self) -> Result<Option<Vec<ListedMachine>>, ProviderError> {
        Ok(Some(self.machines.lock().unwrap().clone()))
    }
    async fn usage_report(
        &self,
        _: i64,
        _: i64,
    ) -> Result<Option<Vec<MachineUsage>>, ProviderError> {
        Ok(Some(self.usage.lock().unwrap().clone()))
    }
    async fn inspect(&self, id: &str) -> Result<Option<Machine>, ProviderError> {
        Ok(self
            .machines
            .lock()
            .unwrap()
            .iter()
            .find(|m| m.id == id)
            .map(|m| Machine {
                id: m.id.clone(),
                name: m.name.clone(),
                state: m.state.clone(),
                vcpu: None,
                memory_bytes: None,
            }))
    }
    async fn sleep(&self, id: &str) -> Result<ProviderOperation, ProviderError> {
        self.slept.lock().unwrap().push(id.into());
        for machine in self.machines.lock().unwrap().iter_mut() {
            if machine.id == id {
                machine.state = MachineState::Paused;
            }
        }
        Ok(ProviderOperation {
            resource_id: id.into(),
            operation_id: None,
            desired: DesiredState::Sleeping,
        })
    }
    async fn delete(&self, id: &str) -> Result<ProviderOperation, ProviderError> {
        self.deleted.lock().unwrap().push(id.into());
        self.machines.lock().unwrap().retain(|m| m.id != id);
        Ok(ProviderOperation {
            resource_id: id.into(),
            operation_id: None,
            desired: DesiredState::Deleted,
        })
    }
    async fn find_allocation(&self, _: &str) -> Result<Option<Machine>, ProviderError> {
        unreachable!()
    }
    async fn allocate(&self, _: &Allocation<'_>) -> Result<Machine, ProviderError> {
        unreachable!()
    }
    async fn resume(&self, _: &str, _: u32) -> Result<ProviderOperation, ProviderError> {
        unreachable!()
    }
    async fn operation(&self, _: &ProviderOperation) -> Result<OperationState, ProviderError> {
        unreachable!()
    }
    async fn execute(&self, _: &str, _: &str) -> Result<ExecResult, ProviderError> {
        unreachable!()
    }
    async fn upload(&self, _: &str, _: &str, _: &[u8]) -> Result<(), ProviderError> {
        unreachable!()
    }
}

/// Every migration of the release manifest (`infra/cloud/database.py`), in order.
fn release_migrations() -> Vec<String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("../../infra/cloud/database.py")).unwrap();
    let start = manifest.find("FILES = (").unwrap();
    let end = start + manifest[start..].find(')').unwrap();
    manifest[start..end]
        .split('"')
        .filter(|item| item.ends_with(".sql"))
        .map(|name| std::fs::read_to_string(root.join("migrations").join(name)).unwrap())
        .collect()
}

async fn fixture() -> (PgPool, PgPool, String) {
    let url = std::env::var("ZUSE_CLOUD_TEST_DATABASE_URL").expect("isolated PostgreSQL required");
    assert!(matches!(
        reqwest::Url::parse(&url).unwrap().host_str(),
        Some("localhost" | "127.0.0.1" | "::1")
    ));
    let admin = PgPool::connect(&url).await.unwrap();
    let schema = format!("reconcile_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let scope = schema.clone();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |connection, _| {
            let scope = scope.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET search_path={scope}"))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    for sql in release_migrations() {
        sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    }
    // Only workspace rows matter here; skip their project/build fixtures.
    sqlx::raw_sql("DO $$ DECLARE c record; BEGIN FOR c IN SELECT conname FROM pg_constraint WHERE conrelid='api_cloud_workspaces'::regclass AND contype='f' LOOP EXECUTE format('ALTER TABLE api_cloud_workspaces DROP CONSTRAINT %I', c.conname); END LOOP; END $$;")
        .execute(&pool)
        .await
        .unwrap();
    (admin, pool, schema)
}

fn control(pool: &PgPool) -> RuntimeControl {
    let signing = ed25519_dalek::SigningKey::from_bytes(&[71; 32]);
    let jwk = json!({"kty":"OKP","crv":"Ed25519","d":URL_SAFE_NO_PAD.encode([71;32]),"x":URL_SAFE_NO_PAD.encode(signing.verifying_key().as_bytes())});
    RuntimeControl::from_pool(
        pool.clone(),
        crate::crypto::MasterKey::parse(&URL_SAFE_NO_PAD.encode([72; 32])).unwrap(),
        crate::tickets::TicketSigner::from_jwk(&jwk.to_string(), "https://api.example.test")
            .unwrap(),
    )
}

/// A workspace whose current computer is `machine`.
#[allow(clippy::too_many_arguments)]
async fn workspace(
    pool: &PgPool,
    id: &str,
    machine: Option<&str>,
    state: &str,
    desired: &str,
    last_activity_at: i64,
    runtime: &str,
    config: Value,
) {
    sqlx::query("INSERT INTO api_cloud_workspaces(workspace_id,account_id,project_id,build_id,provider,provider_sandbox_id,chat_id,initial_session_id,branch,base_ref,state,desired_state,status_code,idempotency_key,next_action_at,created_at,updated_at,last_activity_at,runtime_state,request_config) VALUES($1,'user-1','p','b','boxd',$2,'c-'||$1,'s-'||$1,$1,'main',$3,$4,$3,'k-'||$1,0,0,$5,$5,$6,$7)")
        .bind(id).bind(machine).bind(state).bind(desired).bind(last_activity_at).bind(runtime).bind(config)
        .execute(pool).await.unwrap();
}

async fn schedule_due(pool: &PgPool, at: i64) {
    sqlx::query("UPDATE zuse_rust_provider_reconciliation_schedule SET next_run_at=$1")
        .bind(at)
        .execute(pool)
        .await
        .unwrap();
}

async fn alerts(pool: &PgPool) -> Vec<(String, String, bool)> {
    sqlx::query_as("SELECT kind,subject,resolved_at IS NOT NULL FROM zuse_rust_operator_alerts ORDER BY kind,subject")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// A request configuration the lifecycle can quiesce (as idle sleep requires).
fn pausable(machine: &str) -> Value {
    use zuse_protocol::cloud_lifecycle::CAPABILITY;
    json!({
        "runtimeGeneration":1,"gatewayEpoch":1,"runtimeRpcProtocol":"zuse-rpc-v1",
        "commandDeliveryMode":zuse_protocol::cloud_task_commands::MODE,
        "rustCommandAdmissionVersion":2,"rustCommandMailboxEnabled":true,
        "runtimeBootstrapReceipt":{"workspaceId":"ws-stale","generation":1,"gatewayEpoch":1},
        "runtimeCredentialKeyThumbprint":"credential-key","runtimeSigningKeyThumbprint":"signing-key",
        "rustRuntimeStorage":{"storageIncarnationId":"retained-disk"},
        "rustLifecycleOperation":{"id":"older","action":"resume","phase":"ready"},
        "rustStorageEnrollment":{"phase":"committed","generation":1,"sourceSandboxId":machine,
            "storageIncarnationId":"retained-disk","credentialKeyThumbprint":"credential-key","signingKeyThumbprint":"signing-key",
            "capabilities":[CAPABILITY,"rust-session-commands-v1",zuse_protocol::cloud_task_commands::CAPABILITY]}
    })
}

#[tokio::test]
#[ignore = "requires explicit isolated localhost PostgreSQL"]
async fn the_pass_classifies_against_the_database_and_acts_only_when_safe() {
    let (admin, pool, schema) = fixture().await;
    let control = control(&pool);
    let provider = Arc::new(Listing::default());
    let mut registry = Registry::default();
    registry.insert(provider.clone()).unwrap();
    let stale = NOW - 10 * HOUR;

    // In use: awake, ready, active a minute ago.
    workspace(
        &pool,
        "ws-active",
        Some("vm-active"),
        "ready",
        "ready",
        NOW - 60_000,
        "online",
        json!({}),
    )
    .await;
    // Asleep in the database but awake at the provider.
    workspace(
        &pool,
        "ws-paused",
        Some("vm-paused"),
        "paused",
        "paused",
        stale,
        "offline",
        json!({"rustLifecycleOperation":{"id":"op","action":"pause","phase":"paused"}}),
    )
    .await;
    // Ready, quiet for ten hours, no lease: paused through the lifecycle.
    workspace(
        &pool,
        "ws-stale",
        Some("vm-stale"),
        "ready",
        "ready",
        stale,
        "online",
        pausable("vm-stale"),
    )
    .await;
    sqlx::query("INSERT INTO zuse_rust_control_owners VALUES('workspace','ws-stale','rust',1,0)")
        .execute(&pool)
        .await
        .unwrap();
    // Ready and quiet, but its runtime is offline: it cannot quiesce.
    workspace(
        &pool,
        "ws-offline",
        Some("vm-offline"),
        "ready",
        "ready",
        stale,
        "offline",
        json!({}),
    )
    .await;
    sqlx::query("INSERT INTO zuse_rust_control_owners VALUES('workspace','ws-offline','rust',1,0)")
        .execute(&pool)
        .await
        .unwrap();
    // A deleted workspace whose machine survived (the ledger remembers it).
    workspace(
        &pool,
        "ws-deleted",
        Some("vm-orphan"),
        "ready",
        "ready",
        stale,
        "online",
        json!({}),
    )
    .await;
    sqlx::query("UPDATE api_cloud_workspaces SET state='deleted',desired_state='deleted',provider_sandbox_id=NULL,deleted_at=1 WHERE workspace_id='ws-deleted'")
        .execute(&pool).await.unwrap();
    let ledger: Vec<(String,)> = sqlx::query_as(
        "SELECT workspace_id FROM zuse_rust_machine_ledger WHERE machine_key='id:vm-orphan'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        ledger,
        vec![("ws-deleted".to_owned(),)],
        "the trigger recorded the machine"
    );

    provider.set(vec![
        machine("vm-active", MachineState::Running),
        machine("vm-paused", MachineState::Running),
        machine("vm-stale", MachineState::Running),
        machine("vm-offline", MachineState::Running),
        machine("vm-orphan", MachineState::Running),
        // The other environment's machines, awake and asleep: never touched.
        machine("vm-foreign", MachineState::Running),
        machine("vm-foreign-asleep", MachineState::Paused),
    ]);

    // Dry run (the default): every verdict, no action, alerts recorded.
    let mut dry = config(Mode::DryRun);
    dry.confirm_ms = 0;
    schedule_due(&pool, 0).await;
    let summary = control
        .provider_reconciliation_once(&registry, &dry, NOW)
        .await
        .unwrap()
        .expect("due");
    assert_eq!(
        (
            summary.listed,
            summary.foreign,
            summary.expected,
            summary.should_sleep,
            summary.should_not_exist
        ),
        (7, 2, 1, 3, 1)
    );
    assert_eq!((summary.slept, summary.paused, summary.deleted), (0, 0, 0));
    assert!(provider.slept().is_empty() && provider.deleted().is_empty());
    let open = alerts(&pool).await;
    assert_eq!(open.len(), 4, "{open:?}");
    assert!(
        open.iter()
            .all(|(_, subject, _)| !subject.contains("foreign"))
    );
    // Not due again until the interval has passed.
    assert!(
        control
            .provider_reconciliation_once(&registry, &dry, NOW + 1000)
            .await
            .unwrap()
            .is_none()
    );

    // Acting mode, but inside the confirmation window: still nothing.
    let mut act = config(Mode::Sleep);
    act.confirm_ms = 10 * 60_000;
    schedule_due(&pool, 0).await;
    let summary = control
        .provider_reconciliation_once(&registry, &act, NOW + 60_000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.slept + summary.paused, 0);
    assert!(provider.slept().is_empty());

    // Past the window: the asleep-in-database and orphan machines are slept,
    // the stale ready computer gets a lifecycle pause; the active, offline
    // and foreign machines are left alone.
    schedule_due(&pool, 0).await;
    let later = NOW + 11 * 60_000;
    let summary = control
        .provider_reconciliation_once(&registry, &act, later)
        .await
        .unwrap()
        .unwrap();
    let mut slept = provider.slept();
    slept.sort();
    assert_eq!(slept, vec!["vm-orphan", "vm-paused"]);
    assert_eq!(
        (summary.slept, summary.paused, summary.deleted),
        (2, 1, 0),
        "{:?}",
        summary.errors
    );
    let (state, desired, reason): (String, String, Option<String>) = sqlx::query_as("SELECT state,desired_state,request_config#>>'{rustLifecycleOperation,reason}' FROM api_cloud_workspaces WHERE workspace_id='ws-stale'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(
        (state.as_str(), desired.as_str(), reason.as_deref()),
        ("pausing", "paused", Some("provider-reconciliation"))
    );
    let jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM zuse_rust_jobs WHERE resource_id='ws-stale' AND kind=$1",
    )
    .bind(crate::runtime::lifecycle::JOB_KIND)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(jobs, 1);
    let offline: (String, Option<String>) = sqlx::query_as("SELECT state,(SELECT last_action FROM zuse_rust_provider_machine_observations WHERE machine_id='vm-offline') FROM api_cloud_workspaces WHERE workspace_id='ws-offline'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(
        offline,
        ("ready".to_owned(), Some("pause-unavailable".to_owned()))
    );

    // The orphan is asleep now; sleep mode never deletes, and delete mode
    // waits out the safety window counted from when it was first seen.
    let mut delete = config(Mode::SleepAndDelete);
    delete.confirm_ms = 0;
    schedule_due(&pool, 0).await;
    control
        .provider_reconciliation_once(&registry, &delete, NOW + HOUR)
        .await
        .unwrap();
    assert!(provider.deleted().is_empty(), "inside the safety window");
    schedule_due(&pool, 0).await;
    let summary = control
        .provider_reconciliation_once(&registry, &delete, NOW + delete.delete_after_ms + 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(provider.deleted(), vec!["vm-orphan"]);
    assert_eq!(summary.deleted, 1);
    // On the next pass, machines that agree again (or are gone) have their
    // alerts resolved.
    schedule_due(&pool, 0).await;
    control
        .provider_reconciliation_once(&registry, &delete, NOW + delete.delete_after_ms + 2)
        .await
        .unwrap();
    let open: Vec<_> = alerts(&pool)
        .await
        .into_iter()
        .filter(|(_, _, resolved)| !resolved)
        .collect();
    assert!(
        open.iter()
            .all(|(_, subject, _)| subject != "boxd:vm-paused" && subject != "boxd:vm-orphan"),
        "{open:?}"
    );
    assert!(
        provider
            .slept()
            .iter()
            .chain(provider.deleted().iter())
            .all(|id| !id.contains("foreign") && id != "vm-active" && id != "vm-offline")
    );

    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires explicit isolated localhost PostgreSQL"]
async fn a_resuming_computer_is_never_slept_and_off_does_nothing() {
    let (admin, pool, schema) = fixture().await;
    let control = control(&pool);
    let provider = Arc::new(Listing::default());
    let mut registry = Registry::default();
    registry.insert(provider.clone()).unwrap();
    workspace(
        &pool,
        "ws-paused",
        Some("vm-paused"),
        "paused",
        "paused",
        NOW - 10 * HOUR,
        "offline",
        json!({"rustLifecycleOperation":{"id":"op","action":"pause","phase":"paused"}}),
    )
    .await;
    provider.set(vec![machine("vm-paused", MachineState::Running)]);

    // Off: never claims, never lists, never acts.
    let off = config(Mode::Off);
    schedule_due(&pool, 0).await;
    assert!(
        control
            .provider_reconciliation_once(&registry, &off, NOW)
            .await
            .unwrap()
            .is_none()
    );
    assert!(alerts(&pool).await.is_empty());

    let mut act = config(Mode::Sleep);
    act.confirm_ms = 0;
    // Seen asleep-in-database once (dry run starts its window) ...
    let mut dry = config(Mode::DryRun);
    dry.confirm_ms = 0;
    schedule_due(&pool, 0).await;
    let summary = control
        .provider_reconciliation_once(&registry, &dry, NOW)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.should_sleep, 1);
    // ... then a resume begins: the computer is in transition and is never
    // slept, whatever the earlier verdict was.
    sqlx::query("UPDATE api_cloud_workspaces SET state='resuming',desired_state='ready',updated_at=$1 WHERE workspace_id='ws-paused'")
        .bind(NOW).execute(&pool).await.unwrap();
    schedule_due(&pool, 0).await;
    let summary = control
        .provider_reconciliation_once(&registry, &act, NOW + 60_000)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.expected, 1);
    assert!(
        provider.slept().is_empty(),
        "a resuming computer is never slept"
    );
    let open: Vec<_> = alerts(&pool)
        .await
        .into_iter()
        .filter(|(_, _, resolved)| !resolved)
        .collect();
    assert!(open.is_empty(), "its alert resolved: {open:?}");

    // The worker's due time follows the schedule row only when enabled.
    assert_eq!(
        control.worker_due().await.unwrap().provider_reconciliation,
        None
    );
    control.enable_provider_reconciliation(true);
    let due = control.worker_due().await.unwrap().provider_reconciliation;
    // The cost check has never run, so it is due at once.
    assert_eq!(due, Some(0));

    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires explicit isolated localhost PostgreSQL"]
async fn the_daily_cost_check_records_alerts_for_owned_machines_only() {
    let (admin, pool, schema) = fixture().await;
    let control = control(&pool);
    let provider = Arc::new(Listing::default());
    let mut registry = Registry::default();
    registry.insert(provider.clone()).unwrap();
    let (start, _) = report_day(NOW);
    workspace(
        &pool,
        "ws-zombie",
        Some("vm-zombie"),
        "ready",
        "ready",
        start - 5 * 86_400_000,
        "online",
        json!({}),
    )
    .await;
    *provider.usage.lock().unwrap() = vec![
        MachineUsage {
            machine_id: "vm-zombie".into(),
            running_seconds: 24 * 3600,
            cost_micros: 3_000_000,
        },
        // The other environment's machine, also awake all day: ignored.
        MachineUsage {
            machine_id: "vm-foreign".into(),
            running_seconds: 24 * 3600,
            cost_micros: 3_000_000,
        },
    ];
    let config = config(Mode::DryRun);
    schedule_due(&pool, 0).await;
    let report = control
        .cost_anomalies_once(&registry, &config, NOW)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((report.machines, report.findings), (1, 1));
    let open = alerts(&pool).await;
    assert_eq!(open.len(), 1);
    assert!(
        open[0].0 == "cost:awake-all-day" && open[0].1.starts_with("boxd:vm-zombie:"),
        "{open:?}"
    );
    // Once a day: the next run is the next day's report.
    let next: i64 = sqlx::query_scalar("SELECT next_run_at FROM zuse_rust_provider_reconciliation_schedule WHERE task='cost-anomalies'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(next, next_report_at(NOW));
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
}
