//! Cloud computer overview: the computer and plan at a glance, then one row
//! per part of setup, each linking to the page that owns it. Status always
//! comes from loaded account state.
use super::plan::DEFAULT_MACHINE;
use super::*;
use crate::cloud_settings::kit::{self, Tone};

/// One Overview row: what it is, where it stands, and the page that owns it.
struct SetupRow {
    id: &'static str,
    glyph: &'static str,
    title: &'static str,
    detail: String,
    status: Div,
    page: CloudPage,
}

impl RepositoryPanel {
    fn overview_row(
        &self,
        first: bool,
        row_data: SetupRow,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let page = row_data.page;
        div()
            .id(row_data.id)
            .role(Role::Button)
            .aria_label(row_data.title)
            .tab_index(0)
            .cursor_pointer()
            .hover(|v| v.bg(s::frame_bg().opacity(0.4)))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(OpenPage(page))))
            .child(kit::row_item(
                first,
                Some(kit::icon_tile(row_data.glyph, 28.).into_any_element()),
                row_data.title,
                row_data.detail,
                Some(
                    row()
                        .items_center()
                        .gap(px(10.))
                        .child(row_data.status)
                        .child(
                            icon("alt-arrow-right")
                                .size(px(12.))
                                .text_color(palette::rgb(MUTED)),
                        )
                        .into_any_element(),
                ),
            ))
    }

    fn setup_rows(&self) -> Vec<SetupRow> {
        let github = if self.github_connected() {
            let login = self
                .github
                .as_ref()
                .and_then(|g| g.user.as_ref())
                .map(|u| format!("Connected as {}", u.login))
                .unwrap_or_else(|| "Connected".into());
            let installs = self
                .github
                .as_ref()
                .map(|g| g.installations.iter().filter(|i| !i.suspended).count())
                .unwrap_or(0);
            (
                format!(
                    "{login} · {installs} installation{}",
                    if installs == 1 { "" } else { "s" }
                ),
                kit::pill("Connected", Tone::Ready),
            )
        } else {
            (String::new(), kit::pill("Needed", Tone::Attention))
        };
        let repositories = match self.projects.len() {
            0 => (String::new(), kit::pill("Needed", Tone::Attention)),
            n => (
                self.projects
                    .iter()
                    .take(3)
                    .map(|p| p.display_name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
                    + &if n > 3 {
                        format!(" and {} more", n - 3)
                    } else {
                        String::new()
                    },
                kit::pill(format!("{n} added"), Tone::Ready),
            ),
        };
        let agents = self.agents.connected_summary();
        let agents = if agents.is_empty() {
            (String::new(), kit::pill("Needed", Tone::Attention))
        } else {
            (agents.join(", "), kit::pill("Connected", Tone::Ready))
        };
        let machine = match self.machine.image_state() {
            Some(state) => {
                let (label, detail, _) = machine::image_copy(state);
                (
                    // A ready or standard machine needs no sentence.
                    if matches!(state, "ready" | "none") {
                        String::new()
                    } else {
                        detail.to_owned()
                    },
                    kit::pill(label, machine::image_tone(state)),
                )
            }
            None => (String::new(), kit::pill("Optional", Tone::Neutral)),
        };
        vec![
            SetupRow {
                id: "overview-github",
                glyph: "dither-plug",
                title: "GitHub",
                detail: github.0,
                status: github.1,
                page: CloudPage::Repositories,
            },
            SetupRow {
                id: "overview-repositories",
                glyph: "dither-branch",
                title: "Repositories",
                detail: repositories.0,
                status: repositories.1,
                page: CloudPage::Repositories,
            },
            SetupRow {
                id: "overview-agents",
                glyph: "dither-key",
                title: "Agent accounts",
                detail: agents.0,
                status: agents.1,
                page: CloudPage::Agents,
            },
            SetupRow {
                id: "overview-machine",
                glyph: "dither-monitor",
                title: "Machine",
                detail: machine.0,
                status: machine.1,
                page: CloudPage::Machine,
            },
            SetupRow {
                id: "overview-environments",
                glyph: "dither-sliders",
                title: "Environments",
                detail: String::new(),
                status: kit::pill("Optional", Tone::Neutral),
                page: CloudPage::Environments,
            },
        ]
    }

    /// The computer, its readiness and the plan's minutes.
    fn overview_hero(&self, ready: bool, cx: &mut Context<Self>) -> Div {
        let action = if ready {
            cs::button(
                "cloud-guide-open",
                "Review setup",
                false,
                !self.can_manage(),
                false,
            )
            .on_click(cx.listener(|this, _, _, cx| this.onboarding_reopen(cx)))
        } else {
            kit::primary_button("cloud-guide-open", "Continue setup", self.can_manage())
                .when(self.can_manage(), |b| {
                    b.on_click(cx.listener(|this, _, _, cx| this.onboarding_reopen(cx)))
                })
        };
        let mut card = column().child(
            row()
                .items_center()
                .gap(px(12.))
                .p(px(16.))
                .child(kit::icon_tile("dither-cloud", 36.))
                .child(
                    row()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(15.))
                                .line_height(px(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(palette::rgb(TEXT))
                                .child("Your cloud computer"),
                        )
                        .child(kit::info_tip(
                            "How a cloud chat starts",
                            "Every chat gets a fresh computer from your machine image, checks out only its repository and starts your agent on your own account. It sleeps when quiet and keeps working after you close Zuse.",
                        ))
                        .child(if ready {
                            kit::pill("Ready", Tone::Ready)
                        } else {
                            kit::pill("Setup isn't finished", Tone::Attention)
                        }),
                )
                .child(kit::spec_chip(DEFAULT_MACHINE))
                .child(action),
        );
        if let Some(plan) = self.plan_glance() {
            card = card.child(div().px(px(16.)).pb(px(16.)).child(kit::quantity(
                format!("{} · machine minutes", plan.label()),
                plan.minutes_line(),
                plan.fraction_left(),
                "",
            )));
        }
        kit::container().child(card)
    }

    pub(super) fn render_overview(&self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        let ready = self.onboarding_progress().ready();
        let mut rows = column();
        for (i, data) in self.setup_rows().into_iter().enumerate() {
            rows = rows.child(self.overview_row(i == 0, data, cx));
        }
        let setup = kit::section(
            "Setup",
            "Everything a cloud chat starts with. Open a row to change it.",
            None,
            rows,
        );
        let mut root = column()
            .gap(px(kit::SECTION_GAP))
            .child(self.overview_hero(ready, cx))
            .child(setup);
        if let Some(error) = &self.error {
            root = root.child(cs::note(error, true));
        }
        root
    }

    /// A compact setup reminder above the stacked browser panel.
    pub(super) fn setup_entry(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.loaded || self.owner.is_none() || !self.can_manage() {
            return None;
        }
        let complete = self.onboarding_progress().ready();
        Some(kit::section(
            "Setup",
            "",
            Some(
                cs::button(
                    "cloud-guide-open",
                    if complete {
                        "Review setup"
                    } else {
                        "Continue setup"
                    },
                    !complete,
                    false,
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.onboarding_reopen(cx)))
                .into_any_element(),
            ),
            kit::row_item(
                true,
                None,
                if complete {
                    "Cloud is ready"
                } else {
                    "Setup isn't finished"
                },
                if complete {
                    "GitHub, repositories and a coding agent are connected."
                } else {
                    "Connect a repository and an agent account before starting a cloud chat."
                },
                Some(if complete {
                    kit::pill("Ready", Tone::Ready).into_any_element()
                } else {
                    kit::pill("Needed", Tone::Attention).into_any_element()
                }),
            ),
        ))
    }
}
