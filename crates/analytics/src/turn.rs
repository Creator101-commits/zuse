//! One aggregate per main-agent turn. IDs stay in memory for duplicate suppression.
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};
use zuse_protocol::{AgentEvent, DoneStatus, HarnessId, RunRequest, ToolCall};

pub struct Turn {
    dimensions: Value,
    started: Instant,
    tools: HashMap<String, &'static str>,
    failures: HashSet<String>,
    subagents: HashSet<String>,
    completed_subagents: HashSet<String>,
    usage: Option<(u64, u64)>,
    sum_usage: bool,
    reported_model: Option<String>,
    mixed_models: bool,
    first_token_ms: Option<u64>,
    input_requests: HashSet<String>,
    output_chars: u64,
    reasoning_chars: u64,
    context_tokens: Option<u64>,
    context_window: Option<u64>,
    terminal_status: Option<DoneStatus>,
    attempts: HashSet<String>,
    attempt_failures: u64,
    cache_read: Option<u64>,
    cache_write: Option<u64>,
    reasoning: Option<u64>,
}

impl Turn {
    pub fn new(
        harness: HarnessId,
        request: &RunRequest,
        subagent: bool,
        continuation: bool,
    ) -> Self {
        let mut dimensions = crate::dimensions::request(harness, request);
        dimensions["subagent"] = json!(subagent);
        dimensions["continuation"] = json!(continuation);
        Self {
            dimensions,
            started: Instant::now(),
            tools: HashMap::new(),
            failures: HashSet::new(),
            subagents: HashSet::new(),
            completed_subagents: HashSet::new(),
            usage: None,
            sum_usage: harness == HarnessId::Zuse,
            reported_model: None,
            mixed_models: false,
            first_token_ms: None,
            input_requests: HashSet::new(),
            output_chars: 0,
            reasoning_chars: 0,
            context_tokens: None,
            context_window: None,
            terminal_status: None,
            attempts: HashSet::new(),
            attempt_failures: 0,
            cache_read: None,
            cache_write: None,
            reasoning: None,
        }
    }

    pub fn restart(&self, continuation: bool) -> Self {
        let mut next = Self {
            dimensions: self.dimensions.clone(),
            started: Instant::now(),
            tools: HashMap::new(),
            failures: HashSet::new(),
            subagents: HashSet::new(),
            completed_subagents: HashSet::new(),
            usage: None,
            sum_usage: self.sum_usage,
            reported_model: None,
            mixed_models: false,
            first_token_ms: None,
            input_requests: HashSet::new(),
            output_chars: 0,
            reasoning_chars: 0,
            context_tokens: None,
            context_window: None,
            terminal_status: None,
            attempts: HashSet::new(),
            attempt_failures: 0,
            cache_read: None,
            cache_write: None,
            reasoning: None,
        };
        next.dimensions
            .as_object_mut()
            .unwrap()
            .remove("actual_model");
        next.dimensions["continuation"] = json!(continuation);
        next
    }

    pub fn dimensions(&self) -> Value {
        self.dimensions.clone()
    }

    pub fn observe(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::SessionStarted { model, .. } if !model.is_empty() => {
                if let Some(previous) = &self.reported_model {
                    self.mixed_models |= previous != model;
                }
                self.reported_model = Some(model.clone());
                self.dimensions["actual_model"] = json!(model);
            }
            AgentEvent::TextDelta { text } => {
                self.output_chars = self
                    .output_chars
                    .saturating_add(text.chars().count() as u64);
                if !text.is_empty() && self.first_token_ms.is_none() {
                    self.first_token_ms = Some(self.started.elapsed().as_millis() as u64);
                }
            }
            AgentEvent::ReasoningDelta { text } => {
                self.reasoning_chars = self
                    .reasoning_chars
                    .saturating_add(text.chars().count() as u64);
            }
            AgentEvent::ContextUsage { tokens, window } => {
                if let Some(tokens) = tokens {
                    self.context_tokens = Some(*tokens);
                }
                if let Some(window) = window {
                    self.context_window = Some(*window);
                }
            }
            AgentEvent::Done { status, .. } => self.terminal_status = Some(*status),
            AgentEvent::ToolCall { id, call } => {
                self.tools
                    .entry(id.clone())
                    .or_insert_with(|| category(call));
            }
            AgentEvent::ToolResult {
                id, is_error: true, ..
            } => {
                self.failures.insert(id.clone());
            }
            AgentEvent::Usage {
                input_tokens,
                output_tokens,
            } => {
                self.usage = Some(if self.sum_usage {
                    let previous = self.usage.unwrap_or_default();
                    (
                        previous.0.saturating_add(*input_tokens),
                        previous.1.saturating_add(*output_tokens),
                    )
                } else {
                    (*input_tokens, *output_tokens)
                });
            }
            AgentEvent::InputRequested { request_id, .. } => {
                self.input_requests.insert(request_id.clone());
            }
            AgentEvent::RequestUsage { details } => {
                // Internal request identity only suppresses repeated accounting notifications.
                // No request/account/connection identifiers or raw errors enter the result.
                if let Some(id) = details.get("requestId").and_then(Value::as_str) {
                    let key = format!(
                        "{id}:{}",
                        details
                            .get("retryIndex")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                    );
                    if !self.attempts.insert(key) {
                        return;
                    }
                    if details
                        .get("errorCode")
                        .is_some_and(|value| !value.is_null())
                    {
                        self.attempt_failures += 1;
                    }
                    if let Some(usage) = details.get("usage") {
                        let add = |total: &mut Option<u64>, key: &str| {
                            if let Some(value) = usage.get(key).and_then(Value::as_u64) {
                                *total = Some(total.unwrap_or_default().saturating_add(value));
                            }
                        };
                        add(&mut self.cache_read, "cachedTokens");
                        add(&mut self.cache_write, "cacheWriteTokens");
                        add(&mut self.reasoning, "reasoningTokens");
                    }
                }
            }
            AgentEvent::Subagent {
                parent_tool_use_id: agent_id,
                event,
            } => {
                self.subagents.insert(agent_id.clone());
                if matches!(event.as_ref(), AgentEvent::Done { .. }) {
                    self.completed_subagents.insert(agent_id.clone());
                }
            }
            _ => {}
        }
    }

    /// Used only when the stream ends without a normalized Done event.
    pub fn finish_with_status(&self, status: DoneStatus) -> Value {
        self.finish_inner(self.terminal_status.or(Some(status)))
    }

    pub fn finish(&self) -> Value {
        self.finish_inner(self.terminal_status)
    }

    fn finish_inner(&self, terminal_status: Option<DoneStatus>) -> Value {
        let mut properties = self.dimensions();
        properties["duration_ms"] = json!(self.started.elapsed().as_millis() as u64);
        properties["tool_count"] = json!(self.tools.len());
        properties["tool_failure_count"] = json!(self.failures.len());
        for category in ["shell", "files", "browser", "mcp", "subagent", "other"] {
            properties[format!("{category}_tool_count")] = json!(
                self.tools
                    .values()
                    .filter(|value| **value == category)
                    .count()
            );
        }
        properties["subagent_count"] = json!(self.subagents.len());
        properties["subagent_completed_count"] = json!(self.completed_subagents.len());
        properties["input_request_count"] = json!(self.input_requests.len());
        properties["output_character_count"] = json!(self.output_chars);
        properties["reasoning_character_count"] = json!(self.reasoning_chars);
        properties["usage_report_version"] = json!(1);
        properties["terminal_status"] = json!(match terminal_status {
            Some(DoneStatus::Completed) => "completed",
            Some(DoneStatus::Interrupted) => "interrupted",
            Some(DoneStatus::Errored) => "errored",
            None => "unknown",
        });
        properties["mixed_models"] = json!(self.mixed_models);
        if !self.attempts.is_empty() {
            properties["request_attempt_count"] = json!(self.attempts.len());
            properties["request_failure_count"] = json!(self.attempt_failures);
        }
        for (key, value) in [
            ("cache_read_tokens", self.cache_read),
            ("cache_creation_tokens", self.cache_write),
            ("reasoning_tokens", self.reasoning),
            ("context_tokens", self.context_tokens),
            ("context_window_tokens", self.context_window),
        ] {
            if let Some(value) = value {
                properties[key] = json!(value);
            }
        }
        properties["usage_status"] = json!(if self.usage.is_some() {
            if terminal_status == Some(DoneStatus::Completed) {
                "observed"
            } else {
                "partial"
            }
        } else {
            "unknown"
        });
        if let Some((input, output)) = self.usage {
            properties["input_tokens"] = json!(input);
            properties["output_tokens"] = json!(output);
            properties["usage_scope"] = json!(if self.sum_usage {
                "main_request_sum"
            } else {
                "provider_snapshot"
            });
        }
        if let Some(ms) = self.first_token_ms {
            properties["first_token_ms"] = json!(ms);
        }
        properties
    }
}

fn category(call: &ToolCall) -> &'static str {
    if call.is_subagent_spawn() {
        return "subagent";
    }
    match call {
        ToolCall::Exec { .. } => "shell",
        ToolCall::ReadFile { .. }
        | ToolCall::WriteFile { .. }
        | ToolCall::EditFile { .. }
        | ToolCall::ApplyPatch { .. }
        | ToolCall::Search { .. }
        | ToolCall::Glob { .. } => "files",
        ToolCall::WebFetch { .. } | ToolCall::WebSearch { .. } => "browser",
        ToolCall::Mcp { .. } => "mcp",
        _ => "other",
    }
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
