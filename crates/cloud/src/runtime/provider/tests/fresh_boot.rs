//! Boat's first start: the credential-free helper is retried quickly while the
//! loader still sees the provider's ownership, without repeating verification.
use super::*;
use crate::managed_bases::bootstrap::{ImagePins, Purpose, Request};

struct HelperFixture {
    id: ProviderId,
    /// Loader refusals before the helper answers.
    not_ready: usize,
    /// Answer after the refusals: "ready", "starting" or "rejected".
    answer: &'static str,
    verifications: AtomicUsize,
    helper_calls: AtomicUsize,
}
fn request(id: ProviderId) -> Request {
    Request {
        version: 1,
        provider: id,
        purpose: Purpose::InitialAllocation,
        operation_id: "original-op".into(),
        workspace_id: "workspace-original".into(),
        generation: 1,
        storage_incarnation_id: "original-disk".into(),
        source_machine_id: "original-machine".into(),
        image_pins: ImagePins {
            runtime_sha256: "a".repeat(64),
            supervisor_sha256: "b".repeat(64),
            service_sha256: "c".repeat(64),
            manifest_sha256: "d".repeat(64),
            helper_sha256: "e".repeat(64),
            runtime_update_helper_sha256: None,
        },
    }
}
#[async_trait]
impl Provider for HelperFixture {
    fn id(&self) -> ProviderId {
        self.id
    }
    async fn verify_initial_source(&self, id: &str, _: &str, _: &str) -> Result<(), ProviderError> {
        assert_eq!(id, "original-machine");
        self.verifications.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn inspect(&self, _: &str) -> Result<Option<Machine>, ProviderError> {
        unreachable!()
    }
    async fn find_allocation(&self, _: &str) -> Result<Option<Machine>, ProviderError> {
        unreachable!()
    }
    async fn allocate(&self, _: &Allocation<'_>) -> Result<Machine, ProviderError> {
        unreachable!()
    }
    async fn sleep(&self, _: &str) -> Result<ProviderOperation, ProviderError> {
        unreachable!()
    }
    async fn resume(&self, _: &str, _: u32) -> Result<ProviderOperation, ProviderError> {
        unreachable!()
    }
    async fn delete(&self, _: &str) -> Result<ProviderOperation, ProviderError> {
        unreachable!()
    }
    async fn operation(&self, _: &ProviderOperation) -> Result<OperationState, ProviderError> {
        unreachable!()
    }
    async fn execute(&self, id: &str, command: &str) -> Result<ExecResult, ProviderError> {
        assert_eq!(id, "original-machine");
        let request = request(self.id);
        assert_eq!(command, request.command().unwrap());
        let n = self.helper_calls.fetch_add(1, Ordering::SeqCst);
        let (exit_code, stdout) = if n < self.not_ready {
            (
                1,
                json!({"version":1,"status":"rejected","code":"fresh_boot_loader_path_unsafe"}),
            )
        } else {
            let mut value = serde_json::to_value(&request).unwrap();
            match self.answer {
                "ready" => {
                    value["status"] = json!("ready");
                    value["requestSha256"] = json!(request.digest().unwrap());
                    value["unitStartSubmitted"] = json!(true);
                    value["resident"] = json!({"ipcVerified":true,"unbound":true,"childAlive":false,
                        "recoveryAlive":false,"residentPid":42,"residentStartTicks":7,
                        "bootNonceSha256":"f".repeat(64)});
                    (0, value)
                }
                "starting" => {
                    value["status"] = json!("starting");
                    (0, value)
                }
                _ => (
                    1,
                    json!({"version":1,"status":"rejected","code":"fresh_boot_file_hash_mismatch"}),
                ),
            }
        };
        Ok(ExecResult {
            exit_code,
            stdout: serde_json::to_vec(&stdout).unwrap(),
            stderr: vec![],
        })
    }
    async fn upload(&self, _: &str, _: &str, _: &[u8]) -> Result<(), ProviderError> {
        panic!("The fresh boot helper never receives a boot credential")
    }
}
async fn run(
    id: ProviderId,
    not_ready: usize,
    answer: &'static str,
) -> (Result<Option<Value>, ProviderError>, Arc<HelperFixture>) {
    let fixture = Arc::new(HelperFixture {
        id,
        not_ready,
        answer,
        verifications: AtomicUsize::new(0),
        helper_calls: AtomicUsize::new(0),
    });
    let provider = RuntimeProvider::new(fixture.clone(), "default".into());
    let result = provider
        .bootstrap_initial_runtime(
            "original-machine",
            "managed-artifact",
            "machine-name",
            &request(id),
        )
        .await;
    (result, fixture)
}

#[tokio::test(start_paused = true)]
async fn boat_retries_only_the_helper_quickly_until_the_loader_path_is_safe() {
    let started = tokio::time::Instant::now();
    let (result, fixture) = run(ProviderId::Boat, 6, "ready").await;
    let proof = result.unwrap().expect("ready proof");
    assert!(request(ProviderId::Boat).saved(&proof));
    assert_eq!(fixture.helper_calls.load(Ordering::SeqCst), 7);
    assert_eq!(fixture.verifications.load(Ordering::SeqCst), 1);
    // 100+150+250+400+400+400 ms of backoff: sub-second steps, not a 2 s reschedule.
    assert_eq!(started.elapsed(), std::time::Duration::from_millis(1700));
}

#[tokio::test(start_paused = true)]
async fn boat_retry_is_bounded_and_then_reports_pending() {
    let started = tokio::time::Instant::now();
    let (result, fixture) = run(ProviderId::Boat, usize::MAX, "ready").await;
    assert_eq!(result.unwrap(), None);
    assert!(started.elapsed() <= BOAT_INITIAL_START_WINDOW);
    let calls = fixture.helper_calls.load(Ordering::SeqCst);
    assert!((30..=45).contains(&calls), "{calls}");
    assert_eq!(fixture.verifications.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn boat_retries_a_starting_resident_and_never_retries_a_rejection() {
    let (result, fixture) = run(ProviderId::Boat, 0, "starting").await;
    assert_eq!(result.unwrap(), None);
    assert!(fixture.helper_calls.load(Ordering::SeqCst) > 1);
    let (result, fixture) = run(ProviderId::Boat, 2, "rejected").await;
    assert_eq!(result, Err(ProviderError::Rejected));
    assert_eq!(fixture.helper_calls.load(Ordering::SeqCst), 3);
    assert_eq!(fixture.verifications.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn other_providers_keep_one_helper_call_per_attempt() {
    let started = tokio::time::Instant::now();
    let (result, fixture) = run(ProviderId::Boxd, 1, "ready").await;
    assert_eq!(result.unwrap(), None);
    assert_eq!(fixture.helper_calls.load(Ordering::SeqCst), 1);
    assert_eq!(started.elapsed(), std::time::Duration::ZERO);
    let (result, _) = run(ProviderId::Boxd, 0, "ready").await;
    assert!(result.unwrap().is_some());
}
