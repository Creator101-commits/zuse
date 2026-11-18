//! Whether the signed-in Zuse account has an active Cloud subscription.
//!
//! The Cloud computer settings, the Usage page's Cloud compute tab, cloud
//! destinations in the pickers and new cloud chats follow [`CloudAccess`].
//! The subscription is read (`CloudBillingEntitlements`) when an account
//! signs in, and again whenever the shared account feed carries a `billing`
//! hint (the cloud emits one on every entitlement change) or reconnects. So
//! subscribing, or a subscription ending, shows or hides those surfaces
//! without a restart and without polling.
use super::*;
use serde_json::{Value, json};

/// The account and organization an answer belongs to.
type Owner = (String, Option<String>);

#[derive(Default)]
pub(super) struct Plan {
    owner: Option<Owner>,
    subscribed: Option<bool>,
}

/// What the signed-in account may see of Cloud.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudAccess {
    /// The engine has not reported the account yet.
    Unknown,
    SignedOut,
    /// Signed in; the subscription has not been read yet.
    Checking,
    /// Signed in without an active Cloud subscription.
    SignedIn,
    /// Signed in with an active Cloud subscription.
    Subscribed,
}

impl CloudAccess {
    /// Signed in, with or without a subscription. Billing is offered here.
    pub fn signed_in(self) -> bool {
        matches!(self, Self::Checking | Self::SignedIn | Self::Subscribed)
    }

    /// Cloud computers, their settings and Cloud compute usage are offered.
    pub fn subscribed(self) -> bool {
        self == Self::Subscribed
    }

    /// The answer is final, so views may drop cloud surfaces or leave a page
    /// that is no longer offered. While it is not, they keep what is on
    /// screen rather than flicker during start-up or an engine restart.
    pub fn settled(self) -> bool {
        matches!(self, Self::SignedOut | Self::SignedIn | Self::Subscribed)
    }

    /// Signed in and not known to lack a subscription. Pickers load cloud
    /// destinations while the subscription read is in flight and drop them
    /// if it says no.
    pub fn offers_cloud(self) -> bool {
        matches!(self, Self::Checking | Self::Subscribed)
    }

    /// Settled and not subscribed: new cloud chats go to Billing instead.
    pub fn withholds_cloud(self) -> bool {
        self.settled() && !self.subscribed()
    }
}

impl AppState {
    pub fn cloud_access(&self) -> CloudAccess {
        // Local-only builds (`cloud_gate`) treat every profile as signed out.
        if !zuse_viewport::cloud_gate::enabled() {
            return CloudAccess::SignedOut;
        }
        if self.cloud_account.is_none() {
            return CloudAccess::Unknown;
        }
        let Some(owner) = self.cloud_plan_owner() else {
            return CloudAccess::SignedOut;
        };
        if self.cloud_plan.owner.as_ref() != Some(&owner) {
            return CloudAccess::Checking;
        }
        match self.cloud_plan.subscribed {
            None => CloudAccess::Checking,
            // `ZUSE_CLOUD_BETA=0` turns cloud computers off for everyone.
            Some(true) if crate::cloud_beta() => CloudAccess::Subscribed,
            Some(_) => CloudAccess::SignedIn,
        }
    }

    fn cloud_plan_owner(&self) -> Option<Owner> {
        Some((
            self.cloud_user_id()?.to_owned(),
            self.cloud_organization_id().map(str::to_owned),
        ))
    }

    /// Record a subscription read for `owner`. Ignored once another account
    /// (or organization) is signed in. `None` means the read failed: the
    /// last answer for this owner stands until the next hint.
    pub(crate) fn apply_cloud_subscription(
        &mut self,
        owner: &Owner,
        subscribed: Option<bool>,
    ) -> bool {
        if self.cloud_plan_owner().as_ref() != Some(owner) {
            return false;
        }
        let same_owner = self.cloud_plan.owner.as_ref() == Some(owner);
        let subscribed = match subscribed {
            Some(value) => Some(value),
            None if same_owner => return false,
            None => None,
        };
        if same_owner && self.cloud_plan.subscribed == subscribed {
            return false;
        }
        self.cloud_plan = Plan {
            owner: Some(owner.clone()),
            subscribed,
        };
        // A cloud repository picked for the next chat cannot start one now.
        if subscribed == Some(false) {
            self.cloud_project = None;
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn set_cloud_subscription_for_test(&mut self, subscribed: bool) {
        let owner = self.cloud_plan_owner().expect("a signed-in cloud account");
        self.apply_cloud_subscription(&owner, Some(subscribed));
    }
}

fn owner_of(value: &Value) -> Option<Owner> {
    Some((
        value["user"]["id"].as_str()?.to_owned(),
        value["orgId"].as_str().map(str::to_owned),
    ))
}

fn scoped(owner: &Owner) -> Value {
    json!({"expectedAccountId":owner.0,"expectedOrganizationId":owner.1})
}

fn bound(value: &Value, owner: &Owner) -> bool {
    value["accountId"].as_str() == Some(owner.0.as_str())
        && value["organizationId"].as_str() == owner.1.as_deref()
}

/// Whether one account feed frame may have changed the subscription.
fn rereads_plan(data: &Value) -> bool {
    matches!(data["type"].as_str(), Some("connected" | "reset"))
        || data["events"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|event| event["kind"] == "billing")
}

async fn read_plan(handle: &EngineHandle, owner: &Owner) -> Option<bool> {
    let value = handle
        .client()
        .call_scoped(methods::CLOUD_BILLING_ENTITLEMENTS, scoped(owner))
        .await
        .ok()?;
    if !bound(&value, owner) {
        return None;
    }
    zuse_model::cloud_billing::cloud_subscribed(
        &value["data"],
        chrono::Utc::now().timestamp_millis(),
    )
}

/// Follow the signed-in account and keep its subscription answer current.
pub(super) fn spawn(cx: &mut Context<AppState>, handle: EngineHandle) -> Task<()> {
    if !crate::cloud_beta() || !zuse_viewport::cloud_gate::enabled() {
        return Task::ready(());
    }
    cx.spawn(async move |this, cx| {
        'reconnect: loop {
            let Ok(mut auth) = handle
                .client()
                .subscribe_scoped(methods::CLOUD_ACCOUNT_STATUS, json!({}))
                .await
            else {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                continue;
            };
            let mut next = auth.recv().await;
            'account: loop {
                let Some(value) = next.take() else {
                    continue 'reconnect;
                };
                let Some(owner) = owner_of(&value) else {
                    next = auth.recv().await;
                    continue;
                };
                let mut events: Option<zuse_rpc::RpcSubscription> = None;
                // Backoff for a feed that will not connect, and for a
                // subscription read that failed (no answer to keep yet).
                let mut connect_delay: Option<u64> = None;
                let mut read_delay: Option<u64> = None;
                let mut read = true;
                loop {
                    if std::mem::take(&mut read) {
                        let subscribed = read_plan(&handle, &owner).await;
                        read_delay = subscribed
                            .is_none()
                            .then(|| read_delay.map_or(2, |s| (s * 2).min(60)));
                        let applied = this.update(cx, |state, cx| {
                            if state.apply_cloud_subscription(&owner, subscribed) {
                                cx.notify();
                            }
                        });
                        if applied.is_err() {
                            return;
                        }
                    }
                    let executor = cx.background_executor().clone();
                    let needs_connect = events.is_none();
                    let connect = async {
                        if !needs_connect {
                            futures::future::pending::<()>().await;
                        }
                        if let Some(seconds) = connect_delay {
                            executor
                                .timer(std::time::Duration::from_secs(seconds))
                                .await;
                        }
                        handle
                            .client()
                            .subscribe_scoped(methods::CLOUD_EVENTS, scoped(&owner))
                            .await
                            .ok()
                    };
                    let reread = async {
                        match read_delay {
                            Some(seconds) => {
                                executor
                                    .timer(std::time::Duration::from_secs(seconds))
                                    .await
                            }
                            None => futures::future::pending().await,
                        }
                    };
                    let event = async {
                        match &mut events {
                            Some(events) => events.recv().await,
                            None => futures::future::pending().await,
                        }
                    };
                    tokio::select! {
                        biased;
                        value = auth.recv() => {
                            let Some(value) = value else { continue 'reconnect };
                            if owner_of(&value).as_ref() != Some(&owner) {
                                next = Some(value);
                                continue 'account;
                            }
                        }
                        frame = event => match frame {
                            Some(frame) if bound(&frame, &owner) => {
                                let data = &frame["data"];
                                if matches!(data["type"].as_str(), Some("error" | "closed")) {
                                    events = None;
                                    connect_delay = Some(2);
                                } else {
                                    connect_delay = None;
                                    read = rereads_plan(data);
                                }
                            }
                            Some(_) => {}
                            None => {
                                events = None;
                                connect_delay = Some(2);
                            }
                        },
                        connection = connect => match connection {
                            Some(connection) => events = Some(connection),
                            None => {
                                connect_delay =
                                    Some(connect_delay.map_or(2, |s| (s * 2).min(30)));
                            }
                        },
                        _ = reread => read = true,
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_in(state: &mut AppState, user: &str) {
        state.apply_cloud_account(json!({"user":{"id":user},"orgId":null}));
    }

    #[test]
    fn access_follows_the_account_and_its_subscription() {
        let mut state = AppState::new();
        assert_eq!(state.cloud_access(), CloudAccess::Unknown);
        assert!(!state.cloud_access().settled());
        state.apply_cloud_account(json!({"user":null,"orgId":null}));
        assert_eq!(state.cloud_access(), CloudAccess::SignedOut);
        assert!(state.cloud_access().withholds_cloud());

        signed_in(&mut state, "owner");
        assert_eq!(state.cloud_access(), CloudAccess::Checking);
        assert!(state.cloud_access().signed_in());
        assert!(state.cloud_access().offers_cloud());
        assert!(!state.cloud_access().withholds_cloud());

        state.set_cloud_subscription_for_test(false);
        assert_eq!(state.cloud_access(), CloudAccess::SignedIn);
        assert!(!state.cloud_access().offers_cloud());
        assert!(state.cloud_access().withholds_cloud());

        state.set_cloud_subscription_for_test(true);
        assert_eq!(state.cloud_access(), CloudAccess::Subscribed);

        // A failed reread keeps the last answer.
        let owner = ("owner".to_owned(), None);
        assert!(!state.apply_cloud_subscription(&owner, None));
        assert_eq!(state.cloud_access(), CloudAccess::Subscribed);

        // The subscription ends: the picked cloud repository goes with it.
        state.cloud_project = Some(json!({"projectId":"p"}));
        assert!(state.apply_cloud_subscription(&owner, Some(false)));
        assert_eq!(state.cloud_access(), CloudAccess::SignedIn);
        assert!(state.cloud_project.is_none());
    }

    #[test]
    fn another_account_starts_unchecked_and_late_answers_are_dropped() {
        let mut state = AppState::new();
        signed_in(&mut state, "first");
        state.set_cloud_subscription_for_test(true);
        signed_in(&mut state, "second");
        assert_eq!(state.cloud_access(), CloudAccess::Checking);
        // The first account's read lands after the switch.
        let first = ("first".to_owned(), None);
        assert!(!state.apply_cloud_subscription(&first, Some(true)));
        assert_eq!(state.cloud_access(), CloudAccess::Checking);
        // An organization is its own subscriber.
        state.apply_cloud_account(json!({"user":{"id":"second"},"orgId":"org"}));
        let personal = ("second".to_owned(), None);
        assert!(!state.apply_cloud_subscription(&personal, Some(true)));
        let org = ("second".to_owned(), Some("org".to_owned()));
        assert!(state.apply_cloud_subscription(&org, Some(true)));
        assert_eq!(state.cloud_access(), CloudAccess::Subscribed);
    }

    #[test]
    fn billing_hints_and_reconnects_reread_the_subscription() {
        assert!(rereads_plan(&json!({"type":"connected"})));
        assert!(rereads_plan(&json!({"type":"reset"})));
        assert!(rereads_plan(&json!({"type":"events","events":[
            {"kind":"workspace","resourceId":"w"},
            {"kind":"billing","resourceId":"entitlement"}]})));
        assert!(!rereads_plan(&json!({"type":"events","events":[
            {"kind":"workspace","resourceId":"w"}]})));
        assert!(!rereads_plan(&json!({"type":"revalidate","live":true})));
        assert!(!rereads_plan(&json!({"type":"reconnecting"})));
    }
}
