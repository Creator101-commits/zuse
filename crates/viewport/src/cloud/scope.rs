//! Which account, organization and API origin cloud replies belong to, and
//! when a reply means that scope really changed.

/// One side of a scope comparison.
#[derive(Clone, Copy)]
pub(super) struct Scope<'a> {
    pub account: Option<&'a str>,
    pub organization: Option<&'a str>,
    pub origin: Option<&'a str>,
    /// The origin is the opener's guess, not a cloud reply.
    pub origin_provisional: bool,
}

/// What differs between the scope this view holds and a cloud reply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Change {
    pub account: bool,
    pub organization: bool,
    pub origin: bool,
}

impl Change {
    /// Only an established scope can change: a view that has not yet bound
    /// an account adopts the first reply, and a provisional origin is
    /// replaced by the engine's own.
    pub(super) fn between(current: Scope, reply: Scope) -> Self {
        let bound = current.account.is_some();
        Self {
            account: bound && current.account != reply.account,
            organization: bound && current.organization != reply.organization,
            origin: current.origin.is_some()
                && !current.origin_provisional
                && !same_origin(current.origin, reply.origin),
        }
    }

    pub(super) fn any(self) -> bool {
        self.account || self.organization || self.origin
    }

    /// One-time explanation shown as a notice, not an error.
    pub(super) fn notice(self) -> &'static str {
        if self.account || self.organization {
            "Switched to a different Cloud account. Repositories were reloaded for it; your previous draft was saved."
        } else {
            "Cloud server changed. Repositories were reloaded; your previous draft was saved."
        }
    }
}

/// Origins compare as URLs: `https://host` and `https://host/` are the same
/// server, as are differently cased hosts.
pub(super) fn same_origin(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => normalize(a) == normalize(b),
        (a, b) => a == b,
    }
}

fn normalize(origin: &str) -> String {
    url::Url::parse(origin.trim())
        .map(|url| url.as_str().trim_end_matches('/').to_owned())
        .unwrap_or_else(|_| origin.trim().trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope<'a>(account: &'a str, organization: Option<&'a str>, origin: &'a str) -> Scope<'a> {
        Scope {
            account: Some(account),
            organization,
            origin: Some(origin),
            origin_provisional: false,
        }
    }

    #[test]
    fn origins_compare_as_urls() {
        assert!(same_origin(
            Some("https://api-rust.zuse.sh"),
            Some("https://api-rust.zuse.sh/")
        ));
        assert!(same_origin(
            Some("https://API-Rust.zuse.sh/"),
            Some("https://api-rust.zuse.sh")
        ));
        assert!(!same_origin(
            Some("https://api-rust.zuse.sh"),
            Some("https://api-rust-stag.zuse.sh/")
        ));
        assert!(!same_origin(Some("https://a.example"), None));
        assert!(same_origin(None, None));
    }

    #[test]
    fn a_provisional_origin_or_unbound_view_never_reports_a_change() {
        let reply = scope("owner", None, "https://api-rust-stag.zuse.sh/");
        // The opener guessed production without a trailing slash; the
        // engine actually talks to staging.
        let guessed = Scope {
            origin_provisional: true,
            ..scope("owner", None, "https://api-rust.zuse.sh")
        };
        assert_eq!(Change::between(guessed, reply), Change::default());
        let unbound = Scope {
            account: None,
            organization: None,
            origin: None,
            origin_provisional: false,
        };
        assert!(!Change::between(unbound, reply).any());
        // Same server spelled differently is not a change either.
        let same = scope("owner", None, "https://api-rust-stag.zuse.sh");
        assert!(!Change::between(same, reply).any());
    }

    #[test]
    fn real_account_organization_and_server_changes_are_reported() {
        let reply = scope("owner", Some("team"), "https://api.example/");
        let change = Change::between(scope("other", Some("team"), "https://api.example"), reply);
        assert!(change.account && !change.organization && !change.origin);
        assert!(change.notice().contains("different Cloud account"));
        let change = Change::between(scope("owner", None, "https://api.example"), reply);
        assert!(change.organization && !change.account);
        let change = Change::between(scope("owner", Some("team"), "https://old.example"), reply);
        assert!(change.origin && !change.account && !change.organization);
        assert!(change.notice().contains("server changed"));
    }
}

#[cfg(test)]
mod view_tests {
    use super::super::*;
    use gpui::TestAppContext;
    use std::prelude::v1::test;

    fn context(account: &str, origin: &str, projects: Value) -> Value {
        json!({"id":9000,"ok":{
            "accountId":account,"organizationId":null,"apiOrigin":origin,
            "projects":projects,
            "agentCatalog":{"providers":[{"id":"claude","name":"Claude","defaultModel":"sonnet","models":[{"id":"sonnet","label":"Sonnet"}]}]}
        }})
    }

    type Outgoing = futures::channel::mpsc::UnboundedReceiver<Value>;
    fn view(cx: &mut TestAppContext) -> (gpui::WindowHandle<WorkspaceView>, Outgoing) {
        cx.update(initialize);
        let (requests, outgoing) = futures::channel::mpsc::unbounded();
        let (_events, incoming) = futures::channel::mpsc::unbounded();
        let window = cx.add_window(|window, cx| {
            WorkspaceView::new(
                Transport {
                    requests,
                    events: incoming,
                },
                window,
                cx,
            )
        });
        (window, outgoing)
    }

    /// A staging engine replies with its own origin (and URL spelling) to a
    /// window opened with the production default. That is not a scope
    /// change: the draft stays, no error appears, and a repository is chosen.
    #[gpui::test]
    fn the_first_context_reply_replaces_a_guessed_origin_silently(cx: &mut TestAppContext) {
        let (window, _outgoing) = view(cx);
        window
            .update(cx, |view, _, cx| {
                view.open_cloud_setup(
                    "owner".into(),
                    None,
                    Some("https://api-rust.zuse.sh".into()),
                    cx,
                );
                view.cloud.context = Some(9000);
                view.input
                    .update(cx, |input, cx| input.set_value("Fix the README".into(), cx));
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, _, cx| {
                view.receive(
                    context(
                        "owner",
                        "https://api-rust-stag.zuse.sh/",
                        json!([{"projectId":"p1","repositoryIdentity":"octo/Hello-World","displayName":"Hello-World"}]),
                    ),
                    cx,
                );
                assert_eq!(view.error, None);
                assert_eq!(view.cloud.notice, None);
                assert_eq!(view.input.read(cx).value(cx), "Fix the README");
                assert_eq!(view.cloud.project, 0);
                assert_eq!(view.cloud.project_id.as_deref(), Some("p1"));
                assert_eq!(
                    view.cloud.namespace.as_deref(),
                    Some("https://api-rust-stag.zuse.sh/")
                );
                assert!(!view.cloud.namespace_provisional);
                // Later refreshes compare against the adopted origin.
                view.cloud.context = Some(9000);
                view.receive(
                    context(
                        "owner",
                        "https://api-rust-stag.zuse.sh",
                        json!([{"projectId":"p1","repositoryIdentity":"octo/Hello-World"}]),
                    ),
                    cx,
                );
                assert_eq!(view.cloud.notice, None);
                assert_eq!(view.cloud.project_id.as_deref(), Some("p1"));
            })
            .unwrap();
    }

    /// A real account switch is a one-time notice, not an error, and the same
    /// repository in the new account is selected without asking again.
    #[gpui::test]
    fn an_account_switch_reloads_and_keeps_the_same_repository(cx: &mut TestAppContext) {
        let (window, _outgoing) = view(cx);
        window
            .update(cx, |view, _, cx| {
                view.open_cloud_setup("owner".into(), None, None, cx);
                view.cloud.context = Some(9000);
                view.receive(
                    context(
                        "owner",
                        "https://api.example/",
                        json!([{"projectId":"a","repositoryIdentity":"octo/other"},{"projectId":"p1","repositoryIdentity":"octo/Hello-World"}]),
                    ),
                    cx,
                );
                view.cloud.project = 1;
                view.cloud.project_id = Some("p1".into());
                view.cloud.context = Some(9000);
                view.receive(
                    context(
                        "second",
                        "https://api.example/",
                        json!([{"projectId":"p2","repositoryIdentity":"octo/Hello-World"}]),
                    ),
                    cx,
                );
                assert_eq!(view.error, None, "never \"Something went wrong\"");
                assert!(
                    view.cloud
                        .notice
                        .as_deref()
                        .is_some_and(|notice| notice.contains("different Cloud account"))
                );
                assert_eq!(view.cloud.project, 0);
                assert_eq!(view.cloud.project_id.as_deref(), Some("p2"));
                assert_eq!(view.cloud.account.as_deref(), Some("second"));
                // The next refresh in the same scope clears the notice.
                view.cloud.context = Some(9000);
                view.receive(
                    context(
                        "second",
                        "https://api.example/",
                        json!([{"projectId":"p2","repositoryIdentity":"octo/Hello-World"}]),
                    ),
                    cx,
                );
                assert_eq!(view.cloud.notice, None);
                // A repository the new account lacks is never retargeted.
                view.cloud.context = Some(9000);
                view.receive(
                    context(
                        "third",
                        "https://api.example/",
                        json!([{"projectId":"p3","repositoryIdentity":"octo/elsewhere"}]),
                    ),
                    cx,
                );
                assert_eq!(view.cloud.project, usize::MAX);
            })
            .unwrap();
    }
}
