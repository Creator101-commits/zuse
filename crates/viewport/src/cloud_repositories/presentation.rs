//! Cloud Setup catalog presentation. Mutations remain owned by RepositoryPanel.
use super::*;
use crate::cloud_settings::kit;
use crate::tooltip::TooltipExt as _;
pub(super) fn avatar(name: &str, url: Option<&str>, size: f32) -> AnyElement {
    let owner = name.split('/').next().unwrap_or(name);
    let letters = if size < 22. { 1 } else { 2 };
    let fallback = owner
        .chars()
        .take(letters)
        .collect::<String>()
        .to_uppercase();
    let frame = div()
        .size(px(size))
        .flex_none()
        .rounded_full()
        .overflow_hidden()
        .bg(s::frame_bg())
        .flex()
        .items_center()
        .justify_center()
        .text_size(px((size * 0.45).round().max(9.)))
        .font_weight(FontWeight::MEDIUM)
        .text_color(palette::rgb(MUTED));
    match url {
        // An image that can't load keeps the initials tile.
        Some(url) => frame
            .child(
                img(SharedString::from(url.to_owned()))
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .with_fallback(move || {
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(fallback.clone())
                            .into_any_element()
                    }),
            )
            .into_any_element(),
        None => frame.child(fallback).into_any_element(),
    }
}
/// "18h ago", "3w ago", "4y ago" for a GitHub ISO timestamp.
pub(super) fn ago(iso: &str) -> Option<String> {
    let then = chrono::DateTime::parse_from_rfc3339(iso).ok()?;
    let secs = (chrono::Utc::now() - then.with_timezone(&chrono::Utc))
        .num_seconds()
        .max(0);
    Some(match secs {
        s if s < 3_600 => format!("{}m ago", (s / 60).max(1)),
        s if s < 86_400 => format!("{}h ago", s / 3_600),
        s if s < 7 * 86_400 => format!("{}d ago", s / 86_400),
        s if s < 365 * 86_400 => format!("{}w ago", s / (7 * 86_400)),
        s => format!("{}y ago", s / (365 * 86_400)),
    })
}
/// One entry of an overflow menu.
pub(super) struct MenuItem {
    pub id: SharedString,
    pub glyph: &'static str,
    pub label: SharedString,
    pub danger: bool,
    pub run: std::rc::Rc<dyn Fn(&mut RepositoryPanel, &mut Window, &mut Context<RepositoryPanel>)>,
}
/// GitHub's settings page for one installation: where its owner changes
/// which repositories the app can access.
pub(super) fn installation_url(
    install: &zuse_protocol::cloud_projects::GithubInstallation,
) -> String {
    if install.account_type.eq_ignore_ascii_case("organization") {
        format!(
            "https://github.com/organizations/{}/settings/installations/{}",
            install.account_login, install.installation_id
        )
    } else {
        format!(
            "https://github.com/settings/installations/{}",
            install.installation_id
        )
    }
}
/// "All repositories", or how many the installation was given.
pub(super) fn access_summary(selection: &str, selected: usize) -> String {
    if selection == "selected" {
        format!("{selected} selected")
    } else {
        "All repositories".into()
    }
}
fn owner_of(name: &str) -> &str {
    name.split('/').next().unwrap_or(name)
}
impl RepositoryPanel {
    pub(super) fn github_connected(&self) -> bool {
        self.github
            .as_ref()
            .is_some_and(|g| g.installations.iter().any(|i| !i.suspended))
    }
    /// A "…" button that opens a small menu of `items` below it. One menu is
    /// open at a time; clicking elsewhere closes it. Opened from the
    /// keyboard, its first item takes focus; arrows move, Enter or Space
    /// chooses, Esc closes back to the button.
    pub(super) fn overflow_menu(
        &self,
        id: SharedString,
        label: SharedString,
        disabled: bool,
        items: Vec<MenuItem>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.overflow.as_ref() == Some(&id);
        let toggle = id.clone();
        let count = items.len().min(self.menu_focus.len());
        let mut trigger = div().relative().child(
            cs::icon_button(id.clone(), "more-horizontal", label, disabled, false)
                .when(open, |b| b.track_focus(&self.menu_trigger))
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    this.overflow = if this.overflow.as_ref() == Some(&toggle) {
                        None
                    } else {
                        Some(toggle.clone())
                    };
                    if this.overflow.is_some() && e.is_keyboard() && count > 0 {
                        window.focus(&this.menu_focus[0], cx);
                    }
                    cx.notify();
                })),
        );
        if open && !disabled {
            let handles: Vec<FocusHandle> = self.menu_focus[..count].to_vec();
            let mut menu = kit::menu_surface()
                .id(SharedString::from(format!("{id}-menu")))
                .role(Role::Menu)
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.overflow = None;
                    cx.notify();
                }))
                .capture_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                    let at = handles.iter().position(|h| h.is_focused(window));
                    let step = |delta: usize| at.map_or(0, |i| (i + delta) % handles.len().max(1));
                    match e.keystroke.key.as_str() {
                        "escape" => {
                            this.overflow = None;
                            window.focus(&this.menu_trigger, cx);
                        }
                        "down" if !handles.is_empty() => window.focus(&handles[step(1)], cx),
                        "up" if !handles.is_empty() => {
                            window.focus(&handles[step(handles.len() - 1)], cx)
                        }
                        _ => return,
                    }
                    cx.stop_propagation();
                    cx.notify();
                }));
            for (i, item) in items.into_iter().enumerate() {
                let run = item.run.clone();
                let row = kit::menu_item(item.id, item.glyph, item.label, item.danger)
                    .focus_visible(|v| v.bg(s::frame_bg()))
                    .when_some(self.menu_focus.get(i), |row, handle| {
                        row.track_focus(handle)
                    });
                menu = menu.child(row.on_click(cx.listener(move |this, _, window, cx| {
                    this.overflow = None;
                    run(this, window, cx);
                    cx.notify();
                })));
            }
            trigger = trigger.child(
                div()
                    .absolute()
                    .right(px(0.))
                    .top(relative(1.))
                    .child(deferred(
                        anchored()
                            .anchor(Anchor::TopRight)
                            .offset(point(px(0.), px(4.)))
                            .snap_to_window_with_margin(px(8.))
                            .child(menu),
                    )),
            );
        }
        trigger.into_any_element()
    }
    pub(super) fn repositories(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let busy = self.repository_busy();
        let can_manage = self.can_manage();
        let empty = self.projects.is_empty() && self.adding.is_empty();
        let mut body = column();
        if empty {
            body = body.child(if busy && self.github.is_none() {
                kit::empty_state(
                    "dither-branch",
                    crate::localization::text(
                        "projects-project_setup_dialog_loading_repositories",
                        cx,
                    ),
                    "",
                    None,
                )
            } else if !self.github_connected() {
                kit::empty_state("dither-branch", "Connect GitHub first", "", None)
            } else {
                kit::empty_state(
                    "dither-branch",
                    "No repositories yet",
                    "",
                    Some(self.add_action(cx)),
                )
            });
        }
        for (i, project) in self.projects.iter().enumerate() {
            let id = project.project_id.clone();
            let identity = project
                .repository_identity
                .trim_start_matches("github.com/");
            let repo = self.github.as_ref().and_then(|g| {
                g.repositories
                    .iter()
                    .find(|r| r.name_with_owner.eq_ignore_ascii_case(identity))
            });
            let removing = self.pending.values().any(|p| p.kind == "remove");
            let name = project.display_name.clone();
            let remove = MenuItem {
                id: SharedString::from(format!("remove-cloud-repository-{id}")),
                glyph: "trash-bin-minimalistic",
                label: "Remove repository".into(),
                danger: true,
                run: std::rc::Rc::new({
                    let id = id.clone();
                    move |this: &mut RepositoryPanel,
                          _: &mut Window,
                          cx: &mut Context<RepositoryPanel>| {
                        if this.repository_busy() || !this.can_manage() {
                            return;
                        }
                        this.error = None;
                        this.notice = None;
                        this.call(
                            "remove",
                            methods::CLOUD_PROJECT_REMOVE,
                            json!({"projectId":id}),
                            cx,
                        );
                    }
                }),
            };
            let overflow = self.overflow_menu(
                SharedString::from(format!("cloud-repository-more-{id}")),
                format!("More for {name}").into(),
                busy || !can_manage || removing,
                vec![remove],
                cx,
            );
            body = body.child(
                row()
                    .h(px(48.))
                    .gap(px(10.))
                    .px(px(12.))
                    .when(i > 0, |v| {
                        v.border_t_1()
                            .border_color(palette::rgb(BORDER).opacity(0.4))
                    })
                    .child(avatar(
                        identity,
                        repo.and_then(|r| r.owner_avatar_url.as_deref()),
                        24.,
                    ))
                    .child(
                        column()
                            .flex_1()
                            .min_w_0()
                            .child(s::title_text(name).truncate())
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .line_height(px(13.5))
                                    .text_color(palette::rgb(MUTED))
                                    .child(format!(
                                        "{} · {}",
                                        project.default_branch,
                                        match project.visibility {
                                            zuse_protocol::cloud_projects::ProjectVisibility::Private => "Private",
                                            zuse_protocol::cloud_projects::ProjectVisibility::Public => "Public",
                                        },
                                    )),
                            ),
                    )
                    .when(self.page != CloudPage::All, |v| {
                        let id = id.clone();
                        v.child(
                            cs::button(
                                SharedString::from(format!("cloud-repository-setup-{id}")),
                                "Setup",
                                false,
                                false,
                                false,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_environment(id.clone(), cx);
                                cx.emit(OpenPage(CloudPage::Environments));
                            })),
                        )
                    })
                    .child(overflow),
            );
        }
        // Chosen in the palette, still connecting: shown at once.
        for (i, lower) in self.adding.iter().enumerate() {
            let repo = self.github.as_ref().and_then(|g| {
                g.repositories
                    .iter()
                    .find(|r| r.name_with_owner.eq_ignore_ascii_case(lower))
            });
            let name = repo.map_or(lower.clone(), |r| r.name_with_owner.clone());
            body = body.child(
                row()
                    .h(px(48.))
                    .gap(px(10.))
                    .px(px(12.))
                    .when(i > 0 || !self.projects.is_empty(), |v| {
                        v.border_t_1()
                            .border_color(palette::rgb(BORDER).opacity(0.4))
                    })
                    .child(avatar(
                        &name,
                        repo.and_then(|r| r.owner_avatar_url.as_deref()),
                        24.,
                    ))
                    .child(s::title_text(name.clone()).flex_1().min_w_0().truncate())
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(palette::rgb(MUTED))
                            .child("Adding…"),
                    ),
            );
        }
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .id("cloud-repository-error")
                    .role(Role::Alert)
                    .px(px(12.))
                    .py(px(8.))
                    .bg(s::destructive_text().opacity(0.1))
                    .text_size(px(12.))
                    .text_color(s::destructive_text())
                    .child(error.clone()),
            );
        }
        // A picker appears either in the populated header or in the empty card.
        let header_action = if empty {
            None
        } else {
            Some(self.add_action(cx))
        };
        kit::section(
            "Repositories",
            "Personal, organization and collaborator repositories cloud chats can check out.",
            header_action,
            body,
        )
    }
    pub(super) fn github_group(&self, _window: &Window, cx: &mut Context<Self>) -> Div {
        const HELP: &str = "Cloud agents commit, push and open pull requests as you. Repository access changes made on GitHub refresh here when you come back; existing cloud chats aren't affected.";
        if let Some(body) = self.github_operation_rows(cx) {
            return kit::section("GitHub", HELP, None, body);
        }
        let loading = self
            .pending
            .values()
            .any(|p| matches!(p.kind, "github" | "github-install"));
        let busy = self.repository_busy();
        let can_manage = self.can_manage();
        let configured = self.github.as_ref().is_none_or(|g| g.configured);
        let install = |this: &mut Self, cx: &mut Context<Self>| {
            if !this.repository_busy() && this.can_manage() {
                this.call(
                    "github-install",
                    methods::CLOUD_GITHUB_INSTALL,
                    json!({}),
                    cx,
                );
            }
        };
        let mut body = column();
        let mut action = None;
        if !configured {
            body = body.child(kit::row_item(
                true,
                None,
                "GitHub App is not configured",
                "",
                Some(cs::error_badge("Unavailable").into_any_element()),
            ));
        } else if self
            .github
            .as_ref()
            .is_none_or(|g| g.installations.is_empty() || g.user.is_none())
        {
            body = body.child(kit::empty_state(
                "site-github",
                "Connect GitHub",
                "",
                Some(
                    kit::primary_button("connect-cloud-github", "Connect GitHub", !loading)
                        .when(!loading, |b| {
                            b.on_click(cx.listener(move |this, _, _, cx| install(this, cx)))
                        })
                        .into_any_element(),
                ),
            ));
        } else if let Some(github) = &self.github {
            for (i, installation) in github.installations.iter().enumerate() {
                let organization = installation
                    .account_type
                    .eq_ignore_ascii_case("organization");
                let selected = github
                    .repositories
                    .iter()
                    .filter(|r| {
                        owner_of(&r.name_with_owner)
                            .eq_ignore_ascii_case(&installation.account_login)
                    })
                    .count();
                let url = installation_url(installation);
                let trailing = row()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(palette::rgb(MUTED))
                            .child(access_summary(&installation.repository_selection, selected)),
                    )
                    .child(
                        cs::button(
                            SharedString::from(format!(
                                "manage-github-access-{}",
                                installation.installation_id
                            )),
                            "Manage access",
                            false,
                            false,
                            false,
                        )
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                    );
                body = body.child(kit::row_item(
                    i == 0,
                    Some(avatar(
                        &installation.account_login,
                        installation.avatar_url.as_deref(),
                        24.,
                    )),
                    installation.account_login.clone(),
                    "",
                    Some(
                        row()
                            .items_center()
                            .gap(px(10.))
                            .child(kit::pill(
                                if organization { "Organization" } else { "User" },
                                kit::Tone::Neutral,
                            ))
                            .when(installation.suspended, |v| {
                                v.child(kit::pill("Suspended", kit::Tone::Attention))
                            })
                            .child(trailing)
                            .into_any_element(),
                    ),
                ));
            }
            body = body.child(
                row()
                    .px(px(12.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(palette::rgb(BORDER).opacity(0.7))
                    .child(
                        cs::icon_label_button(
                            "connect-another-github",
                            "plus",
                            "Connect another account or organization",
                            false,
                            busy || !can_manage,
                            loading,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| install(this, cx))),
                    ),
            );
            if let Some(confirm) = self.github_disconnect_confirm_row(cx) {
                body = body.child(confirm);
            }
            let login = github.user.as_ref().map(|u| u.login.clone());
            let mut controls = row().items_center().gap(px(4.));
            if let Some(login) = login {
                controls = controls.child(
                    div()
                        .mr(px(4.))
                        .text_size(px(11.))
                        .text_color(palette::rgb(MUTED))
                        .child(format!("Connected as {login}")),
                );
            }
            controls = controls.child(
                div()
                    .id("refresh-cloud-github-tip")
                    .child(
                        cs::icon_button(
                            "refresh-cloud-github",
                            "refresh",
                            "Refresh",
                            loading,
                            loading,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
                    .tip("Refresh repositories and access"),
            );
            let items = self.github_menu_items();
            if !items.is_empty() {
                controls = controls.child(self.overflow_menu(
                    "cloud-github-more".into(),
                    "More GitHub actions".into(),
                    busy,
                    items,
                    cx,
                ));
            }
            action = Some(controls.into_any_element());
        }
        if let Some(retry) = self.github_revoking_row(cx) {
            body = body.child(retry);
        }
        kit::section("GitHub", HELP, action, body)
    }
}
impl Render for RepositoryPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.viewport = window.viewport_size();
        let page = self.render_page(window, cx);
        let palette =
            (self.picker && self.github_connected()).then(|| self.render_palette(window, cx));
        let toast = self.render_toast(window, cx);
        div()
            .key_context("CloudSettingsPanel")
            .on_action(cx.listener(|this, _: &SaveChanges, _, cx| this.save_changes(cx)))
            .child(page)
            .children(palette)
            .children(toast)
    }
}
impl RepositoryPanel {
    fn render_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        if !self.loaded && self.owner.is_some() {
            self.refresh(cx);
        }
        if self.usage_only {
            return self.render_storage(window, cx);
        }
        match self.page {
            CloudPage::All => {}
            CloudPage::Overview => {
                if self.agent_recovery_open() {
                    return column().gap(px(16.)).child(self.render_agents(window, cx));
                }
                if self.onboarding_visible() {
                    return self.render_onboarding(window, cx);
                }
                return self.render_overview(window, cx);
            }
            CloudPage::Repositories => {
                let github = self.github_group(window, cx);
                // Until GitHub is connected there is nothing to choose from.
                return if self.github_connected() {
                    let repos = self.repositories(window, cx);
                    column().gap(px(16.)).child(repos).child(github)
                } else {
                    column().gap(px(16.)).child(github)
                };
            }
            CloudPage::Environments => {
                return column()
                    .gap(px(16.))
                    .child(self.render_environment(window, cx));
            }
            CloudPage::Machine => return column().gap(px(16.)).child(self.render_machine(cx)),
            CloudPage::Agents => {
                return column().gap(px(16.)).child(self.render_agents(window, cx));
            }
        }
        if self.onboarding_visible() && !self.agent_recovery_open() {
            return self.render_onboarding(window, cx);
        }
        let entry = self.setup_entry(cx);
        let repos = self.repositories(window, cx);
        let github = self.github_group(window, cx);
        let agents = self.render_agents(window, cx);
        let environment = self.render_environment(window, cx);
        let machine = self.render_machine(cx);
        let mut root = column()
            .gap(px(16.))
            .when_some(entry, |root, entry| root.child(entry));
        if self.agent_recovery_open() {
            return root
                .child(agents)
                .child(github)
                .child(repos)
                .child(machine)
                .child(environment);
        }
        if self.github_connected() {
            root = root.child(repos).child(github);
        } else {
            root = root.child(github).child(repos);
        }
        root.child(agents).child(machine).child(environment)
    }
}

#[cfg(test)]
mod git_ux_tests {
    use super::{access_summary, installation_url};
    use zuse_protocol::cloud_projects::GithubInstallation;

    fn install(login: &str, kind: &str) -> GithubInstallation {
        GithubInstallation {
            installation_id: 42,
            account_login: login.into(),
            account_type: kind.into(),
            avatar_url: None,
            repository_selection: "all".into(),
            suspended: false,
        }
    }

    #[test]
    fn manage_access_opens_each_installations_own_settings() {
        assert_eq!(
            installation_url(&install("opcodehq", "Organization")),
            "https://github.com/organizations/opcodehq/settings/installations/42"
        );
        assert_eq!(
            installation_url(&install("swarajbachu", "User")),
            "https://github.com/settings/installations/42"
        );
    }

    #[test]
    fn access_reads_as_all_or_a_count() {
        assert_eq!(access_summary("all", 10), "All repositories");
        assert_eq!(access_summary("selected", 3), "3 selected");
        assert_eq!(access_summary("selected", 0), "0 selected");
    }
}
