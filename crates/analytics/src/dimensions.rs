//! Typed dimensions only; the delivery boundary verifies models against vendor catalogs.
use serde_json::{Value, json};
use zuse_protocol::{ChatConfig, HarnessId, RunRequest};

pub fn provider(harness: HarnessId) -> String {
    match harness {
        HarnessId::CustomAcp(_) => "acp".into(),
        HarnessId::ClaudeCode => "claude".into(),
        HarnessId::Zuse => "native".into(),
        HarnessId::Mock => "other".into(),
        _ => String::from(harness),
    }
}

pub fn config(config: &ChatConfig) -> Value {
    json!({
        "provider": provider(config.harness),
        "model": config.model.as_deref().unwrap_or("default"),
        "reasoning_effort": config.reasoning.map(|value| serde_json::to_value(value).unwrap()).unwrap_or(json!("default")),
        "permission_mode": config.sandbox,
        "plan_mode": config.plan,
    })
}

pub fn request(harness: HarnessId, request: &RunRequest) -> Value {
    let mut properties = config(&ChatConfig {
        harness,
        model: request.model.clone(),
        reasoning: request.reasoning,
        model_options: Default::default(),
        sandbox: request.sandbox,
        plan: request.plan,
    });
    properties["resumed"] = json!(request.resume.is_some());
    properties
}

pub fn input_length(length: usize) -> &'static str {
    match length {
        0 => "empty",
        1..=100 => "1-100",
        101..=500 => "101-500",
        501..=2000 => "501-2000",
        _ => "2001+",
    }
}
