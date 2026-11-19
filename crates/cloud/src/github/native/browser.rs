//! Unknown GitHub identities must explicitly choose in the OAuth browser, not
//! merely in the app holding the initiating WorkOS bearer (login CSRF boundary).
use super::*;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSelection {
    pub state: String,
    pub installation_id: i64,
}
pub struct BrowserTransaction {
    pub actor: String,
    pub owner: String,
    pub operation_id: String,
    /// The initiating app must enter the browser's code before anything links.
    pub confirmation: bool,
}
use zuse_browser_page::{Action, Card, Row as PageRow, Tone};
/// Whose GitHub account an installation lives on, in chooser order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum AccountKind {
    /// The signed-in GitHub user's own account.
    Own,
    Organization,
    /// Another user's personal account that shares repositories with the
    /// signed-in user. Selecting it grants only their writable subset.
    Shared,
}
/// One selectable installation on the chooser page.
#[derive(Debug, PartialEq, Eq)]
struct Choice {
    id: i64,
    login: String,
    avatar: Option<String>,
    kind: AccountKind,
    /// Already linked to this Zuse owner; selecting it again refreshes it.
    connected: bool,
}
impl Choice {
    /// `installation` is a `/user/installations` entry; `profile` is the
    /// signed-in GitHub user. Unusable entries yield `None`.
    fn parse(installation: &Value, profile: &Value, connected: &BTreeSet<i64>) -> Option<Self> {
        let id = installation["id"].as_i64().filter(|n| *n > 0)?;
        if !installation["suspended_at"].is_null() {
            return None;
        }
        let account = &installation["account"];
        let login = account["login"].as_str().filter(|s| valid_component(s))?;
        let kind = match account["type"].as_str() {
            Some("Organization") => AccountKind::Organization,
            Some("User") if account["id"].as_i64() != profile["id"].as_i64() => AccountKind::Shared,
            _ => AccountKind::Own,
        };
        Some(Self {
            id,
            login: login.to_owned(),
            // Validated and sized by the page renderer.
            avatar: account["avatar_url"].as_str().map(str::to_owned),
            kind,
            connected: connected.contains(&id),
        })
    }
    fn meta(&self) -> &'static str {
        match (self.kind, self.connected) {
            (AccountKind::Own, false) => "Personal account",
            (AccountKind::Own, true) => "Personal account · Connected",
            (AccountKind::Organization, false) => "Organization",
            (AccountKind::Organization, true) => "Organization · Connected",
            (AccountKind::Shared, false) => "Personal account · shared with you",
            (AccountKind::Shared, true) => "Personal account · shared with you · Connected",
        }
    }
}
/// Connected first, then the user's own account, organizations, and shared
/// accounts; GitHub's order is kept within each group.
fn sort_choices(choices: &mut [Choice]) {
    choices.sort_by_key(|choice| (!choice.connected, choice.kind));
}
impl NativeGithub {
    pub fn browser_origin_matches(&self, origin: &str) -> bool {
        origin == self.config.origin.origin().ascii_serialization()
    }
    pub(in crate::github::native) async fn browser_row(
        &self,
        state: &str,
        browser: Option<&str>,
    ) -> Result<sqlx::postgres::PgRow, ApiError> {
        let browser = browser
            .filter(|s| s.len() == 64)
            .ok_or_else(|| ApiError::forbidden("github_browser_mismatch"))?;
        if state.len() != 64 {
            return Err(ApiError::bad_request("invalid_github_state"));
        }
        let row=sqlx::query("SELECT o.operation_id,o.actor_id,o.owner_id,o.phase,o.sealed_pending FROM zuse_rust_github_operations o JOIN zuse_rust_github_users u ON u.actor_id=o.actor_id JOIN zuse_rust_github_scopes s ON s.owner_id=o.owner_id WHERE o.state_hash=$1 AND o.browser_hash=$2 AND o.expires_at>$3 AND ((o.actor_epoch=u.epoch AND o.scope_epoch=s.epoch AND u.state<>'revoking') OR o.phase='complete')")
            .bind(hash(state)).bind(hash(browser)).bind(now()).fetch_optional(&self.pool).await?.ok_or_else(||ApiError::forbidden("github_browser_mismatch"))?;
        let owner: String = row.try_get("owner_id")?;
        if !self.enabled(&owner) {
            return Err(ApiError::forbidden("github_native_disabled"));
        }
        Ok(row)
    }
    /// The initiating Zuse account, so a page opened from someone else's link
    /// can say whose account it would connect to.
    pub async fn browser_actor(
        &self,
        state: &str,
        browser: Option<&str>,
    ) -> Result<String, ApiError> {
        Ok(self
            .browser_row(state, browser)
            .await?
            .try_get("actor_id")?)
    }
    pub async fn browser_page(
        &self,
        state: &str,
        browser: Option<&str>,
        account: Option<&str>,
    ) -> Result<String, ApiError> {
        let row = self.browser_row(state, browser).await?;
        let id: String = row.try_get("operation_id")?;
        let phase: String = row.try_get("phase")?;
        if phase == "complete" {
            return Ok(zuse_browser_page::card(&Card {
                hint: Some(
                    "Switch back to your open Zuse window to choose repositories. You can change repository access later in GitHub settings.",
                ),
                ..Card::notice(
                    "GitHub",
                    "Connected",
                    Tone::Success,
                    "GitHub connected",
                    "Return to Zuse to choose repositories and continue. You can close this tab.",
                )
            }));
        }
        if phase != "choosing" {
            return Err(ApiError::conflict("github_operation_unavailable"));
        }
        let pending: Pending = self.open(
            &format!("github-pending:{id}"),
            &row.try_get::<String, _>("sealed_pending")?,
        )?;
        let user = public_user(&pending.profile)?;
        let owner: String = row.try_get("owner_id")?;
        let workspace = match owner.strip_prefix("organization:") {
            Some(organization) => format!("Organization {organization}"),
            None => "Personal".into(),
        };
        let account = account
            .map(str::to_owned)
            .unwrap_or_else(|| "Unavailable".into());
        if let Some(confirmation) = &pending.confirmation {
            return Ok(confirmation::page(
                &id,
                &confirmation.code,
                &user.login,
                &account,
                &workspace,
                pending.browser_installation.filter(|id| *id > 0),
            ));
        }
        let connected: BTreeSet<i64> = sqlx::query_scalar(
            "SELECT installation_id FROM zuse_rust_github_installations WHERE owner_id=$1 AND state='active'",
        )
        .bind(&owner)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .collect();
        let mut installations: Vec<Choice> = Vec::new();
        for page_number in 1..=100 {
            let response = self
                .get(
                    &format!("/user/installations?per_page=100&page={page_number}"),
                    &pending.credentials.access_token,
                )
                .await?;
            let entries = response["installations"]
                .as_array()
                .ok_or_else(unavailable)?;
            for installation in entries {
                let Some(choice) = Choice::parse(installation, &pending.profile, &connected) else {
                    continue;
                };
                if pending
                    .browser_installation
                    .is_some_and(|id| id != choice.id)
                {
                    continue;
                }
                installations.push(choice);
            }
            if entries.len() < 100 {
                break;
            }
            if page_number == 100 {
                return Err(ApiError::unavailable("github_pagination_limit"));
            }
        }
        sort_choices(&mut installations);
        let ids: Vec<String> = installations
            .iter()
            .map(|choice| choice.id.to_string())
            .collect();
        let rows = installations
            .iter()
            .zip(&ids)
            .map(|(choice, id)| PageRow {
                name: &choice.login,
                meta: Some(choice.meta()),
                avatar: choice.avatar.as_deref(),
                action: Action::Form {
                    label: "Use",
                    action: "/v1/cloud/github/browser/select",
                    fields: vec![("state", state.to_owned()), ("installationId", id.clone())],
                    pending: "Connecting this installation… This may take a few seconds.",
                },
            })
            .collect();
        let mut actions = Vec::new();
        if pending.browser_installation.is_none() || pending.browser_installation == Some(0) {
            actions.push(Action::Form {
                label: "Connect while waiting for approval",
                action: "/v1/cloud/github/browser/identity",
                fields: vec![("state", state.to_owned())],
                pending: "Connecting your GitHub account…",
            });
        }
        let install_url = self.install_url(state);
        let refresh = format!("/v1/cloud/github/browser?state={state}");
        actions.push(Action::Link {
            label: "Install or request approval",
            href: &install_url,
            external: true,
        });
        actions.push(Action::Link {
            label: "Check approval",
            href: &refresh,
            external: false,
        });
        Ok(zuse_browser_page::card(&Card {
            integration: "GitHub",
            status: "Choose",
            tone: Tone::Waiting,
            title: "Choose a GitHub installation",
            description: "Pick the installation whose repositories Zuse should use.",
            facts: vec![
                ("GitHub account", user.login.clone()),
                ("Zuse account", account),
                ("Zuse workspace", workspace),
            ],
            rows,
            empty: Some(
                "No installations are available. An organization administrator may need to install the App or approve its access.",
            ),
            actions,
            hint: Some(
                "Continue only if you started this connection from your own Zuse app and the Zuse account above is yours.",
            ),
        }))
    }
    pub fn browser_failure_page(&self, state: &str, code: &str) -> String {
        let (title, hint) = match code {
            "github_organization_approval_required" => (
                "Organization approval required",
                "Ask your GitHub organization owner to approve Members read access for the App. They do not need a Zuse account. Return here and check approval afterward.",
            ),
            "github_installation_admin_required" => (
                "Connect the workspace installation first",
                "Ask a Zuse organization administrator to link this GitHub installation. You can then connect your own GitHub identity to its repositories.",
            ),
            "github_organization_membership_required" | "github_repository_access_denied" => (
                "Repository access is required",
                "Your GitHub account needs active organization membership and writable access to the selected repositories.",
            ),
            _ => (
                "GitHub connection is not finished",
                "Your connection has been preserved for this session. Check the installation and try again, or cancel and start a new connection in Zuse.",
            ),
        };
        let retry = format!("/v1/cloud/github/browser?state={state}");
        zuse_browser_page::card(&Card {
            actions: vec![Action::Link {
                label: "Check approval and retry",
                href: &retry,
                external: false,
            }],
            ..Card::notice("GitHub", "Not finished", Tone::Failure, title, hint)
        })
    }
    /// The browser's cookie is mandatory even when the app knows its own state
    /// and bearer. Once chosen, another installation cannot replace this consent.
    pub async fn confirm_browser(
        &self,
        selection: &BrowserSelection,
        browser: Option<&str>,
    ) -> Result<BrowserTransaction, ApiError> {
        if selection.installation_id <= 0 {
            return Err(ApiError::bad_request("invalid_github_installation"));
        }
        let row = self.browser_row(&selection.state, browser).await?;
        let id: String = row.try_get("operation_id")?;
        let phase: String = row.try_get("phase")?;
        if phase != "choosing" {
            return Err(ApiError::conflict("github_operation_unavailable"));
        }
        let sealed: String = row.try_get("sealed_pending")?;
        let mut pending: Pending = self.open(&format!("github-pending:{id}"), &sealed)?;
        if pending
            .browser_installation
            .is_some_and(|id| id != selection.installation_id)
        {
            return Err(ApiError::conflict("github_browser_choice_changed"));
        }
        pending.browser_installation = Some(selection.installation_id);
        let confirmation = self.require_confirmation(&mut pending);
        let next = self.seal(&format!("github-pending:{id}"), &pending)?;
        let updated=sqlx::query("UPDATE zuse_rust_github_operations SET sealed_pending=$3 WHERE operation_id=$1 AND phase='choosing' AND sealed_pending=$2 AND expires_at>$4")
            .bind(&id).bind(sealed).bind(next).bind(now()).execute(&self.pool).await?;
        if updated.rows_affected() != 1 {
            return Err(ApiError::conflict("github_operation_unavailable"));
        }
        let transaction = BrowserTransaction {
            actor: row.try_get("actor_id")?,
            owner: row.try_get("owner_id")?,
            operation_id: id,
            confirmation,
        };
        if confirmation {
            self.announce_confirmation(&transaction).await;
        }
        Ok(transaction)
    }
}

#[cfg(test)]
mod chooser_tests {
    use super::*;

    fn entry(id: i64, account_id: i64, login: &str, kind: &str) -> Value {
        json!({
            "id": id,
            "account": {
                "id": account_id,
                "login": login,
                "type": kind,
                "avatar_url": format!("https://avatars.githubusercontent.com/u/{account_id}?v=4"),
            },
            "suspended_at": null,
        })
    }

    #[test]
    fn installations_are_labelled_by_whose_account_they_live_on() {
        let profile = json!({"id": 42, "login": "member"});
        let none = BTreeSet::new();
        let own = Choice::parse(&entry(1, 42, "member", "User"), &profile, &none).unwrap();
        assert_eq!(own.kind, AccountKind::Own);
        assert_eq!(own.meta(), "Personal account");
        assert_eq!(
            own.avatar.as_deref(),
            Some("https://avatars.githubusercontent.com/u/42?v=4")
        );
        let org = Choice::parse(&entry(2, 81, "team", "Organization"), &profile, &none).unwrap();
        assert_eq!(org.meta(), "Organization");
        let shared = Choice::parse(&entry(3, 77, "friend", "User"), &profile, &none).unwrap();
        assert_eq!(shared.kind, AccountKind::Shared);
        assert_eq!(shared.meta(), "Personal account · shared with you");
        let mut missing = entry(4, 0, "noavatar", "User");
        missing["account"]["avatar_url"] = Value::Null;
        missing["account"]["id"] = json!(42);
        let missing = Choice::parse(&missing, &profile, &none).unwrap();
        assert_eq!(missing.avatar, None);
        assert_eq!(missing.kind, AccountKind::Own);
    }

    #[test]
    fn unusable_installations_are_skipped() {
        let profile = json!({"id": 42});
        let none = BTreeSet::new();
        let mut suspended = entry(1, 81, "team", "Organization");
        suspended["suspended_at"] = json!("2026-01-01T00:00:00Z");
        assert_eq!(Choice::parse(&suspended, &profile, &none), None);
        assert_eq!(
            Choice::parse(&entry(0, 81, "team", "Organization"), &profile, &none),
            None
        );
        assert_eq!(
            Choice::parse(&entry(1, 81, "<b>", "Organization"), &profile, &none),
            None
        );
    }

    #[test]
    fn connected_installations_are_marked_and_listed_first() {
        let profile = json!({"id": 42});
        let connected = BTreeSet::from([5]);
        let mut choices: Vec<Choice> = [
            entry(1, 77, "friend", "User"),
            entry(2, 81, "team", "Organization"),
            entry(3, 42, "member", "User"),
            entry(4, 82, "other-team", "Organization"),
            entry(5, 83, "linked", "Organization"),
        ]
        .iter()
        .filter_map(|e| Choice::parse(e, &profile, &connected))
        .collect();
        sort_choices(&mut choices);
        let order: Vec<(&str, &str)> = choices
            .iter()
            .map(|c| (c.login.as_str(), c.meta()))
            .collect();
        assert_eq!(
            order,
            [
                ("linked", "Organization · Connected"),
                ("member", "Personal account"),
                ("team", "Organization"),
                ("other-team", "Organization"),
                ("friend", "Personal account · shared with you"),
            ]
        );
        let own = Choice::parse(
            &entry(9, 42, "member", "User"),
            &profile,
            &BTreeSet::from([9]),
        );
        assert_eq!(own.unwrap().meta(), "Personal account · Connected");
        let shared = Choice::parse(
            &entry(9, 7, "friend", "User"),
            &profile,
            &BTreeSet::from([9]),
        );
        assert_eq!(
            shared.unwrap().meta(),
            "Personal account · shared with you · Connected"
        );
    }
}
