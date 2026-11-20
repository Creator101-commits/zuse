//! Durable coordinator contracts shared by all application clients.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const CAPABILITY: &str = "zots.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZotRepository {
    pub device_id: String,
    pub space_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZotModel {
    pub harness: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZotConfig {
    pub name: String,
    pub purpose: String,
    pub home_device_id: String,
    pub repositories: Vec<ZotRepository>,
    pub coordinator_model: ZotModel,
    pub worker_device_id: String,
    pub max_concurrent_workers: u32,
    pub max_followups: u32,
    /// Saved pixel character; absent uses one derived from the Zot's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub avatar: Option<ZotAvatar>,
    /// The user's own picture, shown instead of the mascot: a small square
    /// PNG or JPEG as a `data:` URL (see [`AVATAR_IMAGE_MAX_LEN`]). It lives
    /// beside `avatar` so the mascot stays the fallback for clients, Slack
    /// personas and prompt envelopes that never carry image bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub avatar_image: Option<String>,
}

/// Longest accepted `avatar_image`, in bytes of the `data:` URL. Clients crop
/// to a 128px square first, which lands well under this.
pub const AVATAR_IMAGE_MAX_LEN: usize = 96 * 1024;

/// Whether `value` is an avatar image this version knows how to store.
pub fn avatar_image_ok(value: &str) -> bool {
    value.len() <= AVATAR_IMAGE_MAX_LEN
        && ["data:image/png;base64,", "data:image/jpeg;base64,"]
            .iter()
            .any(|prefix| {
                value.strip_prefix(prefix).is_some_and(|data| {
                    !data.is_empty()
                        && data
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
                })
            })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZotAvatar {
    pub mascot: String,
    pub color: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ZotStatus {
    Active,
    Paused,
    Stopped,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZotWorker {
    pub chat_id: String,
    pub device_id: String,
    pub space_id: String,
    pub objective: String,
    pub followups: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZotDecision {
    pub id: String,
    pub worker_chat_id: Option<String>,
    pub question: String,
    pub answer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Zot {
    pub id: String,
    pub revision: u32,
    #[serde(default)]
    pub activity_revision: u32,
    pub config: ZotConfig,
    pub status: ZotStatus,
    pub coordinator_chat_id: String,
    pub previous_coordinator_chats: Vec<String>,
    pub workers: Vec<ZotWorker>,
    pub decisions: Vec<ZotDecision>,
    #[serde(default)]
    pub issues: Vec<ZotIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZotFile {
    pub path: String,
    pub version: u32,
    pub content: String,
}

/// IDs are caller-minted and reused verbatim after a transport failure.
#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZotCommand {
    pub command_id: String,
    pub zot_id: String,
    pub expected_revision: Option<u32>,
    pub action: ZotAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ZotAction {
    Create {
        config: ZotConfig,
    },
    Configure {
        config: ZotConfig,
    },
    SetStatus {
        status: ZotStatus,
    },
    AdoptWorker {
        worker: ZotWorker,
    },
    Delegate {
        /// Atomically link the new thread and mark this issue in progress.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        issue_id: Option<String>,
        worker: ZotWorker,
        model: ZotModel,
        base: String,
    },
    MessageWorker {
        chat_id: String,
        text: String,
        automatic: bool,
    },
    StopWorker {
        chat_id: String,
    },
    ArchiveWorker {
        chat_id: String,
    },
    ReleaseWorker {
        chat_id: String,
    },
    Decide {
        decision: ZotDecision,
    },
    WriteFile {
        path: String,
        content: String,
        expected_version: u32,
    },
    RestoreFile {
        path: String,
        version: u32,
        expected_version: u32,
    },
    CreateIssue {
        issue_id: String,
        details: ZotIssueDetails,
    },
    EditIssue {
        issue_id: String,
        details: ZotIssueDetails,
    },
    LinkIssueThread {
        issue_id: String,
        chat_id: String,
    },
    LinkIssuePullRequest {
        issue_id: String,
        url: String,
        title: String,
    },
    UnlinkIssueThread {
        issue_id: String,
        chat_id: String,
    },
    UnlinkIssuePullRequest {
        issue_id: String,
        url: String,
    },
    CommentIssue {
        issue_id: String,
        text: String,
        /// Activity entry this comment answers; shown nested under it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        reply_to: Option<String>,
    },
    /// Hand an issue to the user with one question: they own it until they
    /// answer, and their answer hands it back.
    AskUser {
        issue_id: String,
        question: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        recommendation: Option<String>,
    },
    /// Close an issue: merged needs a linked pull request, and either way
    /// the note says what happened.
    CloseIssue {
        issue_id: String,
        outcome: ZotIssueOutcome,
        note: String,
    },
    FreshConversation,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZotEvent {
    pub id: String,
    pub zot_id: String,
    pub worker_chat_id: String,
    pub kind: String,
    pub summary: String,
    pub acknowledged: bool,
}

/// Whether a failed command may still have been applied, because the request
/// or its answer was lost in transit. Only then should a retry reuse the
/// command ID, so the engine's receipt makes it idempotent. Any other failure
/// is a definite rejection (e.g. a revision conflict): nothing was written,
/// and the next attempt is a new command at the current revision.
pub fn command_outcome_unknown(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "transport",
        "connection closed",
        "host unavailable",
        "timed out",
        "timeout",
        "disconnected",
    ]
    .iter()
    .any(|signal| error.contains(signal))
}

/// Canonical command schema exposed by both application discovery and MCP.
pub fn command_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ZotCommand)).expect("command schema serializes")
}

/// Work is complete only when explicitly marked done; a finished model turn is
/// evidence for review, never an automatic completion signal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ZotIssueStatus {
    #[default]
    Backlog,
    Todo,
    InProgress,
    /// Waiting on the user: they own the issue until they answer.
    NeedsYou,
    Blocked,
    InReview,
    Done,
    Canceled,
}
impl ZotIssueStatus {
    pub const ALL: [Self; 8] = [
        Self::Backlog,
        Self::Todo,
        Self::InProgress,
        Self::NeedsYou,
        Self::Blocked,
        Self::InReview,
        Self::Done,
        Self::Canceled,
    ];
    /// Board order: what needs the user first, then live work, then queued,
    /// then finished.
    pub const BOARD: [Self; 8] = [
        Self::NeedsYou,
        Self::InProgress,
        Self::InReview,
        Self::Blocked,
        Self::Todo,
        Self::Backlog,
        Self::Done,
        Self::Canceled,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Backlog => "Backlog",
            Self::Todo => "Todo",
            Self::InProgress => "In progress",
            Self::NeedsYou => "Needs you",
            Self::Blocked => "Blocked",
            Self::InReview => "In review",
            Self::Done => "Done",
            Self::Canceled => "Canceled",
        }
    }
    pub fn terminal(self) -> bool {
        matches!(self, Self::Done | Self::Canceled)
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ZotIssuePriority {
    Urgent,
    High,
    #[default]
    Normal,
    Low,
}
impl ZotIssuePriority {
    pub const ALL: [Self; 4] = [Self::Urgent, Self::High, Self::Normal, Self::Low];
    pub fn label(self) -> &'static str {
        match self {
            Self::Urgent => "Urgent",
            Self::High => "High",
            Self::Normal => "Normal",
            Self::Low => "Low",
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZotIssueDetails {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub status: ZotIssueStatus,
    #[serde(default)]
    pub priority: ZotIssuePriority,
    /// The Zapper (or one of its threads) is responsible. Kept in step with
    /// [`ZotIssueDetails::owner`] for clients that predate owners.
    #[serde(default)]
    pub assigned: bool,
    /// ISO calendar date, or absent.
    pub due_date: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Other issue ids in the same Zot.
    #[serde(default)]
    pub relations: Vec<String>,
    /// Who holds the issue now: exactly one owner at a time. Absent on issues
    /// saved before owners, where `assigned` means the Zapper.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub owner: Option<ZotIssueOwner>,
    /// What must be true for the issue to be done.
    #[serde(default)]
    pub done_criteria: String,
    /// What this issue deliberately leaves alone.
    #[serde(default)]
    pub out_of_scope: String,
    /// The request-level issue this one is a shippable part of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub parent: Option<String>,
}
impl ZotIssueDetails {
    /// The current owner, reading issues saved before owners existed.
    pub fn current_owner(&self) -> Option<ZotIssueOwner> {
        self.owner
            .clone()
            .or_else(|| self.assigned.then_some(ZotIssueOwner::Zapper))
    }
    /// Keep `assigned` and the status in step with the owner: the user owning
    /// an open issue means it needs them, and an issue that no longer needs
    /// them goes back to work.
    pub fn normalize(&mut self, has_thread: bool) {
        let owner = self.current_owner();
        self.assigned = matches!(
            owner,
            Some(ZotIssueOwner::Zapper | ZotIssueOwner::Thread { .. })
        );
        if self.status.terminal() {
            return;
        }
        match owner {
            Some(ZotIssueOwner::User) => self.status = ZotIssueStatus::NeedsYou,
            _ if self.status == ZotIssueStatus::NeedsYou => {
                self.status = if has_thread {
                    ZotIssueStatus::InProgress
                } else {
                    ZotIssueStatus::Todo
                }
            }
            _ => {}
        }
    }
}
/// The single owner of an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ZotIssueOwner {
    /// The Zapper itself, while it plans or decides.
    Zapper,
    /// The person the Zapper works for.
    User,
    /// One of the Zapper's threads, while it builds.
    Thread { chat_id: String },
}
/// How an issue closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ZotIssueOutcome {
    /// Its pull request merged.
    Merged,
    /// Deliberately not done.
    Dropped,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZotIssuePullRequest {
    pub url: String,
    pub title: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZotIssueActivity {
    #[serde(default)]
    pub actor: Option<String>,
    pub id: String,
    #[ts(type = "number")]
    pub created_at: i64,
    pub kind: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reply_to: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZotIssue {
    pub id: String,
    pub number: u32,
    pub details: ZotIssueDetails,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    pub threads: Vec<String>,
    pub pull_requests: Vec<ZotIssuePullRequest>,
    pub activity: Vec<ZotIssueActivity>,
}
impl ZotIssue {
    pub fn code(&self) -> String {
        format!("ZAP-{}", self.number)
    }
}
