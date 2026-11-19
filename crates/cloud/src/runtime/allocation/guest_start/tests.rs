use super::*;
use crate::managed_bases::bootstrap::ImagePins;
use base64::engine::general_purpose::STANDARD;

fn request() -> Request {
    Request {
        version: 1,
        provider: zuse_protocol::cloud_computers::ProviderId::Boat,
        purpose: Purpose::InitialAllocation,
        operation_id: "op-one".into(),
        workspace_id: "workspace-one".into(),
        generation: 1,
        storage_incarnation_id: "disk-one".into(),
        source_machine_id: "machine-one".into(),
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
fn ready(request: &Request) -> Value {
    let mut value = serde_json::to_value(request).unwrap();
    value["status"] = json!("ready");
    value["requestSha256"] = json!(request.digest().unwrap());
    value["unitStartSubmitted"] = json!(true);
    value["resident"] = json!({"ipcVerified":true,"unbound":true,"childAlive":false,
        "recoveryAlive":false,"residentPid":42,"residentStartTicks":7,"bootNonceSha256":"f".repeat(64)});
    value
}

#[test]
fn one_command_carries_the_unchanged_loader_and_request_but_never_the_credential() {
    let request = request();
    let command = command(&request, "launch-lines", "/tmp/zuse-rust-boot-op-one").unwrap();
    assert!(command.starts_with("sudo -n /usr/bin/python3 -I -B -c '"));
    assert!(!command.contains("private-boot-credential"));
    let encoded = command.rsplit(' ').next().unwrap().trim_matches('\'');
    let args: Value = serde_json::from_slice(&STANDARD.decode(encoded).unwrap()).unwrap();
    assert_eq!(args["loader"], LOADER);
    assert_eq!(args["starter"], STARTER);
    assert_eq!(args["request"], request.encoded().unwrap());
    assert_eq!(args["helperSha256"], "e".repeat(64));
    assert_eq!(args["staged"], "/tmp/zuse-rust-boot-op-one");
    assert_eq!(args["launch"], "launch-lines");
    assert_eq!(args["deadlineSeconds"], GUEST_DEADLINE_SECONDS);
    assert_eq!(args["launchSeconds"], GUEST_LAUNCH_SECONDS);
    assert_eq!(
        credential_path("op-one"),
        "/run/zuse-fresh-start/op-one/boot"
    );
}

#[test]
fn the_last_status_read_stops_the_waiting_unit_and_removes_its_credential() {
    // The same name the starter gives the unit (sha256("op-one")[:16]).
    assert_eq!(unit_name("op-one"), "zuse-fresh-start-6c2c7720771a4535");
    let read = status_command("op-one", false);
    assert!(!read.contains("systemctl"));
    assert!(read.contains("'/run/zuse-fresh-start/op-one/status.json'"));
    let last = status_command("op-one", true);
    let stop = last.find("systemctl stop").unwrap();
    let confirm = last.find("ActiveState").unwrap();
    let remove = last.find("rm -f").unwrap();
    let cat = last.find("cat --").unwrap();
    assert!(stop < confirm && confirm < remove && remove < cat, "{last}");
    assert!(last.contains("exit 75"));
    assert!(last.ends_with(
        " sh '/run/zuse-fresh-start/op-one/status.json' '/run/zuse-fresh-start/op-one/boot' 'zuse-fresh-start-6c2c7720771a4535'"
    ));
    // The bounds the worker waits out cover every in-computer timer.
    assert!(SETTLED_AFTER_MS > (GUEST_DEADLINE_SECONDS + GUEST_LAUNCH_SECONDS) * 1000);
    assert!(SETTLED_AFTER_MS > FALLBACK_AFTER_MS);
}

#[test]
fn dispatch_reply_retries_only_transient_refusals() {
    assert_eq!(dispatch_reply(0, br#"{"status":"accepted"}"#), Ok(()));
    assert_eq!(
        dispatch_reply(0, br#"{"status":"accepted","existing":"waiting"}"#),
        Ok(())
    );
    for (code, expected) in [
        ("fresh_start_credential_missing", ProviderError::Unavailable),
        ("fresh_start_unit_failed", ProviderError::Unavailable),
        ("fresh_start_unit_uncertain", ProviderError::Unavailable),
        ("fresh_start_run_unsafe", ProviderError::Rejected),
        ("fresh_start_request_invalid", ProviderError::Rejected),
    ] {
        let body = json!({"version":1,"status":"rejected","code":code}).to_string();
        assert_eq!(dispatch_reply(1, body.as_bytes()), Err(expected), "{code}");
    }
    assert_eq!(
        dispatch_reply(0, b"not json"),
        Err(ProviderError::InvalidResponse)
    );
    assert_eq!(
        dispatch_reply(2, br#"{"status":"accepted"}"#),
        Err(ProviderError::InvalidResponse)
    );
}

#[test]
fn fallback_status_is_accepted_only_with_a_server_valid_receipt() {
    let request = request();
    let record = |status: &str, receipt: Value| json!({"version":1,"operationId":"op-one","status":status,"receipt":receipt});
    assert_eq!(
        outcome(&record("launched", ready(&request)), &request),
        Outcome::Launched
    );
    let mut forged = ready(&request);
    forged["resident"]["childAlive"] = json!(true);
    assert_eq!(
        outcome(&record("launched", forged), &request),
        Outcome::Rejected
    );
    let mut other = record("launched", ready(&request));
    other["operationId"] = json!("op-two");
    assert_eq!(outcome(&other, &request), Outcome::Rejected);
    assert_eq!(
        outcome(&record("launching", ready(&request)), &request),
        Outcome::Failed("cloud_guest_start_launch_unfinished")
    );
    assert_eq!(
        outcome(&record("rejected", Value::Null), &request),
        Outcome::Rejected
    );
    for status in ["timeout", "waiting"] {
        assert_eq!(
            outcome(&record(status, Value::Null), &request),
            Outcome::Failed("cloud_guest_start_timeout")
        );
    }
    assert_eq!(
        outcome(&record("failed", Value::Null), &request),
        Outcome::Failed("cloud_guest_start_failed")
    );
    assert_eq!(
        outcome(&json!({}), &request),
        Outcome::Failed("cloud_guest_start_lost")
    );
}

#[test]
fn in_computer_wait_wakes_on_events_falls_back_and_stops_at_the_deadline() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let starter = dir.path().join("managed-base-fresh-start.py");
    std::fs::File::create(&starter)
        .unwrap()
        .write_all(STARTER.as_bytes())
        .unwrap();
    let output = std::process::Command::new("/usr/bin/python3")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args(["-I", "-B", "-c", include_str!("waiter_tests.py")])
        .arg(&starter)
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stdout == b"ok\n",
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
mod server_path;
