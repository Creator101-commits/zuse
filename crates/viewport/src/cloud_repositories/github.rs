//! One pending GitHub transaction, shared by native and browser settings.
use super::*;
/// The desktop app receives `zuse://github/confirm` links from the OAuth
/// browser; the browser-hosted settings page cannot, so it keeps typing.
const CODE_ARRIVES_BY_LINK: bool = !cfg!(target_arch = "wasm32");
impl RepositoryPanel {
    pub(super) fn github_started(&mut self, data: Value, cx: &mut Context<Self>) {
        if let Some(id) = data["operationId"].as_str() {
            self.github_operation =
                Some(json!({"operationId":id,"state":"authorizing","expiresAt":data["expiresAt"]}));
            self.schedule_github_poll(cx);
        }
        if let Some(url) = data["url"]
            .as_str()
            .filter(|url| url.starts_with("https://"))
        {
            cx.open_url(url);
            self.notice = Some(
                if self.github_operation.is_some() {
                    "Finish signing in with GitHub in your browser. Zuse will continue here."
                } else {
                    "Finish connecting GitHub in your browser, then click Refresh here."
                }
                .into(),
            );
        } else {
            self.error =
                Some("GitHub returned an invalid connection link. Refresh and try again.".into());
        }
    }
    pub(super) fn github_status(&mut self, data: Value, cx: &mut Context<Self>) {
        if self
            .github_operation
            .as_ref()
            .and_then(|op| op["operationId"].as_str())
            != data["operationId"].as_str()
        {
            return;
        }
        if data["state"] == "complete" {
            self.github_operation = None;
            self.github_poll = None;
            self.refresh(cx);
            return;
        }
        self.github_operation = Some(data);
        self.schedule_github_poll(cx);
    }
    pub(super) fn github_action(
        &mut self,
        kind: &'static str,
        action: &str,
        installation: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self
            .github_operation
            .as_ref()
            .and_then(|op| op["operationId"].as_str())
            .map(str::to_owned)
        else {
            return;
        };
        if self.pending.values().any(|p| p.kind == kind) {
            return;
        }
        self.error = None;
        self.call(
            kind,
            methods::CLOUD_GITHUB_INSTALL,
            json!({"action":action,"operationId":id,"installationId":installation}),
            cx,
        );
        cx.notify();
    }
    /// Sends the code the OAuth browser displayed. Only this app's session can
    /// finish linking a GitHub identity that a browser signed in to.
    fn github_confirm(&mut self, cx: &mut Context<Self>) {
        let code = self.github_code.read(cx).value(cx).trim().to_owned();
        self.github_submit_code(code, cx);
    }
    fn github_submit_code(&mut self, code: String, cx: &mut Context<Self>) {
        let Some(id) = self
            .github_operation
            .as_ref()
            .and_then(|op| op["operationId"].as_str())
            .map(str::to_owned)
        else {
            return;
        };
        if self.pending.values().any(|p| p.kind == "github-confirm") {
            return;
        }
        if code.is_empty() {
            self.error = Some("Enter the code shown in your browser.".into());
            cx.notify();
            return;
        }
        self.error = None;
        self.call(
            "github-confirm",
            methods::CLOUD_GITHUB_INSTALL,
            json!({"action":"confirm","operationId":id,"confirmationCode":code}),
            cx,
        );
        cx.notify();
    }
    /// Submits the code the OAuth browser handed back through
    /// `zuse://github/confirm`. Only the operation this panel started, for
    /// the bound account, and is still waiting on is confirmed. Any other id,
    /// such as a connect link someone else started, is ignored: that is what
    /// keeps the code from linking a stranger's GitHub account. Returns
    /// whether the code was submitted.
    pub fn confirm_github_from_link(
        &mut self,
        operation_id: &str,
        code: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(operation) = &self.github_operation else {
            return false;
        };
        if operation_id.is_empty() || operation["operationId"].as_str() != Some(operation_id) {
            return false;
        }
        if matches!(
            operation["state"].as_str(),
            Some("expired" | "cancelled" | "failed" | "complete")
        ) {
            return false;
        }
        if self.pending.values().any(|p| p.kind == "github-confirm") {
            return false;
        }
        self.github_poll = None;
        self.github_submit_code(code.to_owned(), cx);
        self.pending.values().any(|p| p.kind == "github-confirm")
    }
    fn schedule_github_poll(&mut self, cx: &mut Context<Self>) {
        if self.account_feed_live {
            return;
        }
        if self.github_poll.is_some() || self.pending.values().any(|p| p.kind == "github-status") {
            return;
        }
        let Some(operation) = &self.github_operation else {
            return;
        };
        if !matches!(
            operation["state"].as_str(),
            Some("created" | "authorizing" | "exchanging")
        ) {
            return;
        }
        if operation["expiresAt"]
            .as_i64()
            .is_none_or(|expiry| expiry <= chrono::Utc::now().timestamp_millis())
        {
            if let Some(operation) = &mut self.github_operation {
                operation["state"] = json!("expired");
            }
            return;
        }
        self.github_poll = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.github_poll = None;
                this.github_action("github-status", "status", None, cx);
            });
        }));
    }
    /// The low-key GitHub actions behind "…": disconnecting your own
    /// authorization (members never change another member's account).
    pub(super) fn github_menu_items(&self) -> Vec<super::presentation::MenuItem> {
        if !self.github_native || self.github_connection_state != "connected" {
            return vec![];
        }
        vec![super::presentation::MenuItem {
            id: "ask-github-disconnect".into(),
            glyph: "logout-2",
            label: "Disconnect GitHub…".into(),
            danger: true,
            run: std::rc::Rc::new(|this: &mut Self, _: &mut Window, _: &mut Context<Self>| {
                this.github_disconnect_confirm = true;
            }),
        }]
    }

    /// Confirms a disconnect chosen from the menu.
    pub(super) fn github_disconnect_confirm_row(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.github_native
            || self.github_connection_state != "connected"
            || !self.github_disconnect_confirm
        {
            return None;
        }
        let busy = self.repository_busy();
        Some(
            row()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .py(px(10.))
                .border_t_1()
                .border_color(palette::rgb(BORDER).opacity(0.7))
                .child(div().flex_1().min_w_0().child(crate::cloud_settings::kit::title_with_tip(
                    "Disconnect GitHub?",
                    "This revokes your GitHub authorization and scoped Git credentials. Reconnect to use private repositories.",
                )))
                .child(
                    cs::button("keep-github-connected", "Keep connected", false, busy, false)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.github_disconnect_confirm = false;
                            cx.notify();
                        })),
                )
                .child(
                    cs::button("confirm-github-disconnect", "Disconnect", false, busy, busy)
                        .text_color(s::destructive_text())
                        .on_click(cx.listener(|this, _, _, cx| this.disconnect_github(cx))),
                ),
        )
    }

    /// A disconnect GitHub hasn't confirmed yet can be retried.
    pub(super) fn github_revoking_row(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.github_native || self.github_connection_state != "revoking" {
            return None;
        }
        let busy = self.repository_busy();
        Some(cs::setting_row(
            false,
            "Finish disconnecting GitHub",
            "GitHub has not confirmed revocation yet. Retry to finish safely.",
            Some(
                cs::button(
                    "retry-github-disconnect",
                    "Retry disconnect",
                    false,
                    busy,
                    busy,
                )
                .on_click(cx.listener(|this, _, _, cx| this.disconnect_github(cx)))
                .into_any_element(),
            ),
        ))
    }
    fn disconnect_github(&mut self, cx: &mut Context<Self>) {
        if self.repository_busy() {
            return;
        }
        self.call(
            "github-disconnect",
            methods::CLOUD_GITHUB_INSTALL,
            json!({"action":"disconnect"}),
            cx,
        );
        cx.notify();
    }
    pub(super) fn github_operation_rows(&self, cx: &mut Context<Self>) -> Option<Div> {
        let operation = self.github_operation.as_ref()?;
        let state = operation["state"].as_str().unwrap_or("failed");
        let busy = self
            .pending
            .values()
            .any(|p| matches!(p.kind, "github-select" | "github-confirm" | "github-cancel"));
        let title = match state {
            "choosing" => "Choose a GitHub installation",
            "confirming" => "Confirm the GitHub connection",
            "expired" => "GitHub sign-in expired",
            "cancelled" => "GitHub sign-in cancelled",
            "failed" => "GitHub connection needs attention",
            _ => "Finish signing in with GitHub",
        };
        let description = if state == "confirming" {
            let login = operation["user"]["login"]
                .as_str()
                .unwrap_or("a GitHub account");
            if CODE_ARRIVES_BY_LINK {
                format!(
                    "Your browser signed in as {login}. Click Open Zuse there and Zuse finishes connecting. If you did not just sign in to GitHub yourself, cancel."
                )
            } else {
                format!(
                    "Your browser signed in as {login}. Enter the code shown there to connect it to this account. If you did not just sign in to GitHub yourself, cancel."
                )
            }
        } else if state == "choosing" {
            operation["user"]["login"]
                .as_str()
                .map(|login| {
                    format!("Signed in as {login}. Choose where Zuse may access repositories.")
                })
                .unwrap_or_else(|| "Choose where Zuse may access repositories.".into())
        } else {
            "Complete authorization in your browser. This connection belongs to your account."
                .into()
        };
        let mut body = column().child(cs::setting_row(true, title, description, None));
        if state == "confirming" {
            let ready = !self.github_code.read(cx).value(cx).trim().is_empty();
            // On the desktop this is a fallback: the browser hands the code
            // over through the Zuse app link.
            let (title, description) = if CODE_ARRIVES_BY_LINK {
                (
                    "Zuse didn't open?",
                    "Enter the code shown in the browser tab instead.",
                )
            } else {
                (
                    "Confirmation code",
                    "Shown in the browser tab where you signed in to GitHub.",
                )
            };
            body = body.child(cs::setting_row_content(
                false,
                title,
                description,
                None,
                row()
                    .gap(px(8.))
                    .child(
                        s::field()
                            .h(px(28.))
                            .w(px(160.))
                            .child(self.github_code.clone()),
                    )
                    .child(
                        cs::button(
                            "github-confirm-code",
                            "Confirm",
                            !CODE_ARRIVES_BY_LINK,
                            busy || !ready,
                            busy,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.github_confirm(cx))),
                    ),
            ));
        }
        if state == "choosing" {
            if let Some(installations) = operation["installations"].as_array() {
                for installation in installations {
                    let Some(id) = installation["installationId"].as_i64() else {
                        continue;
                    };
                    let login = installation["accountLogin"]
                        .as_str()
                        .unwrap_or("GitHub account")
                        .to_owned();
                    let suspended = installation["suspended"].as_bool().unwrap_or(false);
                    body = body.child(cs::setting_row(
                        false,
                        login,
                        if suspended {
                            "Installation suspended"
                        } else {
                            "Use this installation with your own GitHub identity"
                        },
                        Some(
                            cs::button(
                                SharedString::from(format!("github-installation-{id}")),
                                "Connect",
                                false,
                                busy || suspended,
                                busy,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.github_action("github-select", "select", Some(id), cx)
                            }))
                            .into_any_element(),
                        ),
                    ));
                }
                if installations.is_empty() {
                    body=body.child(cs::setting_row(false,"No installations yet","Install the app on your account or an organization below. If an organization needs approval, GitHub sends the request to its owners; refresh once it's approved.",None));
                }
            }
            if let Some(url) = operation["installUrl"]
                .as_str()
                .filter(|url| url.starts_with("https://github.com/apps/"))
            {
                let url = url.to_owned();
                body = body.child(cs::setting_row(
                    false,
                    "Add an installation",
                    "Install the App or request organization approval, then refresh this list.",
                    Some(
                        cs::button("github-install-app", "Open GitHub", false, busy, false)
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
                            .into_any_element(),
                    ),
                ));
            }
        }
        if let Some(error) = &self.error {
            body = body.child(cs::setting_row(
                false,
                "Connection needs attention",
                error.clone(),
                None,
            ));
        }
        let finished = matches!(state, "expired" | "cancelled" | "failed");
        let controls = row()
            .gap(px(8.))
            .when(finished, |controls| {
                controls.child(
                    cs::button("github-operation-retry", "Try again", true, busy, false).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.github_poll = None;
                            this.github_operation = None;
                            this.error = None;
                            this.call(
                                "github-install",
                                methods::CLOUD_GITHUB_INSTALL,
                                serde_json::json!({}),
                                cx,
                            );
                        }),
                    ),
                )
            })
            .child(
                cs::button("github-operation-refresh", "Refresh", false, busy, false).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.github_action("github-status", "status", None, cx)
                    }),
                ),
            )
            .child(
                cs::button(
                    "github-operation-cancel",
                    if finished { "Close" } else { "Cancel" },
                    false,
                    busy,
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.github_poll = None;
                    this.github_action("github-cancel", "cancel", None, cx);
                })),
            );
        Some(body.child(cs::setting_row_content(false, "", "", None, controls)))
    }
}
#[cfg(test)]
mod confirm_link_tests {
    use super::*;
    use std::prelude::v1::test;
    fn waiting(panel: &mut RepositoryPanel, state: &str, cx: &mut Context<RepositoryPanel>) {
        panel.bind_owner(Some(("account".into(), None)), cx);
        panel.github_operation = Some(json!({"operationId":"op-mine","state":state}));
    }
    fn confirms(panel: &RepositoryPanel) -> usize {
        panel
            .pending
            .values()
            .filter(|p| p.kind == "github-confirm")
            .count()
    }
    #[gpui::test]
    fn only_the_operation_this_app_started_is_confirmed(cx: &mut TestAppContext) {
        cx.update(crate::initialize);
        let window = cx.add_window(RepositoryPanel::new);
        let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::<Value>::new()));
        let seen = sent.clone();
        let panel = window.root(cx).unwrap();
        cx.update(|cx| {
            cx.subscribe(&panel, move |_, event: &Request, _| {
                if let Request::Call { params, .. } = event {
                    seen.borrow_mut().push(params.clone());
                }
            })
            .detach()
        });
        window
            .update(cx, |panel, _, cx| {
                // Nothing pending: a link someone else started is ignored.
                assert!(!panel.confirm_github_from_link("op-mine", "ABCD-EFGH", cx));
                waiting(panel, "confirming", cx);
                assert!(!panel.confirm_github_from_link("op-theirs", "ABCD-EFGH", cx));
                assert!(!panel.confirm_github_from_link("", "ABCD-EFGH", cx));
                assert_eq!(confirms(panel), 0);
                // Its own operation is confirmed once, with no clicks.
                assert!(panel.confirm_github_from_link("op-mine", "ABCDEFGH", cx));
                assert!(!panel.confirm_github_from_link("op-mine", "ABCDEFGH", cx));
                assert_eq!(confirms(panel), 1);
            })
            .unwrap();
        {
            let sent = sent.borrow();
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0]["action"], "confirm");
            assert_eq!(sent[0]["operationId"], "op-mine");
            assert_eq!(sent[0]["confirmationCode"], "ABCDEFGH");
            assert_eq!(sent[0]["expectedAccountId"], "account");
        }
        window
            .update(cx, |panel, _, cx| {
                let id = *panel.pending.keys().next().unwrap();
                panel.receive(
                    id,
                    Ok(json!({"accountId":"account","organizationId":null,"data":{}})),
                    cx,
                );
                assert!(panel.github_operation.is_none());
                assert_eq!(
                    panel.notice.as_deref(),
                    Some("GitHub connected. Choose your repositories below.")
                );
            })
            .unwrap();
    }
    #[gpui::test]
    fn finished_or_reassigned_operations_ignore_the_link(cx: &mut TestAppContext) {
        cx.update(crate::initialize);
        let window = cx.add_window(RepositoryPanel::new);
        window
            .update(cx, |panel, _, cx| {
                for state in ["expired", "cancelled", "failed"] {
                    waiting(panel, state, cx);
                    assert!(!panel.confirm_github_from_link("op-mine", "ABCDEFGH", cx));
                }
                // Switching accounts forgets the operation, so its link is ignored.
                waiting(panel, "confirming", cx);
                panel.bind_owner(Some(("other".into(), None)), cx);
                assert!(!panel.confirm_github_from_link("op-mine", "ABCDEFGH", cx));
                assert_eq!(confirms(panel), 0);
            })
            .unwrap();
    }
}
