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

/// Whether the harness reports its servers' status when a session starts
/// (`AgentEvent::McpServers`).
pub fn reports_status(harness: HarnessId) -> bool {
    harness == HarnessId::ClaudeCode
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

/// Where a configured server runs, without anything that could carry a
/// secret: the program's file name for a command (never its arguments,
/// which can hold keys), or the host for a URL (never its path or query).
fn configured_detail(server: &DetectedServer) -> String {
    if let Some(url) = &server.url {
        // `scheme://user:pass@host:port/path?query` → `host:port`.
        let rest = url
            .trim()
            .split_once("://")
            .map_or(url.trim(), |(_, rest)| rest);
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = authority.rsplit('@').next().unwrap_or_default();
        return if host.is_empty() {
            "Remote".into()
        } else {
            format!("Remote · {host}")
        };
    }
    server
        .command
        .as_deref()
        .and_then(|command| {
            std::path::Path::new(command.trim())
                .file_name()?
                .to_str()
                .map(str::to_owned)
        })
        .filter(|program| !program.is_empty())
        .map(|program| format!("Runs {program}"))
        .unwrap_or_default()
}

/// At most `max` characters, with an ellipsis when cut. The component
/// limits count UTF-16 units; these caps stay well inside them.
fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(max - 1).collect();
    clipped.push('…');
    clipped
}

fn row(name: &str, status: &str, detail: &str) -> String {
    let (name, detail) = (clip(name, 80), clip(detail, 140));
    let (name, detail) = (name.as_str(), detail.as_str());
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
            let reports = reports_status(harness);
            let note = match (servers.is_empty(), config_source(harness).is_some()) {
                (_, false) => format!("Zuse can't read {name}'s MCP settings yet."),
                (true, true) => {
                    format!("No MCP servers are configured for {name} on this computer.")
                }
                (false, true) if reports => format!(
                    "Configured for {name}. Each server's status shows here once the agent has started in this chat."
                ),
                (false, true) => format!(
                    "Configured for {name}. {name} doesn't report whether each one connected."
                ),
            };
            (
                servers
                    .iter()
                    .map(|server| row(&server.name, "configured", &configured_detail(server)))
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
        assert!(spec.contains(r#"McpServer("linear", "configured", "Runs npx")"#));
        assert!(!spec.contains("secret"), "env values never reach the card");

        assert!(spec.contains("Codex doesn't report whether each one connected"));
        let empty = mcp_spec(HarnessId::Cursor, AgentServers::Configured(vec![]));
        zuse_ui_block::parse(&empty).unwrap();
        assert!(empty.contains("No MCP servers are configured for Cursor"));
        let unreadable = mcp_spec(HarnessId::Gemini, AgentServers::Configured(vec![]));
        assert!(unreadable.contains("Zuse can't read Gemini's MCP settings yet."));
        let none = mcp_spec(HarnessId::ClaudeCode, AgentServers::Reported(vec![]));
        zuse_ui_block::parse(&none).unwrap();
        assert!(none.contains("reported no MCP servers"));
    }

    #[test]
    fn configured_rows_never_show_arguments_urls_paths_or_queries() {
        let stdio = DetectedServer {
            source: "Claude".into(),
            name: "github".into(),
            command: Some("/usr/local/bin/gh-mcp".into()),
            args: vec!["--api-key".into(), "sk-live-secret".into()],
            env: Default::default(),
            url: None,
            headers: Default::default(),
            transport: None,
        };
        let remote = DetectedServer {
            name: "docs".into(),
            command: None,
            args: vec![],
            url: Some("https://user:pw@docs.example.com:8443/mcp?token=abc#x".into()),
            ..stdio.clone()
        };
        assert_eq!(configured_detail(&stdio), "Runs gh-mcp");
        assert_eq!(configured_detail(&remote), "Remote · docs.example.com:8443");
        let spec = mcp_spec(
            HarnessId::ClaudeCode,
            AgentServers::Configured(vec![stdio, remote]),
        );
        zuse_ui_block::parse(&spec).unwrap_or_else(|e| panic!("{e}\n{spec}"));
        assert!(spec.contains("status shows here once the agent has started"));
        for secret in ["sk-live-secret", "token=abc", "pw@", "/mcp?"] {
            assert!(!spec.contains(secret), "{secret} leaked: {spec}");
        }
    }

    #[test]
    fn oversized_names_and_details_still_make_a_valid_card() {
        let long = "x".repeat(500);
        let spec = mcp_spec(
            HarnessId::ClaudeCode,
            AgentServers::Reported(vec![server(&long, "connected", Some(&long), 3)]),
        );
        zuse_ui_block::parse(&spec).unwrap_or_else(|e| panic!("{e}"));
        assert!(spec.contains('…'));
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
