//! Best-effort product analytics. No content, provider payloads, or raw errors.
pub mod dimensions;
pub mod turn;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Notify, mpsc, oneshot};

const CAPACITY: usize = 512;
const BATCH_SIZE: usize = 50;

#[derive(Clone)]
pub struct Analytics(Arc<Inner>);
struct Inner {
    state: Mutex<State>,
    path: PathBuf,
    configured: bool,
    sender: mpsc::Sender<Message>,
    changed: Arc<Notify>,
}
struct State {
    preference: Preference,
    account: Option<String>,
    generation: u64,
    models: HashMap<String, HashSet<String>>,
}
#[derive(Serialize, Deserialize)]
struct Preference {
    enabled: bool,
    anonymous_id: String,
    #[serde(default = "install_id")]
    install_id: String,
}
enum Message {
    Event(u64, Value),
    Shutdown(oneshot::Sender<()>),
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Status {
    pub enabled: bool,
    pub configured: bool,
}

pub fn account_id(id: &str) -> String {
    format!(
        "account_{:x}",
        Sha256::digest(format!("zuse:analytics:account:v1:{id}"))
    )
}

impl Analytics {
    /// No key or a debug build without explicit opt-in means no network activity.
    pub fn open(data_dir: &Path) -> Self {
        let key = std::env::var("ZUSE_POSTHOG_KEY")
            .ok()
            .or_else(|| option_env!("ZUSE_POSTHOG_KEY").map(str::to_owned))
            .unwrap_or_default();
        let host = std::env::var("ZUSE_POSTHOG_HOST")
            .ok()
            .or_else(|| option_env!("ZUSE_POSTHOG_HOST").map(str::to_owned))
            .unwrap_or_else(|| "https://us.i.posthog.com".into());
        let configured = !key.trim().is_empty()
            && (!cfg!(debug_assertions)
                || std::env::var("ZUSE_POSTHOG_ENABLE_DEV").as_deref() == Ok("1"))
            && std::env::var("ZUSE_ANALYTICS_ENABLED").as_deref() != Ok("0");
        Self::start(data_dir, key, host, configured)
    }

    fn start(data_dir: &Path, key: String, host: String, configured: bool) -> Self {
        let path = data_dir.join("analytics.json");
        let preference = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(Preference {
                enabled: false,
                anonymous_id: anonymous_id(),
                install_id: install_id(),
            }),
            Err(error) => Preference {
                enabled: error.kind() == std::io::ErrorKind::NotFound,
                anonymous_id: anonymous_id(),
                install_id: install_id(),
            },
        };
        // If persistence is unavailable, do not create a new install on every launch.
        let durable = save(&path, &preference).is_ok();
        let (sender, receiver) = mpsc::channel(CAPACITY);
        let runtime = tokio::runtime::Handle::try_current().ok();
        let this = Self(Arc::new(Inner {
            path,
            state: Mutex::new(State {
                preference,
                account: None,
                generation: 0,
                models: HashMap::new(),
            }),
            configured: configured && durable && runtime.is_some(),
            sender,
            changed: Arc::new(Notify::new()),
        }));
        if let Some(runtime) = runtime {
            runtime.spawn(deliver(Arc::downgrade(&this.0), receiver, key, host));
        }
        this
    }

    pub fn status(&self) -> Status {
        let state = self.0.state.lock().unwrap();
        Status {
            enabled: state.preference.enabled,
            configured: self.0.configured,
        }
    }

    pub fn set_enabled(&self, enabled: bool) -> std::io::Result<Status> {
        let mut state = self.0.state.lock().unwrap();
        // Revoke in memory first, even if the disk write fails.
        state.generation += 1;
        if !enabled {
            state.preference.enabled = false;
        }
        let next = Preference {
            enabled,
            anonymous_id: state.preference.anonymous_id.clone(),
            install_id: state.preference.install_id.clone(),
        };
        let result = save(&self.0.path, &next);
        if result.is_ok() {
            state.preference = next;
        }
        drop(state);
        self.0.changed.notify_one();
        result?;
        Ok(self.status())
    }

    /// Account changes invalidate queued events; logout rotates the anonymous id.
    pub fn set_account(&self, account: Option<&str>) {
        let next = account.map(account_id);
        let mut state = self.0.state.lock().unwrap();
        if next == state.account {
            return;
        }
        if next.is_none() {
            state.preference.anonymous_id = anonymous_id();
            let _ = save(&self.0.path, &state.preference);
        }
        state.account = next;
        state.generation += 1;
        drop(state);
        self.0.changed.notify_one();
    }

    /// Register only built-in vendor catalogs, never user-configured model rows.
    pub fn register_vendor_models(&self, provider: &str, models: impl IntoIterator<Item = String>) {
        let mut state = self.0.state.lock().unwrap();
        state
            .models
            .entry(provider.into())
            .or_default()
            .extend(models.into_iter().filter(|id| {
                id.len() <= 120
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/[]".contains(&byte))
            }));
    }

    pub fn capture(&self, event: &str, properties: Value) {
        self.capture_from("engine", event, properties);
    }

    pub fn capture_from(&self, surface: &str, event: &str, properties: Value) {
        let state = self.0.state.lock().unwrap();
        let Some(mut properties) = sanitize_with_models(event, properties, &state.models) else {
            return;
        };
        if !self.0.configured || !state.preference.enabled {
            return;
        }
        properties["analytics_schema_version"] = json!(2);
        properties["surface"] = json!(if ["engine", "desktop", "web", "ios", "android"]
            .contains(&surface)
        {
            surface
        } else {
            "unknown"
        });
        properties["os"] = json!(std::env::consts::OS);
        properties["architecture"] = json!(std::env::consts::ARCH);
        properties["app_version"] = json!(env!("CARGO_PKG_VERSION"));
        properties["install_id"] = json!(state.preference.install_id);
        properties["identity_kind"] = json!(if state.account.is_some() {
            "account"
        } else {
            "anonymous"
        });
        properties["$process_person_profile"] = json!(false);
        let id = state
            .account
            .as_ref()
            .unwrap_or(&state.preference.anonymous_id);
        let payload = json!({"event": event, "uuid": uuid::Uuid::new_v4(), "timestamp": chrono::Utc::now(), "properties": properties, "distinct_id": id});
        let _ = self
            .0
            .sender
            .try_send(Message::Event(state.generation, payload));
    }

    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        let _ = tokio::time::timeout(Duration::from_secs(3), async {
            if self.0.sender.send(Message::Shutdown(tx)).await.is_ok() {
                let _ = rx.await;
            }
        })
        .await;
    }
}

fn install_id() -> String {
    format!("install_{}", uuid::Uuid::new_v4())
}

fn anonymous_id() -> String {
    format!("anonymous_{}", uuid::Uuid::new_v4())
}
fn save(path: &Path, preference: &Preference) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(&serde_json::to_vec(preference)?)?;
    file.sync_all()?;
    std::fs::rename(temp, path)
}

/// Closed event/property boundary; no caller may pass arbitrary strings.
pub fn sanitize(event: &str, properties: Value) -> Option<Value> {
    sanitize_with_models(event, properties, &HashMap::new())
}

fn sanitize_with_models(
    event: &str,
    properties: Value,
    models: &HashMap<String, HashSet<String>>,
) -> Option<Value> {
    let dimensions = [
        "provider",
        "model",
        "actual_model",
        "reasoning_effort",
        "permission_mode",
        "plan_mode",
        "subagent",
        "continuation",
        "resumed",
    ];
    let event_keys: &[&str] = match event {
        "app opened" | "app closed" => &[],
        "feature used" => &["feature", "outcome", "duration_ms"],
        "chat created" | "chat opened" | "chat archived" | "chat restored" | "chat deleted" => {
            &["projectless"]
        }
        "model changed" => &["previous_model", "previous_provider"],
        "chat settings changed" => &[],
        "message submitted" => &["attachment_count", "input_length_bucket", "submission_kind"],
        "permission answered" | "turn interrupt requested" => &[],
        "turn started" => &[],
        "turn completed" | "turn failed" | "turn interrupted" => &[
            "duration_ms",
            "first_token_ms",
            "tool_count",
            "tool_failure_count",
            "shell_tool_count",
            "files_tool_count",
            "browser_tool_count",
            "mcp_tool_count",
            "subagent_tool_count",
            "other_tool_count",
            "subagent_count",
            "subagent_completed_count",
            "input_request_count",
            "mixed_models",
            "input_tokens",
            "output_tokens",
            "request_attempt_count",
            "request_failure_count",
            "cache_read_tokens",
            "cache_creation_tokens",
            "reasoning_tokens",
            "usage_status",
            "usage_scope",
        ],
        _ => return None,
    };
    let mut output = serde_json::Map::new();
    let provider = properties
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if let Some(properties) = properties.as_object() {
        for (key, value) in properties {
            if !event_keys.contains(&key.as_str()) && !dimensions.contains(&key.as_str()) {
                continue;
            }
            let valid = match (key.as_str(), value) {
                ("provider" | "previous_provider", Value::String(value)) => [
                    "codex",
                    "claude",
                    "cursor",
                    "opencode",
                    "pi",
                    "acp",
                    "native",
                    "gemini",
                    "kiro",
                    "devin",
                    "grok",
                    "hermes",
                    "antigravity",
                    "muse",
                    "other",
                    "unknown",
                ]
                .contains(&value.as_str()),
                ("model" | "actual_model" | "previous_model", Value::String(value)) => {
                    let model_provider = if key == "previous_model" {
                        properties
                            .get("previous_provider")
                            .and_then(Value::as_str)
                            .unwrap_or(provider)
                    } else {
                        provider
                    };
                    let safe = ["custom", "default", "unknown"].contains(&value.as_str())
                        || models
                            .get(model_provider)
                            .is_some_and(|catalog| catalog.contains(value));
                    output.insert(
                        key.clone(),
                        json!(if safe { value.as_str() } else { "custom" }),
                    );
                    continue;
                }
                ("reasoning_effort", Value::String(value)) => [
                    "default",
                    "minimal",
                    "low",
                    "medium",
                    "high",
                    "xhigh",
                    "max",
                    "ultra",
                    "ultracode",
                    "ultrathink",
                ]
                .contains(&value.as_str()),
                ("permission_mode", Value::String(value)) => {
                    ["workspace-write", "auto", "danger-full-access"].contains(&value.as_str())
                }
                ("input_length_bucket", Value::String(value)) => {
                    ["empty", "1-100", "101-500", "501-2000", "2001+"].contains(&value.as_str())
                }
                ("submission_kind", Value::String(value)) => {
                    ["run", "steer"].contains(&value.as_str())
                }
                ("usage_status", Value::String(value)) => {
                    ["unknown", "observed"].contains(&value.as_str())
                }
                ("usage_scope", Value::String(value)) => {
                    ["provider_snapshot", "main_request_sum"].contains(&value.as_str())
                }
                ("outcome", Value::String(value)) => {
                    ["success", "failure"].contains(&value.as_str())
                }
                ("feature", Value::String(value)) => {
                    FEATURES.iter().any(|(_, feature)| feature == value)
                }
                (
                    "plan_mode" | "subagent" | "continuation" | "resumed" | "mixed_models"
                    | "projectless",
                    Value::Bool(_),
                ) => true,
                (key, Value::Number(value))
                    if key.ends_with("_count")
                        || key.ends_with("_ms")
                        || key.ends_with("_tokens") =>
                {
                    value.as_u64().is_some_and(|n| n <= 1_000_000_000_000_000)
                }
                _ => false,
            };
            if valid {
                output.insert(key.clone(), value.clone());
            }
        }
    }
    Some(Value::Object(output))
}

const FEATURES: &[(&str, &str)] = &[
    ("Mutate:createChat", "chat created"),
    ("Mutate:createSpace", "project created"),
    ("Mutate:deleteChat", "chat deleted"),
    ("Mutate:setChatArchived", "chat archived"),
    ("QueueCommand", "command queued"),
    ("QueueMessage", "message queued"),
    ("QueueCommand:run", "message submitted"),
    ("QueueCommand:steer", "message submitted"),
    ("QueueCommand:interrupt", "turn interrupted"),
    ("QueueCommand:respondInput", "permission answered"),
    ("StartProjectCreation", "project creation started"),
    ("AddRepo", "repository added"),
    ("CloneRepo", "repository cloned"),
    ("CreateWorktree", "worktree created"),
    ("DeleteWorktree", "worktree deleted"),
    ("OpenTerminal", "terminal opened"),
    ("StartAgentLogin", "provider login"),
    ("Plugins:connect", "plugin connected"),
    ("Plugins:removeServer", "plugin removed"),
    ("CloudCreate", "cloud workspace created"),
    ("CloudPause", "cloud workspace paused"),
    ("CloudUnarchive", "cloud workspace resumed"),
    ("CloudDelete", "cloud workspace deleted"),
    ("ApplyUpdate", "update applied"),
    ("ForkSideChat", "session forked"),
];
pub fn feature(method: &str, params: &Value) -> Option<&'static str> {
    if method == "QueueCommand" {
        return match params.pointer("/command/kind").and_then(Value::as_str) {
            Some("run" | "steer") => Some("message submitted"),
            Some("interrupt") => Some("turn interrupted"),
            Some("respondInput") => Some("permission answered"),
            _ => None,
        };
    }
    if method == "Plugins" {
        return match params.get("action").and_then(Value::as_str) {
            Some("connect" | "addServer" | "addLocalServer") => Some("plugin connected"),
            Some("removeServer") => Some("plugin removed"),
            _ => None,
        };
    }
    if method == "Mutate" {
        return match params.get("op").and_then(Value::as_str) {
            Some("createChat") => Some("chat created"),
            Some("createSpace") => Some("project created"),
            Some("deleteChat") => Some("chat deleted"),
            Some("setChatArchived") => Some("chat archived"),
            _ => None,
        };
    }
    FEATURES
        .iter()
        .find_map(|(name, feature)| (*name == method).then_some(*feature))
}

async fn deliver(
    inner: std::sync::Weak<Inner>,
    mut receiver: mpsc::Receiver<Message>,
    key: String,
    host: String,
) {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return;
    };
    let endpoint = format!("{}/batch/", host.trim_end_matches('/'));
    let mut batch = Vec::new();
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    let Some(changed) = inner.upgrade().map(|owner| owner.changed.clone()) else {
        return;
    };
    loop {
        let mut shutdown = None;
        tokio::select! {
            message = receiver.recv() => match message {
                Some(Message::Event(generation, event)) => { batch.push((generation, event)); if batch.len() < BATCH_SIZE { continue; } },
                Some(Message::Shutdown(done)) => { shutdown = Some(done); },
                None => break,
            },
            _ = interval.tick() => {},
            _ = changed.notified() => {},
        }
        let Some(owner) = inner.upgrade() else {
            break;
        };
        let (enabled, generation) = {
            let state = owner.state.lock().unwrap();
            (
                owner.configured && state.preference.enabled,
                state.generation,
            )
        };
        batch.retain(|(g, _)| enabled && *g == generation);
        if !batch.is_empty() {
            let events: Vec<_> = batch.drain(..).map(|(_, event)| event).collect();
            // A finite retry budget, identical UUIDs, and cancellable backoff.
            for attempt in 0..3 {
                let current = {
                    let state = owner.state.lock().unwrap();
                    state.preference.enabled && state.generation == generation
                };
                if !current {
                    break;
                }
                let result = tokio::select! {
                    result = client.post(&endpoint).json(&json!({"api_key": key, "batch": events})).send() => result,
                    _ = owner.changed.notified() => break,
                };
                match result {
                    Ok(response) if response.status().is_success() => break,
                    Ok(response)
                        if response.status().is_client_error()
                            && response.status().as_u16() != 429 =>
                    {
                        break;
                    }
                    _ if attempt < 2 => tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(1 << attempt)) => {},
                        _ = owner.changed.notified() => break,
                    },
                    _ => {}
                }
            }
        }
        if let Some(done) = shutdown {
            let _ = done.send(());
            break;
        }
    }
}

#[cfg(test)]
mod tests;
