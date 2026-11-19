//! Local preparation status for an agent that starts before its checkout.
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};
use tokio_util::sync::CancellationToken;
use zuse_protocol::cloud::CloudRuntimeBootstrap;
pub const FILE: &str = "workspace-preparation.json";
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Status {
    workspace: String,
    generation: i64,
    disk: String,
    state: String,
}
fn status(boot: &CloudRuntimeBootstrap, state: &str) -> Result<Status> {
    Ok(Status {
        workspace: boot.workspace_id.clone(),
        generation: boot.runtime_generation,
        disk: boot
            .runtime_storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("runtime_storage_missing"))?
            .storage_incarnation_id
            .clone(),
        state: state.into(),
    })
}
pub fn publish(data: &Path, boot: &CloudRuntimeBootstrap, state: &str) -> Result<()> {
    ensure!(
        ["pending", "ready", "failed", "cancelled"].contains(&state),
        "workspace_preparation_state_invalid"
    );
    crate::identity::atomic_write(
        &data.join(FILE),
        &serde_json::to_vec(&status(boot, state)?)?,
    )
}
pub async fn wait(
    data: &Path,
    workspace: &str,
    generation: i64,
    disk: &str,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let path = data.join(FILE);
        let meta = std::fs::symlink_metadata(&path)?;
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 4096,
            "workspace_preparation_status_invalid"
        );
        let current: Status = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(
            current.workspace == workspace,
            "workspace_preparation_fenced: this check names a different workspace; do not use it here"
        );
        // A command copied from an earlier message can name the generation
        // and disk of a computer that was since recreated. The agent's
        // conversation survives recreation, so it may run that command again.
        // The newer computer's own status is the one that applies: it was
        // published by the runtime that now owns this workspace. Only a status
        // older than the command (its runtime has not published yet) or a
        // different disk within one generation stays unknown.
        let state = if current.generation > generation
            || (current.generation == generation && current.disk == disk)
        {
            current.state.as_str()
        } else if current.generation < generation {
            "pending"
        } else {
            bail!(
                "workspace_preparation_fenced: this check names a different disk for this computer; do not use it here"
            )
        };
        match state {
            "ready" => return Ok(()),
            "failed" => bail!(
                "workspace_preparation_failed: preparing the project files did not finish. Tell the user preparation failed; the chat shows which step"
            ),
            "cancelled" => {
                bail!("workspace_preparation_cancelled: preparation was stopped before it finished")
            }
            "pending" => {}
            _ => bail!("workspace_preparation_status_invalid"),
        }
        tokio::select! {
            _ = cancel.cancelled() => bail!("workspace_preparation_cancelled"),
            _ = tokio::time::sleep_until(deadline) => bail!("workspace_preparation_timeout"),
            _ = tokio::time::sleep(Duration::from_millis(50)) => {},
        }
    }
}
pub fn needs_wait(data: &Path, boot: &CloudRuntimeBootstrap) -> Result<bool> {
    let path = data.join(FILE);
    let meta = std::fs::symlink_metadata(&path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 4096,
        "workspace_preparation_status_invalid"
    );
    let current: Status = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        current == status(boot, &current.state)?,
        "workspace_preparation_fenced"
    );
    Ok(current.state != "ready")
}
/// The installed runtime path for the base this runtime runs from. A home
/// directory base (Prized, no root) has no `/usr/local/bin` copy.
fn runtime_binary(executable: Option<&Path>) -> &'static str {
    if executable.is_some_and(|exe| exe.starts_with("/home/prized/.zuse/")) {
        "/home/prized/.zuse/bin/zuse-cloud-runtime"
    } else {
        "/usr/local/bin/zuse-cloud-runtime"
    }
}
pub fn instruction(data: &Path, boot: &CloudRuntimeBootstrap) -> Result<String> {
    let binding = status(boot, "pending")?;
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    Ok(format!(
        "\n\n[Workspace preparation]\nYour project files, branch and required setup are still being prepared. You may reason from the user's request now. Before reading, searching, changing or running project files, run `{} --data-dir {} --workspace-id {} --generation {} --storage-incarnation-id {} workspace-wait`. Continue only if it succeeds, then read the applicable AGENTS.md and other project instructions. Missing files during preparation are not evidence that they should be created. If preparation fails, explain that failure and wait for recovery; do not retry the user's task independently. This note applies to this message only: a later message without this note means preparation is already complete, so do not run this command again.\n",
        runtime_binary(std::env::current_exe().ok().as_deref()),
        quote(&data.to_string_lossy()),
        quote(&binding.workspace),
        binding.generation,
        quote(&binding.disk)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn boot() -> CloudRuntimeBootstrap {
        serde_json::from_value(serde_json::json!({"workspaceId":"task","zuseAccountId":"owner","chatId":"chat","initialSessionId":"session","runtimeCredential":"secret","runtimeGeneration":1,"gatewayEpoch":1,"runtimeCredentialExpiresAt":9007199254740000i64,"gatewayUrl":"wss://fixture.invalid","gatewayProtocol":"fixture","sealedTranscriptKey":"secret","runtimeStorage":{"format":"zuse-rust-engine-v1","storageIncarnationId":"disk"}})).unwrap()
    }
    #[tokio::test]
    async fn wait_observes_success_failure_timeout_and_binding_without_network() {
        let root = tempfile::tempdir().unwrap();
        let boot = boot();
        let cancel = CancellationToken::new();
        publish(root.path(), &boot, "pending").unwrap();
        assert!(needs_wait(root.path(), &boot).unwrap());
        assert!(
            wait(
                root.path(),
                "task",
                1,
                "disk",
                Duration::from_millis(5),
                &cancel
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("timeout")
        );
        publish(root.path(), &boot, "ready").unwrap();
        assert!(!needs_wait(root.path(), &boot).unwrap());
        wait(
            root.path(),
            "task",
            1,
            "disk",
            Duration::from_secs(1),
            &cancel,
        )
        .await
        .unwrap();
        // A newer command than the published status waits for its runtime.
        assert!(
            wait(
                root.path(),
                "task",
                2,
                "disk",
                Duration::from_millis(5),
                &cancel
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("timeout")
        );
        for (workspace, disk) in [("other", "disk"), ("task", "other-disk")] {
            assert!(
                wait(
                    root.path(),
                    workspace,
                    1,
                    disk,
                    Duration::from_secs(1),
                    &cancel
                )
                .await
                .unwrap_err()
                .to_string()
                .contains("fenced")
            );
        }
        publish(root.path(), &boot, "failed").unwrap();
        assert!(
            wait(
                root.path(),
                "task",
                1,
                "disk",
                Duration::from_secs(1),
                &cancel
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("failed")
        );
        assert!(!instruction(root.path(), &boot).unwrap().contains("secret"));
    }

    /// A recreated computer keeps the agent conversation, including the
    /// generation-1 wait command from its first message. Running that command
    /// again must report the current computer's preparation, not fail.
    #[tokio::test]
    async fn a_command_from_before_recreation_reports_the_current_computer() {
        let root = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        let mut recreated = boot();
        recreated.runtime_generation = 2;
        recreated
            .runtime_storage
            .as_mut()
            .unwrap()
            .storage_incarnation_id = "new-disk".into();
        publish(root.path(), &recreated, "ready").unwrap();
        for disk in ["disk", "new-disk"] {
            wait(
                root.path(),
                "task",
                1,
                disk,
                Duration::from_secs(1),
                &cancel,
            )
            .await
            .unwrap();
        }
        publish(root.path(), &recreated, "failed").unwrap();
        assert!(
            wait(
                root.path(),
                "task",
                1,
                "disk",
                Duration::from_secs(1),
                &cancel
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("workspace_preparation_failed")
        );
    }
}
