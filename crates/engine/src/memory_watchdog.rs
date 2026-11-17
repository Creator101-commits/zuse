//! Stops the one agent that is exhausting memory before the OS stops
//! everything.
//!
//! An agent's build or test run can take most of the machine's memory (a few
//! parallel `cargo build`s took 25 GB of 30). The kernel then swaps until the
//! desktop freezes, and systemd-oomd eventually kills the cgroup holding the
//! memory, which for agents sharing the app's cgroup was the whole app.
//! Agents now run in their own scopes (`zuse_providers::process::scope`), so
//! oomd would pick the agent; this watchdog acts earlier and more gently:
//! when memory pressure is sustained it stops the agent using the most
//! memory, kills what it started, and tells the user in that chat.
//!
//! Linux only: it reads PSI (`/proc/pressure/memory`), `/proc/meminfo`, and
//! each agent's cgroup or process tree.

use std::time::Duration;

/// How often pressure is sampled.
pub(crate) const INTERVAL: Duration = Duration::from_secs(2);
/// Consecutive pressured samples before acting (about 6 s; oomd waits 20 s
/// at 50 %, so this acts first).
const SUSTAINED_SAMPLES: u32 = 3;
/// PSI "some avg10" (percent of time a task stalled on memory) that counts
/// as pressured, when available memory is also low.
const PRESSURE_PERCENT: f64 = 25.0;
/// Available memory below this fraction of RAM, with pressure, is pressured.
const LOW_AVAILABLE: f64 = 0.10;
/// Available memory below this fraction acts at once, with at least
/// `CRITICAL_PRESSURE_PERCENT` stall (or no PSI to check).
const CRITICAL_AVAILABLE: f64 = 0.03;
const CRITICAL_PRESSURE_PERCENT: f64 = 10.0;
/// An agent must use at least this share of RAM (and `MIN_VICTIM_BYTES`) to
/// be stopped: when something outside Zuse is the cause, a modest agent is
/// left alone.
const MIN_VICTIM_SHARE: f64 = 0.15;
const MIN_VICTIM_BYTES: u64 = 1536 << 20;
/// After stopping one agent, give memory time to recover before another.
pub(crate) const COOLDOWN: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sample {
    /// PSI `some avg10`, percent. `None` when PSI is unavailable.
    pub pressure: Option<f64>,
    pub available: u64,
    pub total: u64,
}

impl Sample {
    #[cfg(target_os = "linux")]
    pub fn read() -> Option<Self> {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        let (total, available) = parse_meminfo(&meminfo)?;
        let pressure = std::fs::read_to_string("/proc/pressure/memory")
            .ok()
            .and_then(|psi| parse_psi_some_avg10(&psi));
        Some(Self {
            pressure,
            available,
            total,
        })
    }

    fn available_share(&self) -> f64 {
        self.available as f64 / self.total.max(1) as f64
    }

    fn critical(&self) -> bool {
        // MemAvailable alone can read near zero on a healthy machine (ZFS's
        // ARC, large hugepage reservations aren't counted as available), so
        // where PSI exists tasks must actually be stalling too.
        self.available_share() < CRITICAL_AVAILABLE
            && self.pressure.is_none_or(|p| p >= CRITICAL_PRESSURE_PERCENT)
    }

    fn pressured(&self) -> bool {
        self.critical()
            || (self.available_share() < LOW_AVAILABLE
                && self.pressure.is_some_and(|p| p >= PRESSURE_PERCENT))
    }
}

/// Debounces samples into a decision to act.
#[derive(Default)]
pub(crate) struct Detector {
    streak: u32,
}

impl Detector {
    /// True when this sample, with the ones before it, warrants stopping an agent.
    pub fn observe(&mut self, sample: &Sample) -> bool {
        if !sample.pressured() {
            self.streak = 0;
            return false;
        }
        self.streak += 1;
        sample.critical() || self.streak >= SUSTAINED_SAMPLES
    }

    pub fn reset(&mut self) {
        self.streak = 0;
    }
}

/// The agent to stop among `(key, bytes)` candidates, if any is big enough
/// to be the cause.
pub(crate) fn pick_victim<K: Clone>(candidates: &[(K, u64)], total: u64) -> Option<(K, u64)> {
    let floor = MIN_VICTIM_BYTES.max((total as f64 * MIN_VICTIM_SHARE) as u64);
    candidates
        .iter()
        .filter(|(_, bytes)| *bytes >= floor)
        .max_by_key(|(_, bytes)| *bytes)
        .cloned()
}

fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
            .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
            .map(|kb| kb * 1024)
    };
    Some((field("MemTotal")?, field("MemAvailable")?))
}

fn parse_psi_some_avg10(text: &str) -> Option<f64> {
    text.lines()
        .find_map(|l| l.strip_prefix("some "))?
        .split_whitespace()
        .find_map(|kv| kv.strip_prefix("avg10="))?
        .parse()
        .ok()
}

/// Memory an agent holds: anonymous memory plus what it pushed to swap,
/// which is what reclaim can't simply drop. Read from the agent's own scope
/// when it has one (covers processes that escaped its tree), otherwise
/// summed over its process tree.
#[cfg(target_os = "linux")]
pub(crate) fn agent_bytes(pid: u32) -> u64 {
    if let Some(dir) = zuse_providers::process::scope::cgroup_of(pid) {
        let anon = std::fs::read_to_string(dir.join("memory.stat"))
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("anon ")?.trim().parse::<u64>().ok())
            });
        if let Some(anon) = anon {
            let swap = std::fs::read_to_string(dir.join("memory.swap.current"))
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0);
            return anon + swap;
        }
    }
    tree_bytes(pid)
}

#[cfg(target_os = "linux")]
fn tree_bytes(root: u32) -> u64 {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return 0;
    };
    let table: Vec<(u32, u32)> = entries
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            // Fields after the parenthesised command name: state, ppid, ...
            let ppid = stat
                .rsplit_once(')')?
                .1
                .split_whitespace()
                .nth(1)?
                .parse()
                .ok()?;
            Some((pid, ppid))
        })
        .collect();
    let mut tree = vec![root];
    let mut i = 0;
    while i < tree.len() {
        let parent = tree[i];
        tree.extend(
            table
                .iter()
                .filter(|(_, pp)| *pp == parent)
                .map(|(p, _)| *p),
        );
        i += 1;
    }
    tree.iter()
        .filter_map(|pid| std::fs::read_to_string(format!("/proc/{pid}/status")).ok())
        .map(|status| process_bytes(&status))
        .sum()
}

fn process_bytes(status: &str) -> u64 {
    let kb = |name: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
            .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
            .unwrap_or(0)
    };
    (kb("RssAnon") + kb("VmSwap")) * 1024
}

/// The running watchdog; stops when dropped with its engine.
pub struct MemoryWatchdog(tokio::task::JoinHandle<()>);

impl Drop for MemoryWatchdog {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl MemoryWatchdog {
    /// Watch memory for this engine's agents. `None` off Linux, outside a
    /// Tokio runtime, or with `ZUSE_NO_MEMORY_WATCHDOG` set.
    pub(crate) fn start(sessions: crate::SessionsEngine, docs: crate::DocHost) -> Option<Self> {
        if !cfg!(target_os = "linux") || std::env::var_os("ZUSE_NO_MEMORY_WATCHDOG").is_some() {
            return None;
        }
        let runtime = tokio::runtime::Handle::try_current().ok()?;
        Some(Self(runtime.spawn(watch(sessions, docs))))
    }
}

#[cfg(not(target_os = "linux"))]
async fn watch(_sessions: crate::SessionsEngine, _docs: crate::DocHost) {}

#[cfg(target_os = "linux")]
async fn watch(sessions: crate::SessionsEngine, docs: crate::DocHost) {
    let mut detector = Detector::default();
    let mut tick = tokio::time::interval(INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let Some(sample) = Sample::read() else {
            return; // No /proc/meminfo: nothing to watch.
        };
        if !detector.observe(&sample) {
            continue;
        }
        detector.reset();
        let agents = sessions.agent_processes();
        if agents.is_empty() {
            continue;
        }
        // /proc walks block; keep them off the async workers.
        let Ok(measured) = tokio::task::spawn_blocking(move || {
            agents
                .into_iter()
                .map(|(chat, pid)| ((chat, pid), agent_bytes(pid)))
                .collect::<Vec<_>>()
        })
        .await
        else {
            continue;
        };
        let Some(((chat, pid), bytes)) = pick_victim(&measured, sample.total) else {
            tracing::debug!(
                available = sample.available,
                pressure = ?sample.pressure,
                "memory pressure, but no agent holds enough to be the cause"
            );
            continue;
        };
        tracing::warn!(
            chat = %chat,
            pid,
            bytes,
            available = sample.available,
            pressure = ?sample.pressure,
            "stopping agent: the system is running out of memory"
        );
        let notice = format!(
            "Zuse stopped this agent because it and the processes it started were using {} \
             and the computer was running out of memory. Send a message to continue; to avoid \
             this, ask for fewer parallel jobs (for example `cargo build -j 4`).",
            format_gb(bytes)
        );
        // Resolved now: once the agent exits its /proc entry is gone.
        let scope = zuse_providers::process::scope::cgroup_of(pid);
        if let Err(error) = docs.stop_for_memory(&chat, &notice).await {
            tracing::warn!(chat = %chat, %error, "memory watchdog could not stop the agent");
        }
        // Whatever the agent left behind in its own scope (a build that
        // detached from its process tree) goes too.
        if let Some(scope) = scope {
            let _ = std::fs::write(scope.join("cgroup.kill"), "1");
        }
        tokio::time::sleep(COOLDOWN).await;
    }
}

/// "18.4 GB" for the notice shown in the chat.
pub(crate) fn format_gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    fn sample(pressure: f64, available_gb: u64) -> Sample {
        Sample {
            pressure: Some(pressure),
            available: available_gb * GB,
            total: 30 * GB,
        }
    }

    #[test]
    fn parses_proc_files() {
        let meminfo = "MemTotal:       31700508 kB\nMemFree: 1 kB\nMemAvailable:   25996080 kB\n";
        assert_eq!(
            parse_meminfo(meminfo),
            Some((31700508 * 1024, 25996080 * 1024))
        );
        let psi = "some avg10=42.50 avg60=0.00 avg300=1.01 total=32005176\nfull avg10=9.00 avg60=0.00 avg300=0.97 total=1\n";
        assert_eq!(parse_psi_some_avg10(psi), Some(42.5));
        let status = "Name:\trustc\nRssAnon:\t  2048 kB\nRssFile:\t 999 kB\nVmSwap:\t  1024 kB\n";
        assert_eq!(process_bytes(status), 3072 * 1024);
    }

    #[test]
    fn acts_only_on_sustained_pressure_with_low_memory() {
        let mut d = Detector::default();
        // High pressure with plenty available (e.g. a burst of page-cache
        // reclaim) is not a reason to stop anyone.
        for _ in 0..10 {
            assert!(!d.observe(&sample(80.0, 12)));
        }
        // Low memory and pressure: acts on the third sample in a row.
        assert!(!d.observe(&sample(40.0, 2)));
        assert!(!d.observe(&sample(40.0, 2)));
        assert!(d.observe(&sample(40.0, 2)));
        // One calm sample restarts the count.
        d.reset();
        assert!(!d.observe(&sample(40.0, 2)));
        assert!(!d.observe(&sample(1.0, 2)));
        assert!(!d.observe(&sample(40.0, 2)));
    }

    /// Live: memory a grandchild allocates is charged to the agent's root.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn agent_memory_includes_its_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let ready = dir.path().join("ready");
        let script = format!(
            "python3 -c 'b = bytearray(300 << 20); open(\"{}\", \"w\").write(\"1\"); import time; time.sleep(30)' & wait",
            ready.display()
        );
        let mut parent = tokio::process::Command::new("sh")
            .args(["-c", &script])
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let group = parent.id().unwrap() as i32;
        struct Reap(i32);
        impl Drop for Reap {
            fn drop(&mut self) {
                // SAFETY: SIGKILL to the fixture's own process group.
                unsafe { libc::kill(-self.0, libc::SIGKILL) };
            }
        }
        let _reap = Reap(group);
        for _ in 0..250 {
            if ready.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(ready.exists());
        let bytes = agent_bytes(parent.id().unwrap());
        assert!(bytes >= 300 << 20, "{bytes}");
    }

    #[test]
    fn nearly_out_of_memory_acts_at_once() {
        let mut d = Detector::default();
        // Low "available" but nothing stalling (ZFS ARC holds the rest): no.
        assert!(!d.observe(&sample(0.0, 0)));
        assert!(!d.observe(&sample(5.0, 0)));
        // Stalling: at once.
        assert!(d.observe(&sample(15.0, 0)));
        // No PSI on this kernel: available memory decides.
        let s = Sample {
            pressure: None,
            available: GB / 2,
            total: 30 * GB,
        };
        assert!(d.observe(&s));
    }

    #[test]
    fn stops_the_biggest_agent_only_if_it_is_the_likely_cause() {
        let total = 30 * GB;
        let agents = [("a", GB / 3), ("b", 18 * GB), ("c", 6 * GB)];
        assert_eq!(pick_victim(&agents, total), Some(("b", 18 * GB)));
        // Memory is going to something else; no agent holds 15 % of RAM.
        let small = [("a", GB / 3), ("c", 2 * GB)];
        assert_eq!(pick_victim(&small, total), None);
        assert_eq!(pick_victim::<&str>(&[], total), None);
    }
}
