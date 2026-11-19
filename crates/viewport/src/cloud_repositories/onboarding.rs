//! First-run Cloud onboarding: what a new subscriber sees right after paying
//! or starting the trial. Five short steps — welcome, repositories, setup
//! script, agent accounts, first chat — each skippable, with dithered
//! progress. Completion always comes from real account state, so the flow
//! resumes at the first unfinished step and never disagrees with Settings.
use super::plan::DEFAULT_MACHINE;
use super::*;
use crate::cloud_settings::kit::{self, StepMark, Tone};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    Welcome,
    Repositories,
    Setup,
    Agents,
    Start,
}

impl Step {
    pub(crate) const ALL: [Step; 5] = [
        Step::Welcome,
        Step::Repositories,
        Step::Setup,
        Step::Agents,
        Step::Start,
    ];
    pub(crate) fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
    fn next(self) -> Option<Step> {
        Self::ALL.get(self.index() + 1).copied()
    }
    fn previous(self) -> Option<Step> {
        self.index().checked_sub(1).map(|i| Self::ALL[i])
    }
    /// Fixture and deep-link names.
    pub(crate) fn slug(self) -> &'static str {
        match self {
            Step::Welcome => "welcome",
            Step::Repositories => "repositories",
            Step::Setup => "setup",
            Step::Agents => "agents",
            Step::Start => "start",
        }
    }
    pub(crate) fn from_slug(slug: &str) -> Option<Step> {
        Self::ALL.into_iter().find(|s| s.slug() == slug)
    }
    /// The short label under the progress bar.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Step::Welcome => "Welcome",
            Step::Repositories => "Repositories",
            Step::Setup => "Setup",
            Step::Agents => "Agents",
            Step::Start => "First chat",
        }
    }
    pub(crate) fn title(self) -> &'static str {
        match self {
            Step::Welcome => "Welcome to Cloud",
            Step::Repositories => "Choose your repositories",
            Step::Setup => "Add a setup script",
            Step::Agents => "Connect your coding agents",
            Step::Start => "Start your first cloud chat",
        }
    }
    pub(crate) fn summary(self) -> &'static str {
        match self {
            Step::Welcome => {
                "Your plan is active. Agents now run on your own cloud computer and keep working after you close Zuse."
            }
            Step::Repositories => {
                "Connect GitHub, then pick the repositories agents can check out, commit to and open pull requests on."
            }
            Step::Setup => {
                "Optional. Your image builds by itself with your repositories; add the tools every repository needs."
            }
            Step::Agents => {
                "Cloud chats sign in with your own Codex, Claude or Grok account, so they use your plan."
            }
            Step::Start => {
                "Pick a repository. Your first chat opens on Cloud with it, ready for your message."
            }
        }
    }
    /// Title and summary for the step header. The last step only promises
    /// a chat once a repository and an agent account are both connected.
    pub(crate) fn heading(self, progress: &Progress) -> (&'static str, &'static str) {
        if self == Step::Start && !progress.ready() {
            return (
                "Almost there",
                "A cloud chat needs a repository and an agent account. Finish those below, then start your first chat.",
            );
        }
        (self.title(), self.summary())
    }
    fn glyph(self) -> &'static str {
        match self {
            Step::Welcome => "dither-cloud",
            Step::Repositories => "dither-branch",
            Step::Setup => "dither-term",
            Step::Agents => "dither-key",
            Step::Start => "dither-send",
        }
    }
    /// Steps a person can skip; Welcome and Start bookend the flow.
    pub(crate) fn skippable(self) -> bool {
        matches!(self, Step::Repositories | Step::Setup | Step::Agents)
    }
    pub(crate) fn optional(self) -> bool {
        self == Step::Setup
    }
}

/// What the account already has, from loaded account state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Progress {
    pub github: bool,
    pub repositories: bool,
    pub setup: bool,
    pub agents: bool,
}

impl Progress {
    pub(crate) fn done(&self, step: Step) -> bool {
        match step {
            Step::Welcome => self.github || self.repositories || self.agents,
            Step::Repositories => self.github && self.repositories,
            Step::Setup => self.setup,
            Step::Agents => self.agents,
            Step::Start => false,
        }
    }
    /// What a first cloud chat needs: a repository and an agent account.
    pub(crate) fn ready(&self) -> bool {
        self.done(Step::Repositories) && self.done(Step::Agents)
    }
    /// The first required step still missing.
    pub(crate) fn first_missing(&self) -> Option<Step> {
        [Step::Repositories, Step::Agents]
            .into_iter()
            .find(|s| !self.done(*s))
    }
}

/// Whether onboarding may appear: only for a subscribed account whose setup
/// has loaded, and only to people who can change it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Gate {
    pub subscribed: bool,
    pub loaded: bool,
    pub can_manage: bool,
}

/// Session state for the flow. Survives navigating between Settings pages
/// (one panel backs every Cloud page); a restart resumes from account state.
#[derive(Clone, Debug, Default)]
pub(crate) struct Onboarding {
    step: Option<Step>,
    skipped: BTreeSet<Step>,
    /// "Finish later" was chosen; the flow stays closed until reopened.
    dismissed: bool,
    /// Reopened from Overview after setup was complete or dismissed.
    open: bool,
    /// "Start your first cloud chat" was chosen.
    finished: bool,
}

impl Onboarding {
    pub(crate) fn visible(&self, gate: Gate, progress: &Progress) -> bool {
        gate.subscribed
            && gate.loaded
            && gate.can_manage
            // A flow someone is walking stays up until they finish or leave,
            // even once the last required step completes underneath it.
            && (self.open
                || self.step.is_some()
                || (!self.dismissed && !self.finished && !progress.ready()))
    }
    /// The step on screen: the one chosen, or where a returning person left
    /// off — Welcome before anything is set up, then the first missing step.
    pub(crate) fn current(&self, progress: &Progress) -> Step {
        self.step.unwrap_or_else(|| {
            if !progress.done(Step::Welcome) {
                return Step::Welcome;
            }
            progress
                .first_missing()
                .filter(|s| !self.skipped.contains(s))
                .unwrap_or(Step::Start)
        })
    }
    pub(crate) fn mark(&self, step: Step, progress: &Progress) -> StepMark {
        let current = self.current(progress);
        if step == current {
            StepMark::Current
        } else if progress.done(step) || (step == Step::Welcome && current != Step::Welcome) {
            StepMark::Done
        } else if self.skipped.contains(&step) {
            StepMark::Skipped
        } else if step < current {
            StepMark::Skipped
        } else {
            StepMark::Upcoming
        }
    }
    pub(crate) fn go(&mut self, step: Step) {
        self.step = Some(step);
    }
    pub(crate) fn advance(&mut self, progress: &Progress) {
        let current = self.current(progress);
        if let Some(next) = current.next() {
            self.step = Some(next);
        }
    }
    pub(crate) fn back(&mut self, progress: &Progress) {
        if let Some(previous) = self.current(progress).previous() {
            self.step = Some(previous);
        }
    }
    pub(crate) fn skip(&mut self, progress: &Progress) {
        let current = self.current(progress);
        if current.skippable() {
            self.skipped.insert(current);
            self.advance(progress);
        }
    }
    pub(crate) fn later(&mut self) {
        self.dismissed = true;
        self.open = false;
        self.step = None;
    }
    pub(crate) fn reopen(&mut self) {
        self.open = true;
        self.finished = false;
        self.step = None;
    }
    pub(crate) fn finish(&mut self) {
        self.finished = true;
        self.open = false;
        self.step = None;
    }
}

impl RepositoryPanel {
    pub(super) fn onboarding_progress(&self) -> Progress {
        Progress {
            github: self.github_connected(),
            repositories: !self.projects.is_empty(),
            setup: self.machine.customized(),
            agents: self.agents.any_connected(),
        }
    }
    pub(super) fn onboarding_gate(&self) -> Gate {
        Gate {
            subscribed: self.cloud_subscribed(),
            loaded: self.loaded && self.owner.is_some(),
            can_manage: self.can_manage(),
        }
    }
    pub(super) fn onboarding_visible(&self) -> bool {
        self.onboarding
            .visible(self.onboarding_gate(), &self.onboarding_progress())
    }
    pub(super) fn onboarding_reopen(&mut self, cx: &mut Context<Self>) {
        self.onboarding.reopen();
        cx.notify();
    }
    /// Opens onboarding, optionally at a step (`welcome`, `repositories`,
    /// `setup`, `agents`, `start`). Used by "Set up Cloud" and review fixtures.
    pub fn open_onboarding(&mut self, step: Option<&str>, cx: &mut Context<Self>) {
        self.onboarding.reopen();
        if let Some(step) = step.and_then(Step::from_slug) {
            self.onboarding.go(step);
        }
        cx.notify();
    }
    fn onboarding_update(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Onboarding, &Progress),
    ) {
        let progress = self.onboarding_progress();
        f(&mut self.onboarding, &progress);
        cx.notify();
    }

    fn onboarding_header(&self, current: Step, progress: &Progress) -> Div {
        let marks: Vec<StepMark> = Step::ALL
            .iter()
            .map(|s| self.onboarding.mark(*s, progress))
            .collect();
        let mut labels = row().gap(px(4.)).mt(px(6.));
        for step in Step::ALL {
            labels = labels.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(10.))
                    .line_height(px(13.))
                    .text_color(if step == current {
                        palette::rgb(TEXT).into()
                    } else {
                        palette::rgb(MUTED).opacity(0.8)
                    })
                    .when(step == current, |v| v.font_weight(FontWeight::MEDIUM))
                    .child(step.label()),
            );
        }
        let intro = s::row(false)
            .flex_nowrap()
            .items_start()
            .gap(px(14.))
            .child(kit::icon_tile(current.glyph(), 40.))
            .child(
                column()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(if current == Step::Welcome { 18. } else { 15. }))
                            .line_height(px(if current == Step::Welcome { 24. } else { 20. }))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette::rgb(TEXT))
                            .child(current.heading(progress).0),
                    )
                    .child(s::description_text(current.heading(progress).1)),
            )
            .when(current.optional(), |v| {
                v.child(kit::pill("Optional", Tone::Neutral))
            });
        kit::section(
            "Set up Cloud",
            "",
            Some(
                div()
                    .text_size(px(10.))
                    .text_color(palette::rgb(MUTED))
                    .child(SharedString::from(format!(
                        "Step {} of {}",
                        current.index() + 1,
                        Step::ALL.len()
                    )))
                    .into_any_element(),
            ),
            column()
                .child(
                    column()
                        .px(px(s::ROW_PAD_X))
                        .py(px(s::ROW_PAD_Y))
                        .child(kit::step_progress(&marks))
                        .child(labels),
                )
                .child(intro),
        )
    }

    fn onboarding_plan_card(&self) -> Div {
        let plan = self.plan_glance();
        let (name, status) = match &plan {
            Some(plan) => (
                plan.label(),
                if plan.trial {
                    kit::pill("Trial", Tone::Progress)
                } else {
                    kit::pill("Active", Tone::Ready)
                },
            ),
            None => ("Cloud".to_owned(), kit::pill("Active", Tone::Ready)),
        };
        let minutes = match &plan {
            Some(plan) => kit::quantity(
                "Machine minutes",
                plan.minutes_line(),
                plan.fraction_left(),
                plan.hours_line(),
            ),
            None => kit::quantity("Machine minutes", "Checking your plan…", 0., ""),
        };
        kit::section(
            "Your plan",
            "",
            None,
            column()
                .child(
                    s::row(true).child(
                        column()
                            .flex_1()
                            .min_w_0()
                            .gap(px(14.))
                            .child(
                                row()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .text_size(px(15.))
                                            .line_height(px(20.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(palette::rgb(TEXT))
                                            .child(name),
                                    )
                                    .child(status),
                            )
                            .child(minutes),
                    ),
                )
                .child(kit::row_item(
                    false,
                    None,
                    "Your cloud computer",
                    "",
                    Some(kit::spec_chip(DEFAULT_MACHINE).into_any_element()),
                )),
        )
    }

    fn onboarding_checklist(&self, progress: &Progress, cx: &mut Context<Self>) -> Div {
        let mut list = column();
        for (i, step) in [Step::Repositories, Step::Setup, Step::Agents]
            .into_iter()
            .enumerate()
        {
            let done = progress.done(step);
            let pill = if done {
                kit::pill("Done", Tone::Ready)
            } else if self.onboarding.skipped.contains(&step) {
                kit::pill("Skipped", Tone::Neutral)
            } else if step.optional() {
                kit::pill("Optional", Tone::Neutral)
            } else {
                kit::pill("Needed", Tone::Attention)
            };
            let detail: String = match step {
                Step::Repositories if done => {
                    let n = self.projects.len();
                    format!("{n} repositor{} ready", if n == 1 { "y" } else { "ies" })
                }
                Step::Agents if done => self.agents.connected_summary().join(", "),
                Step::Setup if done => "Your machine has a custom setup script.".into(),
                Step::Setup => "Computers start from the standard machine.".into(),
                _ => step.summary().into(),
            };
            list = list.child(
                div()
                    .id(SharedString::from(format!(
                        "onboarding-check-{}",
                        step.slug()
                    )))
                    .cursor_pointer()
                    .hover(|v| v.bg(s::frame_bg().opacity(0.4)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.onboarding_update(cx, |o, _| o.go(step))
                    }))
                    .child(kit::row_item(
                        i == 0,
                        Some(kit::icon_tile(step.glyph(), 28.).into_any_element()),
                        step.label(),
                        detail,
                        Some(pill.into_any_element()),
                    )),
            );
        }
        list
    }

    fn onboarding_body(
        &mut self,
        current: Step,
        progress: &Progress,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        match current {
            Step::Welcome => column()
                .gap(px(kit::SECTION_GAP))
                .child(self.onboarding_plan_card())
            .child(kit::section(
                "What's next",
                "Three quick steps. Skip any of them and finish later from Settings → Cloud computer.",
                None,
                self.onboarding_checklist(progress, cx),
            )),
            Step::Repositories => {
                if self.github_connected() {
                    column()
                        .gap(px(kit::SECTION_GAP))
                        .child(self.repositories(window, cx))
                } else {
                    column()
                        .gap(px(kit::SECTION_GAP))
                        .child(self.github_group(window, cx))
                }
            }
            Step::Setup => {
                let image = self.image_status_row(true, cx);
                self.render_machine_setup(cx).when_some(image, |v, image| {
                    v.child(kit::section("Your image", "", None, image))
                })
            }
            Step::Agents => self.render_agents(window, cx),
            Step::Start if !progress.ready() => column().gap(px(kit::SECTION_GAP)).child(
                kit::section(
                    "Your setup",
                    "A first chat needs a repository and an agent account.",
                    None,
                    self.onboarding_checklist(progress, cx),
                ),
            ),
            Step::Start => {
                let image = self.image_status_row(true, cx);
                column()
                    .gap(px(kit::SECTION_GAP))
                    .child(kit::section(
                        "Repository",
                        "",
                        None,
                        self.first_chat_repositories(cx),
                    ))
                    .child(kit::section(
                        "Your cloud computer",
                        "",
                        Some(
                            kit::info_tip(
                                "How a cloud chat starts",
                                "A fresh computer starts from your image with the repository checked out, and your agent signs in with your connected account. It sleeps when quiet.",
                            )
                            .into_any_element(),
                        ),
                        image.unwrap_or_else(|| {
                            column().child(kit::row_item(
                                true,
                                Some(kit::icon_tile("dither-monitor", 28.).into_any_element()),
                                "Cloud computer",
                                "",
                                Some(kit::spec_chip(DEFAULT_MACHINE).into_any_element()),
                            ))
                        }),
                    ))
            }
        }
    }

    /// One minimal row: "Finish later" on the left; Back and Skip as quiet
    /// text and the primary Continue on the right. No card around it. On the
    /// setup step, unsaved script edits are saved by Continue ("Save and
    /// continue") instead of a second save bar.
    fn onboarding_footer(&self, current: Step, progress: &Progress, cx: &mut Context<Self>) -> Div {
        let done = progress.done(current);
        let unsaved = current == Step::Setup && self.machine_dirty(cx) && self.can_manage();
        let (label, enabled): (&'static str, bool) = match current {
            Step::Welcome => ("Get started", true),
            Step::Start if progress.ready() => ("Start first chat", true),
            Step::Start => ("Finish setup", true),
            Step::Setup if unsaved => ("Save and continue", true),
            _ => ("Continue", done || current.optional()),
        };
        let mut actions = row().gap(px(4.)).items_center();
        if current.previous().is_some() {
            actions = actions.child(kit::quiet_button("onboarding-back", "Back").on_click(
                cx.listener(|this, _, _, cx| this.onboarding_update(cx, |o, p| o.back(p))),
            ));
        }
        if current.skippable() && !done {
            actions = actions.child(
                kit::quiet_button("onboarding-skip", "Skip for now").on_click(
                    cx.listener(|this, _, _, cx| this.onboarding_update(cx, |o, p| o.skip(p))),
                ),
            );
        }
        actions = actions.child(
            kit::primary_button("onboarding-continue", label, enabled)
                .ml(px(4.))
                .when(enabled, |b| {
                    b.on_click(cx.listener(move |this, _, _, cx| match current {
                        Step::Start => {
                            let progress = this.onboarding_progress();
                            if let Some(missing) = progress.first_missing() {
                                this.onboarding.go(missing);
                            } else {
                                this.onboarding.finish();
                                cx.emit(StartChat {
                                    repository: this.first_chat_repository(),
                                    image: this.image_readiness(),
                                });
                            }
                            cx.notify();
                        }
                        Step::Setup => {
                            if this.machine_dirty(cx) && this.can_manage() {
                                this.save_machine(cx);
                            }
                            this.schedule_image_check(cx);
                            this.onboarding_update(cx, |o, p| o.advance(p));
                        }
                        Step::Repositories => {
                            // The image starts building now, in the background.
                            this.schedule_image_check(cx);
                            this.onboarding_update(cx, |o, p| o.advance(p));
                        }
                        _ => this.onboarding_update(cx, |o, p| o.advance(p)),
                    }))
                }),
        );
        let waiting = match current {
            Step::Repositories if !done => "Add a repository to continue, or skip for now",
            Step::Agents if !done => "Connect one agent to continue, or skip for now",
            _ => "",
        };
        row()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .px(px(4.))
            .pt(px(4.))
            .child(
                row()
                    .items_center()
                    .gap(px(12.))
                    .min_w_0()
                    .child(
                        div()
                            .id("onboarding-later")
                            .role(Role::Button)
                            .tab_index(0)
                            .cursor_pointer()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(palette::rgb(MUTED))
                            .hover(|v| v.text_color(palette::rgb(TEXT)))
                            .child("Finish later")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.onboarding.later();
                                cx.notify();
                            })),
                    )
                    .when(!waiting.is_empty(), |v| {
                        v.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(palette::rgb(MUTED).opacity(0.8))
                                .child(waiting),
                        )
                    }),
            )
            .child(actions)
    }

    pub(super) fn render_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let progress = self.onboarding_progress();
        let current = self.onboarding.current(&progress);
        let header = self.onboarding_header(current, &progress);
        let body = self.onboarding_body(current, &progress, window, cx);
        let footer = self.onboarding_footer(current, &progress, cx);
        column()
            .gap(px(kit::SECTION_GAP))
            .child(header)
            .child(body)
            .child(footer)
    }
}

impl RepositoryPanel {
    /// The repository the first chat opens on: the one picked, else the most
    /// recently added.
    fn first_chat_project(&self) -> Option<&CloudProject> {
        let choices = self.first_chat_choices();
        self.first_chat_project
            .as_ref()
            .and_then(|id| choices.iter().find(|p| &p.project_id == id))
            .or(choices.first())
            .copied()
    }
    /// The connected repositories, newest first.
    fn first_chat_choices(&self) -> Vec<&CloudProject> {
        let mut projects: Vec<&CloudProject> = self.projects.iter().collect();
        projects.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        projects
    }
    fn first_chat_repository(&self) -> Option<FirstChatRepository> {
        self.first_chat_project().map(|p| FirstChatRepository {
            project_id: p.project_id.clone(),
            repository_identity: p.repository_identity.clone(),
            repository_url: p.repository_url.clone(),
            display_name: p.display_name.clone(),
            default_branch: p.default_branch.clone(),
        })
    }
    /// A compact single choice of the connected repositories, newest first.
    fn first_chat_repositories(&self, cx: &mut Context<Self>) -> Div {
        let chosen = self.first_chat_project().map(|p| p.project_id.clone());
        let mut list = column();
        for (i, project) in self.first_chat_choices().into_iter().take(6).enumerate() {
            let selected = chosen.as_deref() == Some(project.project_id.as_str());
            let id = project.project_id.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!(
                        "first-chat-repo-{}",
                        project.project_id
                    )))
                    .role(Role::RadioButton)
                    .aria_label(SharedString::from(project.display_name.clone()))
                    .aria_selected(selected)
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(|v| v.bg(s::frame_bg().opacity(0.4)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.first_chat_project = Some(id.clone());
                        cx.notify();
                    }))
                    .child(kit::row_item(
                        i == 0,
                        Some(
                            icon("folder")
                                .size(px(14.))
                                .text_color(palette::rgb(MUTED))
                                .into_any_element(),
                        ),
                        project.display_name.clone(),
                        "",
                        Some(radio(selected).into_any_element()),
                    )),
            );
        }
        list
    }
}

/// A small radio mark for single-choice rows.
fn radio(selected: bool) -> Div {
    div()
        .flex_none()
        .size(px(14.))
        .rounded_full()
        .border_1()
        .border_color(if selected {
            kit::accent()
        } else {
            Hsla::from(palette::rgb(MUTED)).opacity(0.6)
        })
        .flex()
        .items_center()
        .justify_center()
        .when(selected, |v| {
            v.child(div().size(px(6.)).rounded_full().bg(kit::accent()))
        })
}

/// The repository a first chat opens on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FirstChatRepository {
    pub project_id: String,
    pub repository_identity: String,
    pub repository_url: String,
    pub display_name: String,
    pub default_branch: String,
}

/// Asks the host to start a new cloud chat (the last onboarding step) on
/// `repository`, with what is known about the image.
#[derive(Clone, Debug)]
pub struct StartChat {
    pub repository: Option<FirstChatRepository>,
    pub image: super::ImageReadiness,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    fn gate() -> Gate {
        Gate {
            subscribed: true,
            loaded: true,
            can_manage: true,
        }
    }
    const NOTHING: Progress = Progress {
        github: false,
        repositories: false,
        setup: false,
        agents: false,
    };
    const READY: Progress = Progress {
        github: true,
        repositories: true,
        setup: false,
        agents: true,
    };

    #[test]
    fn only_subscribed_loaded_managers_see_onboarding() {
        let flow = Onboarding::default();
        assert!(flow.visible(gate(), &NOTHING));
        for blocked in [
            Gate {
                subscribed: false,
                ..gate()
            },
            Gate {
                loaded: false,
                ..gate()
            },
            Gate {
                can_manage: false,
                ..gate()
            },
        ] {
            assert!(!flow.visible(blocked, &NOTHING));
        }
        // Even reopened, an unsubscribed account never sees it.
        let mut reopened = Onboarding::default();
        reopened.reopen();
        assert!(!reopened.visible(
            Gate {
                subscribed: false,
                ..gate()
            },
            &READY
        ));
    }

    #[test]
    fn a_new_subscriber_walks_every_step_in_order() {
        let mut flow = Onboarding::default();
        let mut progress = NOTHING;
        assert_eq!(flow.current(&progress), Step::Welcome);
        flow.advance(&progress);
        assert_eq!(flow.current(&progress), Step::Repositories);
        progress.github = true;
        progress.repositories = true;
        flow.advance(&progress);
        assert_eq!(flow.current(&progress), Step::Setup);
        flow.advance(&progress);
        assert_eq!(flow.current(&progress), Step::Agents);
        progress.agents = true;
        flow.advance(&progress);
        assert_eq!(flow.current(&progress), Step::Start);
        // Completing the last step doesn't yank the flow away.
        assert!(progress.ready());
        assert!(flow.visible(gate(), &progress));
        flow.finish();
        assert!(!flow.visible(gate(), &progress));
        // Advancing past the end stays on Start.
        flow.go(Step::Start);
        flow.advance(&progress);
        assert_eq!(flow.current(&progress), Step::Start);
    }

    #[test]
    fn skipping_marks_steps_and_moves_on() {
        let mut flow = Onboarding::default();
        flow.go(Step::Repositories);
        flow.skip(&NOTHING);
        assert_eq!(flow.current(&NOTHING), Step::Setup);
        assert_eq!(flow.mark(Step::Repositories, &NOTHING), StepMark::Skipped);
        flow.skip(&NOTHING);
        flow.skip(&NOTHING);
        assert_eq!(flow.current(&NOTHING), Step::Start);
        assert_eq!(flow.mark(Step::Agents, &NOTHING), StepMark::Skipped);
        assert_eq!(flow.mark(Step::Welcome, &NOTHING), StepMark::Done);
        // Welcome and Start can't be skipped.
        flow.skip(&NOTHING);
        assert_eq!(flow.current(&NOTHING), Step::Start);
        flow.go(Step::Welcome);
        flow.skip(&NOTHING);
        assert_eq!(flow.current(&NOTHING), Step::Welcome);
        // Back walks one step at a time and stops at Welcome.
        flow.go(Step::Agents);
        flow.back(&NOTHING);
        assert_eq!(flow.current(&NOTHING), Step::Setup);
        flow.go(Step::Welcome);
        flow.back(&NOTHING);
        assert_eq!(flow.current(&NOTHING), Step::Welcome);
    }

    #[test]
    fn a_returning_person_resumes_at_the_first_unfinished_step() {
        let flow = Onboarding::default();
        // GitHub connected but no repository yet: back to Repositories.
        let partial = Progress {
            github: true,
            ..NOTHING
        };
        assert_eq!(flow.current(&partial), Step::Repositories);
        // Repositories done, agents missing: straight to Agents (the
        // optional setup step is not forced on resume).
        let agents = Progress {
            github: true,
            repositories: true,
            ..NOTHING
        };
        assert_eq!(flow.current(&agents), Step::Agents);
        assert_eq!(flow.mark(Step::Setup, &agents), StepMark::Skipped);
        // A skipped required step isn't forced again in the same session.
        let mut skipped = Onboarding::default();
        skipped.go(Step::Agents);
        skipped.skip(&agents);
        skipped.step = None;
        assert_eq!(skipped.current(&agents), Step::Start);
        // Ready accounts land on Start when reopened.
        let mut reopened = Onboarding::default();
        reopened.reopen();
        assert_eq!(reopened.current(&READY), Step::Start);
    }

    #[test]
    fn finish_later_closes_until_reopened() {
        let mut flow = Onboarding::default();
        flow.go(Step::Agents);
        flow.later();
        assert!(!flow.visible(gate(), &NOTHING));
        flow.reopen();
        assert!(flow.visible(gate(), &NOTHING));
        // Reopening resumes rather than restarting at Welcome once begun.
        let begun = Progress {
            github: true,
            ..NOTHING
        };
        assert_eq!(flow.current(&begun), Step::Repositories);
        // A complete setup doesn't open by itself.
        assert!(!Onboarding::default().visible(gate(), &READY));
    }

    #[test]
    fn the_last_step_only_promises_a_chat_when_ready() {
        assert_eq!(Step::Start.heading(&NOTHING).0, "Almost there");
        assert_eq!(Step::Start.heading(&READY).0, Step::Start.title());
        assert_eq!(Step::Agents.heading(&NOTHING).0, Step::Agents.title());
        // Steps passed without finishing read as skipped on the bar.
        let mut flow = Onboarding::default();
        flow.go(Step::Start);
        assert_eq!(flow.mark(Step::Repositories, &NOTHING), StepMark::Skipped);
        assert_eq!(flow.mark(Step::Repositories, &READY), StepMark::Done);
        assert_eq!(flow.mark(Step::Start, &NOTHING), StepMark::Current);
    }

    #[test]
    fn steps_round_trip_their_slugs() {
        for step in Step::ALL {
            assert_eq!(Step::from_slug(step.slug()), Some(step));
        }
        assert_eq!(Step::from_slug("nope"), None);
        assert!(Step::Setup.optional());
        assert!(!Step::Welcome.skippable() && !Step::Start.skippable());
    }

    /// Every string these pages can render, by source: provider names never
    /// appear. People only ever see "Cloud".
    #[test]
    fn cloud_computer_pages_never_name_a_provider() {
        const PAGES: [(&str, &str); 13] = [
            ("onboarding.rs", include_str!("onboarding.rs")),
            ("image_status.rs", include_str!("image_status.rs")),
            ("overview.rs", include_str!("overview.rs")),
            ("machine.rs", include_str!("machine.rs")),
            ("agents.rs", include_str!("agents.rs")),
            ("presentation.rs", include_str!("presentation.rs")),
            ("github.rs", include_str!("github.rs")),
            ("form.rs", include_str!("form.rs")),
            ("plan.rs", include_str!("plan.rs")),
            ("environment.rs", include_str!("environment.rs")),
            ("environment/view.rs", include_str!("environment/view.rs")),
            ("secrets/sheet.rs", include_str!("secrets/sheet.rs")),
            ("kit.rs", include_str!("../cloud_settings/kit.rs")),
        ];
        const PROVIDERS: [&str; 5] = ["boxd", "boat", "prized", "e2b", "daytona"];
        for (file, source) in PAGES {
            // Tests may name providers in fixtures; shipped copy may not.
            let shipped = source.split("#[cfg(test)]").next().unwrap_or(source);
            for literal in string_literals(shipped) {
                let lower = literal.to_lowercase();
                for name in PROVIDERS {
                    let named = lower.match_indices(name).any(|(at, _)| {
                        let before = lower[..at].chars().next_back();
                        let after = lower[at + name.len()..].chars().next();
                        !before.is_some_and(char::is_alphanumeric)
                            && !after.is_some_and(char::is_alphanumeric)
                    });
                    assert!(!named, "{file} shows provider name {name:?} in {literal:?}");
                }
            }
        }
    }

    /// The contents of each `"..."` literal in Rust source. Line comments
    /// and the `'"'` char literal are skipped.
    fn string_literals(source: &str) -> Vec<String> {
        let mut out = vec![];
        let mut chars = source.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\'' => {
                    let mut ahead = chars.clone();
                    if ahead.next() == Some('"') && ahead.next() == Some('\'') {
                        chars.next();
                        chars.next();
                    }
                }
                '/' if chars.peek() == Some(&'/') => {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                '"' => {
                    let mut literal = String::new();
                    while let Some(c) = chars.next() {
                        match c {
                            '\\' => {
                                if let Some(next) = chars.next() {
                                    literal.push(next);
                                }
                            }
                            '"' => break,
                            c => literal.push(c),
                        }
                    }
                    out.push(literal);
                }
                _ => {}
            }
        }
        out
    }

    #[test]
    fn the_literal_scanner_finds_strings() {
        let found = string_literals("let a = \"Boxd\"; // \"boat\"\nlet b = '\"'; f(\"x \\\" y\")");
        assert_eq!(found, vec!["Boxd".to_owned(), "x \" y".to_owned()]);
    }
}
