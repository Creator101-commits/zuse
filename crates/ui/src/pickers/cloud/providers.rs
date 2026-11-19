//! Which infrastructure new cloud computers use. People choose a size, never
//! a provider: new drafts take the account's default provider (or the first
//! ready one) and task bindings stay authoritative. Provider ids are internal
//! and never shown.
use serde_json::Value;
use zuse_protocol::cloud_billing::plans::{self, SizeOption};

#[derive(Clone, Debug)]
pub(super) struct Provider {
    pub id: String,
    pub reason: Option<String>,
}
impl Provider {
    pub fn ready(&self) -> bool {
        self.reason.is_none()
    }
}
const NOT_READY: &str = "Cloud computers are not ready for this account yet. Open Cloud setup, or ask your administrator to finish the managed environment.";
pub(super) fn available(context: &Value) -> Vec<Provider> {
    let mut rows: Vec<_> = context["computerCapabilities"]["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let id = row["id"].as_str()?;
            if !matches!(id, "boxd" | "boat" | "e2b" | "daytona") {
                return None;
            }
            let catalog = &context["providerAgentCatalogs"][id];
            let catalog_ready = serde_json::from_value::<
                zuse_protocol::cloud_catalog::CloudAgentCatalog,
            >(catalog.clone())
            .ok()
            .is_some_and(|catalog| {
                !catalog.providers.is_empty()
                    && catalog.providers.iter().all(|agent| {
                        !agent.models.is_empty()
                            && agent.resolve_model(&agent.default_model).is_some()
                    })
            });
            let ready = serde_json::from_value::<
                zuse_protocol::cloud_computers::ProviderCapabilities,
            >(row.clone())
            .ok()
            .is_some_and(|cap| cap.can_create() && cap.command_admission_version == Some(2))
                && catalog_ready;
            let reason = (!ready).then(|| {
                row["unavailableReason"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty() && !plans::names_infrastructure(s))
                    .map(str::to_owned)
                    .unwrap_or_else(|| NOT_READY.to_owned())
            });
            Some(Provider {
                id: id.into(),
                reason,
            })
        })
        .collect();
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    rows.dedup_by(|a, b| a.id == b.id);
    rows
}
/// The provider for a new computer: the account default when it is ready,
/// otherwise the first ready one.
pub(super) fn select(context: &Value) -> Result<String, String> {
    let rows = available(context);
    let default = context["computerCapabilities"]["defaultProvider"].as_str();
    rows.iter()
        .find(|row| Some(row.id.as_str()) == default && row.ready())
        .or_else(|| rows.iter().find(|row| row.ready()))
        .map(|row| row.id.clone())
        .ok_or_else(|| {
            rows.iter()
                .find_map(|row| row.reason.clone())
                .unwrap_or_else(|| NOT_READY.into())
        })
}
/// Sizes the account can pick, with plan limits. Empty from older servers.
pub(super) fn sizes(context: &Value) -> Vec<SizeOption> {
    serde_json::from_value(context["computerCapabilities"]["computerSizes"].clone())
        .unwrap_or_default()
}
/// "Large · 8 vCPU · 32 GB · Cloud Max" for a size the plan does not include.
pub(super) fn size_label(size: &SizeOption) -> String {
    match size.requires_plan.filter(|_| !size.available) {
        Some(plan) => format!("{} · {}", size.label, plan.plan().name),
        None => size.label.clone(),
    }
}
pub(super) const SIZE_REQUIRES_CLOUD_MAX: &str =
    "Large cloud computers need Cloud Max. Upgrade in Settings → Billing, or choose Standard.";
/// The size to send. With plan sizes, a remembered size is used while the
/// plan allows it; otherwise the provider's own sizes decide, and a literal
/// `default` is never invented for providers whose ids differ.
pub(super) fn size(
    context: &Value,
    provider: &str,
    preferred: Option<&str>,
) -> Result<String, String> {
    let unavailable =
        || "No cloud computer size is available right now. Refresh Cloud setup.".to_owned();
    let offered = sizes(context);
    if !offered.is_empty() {
        if let Some(size) = offered.iter().find(|s| Some(s.id.as_str()) == preferred) {
            return if size.available {
                Ok(size.id.clone())
            } else {
                Err(SIZE_REQUIRES_CLOUD_MAX.into())
            };
        }
        return offered
            .iter()
            .find(|s| s.available && s.id == plans::DEFAULT_SIZE)
            .or_else(|| offered.iter().find(|s| s.available))
            .map(|s| s.id.clone())
            .ok_or_else(unavailable);
    }
    let row = context["computerCapabilities"]["providers"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == provider))
        .ok_or_else(unavailable)?;
    let capability =
        serde_json::from_value::<zuse_protocol::cloud_computers::ProviderCapabilities>(row.clone())
            .map_err(|_| unavailable())?;
    if !capability.can_create() {
        return Err(unavailable());
    }
    let sizes: Vec<_> = capability
        .sizes
        .iter()
        .filter(|size| {
            !size.id.trim().is_empty() && size.vcpus > 0 && size.memory_mib > 0 && size.disk_gib > 0
        })
        .collect();
    sizes
        .iter()
        .find(|size| Some(size.id.as_str()) == preferred)
        .or_else(|| sizes.iter().find(|size| size.id == "default"))
        .or_else(|| sizes.first())
        .map(|size| size.id.clone())
        .ok_or_else(unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use zuse_protocol::cloud_billing::plans::names_infrastructure;
    fn context() -> Value {
        json!({"computerCapabilities":{"providers":[{"id":"boxd","configured":true,"managedBaseReady":false,"commandAdmissionVersion":2,"preservesDisk":true,"preservesMemory":true,"supportsSnapshots":true,"supportsDocker":false,"sizes":[{"id":"default","label":"Standard","vcpus":2,"memoryMib":2048,"diskGib":8}],"unavailableReason":"Boxd is not enabled for this staging account."},{"id":"e2b","configured":true,"managedBaseReady":true,"commandAdmissionVersion":2,"preservesDisk":true,"preservesMemory":true,"supportsSnapshots":true,"supportsDocker":false,"sizes":[{"id":"default","label":"Standard","vcpus":2,"memoryMib":2048,"diskGib":8}],"unavailableReason":null}]},"providerAgentCatalogs":{"e2b":{"providers":[{"id":"codex","name":"Codex","defaultModel":"e2b-model","models":[{"id":"e2b-model","label":"E2B model"}]}]}}})
    }
    #[test]
    fn new_computers_use_the_default_provider_or_the_first_ready_one() {
        let mut c = context();
        assert_eq!(select(&c).unwrap(), "e2b");
        // An unready default falls back to a ready provider.
        c["computerCapabilities"]["defaultProvider"] = json!("boxd");
        assert_eq!(select(&c).unwrap(), "e2b");
        c["computerCapabilities"]["defaultProvider"] = json!("e2b");
        assert_eq!(select(&c).unwrap(), "e2b");
    }
    #[test]
    fn reasons_never_name_the_infrastructure() {
        let rows = available(&context());
        let reason = rows[0].reason.as_deref().unwrap();
        assert!(!names_infrastructure(reason), "{reason}");
        let mut c = context();
        c["providerAgentCatalogs"]["e2b"] = json!({"providers":[]});
        let error = select(&c).unwrap_err();
        assert!(!names_infrastructure(&error), "{error}");
        let error = size(&c, "missing", None).unwrap_err();
        assert!(!names_infrastructure(&error), "{error}");
    }
    #[test]
    fn plan_sizes_decide_and_large_needs_cloud_max() {
        let mut c = context();
        c["computerCapabilities"]["computerSizes"] = json!([
            {"id":"default","label":"Standard · 4 vCPU · 16 GB","vcpus":4,"memoryMib":16384,"machineRate":4,"available":true},
            {"id":"large","label":"Large · 8 vCPU · 32 GB","vcpus":8,"memoryMib":32768,"machineRate":8,"available":false,"requiresPlan":"cloud-max"}
        ]);
        assert_eq!(size(&c, "e2b", None).unwrap(), "default");
        assert_eq!(size(&c, "e2b", Some("small")).unwrap(), "default");
        assert_eq!(
            size(&c, "e2b", Some("large")).unwrap_err(),
            SIZE_REQUIRES_CLOUD_MAX
        );
        let labels: Vec<_> = sizes(&c).iter().map(size_label).collect();
        assert_eq!(
            labels,
            [
                "Standard · 4 vCPU · 16 GB",
                "Large · 8 vCPU · 32 GB · Cloud Max"
            ]
        );
        c["computerCapabilities"]["computerSizes"][1]["available"] = json!(true);
        assert_eq!(size(&c, "e2b", Some("large")).unwrap(), "large");
        assert_eq!(size_label(&sizes(&c)[1]), "Large · 8 vCPU · 32 GB");
    }
    #[test]
    fn size_selection_uses_supported_preference_or_advertised_default_and_never_invents_a_size() {
        let mut c = context();
        let standard =
            json!({"id":"default","label":"Standard","vcpus":2,"memoryMib":2048,"diskGib":8});
        c["computerCapabilities"]["providers"][1]["sizes"] = json!([{"id":"large","label":"Large","vcpus":4,"memoryMib":8192,"diskGib":16},standard]);
        assert_eq!(size(&c, "e2b", Some("large")).unwrap(), "large");
        assert_eq!(size(&c, "e2b", Some("small")).unwrap(), "default");
        c["computerCapabilities"]["providers"][1]["id"] = json!("boat");
        c["computerCapabilities"]["providers"][1]["sizes"] =
            json!([{"id":"small","label":"Small","vcpus":2,"memoryMib":4096,"diskGib":51}]);
        assert_eq!(size(&c, "boat", Some("default")).unwrap(), "small");
        c["computerCapabilities"]["providers"][1]["sizes"] = json!([]);
        assert!(size(&c, "boat", None).is_err());
        assert!(size(&c, "missing", None).is_err());
    }
}
