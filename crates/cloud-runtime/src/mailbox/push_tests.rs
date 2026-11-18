//! Lease pacing against a fixture server: gateway hints wake the mailbox,
//! an idle pushed runtime stays quiet, and a runtime without push long-polls.
use super::*;
use axum::{Json, Router, extract::State, routing::post};
use std::sync::atomic::{AtomicUsize, Ordering};
use zuse_rpc::cloud::CommandPush;

#[derive(Default)]
struct Admits(AtomicUsize);
#[async_trait]
impl Executor for Admits {
    async fn admitted(&self, _: &MessageSend, _: &str) -> Result<bool> {
        Ok(false)
    }
    async fn native_admitted(&self, _: &CloudSessionCommand, _: &str) -> Result<bool> {
        Ok(false)
    }
    async fn admit_native(&self, _: &CloudSessionCommand, _: &str, _: i64) -> Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn admit(&self, _: &MessageSend, _: &str, _: i64) -> Result<()> {
        bail!("native-only fixture")
    }
}
#[derive(Default)]
struct Server {
    queued: Mutex<Vec<RuntimeLease>>,
    waits: Mutex<Vec<u64>>,
}
impl Server {
    fn requests(&self) -> usize {
        self.waits.lock().unwrap().len()
    }
}
fn command(key: &CommandKey, id: &str) -> RuntimeLease {
    let command: CloudSessionCommand = serde_json::from_value(
        json!({"commandId":id,"sessionId":format!("session-{id}"),"payload":{"kind":"interrupt"}}),
    )
    .unwrap();
    let plain = serde_json::to_vec(&command).unwrap();
    let mut envelope:CommandEnvelope=serde_json::from_value(json!({"protocolVersion":3,"workspaceId":"workspace","sessionId":command.session_id,"commandId":id,"kind":"zuse.session.interrupt","fingerprint":key.fingerprint(&plain),"schemaVersion":1,"keyVersion":1,"destructionFence":0,"createdAt":now_ms(),"iv":"","ciphertext":"","dependencies":[]})).unwrap();
    (envelope.iv, envelope.ciphertext) = key.encrypt(&envelope, &plain).unwrap();
    RuntimeLease {
        command: envelope,
        workspace_sequence: 1,
        lease_token: format!("lease-{id}"),
        lease_deadline: now_ms() + 60000,
        runtime_generation: 1,
        provider_sandbox_id: "machine".into(),
        storage_incarnation_id: "disk".into(),
    }
}
/// Hands out one queued command per lease, so draining takes several.
/// An empty long-poll holds briefly (scaled down from `waitMs`).
async fn lease(State(state): State<Arc<Server>>, Json(request): Json<Value>) -> Json<LeasePage> {
    let wait = request["waitMs"].as_u64().unwrap();
    state.waits.lock().unwrap().push(wait);
    let next = state.queued.lock().unwrap().pop();
    if next.is_none() && wait > 0 {
        tokio::time::sleep(Duration::from_millis(wait / 100)).await;
    }
    Json(LeasePage {
        leases: next.into_iter().collect(),
        nonterminal_count: 0,
        mailbox_revision: 1,
        fence_required: false,
    })
}
struct Fixture {
    server: Arc<Server>,
    executor: Arc<Admits>,
    key: CommandKey,
    cancel: CancellationToken,
    runtime: tokio::task::JoinHandle<Result<()>>,
    _dir: tempfile::TempDir,
}
async fn start(canonical: bool, push: Arc<CommandPush>) -> Fixture {
    let key_bytes = B64.encode([23; 32]);
    let server = Arc::new(Server::default());
    let executor = Arc::new(Admits::default());
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("receipts.sqlite3");
    drop(Connection::open(&database).unwrap());
    let mut boot = json!({"workspaceId":"workspace","zuseAccountId":"owner","chatId":"thread","initialSessionId":"session","runtimeCredential":"synthetic","runtimeGeneration":1,"gatewayEpoch":1,"runtimeCredentialExpiresAt":9007199254740000i64,"gatewayUrl":"wss://fixture.invalid","gatewayProtocol":"fixture","sealedTranscriptKey":"synthetic","runtimeStorage":{"format":"zuse-rust-engine-v1","storageIncarnationId":"disk"},"providerSandboxId":"machine"});
    if canonical {
        boot["commandAdmissionVersion"] = json!(2);
        boot["commandDeliveryMode"] = json!("server-admitted-v2");
    }
    let boot: CloudRuntimeBootstrap = serde_json::from_value(boot).unwrap();
    let mailbox = Mailbox::new(
        &database,
        &boot,
        CommandKey::parse(&key_bytes).unwrap(),
        executor.clone(),
    )
    .unwrap()
    .with_command_push(push);
    assert_eq!(mailbox.canonical, canonical);
    let app = Router::new()
        .route(
            "/v1/cloud/workspaces/workspace/runtime/commands/lease",
            post(lease),
        )
        .route(
            "/v1/cloud/workspaces/workspace/runtime/commands/ack",
            post(|| async { Json(json!({"ok":true})) }),
        )
        .with_state(server.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let authority = Authority::new(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let (sender, receiver) = watch::channel(boot);
    let runtime = tokio::spawn(async move {
        let _sender = sender;
        mailbox.run(authority, receiver, token).await
    });
    Fixture {
        server,
        executor,
        key: CommandKey::parse(&key_bytes).unwrap(),
        cancel,
        runtime,
        _dir: dir,
    }
}
async fn until(what: &str, check: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

async fn pushed_runtime_leases_on_hints_and_drains(canonical: bool) {
    let push = Arc::new(CommandPush::default());
    push.received();
    let f = start(canonical, push.clone()).await;
    // Startup leases, then the pending connect hint leases once more; both
    // find nothing and the runtime stays quiet.
    until("the startup lease", || f.server.requests() >= 1).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let idle = f.server.requests();
    assert!(idle <= 2, "startup took {idle} leases");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        f.server.requests(),
        idle,
        "an idle pushed runtime does not poll"
    );
    assert!(f.server.waits.lock().unwrap().iter().all(|wait| *wait == 0));
    // Three commands arrive and the gateway sends one hint.
    *f.server.queued.lock().unwrap() = (0..3).map(|i| command(&f.key, &format!("c{i}"))).collect();
    push.hint();
    until("every command", || f.executor.0.load(Ordering::SeqCst) == 3).await;
    // It drains with back-to-back leases until one comes back empty.
    until("the empty lease", || {
        f.server.queued.lock().unwrap().is_empty() && f.server.requests() >= idle + 4
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let settled = f.server.requests();
    assert!(
        (idle + 4..=idle + 7).contains(&settled),
        "drained with {settled} leases, then idle"
    );
    assert!(f.server.waits.lock().unwrap().iter().all(|wait| *wait == 0));
    // Losing the gateway switches straight to long-polling.
    push.disconnected();
    until("long-polling", || {
        f.server.waits.lock().unwrap().last() == Some(&(pacing::LONG_POLL_MS as u64))
    })
    .await;
    f.cancel.cancel();
    f.runtime.await.unwrap().unwrap();
}

#[tokio::test]
async fn serial_mailbox_leases_on_hints_and_drains_until_empty() {
    pushed_runtime_leases_on_hints_and_drains(false).await;
}

#[tokio::test]
async fn concurrent_mailbox_leases_on_hints_and_drains_until_empty() {
    pushed_runtime_leases_on_hints_and_drains(true).await;
}

#[tokio::test]
async fn without_push_the_mailbox_long_polls_at_the_server_maximum() {
    let f = start(true, Arc::new(CommandPush::default())).await;
    until("a few long-polls", || f.server.requests() >= 3).await;
    assert!(
        f.server
            .waits
            .lock()
            .unwrap()
            .iter()
            .all(|wait| *wait == 25_000)
    );
    // Commands still flow with no gateway at all.
    f.server
        .queued
        .lock()
        .unwrap()
        .push(command(&f.key, "polled"));
    until("the polled command", || {
        f.executor.0.load(Ordering::SeqCst) == 1
    })
    .await;
    f.cancel.cancel();
    f.runtime.await.unwrap().unwrap();
}
