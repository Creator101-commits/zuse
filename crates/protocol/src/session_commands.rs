//! Canonical session command payloads shared by transport, engine and clients.
use crate::{RunRequest, UserInputAnswer};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionCommandKind {
    Run,
    Steer,
    Interrupt,
    RespondInput,
    StopBackgroundTask,
    ChatCommand,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionCommandPayload {
    #[serde(rename_all = "camelCase")]
    Run {
        request: RunRequest,
        /// Client-minted message id for the optimistic user entry (dedup key).
        message_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Steer {
        prompt: String,
        message_id: Option<String>,
    },
    Interrupt {},
    #[serde(rename_all = "camelCase")]
    RespondInput {
        request_id: String,
        answers: Vec<UserInputAnswer>,
    },
    /// Stop one background task of the chat's runtime (a shell, a dev
    /// server) without ending the turn. Hosts that predate it skip it.
    #[serde(rename_all = "camelCase")]
    StopBackgroundTask {
        task_id: String,
    },
    /// A slash command Zuse answers itself (`/mcp`): the host writes `text`
    /// as the user's message and a card as the reply, without involving the
    /// agent. Hosts that don't know `name` reject it.
    #[serde(rename_all = "camelCase")]
    ChatCommand {
        name: String,
        message_id: String,
        text: String,
    },
}

impl SessionCommandPayload {
    pub fn kind(&self) -> SessionCommandKind {
        match self {
            SessionCommandPayload::Run { .. } => SessionCommandKind::Run,
            SessionCommandPayload::Steer { .. } => SessionCommandKind::Steer,
            SessionCommandPayload::Interrupt {} => SessionCommandKind::Interrupt,
            SessionCommandPayload::RespondInput { .. } => SessionCommandKind::RespondInput,
            SessionCommandPayload::StopBackgroundTask { .. } => {
                SessionCommandKind::StopBackgroundTask
            }
            SessionCommandPayload::ChatCommand { .. } => SessionCommandKind::ChatCommand,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandBasedOn {
    pub turn_id: Option<String>,
    pub frontier: Option<String>,
}
