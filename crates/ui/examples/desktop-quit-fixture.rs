//! Exercise the platform's real quit callback against an isolated profile.
use gpui::App;
use std::{path::PathBuf, time::Duration};
use zuse_engine::{InstanceLock, desktop_shutdown::DesktopShutdown};

fn main() {
    let profile = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("isolated fixture profile"),
    );
    std::fs::create_dir_all(&profile).unwrap();
    let lock = std::sync::Arc::new(std::sync::Mutex::new(Some(
        InstanceLock::acquire(&profile).unwrap(),
    )));
    let runtime = zuse_engine::runtime().unwrap();
    let handle = runtime.handle().clone();
    let shutdown = DesktopShutdown::default();
    let owner = shutdown.clone();
    gpui_platform::application().run(move |cx: &mut App| {
        cx.on_app_quit(move |_| {
            let lock = lock.lock().unwrap().take();
            let snapshot = profile.join("saved-before-termination");
            let completion = owner.start(&handle, async move {
                // Longer than GPUI's quit deadline, like a real engine drain.
                tokio::time::sleep(Duration::from_secs(1)).await;
                std::fs::write(snapshot, "settled").unwrap();
                drop(lock);
            });
            let termination = owner.before_termination(&handle, Duration::from_secs(3));
            async move {
                let _ = completion.await;
                drop(termination);
            }
        })
        .detach();
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            // On macOS this takes the same NSApplication terminate: path as Cmd+Q.
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
    // AppKit exits during termination, so this cannot be the only drain path.
    shutdown.finish(runtime, Duration::from_secs(3), Duration::from_millis(100));
}
