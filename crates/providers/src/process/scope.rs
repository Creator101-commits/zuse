//! Linux: each agent CLI runs in its own transient systemd scope.
//!
//! Without this, an agent and everything it starts (shells, `cargo build`,
//! test runners) are charged to the app's own cgroup. Under memory pressure
//! systemd-oomd kills the biggest leaf cgroup under the user manager, and
//! that was the whole app: every chat, the UI and the engine, because one
//! agent's build used 20+ GB. Once each agent has its own scope, oomd (and
//! the engine's memory watchdog, which reads the same cgroup) charge the
//! memory to the agent that used it and stop only that agent.
//!
//! The scope is a child of `app-zuse.slice` and is bound to the unit the
//! engine runs in (`BindsTo=`), so stopping or killing the app still stops
//! its agents, as it did when they shared its cgroup.
//!
//! Best effort: no user systemd (macOS, containers, an engine started outside
//! a user session, `busctl` missing) leaves the agent where it was.

/// Slice that groups every agent scope.
const SLICE: &str = "app-zuse.slice";
/// Unit-name prefix the watchdog uses to recognise an agent scope.
pub(crate) const UNIT_PREFIX: &str = "zuse-agent-";

/// Move `pid` (a freshly spawned agent) into a new scope of its own. Returns
/// once systemd has accepted the request, or immediately when there is no
/// user manager to ask.
pub async fn adopt(pid: u32) {
    use std::sync::atomic::{AtomicBool, Ordering};
    // Set once busctl is missing or the bus doesn't answer: later agents
    // shouldn't each wait on it.
    static UNAVAILABLE: AtomicBool = AtomicBool::new(false);
    if UNAVAILABLE.load(Ordering::Relaxed) {
        return;
    }
    let Some(owner) = owner_unit() else { return };
    let unit = format!(
        "{UNIT_PREFIX}{pid}-{}.scope",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let pid = pid.to_string();
    let mut args: Vec<&str> = vec![
        "--user",
        "--quiet",
        "call",
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
        "StartTransientUnit",
        "ssa(sv)a(sa(sv))",
        &unit,
        "fail",
    ];
    let mut props: Vec<[&str; 4]> = vec![
        ["PIDs", "au", "1", &pid],
        ["Slice", "s", SLICE, ""],
        ["CollectMode", "s", "inactive-or-failed", ""],
        ["Description", "s", "Zuse agent", ""],
    ];
    if let Some(owner) = owner.as_deref() {
        props.push(["BindsTo", "as", "1", owner]);
        props.push(["After", "as", "1", owner]);
    }
    let count = props.len().to_string();
    args.push(&count);
    for prop in &props {
        args.extend(prop.iter().copied().filter(|v| !v.is_empty()));
    }
    args.push("0"); // no auxiliary units
    let result = tokio::process::Command::new("busctl")
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(std::time::Duration::from_secs(1), result).await {
        Ok(Ok(out)) if out.status.success() => {}
        Ok(Ok(out)) => tracing::debug!(
            unit,
            stderr = %String::from_utf8_lossy(&out.stderr).trim(),
            "agent scope not created"
        ),
        Ok(Err(e)) => {
            UNAVAILABLE.store(true, Ordering::Relaxed);
            tracing::debug!(unit, "agent scopes disabled: {e}");
        }
        Err(_) => {
            UNAVAILABLE.store(true, Ordering::Relaxed);
            tracing::debug!(unit, "agent scopes disabled: busctl timed out");
        }
    }
}

/// `Some(owner)` when this process is managed by the user's systemd
/// instance, where `owner` is the unit to bind agent scopes to (`None` when
/// that unit can't be named). `None` (outer) means: don't create scopes.
fn owner_unit() -> Option<Option<String>> {
    static OWNER: std::sync::OnceLock<Option<Option<String>>> = std::sync::OnceLock::new();
    OWNER
        .get_or_init(|| {
            if std::env::var_os("ZUSE_NO_AGENT_SCOPES").is_some() {
                return None;
            }
            let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
            // SAFETY: getuid has no preconditions and cannot fail.
            owner_from_cgroup(&cgroup, unsafe { libc::getuid() })
        })
        .clone()
}

fn owner_from_cgroup(cgroup: &str, uid: u32) -> Option<Option<String>> {
    // cgroup v2 only: a single "0::/path" line.
    let path = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?;
    let manager = format!("/user@{uid}.service/");
    let (_, inside) = path.split_once(&manager)?;
    let leaf = inside.rsplit('/').next()?;
    let unit = (leaf.ends_with(".scope") || leaf.ends_with(".service")).then(|| leaf.to_string());
    Some(unit)
}

/// The agent scope `pid` runs in, as a cgroup directory, if it has one.
pub fn cgroup_of(pid: u32) -> Option<std::path::PathBuf> {
    let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let path = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?;
    let leaf = path.rsplit('/').next()?;
    leaf.starts_with(UNIT_PREFIX)
        .then(|| std::path::Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_is_the_leaf_unit_under_the_user_manager() {
        let desktop = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-gnome-zuse\\x2dpreview-5169.scope\n";
        assert_eq!(
            owner_from_cgroup(desktop, 1000),
            Some(Some("app-gnome-zuse\\x2dpreview-5169.scope".into()))
        );
        let service =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/zuse-serve.service\n";
        assert_eq!(
            owner_from_cgroup(service, 1000),
            Some(Some("zuse-serve.service".into()))
        );
    }

    /// Live: a spawned process ends up in its own `zuse-agent-*` scope.
    /// Skips where there is no user systemd to ask (CI containers).
    #[tokio::test]
    async fn adopted_process_runs_in_its_own_scope() {
        if owner_unit().is_none() {
            return;
        }
        let mut child = tokio::process::Command::new("sleep")
            .arg("30")
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        assert_eq!(cgroup_of(pid), None);
        adopt(pid).await;
        let mut scope = None;
        for _ in 0..100 {
            scope = cgroup_of(pid);
            if scope.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let scope = scope.expect("moved into an agent scope");
        assert!(
            scope.to_string_lossy().contains("/app-zuse.slice/"),
            "{scope:?}"
        );
        // The watchdog's last resort ends everything in the scope.
        std::fs::write(scope.join("cgroup.kill"), "1").unwrap();
        let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(!status.success());
    }

    #[test]
    fn no_scopes_outside_the_user_manager() {
        // An ssh session or a system service: the user manager can't bind to it.
        assert_eq!(
            owner_from_cgroup("0::/user.slice/user-1000.slice/session-3.scope\n", 1000),
            None
        );
        assert_eq!(
            owner_from_cgroup("0::/system.slice/zuse.service\n", 1000),
            None
        );
        // Another user's manager.
        assert_eq!(
            owner_from_cgroup(
                "0::/user.slice/user-1001.slice/user@1001.service/app.slice/x.scope\n",
                1000
            ),
            None
        );
        // cgroup v1 hybrid without a unified line.
        assert_eq!(
            owner_from_cgroup("1:name=systemd:/user.slice\n", 1000),
            None
        );
    }
}
