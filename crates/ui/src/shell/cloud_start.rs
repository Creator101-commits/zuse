//! Cloud-only projects use the current native new-conversation canvas.
use super::*;
impl Shell {
    pub(super) fn open_new_cloud_session(
        &mut self,
        project: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        // New cloud chats are for Cloud subscribers; anyone else lands on
        // Billing (or General when signed out) to subscribe first.
        if self.state.read(cx).cloud_access().withholds_cloud() {
            self.open_settings(SettingsSection::Billing, cx);
            return;
        }
        self.sync_cloud_tab_scope(cx);
        if !self.cloud_tab_navigation_ready(cx) {
            return;
        }
        let project = self
            .state
            .read(cx)
            .cloud_sidebar_projects()
            .into_iter()
            .find(|row| {
                row["projectId"] == project["projectId"] && row["accountId"] == project["accountId"]
            });
        let Some(project) = project else {
            self.open_settings(SettingsSection::Devices, cx);
            return;
        };
        self.state
            .update(cx, |state, _| state.cloud_project = Some(project.clone()));
        // Cloud is a destination of the native composer, not another window.
        // Bypass open_new_session's cloud-only dispatch to avoid recursion.
        self.reset_new_session_canvas(None, cx);
    }
}
