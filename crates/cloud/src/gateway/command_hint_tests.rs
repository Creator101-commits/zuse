//! A room tells its Rust runtime "you have a command" when its mailbox moves.
use super::*;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

struct Mailbox {
    changes: tokio::sync::broadcast::Sender<()>,
    revision: AtomicI64,
    reads: AtomicUsize,
}
#[async_trait::async_trait]
impl LeaseControl for Mailbox {
    async fn gateway_lease(&self, _: &str, _: &str, _: (i64, i64)) -> Result<(), ApiError> {
        Ok(())
    }
    async fn release_gateway(&self, _: &str, _: &str) {}
    fn mailbox_changes(&self) -> Option<tokio::sync::broadcast::Receiver<()>> {
        Some(self.changes.subscribe())
    }
    async fn mailbox_revision(&self, workspace: &str) -> Result<i64, ApiError> {
        assert_eq!(workspace, "hinted");
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.revision.load(Ordering::SeqCst))
    }
}
fn mailbox(revision: i64) -> Arc<Mailbox> {
    Arc::new(Mailbox {
        changes: tokio::sync::broadcast::channel(16).0,
        revision: AtomicI64::new(revision),
        reads: AtomicUsize::new(0),
    })
}
fn grant(runtime: bool, protocol: &str) -> Grant {
    Grant {
        workspace: "hinted".into(),
        generation: 1,
        epoch: 1,
        runtime,
        protocol: protocol.into(),
        actor: "owner".into(),
        permission: "edit".into(),
        expires_at: now() + 60_000,
    }
}
fn peer(grant: Grant) -> (Peer, mpsc::Receiver<Buffered>) {
    let (tx, rx) = mpsc::channel(64);
    let peer = Peer {
        grant,
        tx,
        close: watch::Sender::new(None),
        pending_until: None,
        ready: false,
        detached: false,
    };
    (peer, rx)
}
/// Counts hints received within `within`, skipping other controls.
async fn hints(rx: &mut mpsc::Receiver<Buffered>, within: Duration) -> usize {
    let mut count = 0;
    let deadline = tokio::time::Instant::now() + within;
    while let Ok(Some(buffered)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        if let Message::Text(text) = buffered.message {
            let value: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
            if value["type"] == "runtime.command" {
                assert_eq!(value, json!({"type":"runtime.command"}));
                count += 1;
            }
        }
    }
    count
}

#[tokio::test]
async fn runtime_hears_of_new_commands_without_asking() {
    let hub = Hub::default();
    let control = mailbox(7);
    let rust = zuse_protocol::cloud_gateway::RUST_PROTOCOL;
    let session = hub
        .room_with_control(control.clone(), &grant(true, rust))
        .await
        .unwrap();
    let room = session.0.clone();
    let (runtime, mut rx) = peer(grant(true, rust));
    room.attach("runtime".into(), runtime);
    assert_eq!(
        hints(&mut rx, Duration::from_millis(50)).await,
        1,
        "attaching announces push and asks for a lease"
    );
    // The first read records the revision and hints once more.
    control.changes.send(()).unwrap();
    assert_eq!(hints(&mut rx, Duration::from_millis(200)).await, 1);
    // Changes that leave this revision alone (other workspaces) send nothing.
    for _ in 0..5 {
        control.changes.send(()).unwrap();
    }
    assert_eq!(hints(&mut rx, Duration::from_millis(300)).await, 0);
    // A committed command bumps the revision: hinted at once.
    control.revision.store(8, Ordering::SeqCst);
    control.changes.send(()).unwrap();
    assert_eq!(hints(&mut rx, Duration::from_millis(200)).await, 1);
    // A command committed by another process, with no event here, is found
    // by the periodic re-read.
    control.revision.store(9, Ordering::SeqCst);
    assert_eq!(
        hints(&mut rx, COMMAND_HINT_RECHECK + Duration::from_millis(500)).await,
        1
    );
    room.close_all(UNAVAILABLE);
}

#[tokio::test]
async fn older_runtime_protocols_and_unattached_rooms_get_no_hints() {
    let hub = Hub::default();
    let control = mailbox(1);
    let older = zuse_protocol::cloud_gateway::PENDING_PROTOCOL;
    let session = hub
        .room_with_control(control.clone(), &grant(true, older))
        .await
        .unwrap();
    let room = session.0.clone();
    let (client, _client_rx) = peer(grant(false, older));
    room.attach("client".into(), client);
    control.changes.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        control.reads.load(Ordering::SeqCst),
        0,
        "no runtime attached: no database read"
    );
    let (runtime, mut rx) = peer(grant(true, older));
    room.attach("runtime".into(), runtime);
    control.revision.store(2, Ordering::SeqCst);
    control.changes.send(()).unwrap();
    assert_eq!(hints(&mut rx, Duration::from_millis(200)).await, 0);
    room.close_all(UNAVAILABLE);
}
