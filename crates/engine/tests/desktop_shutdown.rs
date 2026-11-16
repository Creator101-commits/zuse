//! Quit-hook cancellation must not strand the engine or its profile lock.
use std::{
    process::Command,
    time::{Duration, Instant},
};

use zuse_engine::{
    InstanceLock,
    desktop_shutdown::{DesktopShutdown, Outcome},
};

#[test]
fn dropping_the_ui_observer_still_drains_and_releases_the_profile() {
    let profile = tempfile::tempdir().unwrap();
    let lock = InstanceLock::acquire(profile.path()).unwrap();
    let snapshot = profile.path().join("saved");
    let saved = snapshot.clone();
    let runtime = zuse_engine::runtime().unwrap();
    let shutdown = DesktopShutdown::default();
    let observer = shutdown.start(runtime.handle(), async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        std::fs::write(saved, "settled turn").unwrap();
        drop(lock);
    });
    // GPUI cancels this observer after its shorter quit-hook deadline.
    drop(observer);
    assert_eq!(
        shutdown.finish(runtime, Duration::from_secs(2), Duration::from_millis(100)),
        Outcome::Complete
    );
    assert_eq!(std::fs::read_to_string(snapshot).unwrap(), "settled turn");
    assert!(InstanceLock::holder(profile.path()).is_none());
    InstanceLock::acquire(profile.path()).unwrap();
}

#[test]
fn a_stalled_drain_has_a_bounded_exit_and_does_not_keep_the_profile() {
    let profile = tempfile::tempdir().unwrap();
    let lock = InstanceLock::acquire(profile.path()).unwrap();
    let runtime = zuse_engine::runtime().unwrap();
    let shutdown = DesktopShutdown::default();
    drop(shutdown.start(runtime.handle(), async move {
        std::future::pending::<()>().await;
        drop(lock);
    }));
    let started = Instant::now();
    assert_eq!(
        shutdown.finish(
            runtime,
            Duration::from_millis(50),
            Duration::from_millis(100)
        ),
        Outcome::TimedOut
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    InstanceLock::acquire(profile.path()).unwrap();
}

#[test]
fn blocking_worker_child() {
    let Some(profile) = std::env::var_os("ZUSE_SHUTDOWN_TEST_PROFILE") else {
        return;
    };
    let profile = std::path::PathBuf::from(profile);
    let lock = InstanceLock::acquire(&profile).unwrap();
    let runtime = zuse_engine::runtime().unwrap();
    let (ready, started) = std::sync::mpsc::channel();
    runtime.spawn_blocking(move || {
        ready.send(()).unwrap();
        // A stuck provider/filesystem operation cannot be cancelled by Tokio.
        loop {
            std::thread::sleep(Duration::from_secs(60));
            std::hint::black_box(&lock);
        }
    });
    started.recv_timeout(Duration::from_secs(2)).unwrap();
    DesktopShutdown::default().finish(
        runtime,
        Duration::from_millis(50),
        Duration::from_millis(50),
    );
    std::fs::write(profile.join("returned"), "runtime stopped waiting").unwrap();
}

#[test]
fn a_stuck_blocking_worker_cannot_leave_a_closed_desktop_process_owning_the_profile() {
    let profile = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "blocking_worker_child", "--nocapture"])
        .env("ZUSE_SHUTDOWN_TEST_PROFILE", profile.path())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("closed desktop remained alive waiting for a blocking worker");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
    assert!(profile.path().join("returned").is_file());
    InstanceLock::acquire(profile.path()).unwrap();
}
