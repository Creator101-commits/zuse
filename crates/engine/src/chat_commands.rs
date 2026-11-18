//! Slash commands Zuse answers itself, in the transcript, instead of sending
//! them to the agent: the user's `/mcp` becomes a card listing the agent's
//! own MCP servers next to the reader's live Zuse plugins.
//!
//! The card is an ordinary UI block. Server rows are a snapshot of what the
//! harness reported; `ConnectedPlugins()` is filled in live by the reader's
//! app, so it never goes stale.

use zuse_protocol::{HarnessId, HarnessMcpServer, plugin_setup::DetectedServer};

/// Commands this host answers.
pub const NAMES: &[&str] = &["mcp"];

/// Rows past this many are summarized (a Card holds at most 64 children).
const MAX_ROWS: usize = 48;

/// What `/mcp` knows about the agent's servers.
pub enum AgentServers {
    /// The chat's latest session reported these.
    Reported(Vec<HarnessMcpServer>),
    /// No session has reported yet: these are configured for the harness.
    Configured(Vec<DetectedServer>),
}

/// The harness's name as people know it.
pub fn harness_name(harness: HarnessId) -> &'static str {
    match harness {
        HarnessId::ClaudeCode => "Claude Code",
        HarnessId::Codex => "Codex",
        HarnessId::Cursor => "Cursor",
        HarnessId::Gemini => "Gemini",
        HarnessId::Opencode => "OpenCode",
        HarnessId::Zuse => "Zuse",
        _ => "Agent",
    }
}

/// The config `local_mcp::discover` labels a harness's servers with.
pub fn config_source(harness: HarnessId) -> Option<&'static str> {
    match harness {
        HarnessId::ClaudeCode => Some("Claude"),
        HarnessId::Codex => Some("Codex"),
        HarnessId::Cursor => Some("Cursor"),
        _ => None,
    }
}

fn quote(text: &str) -> String {
    serde_json::to_string(text).expect("strings serialize")
}

/// The harness's word for where a server is configured, in plain words.
fn source_label(name: &str, source: Option<&str>) -> Option<String> {
    Some(match source? {
        "dynamic" if name == "zuse" => "Built in".into(),
        "dynamic" => "Added for this session".into(),
        "user" => "Your config".into(),
        "project" => "This project".into(),
        "local" => "This folder".into(),
        "plugin" => "Agent plugin".into(),
        "claudeai" => "claude.ai connector".into(),
        "enterprise" | "managed" => "Managed by your organization".into(),
        other if !other.trim().is_empty() => other.to_string(),
        _ => return None,
    })
}

/// The `McpServer` status for the harness's wording.
fn status(status: &str) -> &'static str {
    match status {
        "connected" => "connected",
        "needs-auth" | "needs_auth" | "auth-required" => "needs-auth",
        "failed" | "error" => "failed",
        "disabled" => "disabled",
        _ => "pending",
    }
}

fn status_rank(status: &str) -> u8 {
    match status {
        "connected" => 0,
        "pending" => 1,
        "needs-auth" => 2,
        "failed" => 3,
        _ => 4,
    }
}

fn tools_label(count: usize) -> String {
    if count == 1 {
        "1 tool".into()
    } else {
        format!("{count} tools")
    }
}

/// One `McpServer(...)` row for a reported server.
fn reported_row(server: &HarnessMcpServer, harness: HarnessId) -> String {
    let status = status(&server.status);
    let mut detail: Vec<String> = source_label(&server.name, server.source.as_deref())
        .into_iter()
        .collect();
    match status {
        "connected" if server.tools > 0 => detail.push(tools_label(server.tools)),
        "needs-auth" if harness == HarnessId::ClaudeCode => {
            detail.push("Sign in by running claude, then /mcp, in a terminal".into())
        }
        _ => {}
    }
    row(&server.name, status, &detail.join(" · "))
}

fn row(name: &str, status: &str, detail: &str) -> String {
    if detail.is_empty() {
        format!("McpServer({}, {})", quote(name), quote(status))
    } else {
        format!(
            "McpServer({}, {}, {})",
            quote(name),
            quote(status),
            quote(detail)
        )
    }
}

/// The `/mcp` card for a chat running `harness`.
pub fn mcp_spec(harness: HarnessId, servers: AgentServers) -> String {
    let name = harness_name(harness);
    let (mut rows, note): (Vec<String>, Option<String>) = match servers {
        AgentServers::Reported(mut servers) => {
            servers.retain(|server| !server.name.trim().is_empty());
            servers.sort_by_key(|server| status_rank(status(&server.status)));
            let note = servers
                .is_empty()
                .then(|| format!("{name} reported no MCP servers for this chat."));
            (
                servers
                    .iter()
                    .map(|server| reported_row(server, harness))
                    .collect(),
                note,
            )
        }
        AgentServers::Configured(servers) => {
            let note = if servers.is_empty() {
                format!("No MCP servers are configured for {name} on this computer.")
            } else {
                format!(
                    "Configured for {name}. Each server's status shows here once the agent has started in this chat."
                )
            };
            (
                servers
                    .iter()
                    .map(|server| row(&server.name, "configured", &server.summary()))
                    .collect(),
                Some(note),
            )
        }
    };
    let hidden = rows.len().saturating_sub(MAX_ROWS);
    rows.truncate(MAX_ROWS);
    let mut lines = vec![
        "root = Card([agent, plugins])".to_string(),
        "plugins = Card([live], \"Zuse plugins\")".to_string(),
        "live = ConnectedPlugins()".to_string(),
    ];
    let mut agent = Vec::new();
    if let Some(note) = note {
        agent.push("note".to_string());
        lines.push(format!("note = Text({})", quote(&note)));
    }
    for (index, row) in rows.iter().enumerate() {
        agent.push(format!("s{index}"));
        lines.push(format!("s{index} = {row}"));
    }
    if hidden > 0 {
        agent.push("more".to_string());
        lines.push(format!(
            "more = Text({})",
            quote(&format!("And {hidden} more."))
        ));
    }
    lines.insert(
        1,
        format!(
            "agent = Card([{}], {})",
            agent.join(", "),
            quote(&format!("{name} MCP servers"))
        ),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, status: &str, source: Option<&str>, tools: usize) -> HarnessMcpServer {
        HarnessMcpServer {
            name: name.into(),
            status: status.into(),
            source: source.map(Into::into),
            tools,
        }
    }

    #[test]
    fn reported_servers_become_rows_beside_live_plugins() {
        let spec = mcp_spec(
            HarnessId::ClaudeCode,
            AgentServers::Reported(vec![
                server("vidiq", "needs-auth", Some("user"), 0),
                server("zuse", "connected", Some("dynamic"), 31),
                server("paper", "failed", Some("user"), 0),
                server("claude.ai \"Docs\"", "connected", Some("claudeai"), 1),
                server("posthog", "pending", None, 0),
            ]),
        );
        let parsed = zuse_ui_block::parse(&spec).unwrap_or_else(|e| panic!("{e}\n{spec}"));
        assert_eq!(parsed.source, spec, "already canonical");
        assert!(spec.contains(r#"s0 = McpServer("zuse", "connected", "Built in · 31 tools")"#));
        assert!(spec.contains(
            r#"s1 = McpServer("claude.ai \"Docs\"", "connected", "claude.ai connector · 1 tool")"#
        ));
        assert!(spec.contains(r#"s2 = McpServer("posthog", "pending")"#));
        assert!(spec.contains(r#"s3 = McpServer("vidiq", "needs-auth", "Your config · Sign in"#));
        assert!(spec.contains(r#"s4 = McpServer("paper", "failed", "Your config")"#));
        assert!(spec.contains("live = ConnectedPlugins()"));
        assert!(spec.contains(r#""Claude Code MCP servers""#));
    }

    #[test]
    fn configured_servers_show_until_a_session_reports() {
        let configured = vec![DetectedServer {
            source: "Codex".into(),
            name: "linear".into(),
            command: Some("npx".into()),
            args: vec!["-y".into(), "linear-mcp".into()],
            env: [("LINEAR_KEY".to_string(), "secret".to_string())].into(),
            url: None,
            headers: Default::default(),
            transport: None,
        }];
        let spec = mcp_spec(HarnessId::Codex, AgentServers::Configured(configured));
        zuse_ui_block::parse(&spec).unwrap_or_else(|e| panic!("{e}\n{spec}"));
        assert!(spec.contains(r#"McpServer("linear", "configured", "npx -y linear-mcp")"#));
        assert!(!spec.contains("secret"), "env values never reach the card");
        assert!(spec.contains("once the agent has started"));

        let empty = mcp_spec(HarnessId::Cursor, AgentServers::Configured(vec![]));
        zuse_ui_block::parse(&empty).unwrap();
        assert!(empty.contains("No MCP servers are configured for Cursor"));
        let none = mcp_spec(HarnessId::ClaudeCode, AgentServers::Reported(vec![]));
        zuse_ui_block::parse(&none).unwrap();
        assert!(none.contains("reported no MCP servers"));
    }

    #[test]
    fn long_lists_are_capped_and_summarized() {
        let many = (0..70)
            .map(|i| server(&format!("server-{i}"), "connected", None, 2))
            .collect();
        let spec = mcp_spec(HarnessId::ClaudeCode, AgentServers::Reported(many));
        zuse_ui_block::parse(&spec).unwrap_or_else(|e| panic!("{e}"));
        assert!(spec.contains(&format!("s{} = ", MAX_ROWS - 1)));
        assert!(!spec.contains(&format!("s{} = ", MAX_ROWS)));
        assert!(spec.contains("And 22 more."));
    }
}
