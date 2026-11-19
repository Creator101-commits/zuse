//! The account's image builds by itself. Adding repositories or saving the
//! setup script queues a build on the server; this is the backstop for the
//! cases it can't see: an account that never saved a machine version, or one
//! whose image is needed but has nothing queued. It runs debounced, once per
//! account state, only for people who can manage Cloud, and never starts a
//! build while one is waiting or running. Progress shows in onboarding and
//! on the Overview with one shared row.
use super::*;
use crate::cloud_settings::kit::{self, Tone};

/// What the backstop does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutoBuild {
    /// No machine version yet: save the standard one, which queues the
    /// image of the account's repositories.
    SaveFirstVersion,
    /// The image is needed but nothing is queued: ask for a build.
    Rebuild,
}

/// The account state the backstop decides from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ImageFacts<'a> {
    pub subscribed: bool,
    pub can_manage: bool,
    /// A machine read, save or rebuild is in flight.
    pub busy: bool,
    /// Unsaved setup script edits: the person is about to save them.
    pub editing: bool,
    pub repositories: usize,
    /// The machine revision; `None` until it loads.
    pub revision: Option<i64>,
    /// `none`, `queued`, `building`, `ready` or `failed`.
    pub state: Option<&'a str>,
    /// The server reports a build row for the current recipe.
    pub has_build: bool,
}

/// Whether to start the image build without a click, and how.
pub(crate) fn auto_build(facts: &ImageFacts) -> Option<AutoBuild> {
    if !facts.subscribed
        || !facts.can_manage
        || facts.busy
        || facts.editing
        || facts.repositories == 0
    {
        return None;
    }
    if facts.revision? == 0 {
        return Some(AutoBuild::SaveFirstVersion);
    }
    // Building, ready and failed images are left alone: a failure is
    // retried by the person, never in a loop.
    (facts.state? == "queued" && !facts.has_build).then_some(AutoBuild::Rebuild)
}

/// How long account changes settle before the backstop looks.
pub(crate) const SETTLE: std::time::Duration = std::time::Duration::from_millis(1200);

/// What a first chat can say about the image.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageReadiness {
    #[default]
    Unknown,
    /// Queued or building: chats start without it until it verifies.
    Building,
    Ready,
    Failed,
}

impl RepositoryPanel {
    fn image_facts(&self, cx: &App) -> ImageFacts<'_> {
        let version = self.machine.version();
        ImageFacts {
            subscribed: self.cloud_subscribed(),
            can_manage: self.can_manage(),
            busy: self.pending.values().any(|p| p.kind.starts_with("machine")),
            editing: self.machine_dirty(cx),
            repositories: self.projects.len(),
            revision: version.map(|v| v.revision),
            state: version.map(|v| v.image_state.as_str()),
            has_build: version.is_some_and(|v| v.image.is_some()),
        }
    }

    /// Look again once account changes settle. Repeated calls in a burst
    /// (several repositories added, a feed hint) collapse into one look.
    pub(super) fn schedule_image_check(&mut self, cx: &mut Context<Self>) {
        if self.owner.is_none() || !self.loaded {
            return;
        }
        self.image_check = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            let _ = this.update(cx, |this, cx| {
                this.image_check = None;
                this.run_image_check(cx);
            });
        }));
    }

    pub(super) fn run_image_check(&mut self, cx: &mut Context<Self>) {
        let Some(action) = auto_build(&self.image_facts(cx)) else {
            return;
        };
        let Some(owner) = self.owner.clone() else {
            return;
        };
        // One automatic attempt per account state, so a build the server
        // drops is never re-requested in a loop.
        let key = (
            owner,
            self.machine.version().map_or(0, |v| v.revision),
            self.projects.len(),
        );
        if self.image_auto.as_ref() == Some(&key) {
            return;
        }
        self.image_auto = Some(key);
        match action {
            AutoBuild::SaveFirstVersion => {
                let Some(recipe) = self.machine.version().map(|v| v.recipe.clone()) else {
                    return;
                };
                self.call(
                    "machine-init",
                    methods::CLOUD_MACHINE_SAVE,
                    json!({"expectedRevision":0,"recipe":recipe}),
                    cx,
                );
            }
            AutoBuild::Rebuild => {
                self.call(
                    "machine-auto",
                    methods::CLOUD_MACHINE_REBUILD,
                    json!({}),
                    cx,
                );
            }
        }
        cx.notify();
    }

    pub(super) fn image_readiness(&self) -> ImageReadiness {
        match self.machine.image_state() {
            Some("queued" | "building") => ImageReadiness::Building,
            Some("ready") => ImageReadiness::Ready,
            Some("failed") => ImageReadiness::Failed,
            _ => ImageReadiness::Unknown,
        }
    }

    /// One row for the account's image: building with the dithered meter,
    /// ready, or failed with Retry. `None` until there is something to say.
    pub(super) fn image_status_row(&self, first: bool, cx: &mut Context<Self>) -> Option<Div> {
        let version = self.machine.version()?;
        let build = version.image.as_ref();
        let repositories = build
            .map(|b| b.repositories.iter().filter(|r| r.state == "baked").count())
            .filter(|n| *n > 0)
            .unwrap_or(self.projects.len());
        let carries = match repositories {
            0 => String::new(),
            1 => " · 1 repository".to_owned(),
            n => format!(" · {n} repositories"),
        };
        let (title, detail, pill, meter): (&str, String, Div, Option<f32>) =
            match version.image_state.as_str() {
                "queued" => (
                    "Preparing your cloud computer",
                    format!("Image build queued{carries}"),
                    kit::pill("Queued", Tone::Progress),
                    Some(0.04),
                ),
                "building" => {
                    let phase = build.and_then(|b| b.phase.as_deref()).unwrap_or_default();
                    (
                        "Preparing your cloud computer",
                        format!("Building image · {}", machine::phase_copy(phase)),
                        kit::pill("Building", Tone::Progress),
                        Some(machine::build_fraction(phase)),
                    )
                }
                "ready" => (
                    "Your image is ready",
                    format!("New computers start from it{carries}"),
                    kit::pill("Ready", Tone::Ready),
                    None,
                ),
                "failed" => (
                    "Image build failed",
                    // The log itself lives on the Machine page.
                    machine::error_copy(version.image_error.as_deref().unwrap_or_default())
                        .replace(" Its output is below.", ""),
                    kit::pill("Failed", Tone::Error),
                    None,
                ),
                _ => return None,
            };
        let failed = version.image_state == "failed";
        let busy = self.pending.values().any(|p| p.kind.starts_with("machine"));
        let retry = (failed && self.can_manage()).then(|| {
            cs::button("cloud-image-retry", "Retry", false, busy, false)
                .on_click(cx.listener(|this, _, _, cx| this.rebuild_machine(cx)))
                .into_any_element()
        });
        let trailing = row()
            .items_center()
            .gap(px(8.))
            .child(pill)
            .children(retry)
            .into_any_element();
        let mut body = column().child(kit::row_item(
            first,
            Some(kit::icon_tile("dither-monitor", 28.).into_any_element()),
            title,
            detail,
            Some(trailing),
        ));
        if let Some(fraction) = meter {
            body = body.child(
                div()
                    .px(px(kit::INSET))
                    .pb(px(12.))
                    .child(kit::meter(fraction, 6.)),
            );
        }
        Some(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn ready_account() -> ImageFacts<'static> {
        ImageFacts {
            subscribed: true,
            can_manage: true,
            busy: false,
            editing: false,
            repositories: 2,
            revision: Some(3),
            state: Some("queued"),
            has_build: false,
        }
    }

    #[test]
    fn a_needed_image_with_nothing_queued_is_requested() {
        assert_eq!(auto_build(&ready_account()), Some(AutoBuild::Rebuild));
        // A first machine version queues the image of the repositories.
        let fresh = ImageFacts {
            revision: Some(0),
            state: Some("none"),
            ..ready_account()
        };
        assert_eq!(auto_build(&fresh), Some(AutoBuild::SaveFirstVersion));
    }

    #[test]
    fn nothing_starts_without_repositories_permission_or_a_subscription() {
        for blocked in [
            ImageFacts {
                repositories: 0,
                ..ready_account()
            },
            ImageFacts {
                can_manage: false,
                ..ready_account()
            },
            ImageFacts {
                subscribed: false,
                ..ready_account()
            },
            // Still loading the machine.
            ImageFacts {
                revision: None,
                state: None,
                ..ready_account()
            },
        ] {
            assert_eq!(auto_build(&blocked), None, "{blocked:?}");
        }
    }

    #[test]
    fn never_a_second_build_or_a_retry_loop() {
        for state in ["building", "ready", "failed", "none"] {
            let facts = ImageFacts {
                state: Some(state),
                has_build: state != "none",
                ..ready_account()
            };
            assert_eq!(auto_build(&facts), None, "{state}");
        }
        // Already queued on the server.
        assert_eq!(
            auto_build(&ImageFacts {
                has_build: true,
                ..ready_account()
            }),
            None
        );
        // A request in flight, or a script being edited, waits.
        assert_eq!(
            auto_build(&ImageFacts {
                busy: true,
                ..ready_account()
            }),
            None
        );
        assert_eq!(
            auto_build(&ImageFacts {
                editing: true,
                ..ready_account()
            }),
            None
        );
    }
}
