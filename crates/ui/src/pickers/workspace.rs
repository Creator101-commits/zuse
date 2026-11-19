//! Shared workspace menu presentation over the native checkout controller.
use super::*;
use zuse_protocol::presentation::WorkspaceMode;

/// One row of the workspace menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceChoice {
    Worktree,
    Local,
    /// Runs on the account's cloud computer.
    Cloud,
}

impl Pickers {
    pub(super) fn render_creation_workspace(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.cloud.task_binding.is_some() || self.workspace_choices(cx).len() < 2 {
            return None;
        }
        // Cloud's repositories load with the menu, so picking Cloud is instant.
        self.ensure_cloud(cx);
        let on_cloud = self.state.read(cx).cloud_project.is_some();
        if !on_cloud {
            self.ensure_refs(false, cx);
        }
        let theme = Theme::of(cx).clone();
        let mut trigger = self
            .tray_chip(
                PickerKind::Checkout,
                "picker-checkout",
                if on_cloud {
                    crate::icons::DITHER_CLOUD
                } else if self.config.checkout == CheckoutKind::NewWorktree {
                    crate::icons::DITHER_BRANCH
                } else {
                    crate::icons::DITHER_FOLDER
                },
                if on_cloud {
                    "Cloud".into()
                } else {
                    self.checkout_label().into()
                },
                &theme,
                cx,
            )
            .flex_none();
        if self.mounted_kind() == Some(PickerKind::Checkout) {
            let content = self.render_checkout_popover(cx);
            let popup = self.popover_frame(224., content, cx);
            trigger = trigger.child(popover::anchored_menu_above(
                "checkout-popover",
                popup,
                self.open.closing_since(),
            ));
        }
        Some(trigger.into_any_element())
    }

    pub(super) fn remembered_checkout(
        state: &Entity<AppState>,
        defaults: &ComposerDefaults,
        cx: &App,
    ) -> CheckoutKind {
        let Some(space) = state
            .read(cx)
            .selected_space_row()
            .filter(|s| s.git_detected)
        else {
            return CheckoutKind::Local;
        };
        match defaults
            .workspace_by_project
            .get(&zuse_protocol::view::project_key(space))
            .copied()
            .unwrap_or_default()
        {
            WorkspaceMode::Worktree => CheckoutKind::NewWorktree,
            WorkspaceMode::Local => CheckoutKind::Local,
        }
    }

    /// The workspace menu's rows: Worktree and Local for a git checkout
    /// here, and Cloud for a subscriber (or a chat already on Cloud).
    pub(super) fn workspace_choices(&self, cx: &App) -> Vec<WorkspaceChoice> {
        let state = self.state.read(cx);
        let mut choices = Vec::new();
        if state.selected_space_row().is_some_and(|s| s.git_detected) {
            choices.extend([WorkspaceChoice::Worktree, WorkspaceChoice::Local]);
        }
        if state.cloud_project.is_some()
            || (crate::cloud_beta()
                && zuse_viewport::cloud_gate::enabled()
                && state.cloud_access().subscribed())
        {
            choices.push(WorkspaceChoice::Cloud);
        }
        choices
    }

    /// The selected row of the workspace menu.
    pub(super) fn workspace_choice(&self, cx: &App) -> WorkspaceChoice {
        if self.state.read(cx).cloud_project.is_some() {
            WorkspaceChoice::Cloud
        } else if self.config.checkout == CheckoutKind::NewWorktree {
            WorkspaceChoice::Worktree
        } else {
            WorkspaceChoice::Local
        }
    }

    pub(super) fn pick_workspace(&mut self, choice: WorkspaceChoice, cx: &mut Context<Self>) {
        match choice {
            WorkspaceChoice::Cloud => self.pick_cloud_workspace(cx),
            WorkspaceChoice::Worktree | WorkspaceChoice::Local => {
                if self.state.read(cx).cloud_project.is_some() {
                    self.leave_cloud(cx);
                }
                self.pick_checkout(
                    if choice == WorkspaceChoice::Worktree {
                        CheckoutKind::NewWorktree
                    } else {
                        CheckoutKind::Local
                    },
                    cx,
                );
            }
        }
    }

    pub(super) fn render_checkout_popover(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.workspace_choice(cx);
        let cloud_detail = self.cloud_workspace_detail(cx);
        div()
            .flex()
            .flex_col()
            .child(zuse_viewport::creation_style::workspace_heading())
            .children(
                self.workspace_choices(cx)
                    .into_iter()
                    .enumerate()
                    .map(|(i, choice)| {
                        let active = self.workspace_keyboard && self.active == i;
                        match choice {
                            WorkspaceChoice::Cloud => {
                                zuse_viewport::creation_style::cloud_workspace_option(
                                    ("checkout-row", i),
                                    cloud_detail,
                                    current == choice,
                                    active,
                                )
                            }
                            _ => zuse_viewport::creation_style::workspace_option(
                                ("checkout-row", i),
                                choice == WorkspaceChoice::Worktree,
                                current == choice,
                                active,
                            ),
                        }
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.pick_workspace(choice, cx)),
                        )
                    }),
            )
            .into_any_element()
    }
}
