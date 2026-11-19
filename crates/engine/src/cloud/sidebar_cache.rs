//! Read-only catalog snapshots, never compute authority or agent credentials.
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
const PROJECTS: &str = zuse_protocol::methods::CLOUD_PROJECTS;
const LIST: &str = zuse_protocol::methods::CLOUD_WORKSPACES;
pub(super) fn supports(method: &str) -> bool {
    matches!(method, PROJECTS | LIST)
}
fn namespace(origin: &str, account: &str, organization: Option<&str>, method: &str) -> String {
    zuse_cloud_projection::digest(
        &serde_json::to_vec(&(origin, account, organization, method)).unwrap(),
    )
}
/// How long a task archived here stays out of saved lists. A list read that
/// started before the server recorded the archive can land afterwards; it
/// must not put the row back on disk, where a restart would show it again.
const ARCHIVED_HOLD: Duration = Duration::from_secs(10 * 60);
static ARCHIVED: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
fn recently_archived(workspace: &str) -> bool {
    let mut archived = ARCHIVED.lock().unwrap_or_else(|e| e.into_inner());
    archived.retain(|_, at| at.elapsed() < ARCHIVED_HOLD);
    archived.contains_key(workspace)
}
/// A task was archived: drop it from the saved list now and keep it out of
/// lists saved over the next few minutes.
pub(super) fn archived(origin: &str, account: &str, organization: Option<&str>, workspace: &str) {
    ARCHIVED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(workspace.to_owned(), Instant::now());
    let key = namespace(origin, account, organization, LIST);
    if let Some(saved) = zuse_cloud_projection::protected_metadata::read(&key) {
        zuse_cloud_projection::protected_metadata::write(
            &key,
            &without_workspace(saved, workspace),
        );
    }
}
/// A task came back from the archive; saved lists may list it again.
pub(super) fn unarchived(workspace: &str) {
    ARCHIVED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(workspace);
}
fn without_workspace(mut saved: Value, workspace: &str) -> Value {
    if let Some(rows) = saved["workspaces"].as_array_mut() {
        rows.retain(|row| row["workspaceId"].as_str() != Some(workspace));
    }
    saved
}
fn snapshot(method: &str, value: &Value) -> Value {
    snapshot_without(method, value, &recently_archived)
}
fn snapshot_without(method: &str, value: &Value, hidden: &dyn Fn(&str) -> bool) -> Value {
    let (rows, fields): (&Value, &[&str]) = if method == PROJECTS {
        (
            &value["data"]["projects"],
            &[
                "projectId",
                "displayName",
                "repositoryIdentity",
                "repositoryUrl",
                "name",
                "title",
                "defaultBranch",
                "baseRef",
                "providerId",
                "environmentRevision",
                "environmentConfigured",
                "canCreate",
            ],
        )
    } else {
        (
            &value["workspaces"],
            &[
                "workspaceId",
                "projectId",
                "title",
                "branch",
                "state",
                "desiredState",
                "runtimeState",
                "initialSessionId",
                "chatId",
                "providerId",
                "updatedAt",
                "createdAt",
                "rpcProtocol",
                "statusCode",
                // The agent id (never its connection) lets a saved row show
                // the right agent icon before the fresh list arrives.
                "agent",
            ],
        )
    };
    let mut saved = vec![];
    let mut partial = false;
    for row in rows.as_array().into_iter().flatten() {
        if method == LIST && row["workspaceId"].as_str().is_some_and(hidden) {
            continue;
        }
        let mut clean = serde_json::Map::new();
        for key in fields {
            if let Some(value) = row.get(*key) {
                if value.is_string() || value.is_boolean() || value.is_number() || value.is_null() {
                    clean.insert((*key).to_owned(), value.clone());
                }
            }
        }
        saved.push(Value::Object(clean));
        if serde_json::to_vec(&saved).map_or(true, |v| v.len() > 60000) {
            saved.pop();
            partial = true;
            break;
        }
    }
    if method == PROJECTS {
        json!({"data":{"projects":saved},"cachePartial":partial})
    } else {
        json!({"workspaces":saved,"cachePartial":partial})
    }
}
pub(super) fn read(origin: &str, account: &str, organization: Option<&str>, method: &str) -> Value {
    let mut value = zuse_cloud_projection::protected_metadata::read(&namespace(
        origin,
        account,
        organization,
        method,
    ))
    .unwrap_or_else(|| json!({"cacheMiss":true}));
    value["accountId"] = json!(account);
    value["organizationId"] = json!(organization);
    value["apiOrigin"] = json!(origin);
    value["provisional"] = json!(true);
    value
}
pub(super) fn store(
    origin: &str,
    account: &str,
    organization: Option<&str>,
    method: &str,
    value: &Value,
) {
    if supports(method) {
        zuse_cloud_projection::protected_metadata::write(
            &namespace(origin, account, organization, method),
            &snapshot(method, value),
        );
    }
}
pub(super) fn clear_all() {
    zuse_cloud_projection::protected_metadata::clear(None);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_namespace_isolates_origin_actor_org_and_kind() {
        let base = namespace("https://a", "actor", None, PROJECTS);
        for other in [
            namespace("https://b", "actor", None, PROJECTS),
            namespace("https://a", "other", None, PROJECTS),
            namespace("https://a", "actor", Some("org"), PROJECTS),
            namespace("https://a", "actor", None, LIST),
        ] {
            assert_ne!(base, other);
        }
    }
    #[test]
    fn stored_catalog_has_no_credentials_authority_or_transcript_payloads() {
        let v = snapshot(
            PROJECTS,
            &json!({"data":{"projects":[{"projectId":"p","repositoryIdentity":"github.com/o/p","token":"secret","environment":{"secret":"hidden"}}]},"agentConnections":[{"credential":"secret"}]}),
        );
        assert_eq!(v["data"]["projects"][0]["projectId"], "p");
        let text = v.to_string();
        assert!(
            !text.contains("secret")
                && !text.contains("credential")
                && !text.contains("agentConnections")
        );
        let v = snapshot(
            LIST,
            &json!({"workspaces":[{"workspaceId":"w","state":"paused","agent":"codex","agentConnectionId":"connection-secret","bootstrap":{"secret":"hidden"},"messages":["private text"]}]}),
        );
        assert_eq!(v["workspaces"][0]["state"], "paused");
        assert_eq!(v["workspaces"][0]["agent"], "codex");
        assert!(!v.to_string().contains("private text"));
        assert!(!v.to_string().contains("connection-secret"));
    }
    #[test]
    fn archived_tasks_leave_the_saved_list_and_stale_lists_cannot_save_them_again() {
        let saved = snapshot(
            LIST,
            &json!({"workspaces":[{"workspaceId":"kept","state":"paused"},{"workspaceId":"gone","state":"paused"}]}),
        );
        // Archiving rewrites the saved list without the task, so a restart
        // paints only the rows that are still listed.
        let saved = without_workspace(saved, "gone");
        assert_eq!(saved["workspaces"].as_array().unwrap().len(), 1);
        assert_eq!(saved["workspaces"][0]["workspaceId"], "kept");
        // A list read before the server recorded the archive still lists the
        // task as paused; it is not saved back.
        let stale = snapshot_without(
            LIST,
            &json!({"workspaces":[{"workspaceId":"kept","state":"paused"},{"workspaceId":"gone","state":"paused"}]}),
            &|id| id == "gone",
        );
        assert_eq!(stale["workspaces"].as_array().unwrap().len(), 1);
        assert_eq!(stale["workspaces"][0]["workspaceId"], "kept");
    }
    #[test]
    fn unarchived_tasks_are_saved_again() {
        ARCHIVED
            .lock()
            .unwrap()
            .insert("restored-task".into(), Instant::now());
        assert!(recently_archived("restored-task"));
        unarchived("restored-task");
        assert!(!recently_archived("restored-task"));
    }
}
