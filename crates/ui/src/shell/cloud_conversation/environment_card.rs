//! The rows of the cloud computer card, laid out like the previous Zuse
//! workspace summary card: who and what (repository, created, setup), then
//! the machine (environment, status, CPU, memory, disk). A row appears only
//! when its value is known; nothing here guesses.
use super::computer::{ComputerSummary, Tone, preparation_steps};
use serde_json::Value;
use zuse_viewport::CloudPreparationStepState as Step;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Repository {
    pub name: String,
    /// Where the repository opens in a browser, when it has a web address.
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Created {
    /// "3 days ago".
    pub relative: String,
    /// The exact local time, for the row's tooltip.
    pub exact: Option<String>,
}

/// The status row's inline action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StatusAction {
    /// Restart the runtime in place (asks first).
    Restart,
    /// Wake a sleeping computer.
    Wake,
}
impl StatusAction {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Restart => "Restart",
            Self::Wake => "Wake",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Card {
    pub repository: Option<Repository>,
    pub created: Option<Created>,
    pub setup: Option<(&'static str, Tone)>,
    pub environment: String,
    pub status: &'static str,
    pub tone: Tone,
    pub action: Option<StatusAction>,
    /// How the computer last woke, for the status row's tooltip.
    pub last_wake: Option<String>,
    /// What a starting computer is doing now ("Syncing repository").
    pub startup: Option<String>,
    pub cpu: Option<String>,
    pub memory: Option<String>,
    pub disk: Option<String>,
    /// The computer's size, until a live machine sample replaces it.
    pub size: Option<String>,
}

pub(super) struct Inputs<'a> {
    pub summary: &'a ComputerSummary,
    /// The computer record from the account API.
    pub row: &'a Value,
    /// The chat's launch configuration (carries the project's repository).
    pub config: &'a Value,
    /// The machine's last CPU, memory and disk sample.
    pub resources: &'a Value,
    pub now: chrono::DateTime<chrono::Utc>,
    /// The server permits a runtime restart now.
    pub restart_allowed: bool,
}

pub(super) fn card(inputs: Inputs<'_>) -> Card {
    let Inputs {
        summary,
        row,
        config,
        resources,
        now,
        restart_allowed,
    } = inputs;
    let running = summary.tone == Tone::Running;
    let action = if summary.sleeping {
        Some(StatusAction::Wake)
    } else if running && restart_allowed {
        Some(StatusAction::Restart)
    } else {
        None
    };
    let startup = (summary.tone == Tone::Busy)
        .then(|| zuse_viewport::cloud_preparation_progress(row, false))
        .filter(|progress| progress.visible && progress.busy)
        .map(|progress| progress.title);
    let (cpu, memory, disk) = if running {
        machine(resources)
    } else {
        (None, None, None)
    };
    let sampled = cpu.is_some() || memory.is_some() || disk.is_some();
    Card {
        repository: summary.repository.clone().map(|name| Repository {
            name,
            url: repository_url(row).or_else(|| repository_url(config)),
        }),
        created: row["createdAt"]
            .as_i64()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|at| Created {
                relative: crate::pull_requests::relative_time(at, now),
                exact: summary.created.clone(),
            }),
        setup: setup(row),
        // One Cloud option: the infrastructure behind a computer is never shown.
        environment: "Cloud".into(),
        status: summary.label,
        tone: summary.tone,
        action,
        last_wake: summary.last_wake.clone().filter(|_| running),
        startup,
        cpu,
        memory,
        disk,
        size: summary.size.clone().filter(|_| !sampled),
    }
}

/// The project's setup commands: how their last run went.
fn setup(row: &Value) -> Option<(&'static str, Tone)> {
    let step = preparation_steps(row)
        .into_iter()
        .find(|step| step.key == "environment")?;
    Some(match step.state {
        Step::Completed => ("Setup succeeded", Tone::Running),
        Step::Running => ("Running setup…", Tone::Busy),
        Step::Failed => ("Setup failed", Tone::Failed),
        Step::Pending => ("Waiting to run setup", Tone::Idle),
    })
}

/// A web address for the repository: its own URL, or one derived from a
/// `github.com/owner/name` identity. Never a non-web scheme.
fn repository_url(row: &Value) -> Option<String> {
    if let Some(url) = row["repositoryUrl"].as_str() {
        let url = url.trim().trim_end_matches(".git");
        if url.starts_with("https://") || url.starts_with("http://") {
            return Some(url.to_owned());
        }
    }
    let identity = row["repositoryIdentity"].as_str()?.trim();
    let path = identity.strip_prefix("github.com/")?;
    (path.split('/').filter(|part| !part.is_empty()).count() == 2)
        .then(|| format!("https://github.com/{}", path.trim_end_matches(".git")))
}

/// "7 cores · 2.3% used", "16.0 GB · 2.2% used", "63.9 GB · 84.2% used".
fn machine(resources: &Value) -> (Option<String>, Option<String>, Option<String>) {
    let percent = |used: f64, total: f64| {
        if total > 0.0 {
            used / total * 100.0
        } else {
            0.0
        }
    };
    let cpu = resources["cpuPercent"]
        .as_f64()
        .map(|used| match resources["cores"].as_u64() {
            Some(cores) => format!(
                "{cores} {} · {used:.1}% used",
                if cores == 1 { "core" } else { "cores" }
            ),
            None => format!("{used:.1}% used"),
        });
    let bytes = |used: &str, total: &str| {
        let used = resources[used].as_f64()?;
        let total = resources[total].as_f64().filter(|total| *total > 0.0)?;
        Some(format!(
            "{:.1} GB · {:.1}% used",
            total / 1e9,
            percent(used, total)
        ))
    };
    (
        cpu,
        bytes("memoryUsedBytes", "memoryTotalBytes"),
        bytes("diskUsedBytes", "diskTotalBytes"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn summary(label: &'static str, tone: Tone) -> ComputerSummary {
        ComputerSummary {
            label,
            tone,
            sleeping: label == "Sleeping",
            repository: Some("zuse".into()),
            branch: None,
            created: Some("Nov 14, 9:30 AM".into()),
            size: Some("Large".into()),
            last_wake: Some("Woke from memory · 1.2 s".into()),
        }
    }
    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp_millis(1_763_400_000_000).unwrap()
    }
    fn build(summary: &ComputerSummary, row: &Value, resources: &Value, restart: bool) -> Card {
        card(Inputs {
            summary,
            row,
            config: &json!({}),
            resources,
            now: now(),
            restart_allowed: restart,
        })
    }

    #[test]
    fn running_computer_shows_identity_setup_and_machine_rows() {
        let row = json!({
            "state": "ready",
            "createdAt": 1_763_400_000_000i64 - 3 * 86_400_000,
            "repositoryIdentity": "github.com/opcodehq/zuse",
            "preparation": {"environment": {"state": "completed"}},
        });
        let resources = json!({
            "cpuPercent": 2.34, "cores": 7,
            "memoryUsedBytes": 352_000_000u64, "memoryTotalBytes": 16_000_000_000u64,
            "diskUsedBytes": 53_800_000_000u64, "diskTotalBytes": 63_900_000_000u64,
        });
        let card = build(&summary("Running", Tone::Running), &row, &resources, true);
        assert_eq!(
            card.repository,
            Some(Repository {
                name: "zuse".into(),
                url: Some("https://github.com/opcodehq/zuse".into()),
            })
        );
        let created = card.created.unwrap();
        assert_eq!(created.relative, "3 days ago");
        assert_eq!(created.exact.as_deref(), Some("Nov 14, 9:30 AM"));
        assert_eq!(card.setup, Some(("Setup succeeded", Tone::Running)));
        assert_eq!(card.environment, "Cloud");
        assert!(!zuse_protocol::cloud_billing::plans::names_infrastructure(
            &card.environment
        ));
        assert_eq!(card.status, "Running");
        assert_eq!(card.action, Some(StatusAction::Restart));
        assert_eq!(card.startup, None);
        assert_eq!(card.cpu.as_deref(), Some("7 cores · 2.3% used"));
        assert_eq!(card.memory.as_deref(), Some("16.0 GB · 2.2% used"));
        assert_eq!(card.disk.as_deref(), Some("63.9 GB · 84.2% used"));
        assert_eq!(card.size, None, "a live sample replaces the size");
        assert_eq!(card.last_wake.as_deref(), Some("Woke from memory · 1.2 s"));
    }

    #[test]
    fn rows_without_data_are_omitted() {
        let mut bare = summary("Running", Tone::Running);
        bare.repository = None;
        bare.created = None;
        let card = build(&bare, &json!({"state": "ready"}), &json!(null), false);
        assert_eq!(card.repository, None);
        assert_eq!(card.created, None);
        assert_eq!(card.setup, None, "no setup receipt, no setup row");
        assert_eq!(card.environment, "Cloud");
        assert_eq!(card.action, None, "restart needs the server's permission");
        assert_eq!((card.cpu, card.memory, card.disk), (None, None, None));
        assert_eq!(card.size.as_deref(), Some("Large"));
    }

    #[test]
    fn starting_computer_shows_its_startup_phase_and_setup_progress() {
        let row = json!({
            "state": "setup",
            "startupPhase": "syncing-repository",
            "preparation": {"repository": {"state": "completed"}, "environment": {"state": "running"}},
        });
        let resources = json!({"cpuPercent": 50.0, "cores": 2});
        let card = build(&summary("Starting…", Tone::Busy), &row, &resources, true);
        assert_eq!(card.startup.as_deref(), Some("Preparing environment"));
        assert_eq!(card.setup, Some(("Running setup…", Tone::Busy)));
        assert_eq!(card.action, None, "no restart while starting");
        assert_eq!(card.cpu, None, "machine rows only while running");
        assert_eq!(card.last_wake, None);
    }

    #[test]
    fn sleeping_computer_offers_wake_and_failed_setup_reads_as_failed() {
        let row = json!({
            "state": "paused",
            "preparation": {"environment": {"state": "failed"}},
        });
        let card = build(&summary("Sleeping", Tone::Idle), &row, &json!(null), true);
        assert_eq!(card.action, Some(StatusAction::Wake));
        assert_eq!(card.setup, Some(("Setup failed", Tone::Failed)));
        assert_eq!(card.startup, None);
    }

    #[test]
    fn repository_links_are_web_addresses_only() {
        assert_eq!(
            repository_url(&json!({"repositoryUrl": "https://github.com/a/b.git"})).as_deref(),
            Some("https://github.com/a/b")
        );
        assert_eq!(
            repository_url(&json!({"repositoryUrl": "git@github.com:a/b.git"})),
            None
        );
        assert_eq!(
            repository_url(&json!({"repositoryIdentity": "gitlab.com/a/b"})),
            None
        );
        assert_eq!(
            repository_url(&json!({"repositoryIdentity": "github.com/a"})),
            None
        );
    }
}
