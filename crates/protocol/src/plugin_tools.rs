//! Who a plugin is, for the places that only hold its id: a `@Linear` chip
//! (`zuse-plugin:linear`) or a `plugins_call` row whose address names
//! `tools.<pluginId>.<scope>.<connection>.<tool>`. Both resolve the id against
//! the bundled catalog, so old messages and rows still find their plugin.
use std::sync::LazyLock;

use crate::plugins::PluginDefinition;

static BUNDLED: LazyLock<Vec<PluginDefinition>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("plugin-catalog.json")).expect("bundled plugin catalog")
});

/// The catalog that ships with the app.
pub fn bundled_catalog() -> &'static [PluginDefinition] {
    &BUNDLED
}

/// The bundled catalog entry for `plugin_id`.
pub fn bundled_definition(plugin_id: &str) -> Option<&'static PluginDefinition> {
    BUNDLED.iter().find(|entry| entry.id == plugin_id)
}

/// The bundled catalog's domain for `plugin_id` (`linear` → `linear.app`), the
/// key its logo is stored under.
pub fn bundled_domain(plugin_id: &str) -> Option<&'static str> {
    bundled_definition(plugin_id)
        .map(|entry| entry.domain.as_str())
        .filter(|domain| !domain.is_empty())
}

/// The plugin and tool a plugin tool address names:
/// `tools.linear.user.<connection>.list_issues` → `("linear", "list_issues")`.
pub fn parse_tool_address(address: &str) -> Option<(&str, &str)> {
    let rest = address.trim().strip_prefix("tools.")?;
    let mut parts = rest.splitn(4, '.');
    let plugin = parts.next()?;
    let _scope = parts.next()?;
    let _connection = parts.next()?;
    let tool = parts.next()?;
    (!plugin.is_empty() && !tool.is_empty()).then_some((plugin, tool))
}

/// A tool's name as words: `list_issues` → `List issues`.
pub fn humanize_tool_name(name: &str) -> String {
    let words = name
        .split(|c: char| c == '_' || c == '-' || c == '.' || c.is_whitespace())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    capitalize(&words)
}

/// A plugin id as a name, for one the catalog doesn't know:
/// `microsoft-learn` → `Microsoft Learn`.
pub fn humanize_plugin_id(id: &str) -> String {
    id.split(['-', '_', '.'])
        .filter(|word| !word.is_empty())
        .map(capitalize)
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A call to one of Zuse's plugin tools, named for people: the plugin and what
/// it did (`Linear` · `List issues`), or `Plugins` · `Search tools` for the
/// calls that aren't about one plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginToolCall {
    /// The plugin the call reached, once its address is known.
    pub plugin_id: Option<String>,
    pub plugin: String,
    pub action: String,
}

impl PluginToolCall {
    pub fn label(&self) -> String {
        format!("{} · {}", self.plugin, self.action)
    }
}

const GENERIC: &str = "Plugins";

/// `server`/`tool` is one of Zuse's plugin tools; `input` its arguments, which
/// may still be arriving. `None` for every other tool.
pub fn plugin_tool_call(
    server: &str,
    tool: &str,
    input: Option<&serde_json::Value>,
) -> Option<PluginToolCall> {
    if !server.eq_ignore_ascii_case("zuse") {
        return None;
    }
    let generic = |action: &str| PluginToolCall {
        plugin_id: None,
        plugin: GENERIC.into(),
        action: action.into(),
    };
    let addressed = |fallback: &str, schema: bool| {
        let Some((plugin_id, tool)) = input
            .and_then(|input| input.get("address"))
            .and_then(serde_json::Value::as_str)
            .and_then(parse_tool_address)
        else {
            return generic(fallback);
        };
        let plugin = bundled_definition(plugin_id)
            .map(|entry| entry.name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| humanize_plugin_id(plugin_id));
        let action = humanize_tool_name(tool);
        PluginToolCall {
            plugin_id: Some(plugin_id.to_owned()),
            plugin,
            action: if schema {
                format!("{action} (schema)")
            } else {
                action
            },
        }
    };
    Some(match tool {
        "plugins_call" => addressed("Call tool", false),
        "plugins_schema" => addressed("Read tool schema", true),
        "plugins_search" => generic("Search tools"),
        "plugins_list" => generic("List tools"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn addresses_name_their_plugin_and_tool() {
        assert_eq!(
            parse_tool_address("tools.linear.user.c7d4.list_issues"),
            Some(("linear", "list_issues"))
        );
        assert_eq!(
            parse_tool_address("tools.microsoft-learn.user.c7d4.microsoft_docs_fetch"),
            Some(("microsoft-learn", "microsoft_docs_fetch"))
        );
        // A partial address (still streaming) or another shape names nothing.
        assert_eq!(parse_tool_address("tools.linear.user."), None);
        assert_eq!(parse_tool_address("tools.linear"), None);
        assert_eq!(parse_tool_address("mail/send"), None);
    }

    #[test]
    fn names_read_as_words() {
        assert_eq!(humanize_tool_name("list_issues"), "List issues");
        assert_eq!(
            humanize_tool_name("microsoft_docs_fetch"),
            "Microsoft docs fetch"
        );
        assert_eq!(humanize_tool_name("get-Team"), "Get team");
        assert_eq!(humanize_plugin_id("microsoft-learn"), "Microsoft Learn");
    }

    #[test]
    fn catalog_plugins_resolve_their_domain() {
        assert_eq!(bundled_domain("linear"), Some("linear.app"));
        assert_eq!(bundled_domain("not-a-plugin"), None);
    }

    #[test]
    fn plugin_calls_name_the_plugin_and_action() {
        let call = plugin_tool_call(
            "zuse",
            "plugins_call",
            Some(&json!({"address":"tools.linear.user.c1.list_issues","arguments":{}})),
        )
        .unwrap();
        assert_eq!(call.plugin_id.as_deref(), Some("linear"));
        assert_eq!(call.label(), "Linear · List issues");

        let unknown = plugin_tool_call(
            "zuse",
            "plugins_call",
            Some(&json!({"address":"tools.microsoft-learn.user.c1.microsoft_docs_fetch"})),
        )
        .unwrap();
        assert_eq!(unknown.label(), "Microsoft Learn · Microsoft docs fetch");
    }

    #[test]
    fn calls_without_an_address_yet_fall_back_to_a_neutral_label() {
        let streaming = plugin_tool_call("zuse", "plugins_call", Some(&json!({}))).unwrap();
        assert_eq!(streaming.plugin_id, None);
        assert_eq!(streaming.label(), "Plugins · Call tool");
        assert_eq!(
            plugin_tool_call("zuse", "plugins_call", None)
                .unwrap()
                .label(),
            "Plugins · Call tool"
        );
        assert_eq!(
            plugin_tool_call("zuse", "plugins_search", Some(&json!({"query":"x"})))
                .unwrap()
                .label(),
            "Plugins · Search tools"
        );
        assert_eq!(
            plugin_tool_call("zuse", "plugins_list", None)
                .unwrap()
                .label(),
            "Plugins · List tools"
        );
        assert_eq!(plugin_tool_call("zuse", "show_ui", None), None);
        assert_eq!(plugin_tool_call("gh", "plugins_call", None), None);
    }
}
