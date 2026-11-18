//! The worker's side: one dispatch, enrollment wakes the job, one fallback.
use super::*;
use crate::{crypto::MasterKey, tickets::TicketSigner};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

/// A Boat computer whose first start waits inside the guest.
struct Guest {
    machine: Mutex<Option<Machine>>,
    status: Mutex<Value>,
    dispatches: AtomicUsize,
    status_reads: AtomicUsize,
    inspections: AtomicUsize,
    helper_calls: AtomicUsize,
    launches: AtomicUsize,
    starting_inspections: usize,
}
#[async_trait]
impl AllocationProvider for Guest {
    fn provider_id(&self) -> &'static str {
        "boat"
    }
    fn guest_start(&self) -> bool {
        true
    }
    async fn find(&self, _: &str) -> Result<Option<Machine>, ProviderError> {
        Ok(self.machine.lock().unwrap().clone())
    }
    async fn inspect_id(&self, _: &str) -> Result<Option<Machine>, ProviderError> {
        let mut machine = self.machine.lock().unwrap().clone();
        // Boat answers create while still provisioning; it runs shortly after.
        if self.inspections.fetch_add(1, Ordering::SeqCst) < self.starting_inspections
            && let Some(m) = machine.as_mut()
        {
            m.state = MachineState::Starting;
        }
        Ok(machine)
    }
    async fn create(&self, _: &str, name: &str) -> Result<Machine, ProviderError> {
        let m = Machine {
            id: "original-machine".into(),
            name: name.into(),
            state: MachineState::Starting,
            vcpu: None,
            memory_bytes: None,
        };
        let mut running = m.clone();
        running.state = MachineState::Running;
        *self.machine.lock().unwrap() = Some(running);
        Ok(m)
    }
    async fn bootstrap_initial_runtime(
        &self,
        _: &str,
        _: &str,
        _: &str,
        r: &Request,
    ) -> Result<Option<Value>, ProviderError> {
        self.helper_calls.fetch_add(1, Ordering::SeqCst);
        r.response(&ready(r))
    }
    async fn launch(&self, _: &str, _: &RuntimeLaunch) -> Result<(), ProviderError> {
        self.launches.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn start_in_guest(&self, start: &GuestStart<'_>) -> Result<(), ProviderError> {
        assert_eq!(start.machine, "original-machine");
        assert_eq!(start.artifact, "managed-artifact");
        assert!(start.request.valid());
        assert_eq!(start.launch.operation_id, start.request.operation_id);
        assert!(!start.launch.boot_token.is_empty());
        self.dispatches.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn guest_start_status(&self, machine: &str, _: &str) -> Result<Value, ProviderError> {
        assert_eq!(machine, "original-machine");
        self.status_reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.status.lock().unwrap().clone())
    }
}

async fn job(pool: &sqlx::PgPool, workspace: &str) -> (String, i64, Value) {
    sqlx::query_as("SELECT state,due_at,payload FROM zuse_rust_jobs WHERE resource_id=$1")
        .bind(workspace)
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn due_now(pool: &sqlx::PgPool, workspace: &str) {
    sqlx::query("UPDATE zuse_rust_jobs SET due_at=0 WHERE resource_id=$1")
        .bind(workspace)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn one_dispatch_then_enrollment_wakes_the_job_and_failures_fall_back_once() {
    let url = std::env::var("ZUSE_CLOUD_TEST_DATABASE_URL").expect("isolated test database");
    assert!(matches!(
        reqwest::Url::parse(&url).unwrap().host_str(),
        Some("localhost" | "127.0.0.1" | "::1")
    ));
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    for case in [
        "enrolled",
        "early-wake",
        "timeout",
        "launched-not-enrolled",
        "rejected",
    ] {
        let schema = format!("guest_start_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let scope = schema.clone();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .after_connect(move |c, _| {
                let scope = scope.clone();
                Box::pin(async move {
                    sqlx::query(&format!("SET search_path={scope}"))
                        .execute(c)
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .unwrap();
        for script in [
            include_str!("../../../../../tests/fixtures/legacy-catalog.sql"),
            jobs::MIGRATION,
            crate::runtime::allocation::MIGRATION,
            crate::runtime::deletion::MIGRATION,
        ] {
            sqlx::raw_sql(script).execute(&pool).await.unwrap();
        }
        let workspace = format!("ws-{}", uuid::Uuid::new_v4().simple());
        let config = json!({"runtimeGeneration":1,"gatewayEpoch":1,"rustProviderControllerVersion":1,
            "managedReleaseId":"explicit-fresh-release","rustFastStartVersion":1,"rustGuestStartVersion":1,
            "initialRuntimeBootstrap":{"version":1,"mode":"post-hydration-first-start-v1",
            "runtimeSha256":"a".repeat(64),"supervisorSha256":"b".repeat(64),"serviceSha256":"c".repeat(64),
            "manifestSha256":"d".repeat(64),"helperSha256":"e".repeat(64)}});
        sqlx::query("UPDATE api_cloud_workspaces SET workspace_id=$1,provider='boat',provider_sandbox_id=NULL,runtime_credential_hash=NULL,runtime_boot_token_hash=NULL,state='queued',desired_state='ready',runtime_state='offline',request_config=$2 WHERE workspace_id='ws-bob'").bind(&workspace).bind(config).execute(&pool).await.unwrap();
        let now = clock_ms();
        sqlx::query("INSERT INTO zuse_rust_control_owners VALUES('workspace',$1,'rust',1,$2)")
            .bind(&workspace)
            .bind(now)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO api_cloud_workspace_launch_intents VALUES($1,'user-bob','chat','session','turn','command','test-ciphertext',$2,$3)").bind(&workspace).bind(now+86400000).bind(now).execute(&pool).await.unwrap();
        let signing = ed25519_dalek::SigningKey::from_bytes(&[74; 32]);
        let jwk = json!({"kty":"OKP","crv":"Ed25519","d":URL_SAFE_NO_PAD.encode([74;32]),"x":URL_SAFE_NO_PAD.encode(signing.verifying_key().as_bytes())});
        let control = RuntimeControl::from_pool(
            pool.clone(),
            MasterKey::parse(&URL_SAFE_NO_PAD.encode([75; 32])).unwrap(),
            TicketSigner::from_jwk(&jwk.to_string(), "https://api.example.test").unwrap(),
        );
        let operation = control
            .queue_allocation(&workspace, "managed-artifact", now)
            .await
            .unwrap();
        let guest = Guest {
            machine: Mutex::new(None),
            status: Mutex::new(json!({})),
            dispatches: AtomicUsize::new(0),
            status_reads: AtomicUsize::new(0),
            inspections: AtomicUsize::new(0),
            helper_calls: AtomicUsize::new(0),
            launches: AtomicUsize::new(0),
            starting_inspections: 2,
        };

        // Create, wait for running, one dispatch. No helper, no server launch.
        control.allocate_once(&guest, clock_ms()).await.unwrap();
        assert_eq!(guest.dispatches.load(Ordering::SeqCst), 1, "{case}");
        assert_eq!(guest.helper_calls.load(Ordering::SeqCst), 0);
        assert_eq!(guest.launches.load(Ordering::SeqCst), 0);
        assert!(guest.inspections.load(Ordering::SeqCst) >= 3);
        let phase: String =
            sqlx::query_scalar("SELECT phase FROM zuse_rust_allocations WHERE operation_id=$1")
                .bind(&operation)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(phase, "launching");
        let (state, due, payload) = job(&pool, &workspace).await;
        assert_eq!(state, "pending");
        assert!(due >= clock_ms() + 60_000, "fallback is far away: {due}");
        assert_eq!(payload[DISPATCHED_FLAG], true);
        let row: Value = sqlx::query_scalar(
            "SELECT to_jsonb(w) FROM api_cloud_workspaces w WHERE workspace_id=$1",
        )
        .bind(&workspace)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row["status_code"], "starting-runtime");
        assert_eq!(row["request_config"][MARKER]["operationId"], operation);
        assert_eq!(
            row["request_config"][MARKER]["machineId"],
            "original-machine"
        );

        match case {
            "enrolled" => {
                // The runtime's own enrollment is the wake-up event.
                let mut tx = pool.begin().await.unwrap();
                sqlx::query("UPDATE api_cloud_workspaces SET runtime_credential_hash='enrolled' WHERE workspace_id=$1").bind(&workspace).execute(&mut *tx).await.unwrap();
                wake_guest_started_allocation(&mut tx, &workspace, 1, clock_ms())
                    .await
                    .unwrap();
                tx.commit().await.unwrap();
                let (_, due, _) = job(&pool, &workspace).await;
                assert!(due <= clock_ms());
                assert!(control.allocate_once(&guest, clock_ms()).await.unwrap());
                let (state, due, _) = job(&pool, &workspace).await;
                assert_eq!(state, "pending");
                assert!(due <= clock_ms() + 2_500);
                assert_eq!(guest.status_reads.load(Ordering::SeqCst), 0);
                assert_eq!(guest.dispatches.load(Ordering::SeqCst), 1);
                assert_eq!(guest.launches.load(Ordering::SeqCst), 0);
            }
            "early-wake" => {
                let inspections = guest.inspections.load(Ordering::SeqCst);
                due_now(&pool, &workspace).await;
                control.allocate_once(&guest, clock_ms()).await.unwrap();
                let (_, due, _) = job(&pool, &workspace).await;
                assert!(due >= clock_ms() + 60_000, "deferred back to the fallback");
                assert_eq!(guest.inspections.load(Ordering::SeqCst), inspections);
                assert_eq!(guest.status_reads.load(Ordering::SeqCst), 0);
                assert_eq!(guest.dispatches.load(Ordering::SeqCst), 1);
            }
            _ => {
                let status = match case {
                    "timeout" => {
                        json!({"version":1,"operationId":operation,"status":"timeout","code":"fresh_start_path_unsafe_until_deadline"})
                    }
                    "rejected" => {
                        json!({"version":1,"operationId":operation,"status":"rejected","code":"fresh_start_helper_rejected"})
                    }
                    _ => {
                        let r: Request = serde_json::from_value(json!({"version":1,"provider":"boat","purpose":"initial-allocation","operationId":operation,"workspaceId":workspace,"generation":1,"storageIncarnationId":row["request_config"]["rustRuntimeStorage"]["storageIncarnationId"],"sourceMachineId":"original-machine","imagePins":{"runtimeSha256":"a".repeat(64),"supervisorSha256":"b".repeat(64),"serviceSha256":"c".repeat(64),"manifestSha256":"d".repeat(64),"helperSha256":"e".repeat(64)}})).unwrap();
                        json!({"version":1,"operationId":operation,"status":"launched","receipt":ready(&r)})
                    }
                };
                *guest.status.lock().unwrap() = status;
                sqlx::query("UPDATE api_cloud_workspaces SET request_config=jsonb_set(request_config,'{rustGuestStart,fallbackAt}','0') WHERE workspace_id=$1").bind(&workspace).execute(&pool).await.unwrap();
                due_now(&pool, &workspace).await;
                let _ = control.allocate_once(&guest, clock_ms()).await;
                assert_eq!(guest.status_reads.load(Ordering::SeqCst), 1);
                let (code, marker): (Option<String>, Value) = sqlx::query_as("SELECT j.last_error_code,w.request_config->'rustGuestStart' FROM zuse_rust_jobs j JOIN api_cloud_workspaces w ON w.workspace_id=j.resource_id WHERE j.resource_id=$1").bind(&workspace).fetch_one(&pool).await.unwrap();
                let (expected, marked) = match case {
                    "timeout" => ("cloud_guest_start_timeout", "cloud_guest_start_timeout"),
                    "rejected" => ("cloud_provider_rejected", "cloud_guest_start_rejected"),
                    _ => (
                        "cloud_guest_start_not_enrolled",
                        "cloud_guest_start_not_enrolled",
                    ),
                };
                assert_eq!(code.as_deref(), Some(expected));
                assert_eq!(marker["serverPath"], true);
                assert_eq!(marker["code"], marked);
                // The next attempt uses the server path exactly once, never
                // another status read or in-computer dispatch.
                due_now(&pool, &workspace).await;
                control.allocate_once(&guest, clock_ms()).await.unwrap();
                assert_eq!(guest.helper_calls.load(Ordering::SeqCst), 1);
                assert_eq!(guest.launches.load(Ordering::SeqCst), 1);
                assert_eq!(guest.dispatches.load(Ordering::SeqCst), 1);
                assert_eq!(guest.status_reads.load(Ordering::SeqCst), 1);
            }
        }
        pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
    }
    admin.close().await;
}
