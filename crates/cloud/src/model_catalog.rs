//! The curated Claude Code catalogue cloud computers offer. This catalogue never probes a CLI.
//!
//! Managed bases carry the same Claude Code catalogue as the desktop's local
//! picker: base candidates read the checked-in copy
//! `infra/cloud/managed-base-claude-catalog.json`, which a test keeps
//! byte-for-byte equal to [`claude_code`].
use zuse_protocol::{
    HarnessId,
    cloud_catalog::{CloudAgentCatalog, CloudAgentModels},
};

/// Default kept from the first cloud catalogue so existing launch defaults do not move.
pub const CLAUDE_CODE_DEFAULT_MODEL: &str = "claude-sonnet-5";

/// Claude Code's curated models, labels, effort ladders and options. Model ids are the
/// concrete ids the Claude Code CLI accepts with `--model`; the aliases `opus`, `sonnet`,
/// `haiku` and `fable` still resolve through [`CloudAgentModels::resolve_model`].
pub fn claude_code() -> CloudAgentModels {
    CloudAgentModels {
        id: HarnessId::ClaudeCode,
        name: "Claude Code".into(),
        default_model: CLAUDE_CODE_DEFAULT_MODEL.into(),
        models: zuse_providers::claude::catalog::static_models(),
    }
}

/// Canonical JSON for the managed-base catalogue file (stable key order, trailing newline).
pub fn claude_code_base_catalog_json() -> String {
    let mut value = serde_json::to_value(claude_code()).expect("catalog serializes");
    // `preserve_order` can be on through the dependency graph; sort every nested object.
    value.sort_all_objects();
    let mut text = serde_json::to_string_pretty(&value).expect("catalog serializes");
    text.push('\n');
    text
}

pub fn launch_catalog() -> CloudAgentCatalog {
    CloudAgentCatalog {
        providers: vec![claude_code()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zuse_protocol::ReasoningLevel;

    const BASE_CATALOG: &str =
        include_str!("../../../infra/cloud/managed-base-claude-catalog.json");

    #[test]
    fn managed_base_catalog_file_matches_the_curated_catalog() {
        assert_eq!(
            BASE_CATALOG,
            claude_code_base_catalog_json(),
            "infra/cloud/managed-base-claude-catalog.json is stale; regenerate it with \
             `cargo run -p zuse-cloud-api -- print-claude-base-catalog > infra/cloud/managed-base-claude-catalog.json`"
        );
        let file: CloudAgentModels = serde_json::from_str(BASE_CATALOG).unwrap();
        assert_eq!(
            file.models,
            zuse_providers::claude::catalog::static_models()
        );
        assert_eq!(file.id, HarnessId::ClaudeCode);
        assert_eq!(file.default_model, CLAUDE_CODE_DEFAULT_MODEL);
    }

    #[test]
    fn base_catalog_offers_every_family_with_effort_levels() {
        let catalog = claude_code();
        assert!(catalog.models.len() > 1);
        assert!(catalog.resolve_model(&catalog.default_model).is_some());
        for family in ["claude-opus-", "claude-sonnet-", "claude-fable-"] {
            let model = catalog
                .models
                .iter()
                .find(|m| m.id.starts_with(family))
                .unwrap_or_else(|| panic!("{family} missing"));
            assert!(!model.label.is_empty() && model.label != model.id);
            for level in [
                ReasoningLevel::Low,
                ReasoningLevel::Medium,
                ReasoningLevel::High,
                ReasoningLevel::Max,
            ] {
                assert!(
                    model.reasoning_levels.contains(&level),
                    "{family} {level:?}"
                );
            }
        }
        let haiku = catalog
            .models
            .iter()
            .find(|m| m.id.starts_with("claude-haiku-"))
            .expect("haiku missing");
        assert!(haiku.options.iter().any(|o| o.id == "thinking"));
    }

    #[test]
    fn aliases_and_concrete_ids_resolve() {
        let catalog = claude_code();
        for (alias, family) in [
            ("opus", "claude-opus-"),
            ("sonnet", "claude-sonnet-"),
            ("haiku", "claude-haiku-"),
            ("fable", "claude-fable-"),
        ] {
            let model = catalog
                .resolve_model(alias)
                .unwrap_or_else(|| panic!("{alias} does not resolve"));
            assert!(model.id.starts_with(family), "{alias} -> {}", model.id);
        }
        assert_eq!(catalog.resolve_model("opus").unwrap().id, "claude-opus-5-5");
        for model in &catalog.models {
            assert_eq!(catalog.resolve_model(&model.id), Some(model));
        }
        assert!(catalog.resolve_model("claude-unknown-1").is_none());
    }
}
