//! Which networked features this build shows.
//!
//! [`enabled`] covers cloud: the Zuse account (sign-in, Billing), cloud
//! workspaces, sync, account-linked computers and hosted plugins.
//! [`remote_enabled`] covers remote access: pairing other computers, phones
//! and browsers with this one.
//!
//! Cloud is on in every desktop build. Within it, the desktop app offers
//! cloud computers only to accounts with an active Cloud subscription (see
//! `zuse_ui::state::CloudAccess`), so a signed-out or unsubscribed account
//! sees the local app plus a way to sign in and subscribe. Remote access
//! stays off in release builds and on in development builds.
//!
//! `ZUSE_CLOUD` / `ZUSE_REMOTE` set to `1` turn a feature on and `0` turn it
//! off (`ZUSE_CLOUD=0` previews the local-only app). Browsers have no
//! process environment and no subscription gate yet, so they keep the build
//! default for both: on in development builds, off in release builds.

use std::sync::OnceLock;

/// Cloud features. Read once per process; a restart picks up a changed
/// `ZUSE_CLOUD`.
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| resolve(std::env::var("ZUSE_CLOUD").ok().as_deref(), CLOUD_DEFAULT))
}

/// Remote access (pairing and other computers). Read once per process; a
/// restart picks up a changed `ZUSE_REMOTE`.
pub fn remote_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        resolve(
            std::env::var("ZUSE_REMOTE").ok().as_deref(),
            cfg!(debug_assertions),
        )
    })
}

/// Placeholder for the archived-chats search, which only covers Cloud
/// archives when cloud is on.
pub fn archive_search_placeholder() -> &'static str {
    if enabled() {
        "Search saved chats or Cloud archives…"
    } else {
        "Search saved chats…"
    }
}

/// Desktop builds always offer cloud: the account and its subscription
/// decide what shows. The browser UI keeps the build default.
const CLOUD_DEFAULT: bool = !cfg!(target_arch = "wasm32") || cfg!(debug_assertions);

fn resolve(value: Option<&str>, default: bool) -> bool {
    match value.map(str::trim) {
        Some("1" | "true" | "on") => true,
        Some("0" | "false" | "off") => false,
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn env_overrides_the_build_default() {
        for default in [true, false] {
            assert!(resolve(Some("1"), default));
            assert!(resolve(Some(" true "), default));
            assert!(!resolve(Some("0"), default));
            assert!(!resolve(Some("off"), default));
            assert_eq!(resolve(None, default), default);
            assert_eq!(resolve(Some("maybe"), default), default);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn desktop_builds_offer_cloud_by_default() {
        const { assert!(super::CLOUD_DEFAULT) };
    }
}
