use super::*;

fn request() -> RunRequest {
    serde_json::from_value(json!({"prompt":"private", "model":"selected-model",
        "cwd":"/private", "sandbox":"workspace-write", "resume":null}))
    .unwrap()
}
fn turn() -> Turn {
    Turn::new(HarnessId::Zuse, &request(), false, false)
}
fn done(status: DoneStatus) -> AgentEvent {
    AgentEvent::Done {
        status,
        result: Some("private result".into()),
        error: None,
        session_id: None,
    }
}
fn session(model: &str) -> AgentEvent {
    AgentEvent::SessionStarted {
        harness: HarnessId::Zuse,
        model: model.into(),
        tools: vec![],
        cwd: "/private".into(),
        session_id: "private-session".into(),
        assistant_message_id: "private-message".into(),
    }
}

#[test]
fn duplicate_prompts_and_nested_measurements_are_excluded() {
    let mut turn = turn();
    let prompt = AgentEvent::InputRequested {
        request_id: "private-id".into(),
        questions: vec![],
    };
    turn.observe(&prompt);
    turn.observe(&prompt);
    for event in [
        prompt,
        AgentEvent::TextDelta {
            text: "child-secret".into(),
        },
        AgentEvent::Usage {
            input_tokens: 500,
            output_tokens: 600,
        },
        AgentEvent::ContextUsage {
            tokens: Some(1000),
            window: Some(2000),
        },
        done(DoneStatus::Completed),
    ] {
        turn.observe(&AgentEvent::Subagent {
            parent_tool_use_id: "child-private".into(),
            event: Box::new(event),
        });
    }
    let report = turn.finish();
    assert_eq!(report["input_request_count"], 1);
    assert_eq!(report["output_character_count"], 0);
    assert_eq!(report["subagent_count"], 1);
    assert_eq!(report["subagent_completed_count"], 1);
    assert_eq!(report["usage_status"], "unknown");
    assert!(report.get("input_tokens").is_none());
    assert!(report.get("context_tokens").is_none());
    assert!(!report.to_string().contains("private"));
}

#[test]
fn zero_usage_and_context_remain_measured_and_text_is_only_counted() {
    let mut turn = turn();
    turn.observe(&AgentEvent::TextDelta {
        text: "aé🦀".into(),
    });
    turn.observe(&AgentEvent::ReasoningDelta {
        text: "思考".into(),
    });
    turn.observe(&AgentEvent::ContextUsage {
        tokens: Some(30),
        window: Some(100),
    });
    turn.observe(&AgentEvent::ContextUsage {
        tokens: Some(0),
        window: None,
    });
    turn.observe(&AgentEvent::Usage {
        input_tokens: 0,
        output_tokens: 0,
    });
    turn.observe(&done(DoneStatus::Completed));
    let report = turn.finish();
    assert_eq!(report["output_character_count"], 3);
    assert_eq!(report["reasoning_character_count"], 2);
    assert_eq!(report["context_tokens"], 0);
    assert_eq!(report["context_window_tokens"], 100);
    assert_eq!(report["input_tokens"], 0);
    assert_eq!(report["usage_status"], "observed");
    assert_eq!(report["usage_report_version"], 1);
    assert_eq!(report["terminal_status"], "completed");
    assert!(!report.to_string().contains("思考"));
}

#[test]
fn interrupted_and_missing_terminal_usage_are_partial() {
    let mut turn = turn();
    assert_eq!(
        turn.finish_with_status(DoneStatus::Interrupted)["usage_status"],
        "unknown"
    );
    turn.observe(&AgentEvent::Usage {
        input_tokens: 5,
        output_tokens: 0,
    });
    assert_eq!(turn.finish()["usage_status"], "partial");
    turn.observe(&done(DoneStatus::Interrupted));
    assert_eq!(turn.finish()["terminal_status"], "interrupted");
    assert_eq!(
        turn.finish_with_status(DoneStatus::Errored)["terminal_status"],
        "interrupted"
    );
    turn.observe(&done(DoneStatus::Errored));
    assert_eq!(turn.finish()["terminal_status"], "errored");
}

#[test]
fn restart_does_not_inherit_actual_model_or_mix_previous_turn() {
    let mut previous = turn();
    previous.observe(&session("first-model"));
    previous.observe(&session("second-model"));
    assert_eq!(previous.finish()["mixed_models"], true);
    let mut next = previous.restart(true);
    assert!(next.finish().get("actual_model").is_none());
    next.observe(&session("third-model"));
    assert_eq!(next.finish()["mixed_models"], false);
    assert_eq!(next.finish()["actual_model"], "third-model");
    assert_eq!(next.finish()["continuation"], true);
    assert_eq!(next.finish()["usage_status"], "unknown");
}

#[test]
fn service_tier_is_a_finite_selected_dimension() {
    for (selected, expected) in [
        ("default", Some("default")),
        ("priority", Some("fast")),
        ("fast", Some("fast")),
        ("flex", Some("flex")),
        ("private-custom", None),
    ] {
        let mut request = request();
        request
            .model_options
            .insert("serviceTier".into(), json!(selected));
        let report = Turn::new(HarnessId::Codex, &request, false, false).finish();
        assert_eq!(report.get("service_tier").and_then(Value::as_str), expected);
    }
}

#[test]
fn service_tier_uses_only_each_harness_supported_options() {
    for (selected, expected) in [
        (json!(true), Some("fast")),
        (json!("on"), Some("fast")),
        (json!("true"), Some("fast")),
        (json!(false), Some("default")),
        (json!("off"), Some("default")),
        (json!("false"), Some("default")),
        (json!("private"), None),
        (json!(1), None),
    ] {
        let mut request = request();
        request.model_options.insert("fastMode".into(), selected);
        let report = Turn::new(HarnessId::ClaudeCode, &request, false, false).finish();
        assert_eq!(report.get("service_tier").and_then(Value::as_str), expected);
    }
    for harness in [HarnessId::Zuse, HarnessId::Mock, HarnessId::ClaudeCode] {
        let mut request = request();
        assert!(
            Turn::new(harness, &request, false, false)
                .finish()
                .get("service_tier")
                .is_none()
        );
        request
            .model_options
            .insert("serviceTier".into(), json!("fast"));
        assert!(
            Turn::new(harness, &request, false, false)
                .finish()
                .get("service_tier")
                .is_none()
        );
    }
    let mut request = request();
    request.model_options.insert("fastMode".into(), json!(true));
    assert!(
        Turn::new(HarnessId::Codex, &request, false, false)
            .finish()
            .get("service_tier")
            .is_none()
    );
}
