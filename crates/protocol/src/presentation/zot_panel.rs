//! Zot panel tabs shared by the native and browser clients.

/// Pages of a Zot's side panel. Only the active page shows its label; the
/// rest collapse to icons and expand when selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ZotTab {
    #[default]
    Overview,
    Issues,
    Threads,
    Drive,
    History,
}

impl ZotTab {
    pub const ALL: [ZotTab; 5] = [
        Self::Overview,
        Self::Issues,
        Self::Threads,
        Self::Drive,
        Self::History,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Issues => "Issues",
            Self::Threads => "Threads",
            Self::Drive => "Drive",
            Self::History => "History",
        }
    }

    /// Icon name shared by both clients' asset sets.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Overview => "list",
            Self::Issues => "clock-circle",
            Self::Threads => "chat-round-line",
            Self::Drive => "folder-open",
            Self::History => "history",
        }
    }

    /// Width the expanded label grows to, from a per-character estimate at
    /// the 12px tab type size. Generous so the label never clips.
    pub fn label_width(self) -> f32 {
        self.label().chars().count() as f32 * 7.5 + 4.0
    }
}

/// Selection with the previous page kept so both the expanding and the
/// collapsing tab animate; `epoch` restarts the animation on every change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ZotTabs {
    pub active: ZotTab,
    pub previous: Option<ZotTab>,
    pub epoch: u64,
}

impl ZotTabs {
    pub fn select(&mut self, tab: ZotTab) -> bool {
        if self.active == tab {
            return false;
        }
        self.previous = Some(self.active);
        self.active = tab;
        self.epoch += 1;
        true
    }

    /// Label reveal (0 hidden, 1 shown) for `tab` at animation progress `t`.
    pub fn reveal(&self, tab: ZotTab, t: f32) -> f32 {
        if tab == self.active {
            if self.epoch == 0 { 1.0 } else { t }
        } else if Some(tab) == self.previous {
            1.0 - t
        } else {
            0.0
        }
    }
}

/// Section pages that always sit at the head of the native panel's strip.
/// History is not one: it opens from the strip's overflow menu.
pub const ZOT_SECTIONS: [ZotTab; 4] = [
    ZotTab::Overview,
    ZotTab::Issues,
    ZotTab::Threads,
    ZotTab::Drive,
];

/// One page of the native Zapper panel: a section, or a closable tab opened
/// from one (an issue, a Drive document, the version history).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ZotPage {
    Section(ZotTab),
    /// An issue, by id.
    Issue(String),
    /// A Drive document, by path.
    File(String),
    /// A pull request linked from an issue, by URL.
    PullRequest(String),
    History,
}

impl Default for ZotPage {
    fn default() -> Self {
        Self::Section(ZotTab::Overview)
    }
}

impl ZotPage {
    pub fn closable(&self) -> bool {
        !matches!(self, Self::Section(_))
    }
}

/// The native panel's tab strip: fixed sections followed by closable tabs,
/// like an editor. Closing a tab returns to the page shown before it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ZotPanelTabs {
    active: ZotPage,
    /// The section last shown. It keeps its label while a closable tab is
    /// active, so the section group never changes width under the pointer.
    section: ZotTab,
    /// Closable tabs, in strip order.
    open: Vec<ZotPage>,
    /// Pages shown before the active one, most recent last.
    trail: Vec<ZotPage>,
}

impl ZotPanelTabs {
    pub fn active(&self) -> &ZotPage {
        &self.active
    }

    /// The section that shows its label: the active one, or the one a
    /// closable tab was opened over.
    pub fn section(&self) -> ZotTab {
        self.section
    }

    pub fn open_tabs(&self) -> &[ZotPage] {
        &self.open
    }

    pub fn is_active(&self, page: &ZotPage) -> bool {
        &self.active == page
    }

    /// The issue the active tab shows, if any.
    pub fn active_issue(&self) -> Option<&str> {
        match &self.active {
            ZotPage::Issue(id) => Some(id),
            _ => None,
        }
    }

    /// Show `page`, adding it to the strip when it is closable and not open
    /// yet. Returns whether anything changed.
    pub fn show(&mut self, page: ZotPage) -> bool {
        if self.active == page {
            return false;
        }
        match &page {
            ZotPage::Section(tab) => self.section = *tab,
            _ if !self.open.contains(&page) => self.open.push(page.clone()),
            _ => {}
        }
        let previous = std::mem::replace(&mut self.active, page);
        self.trail.retain(|p| p != &previous && p != &self.active);
        self.trail.push(previous);
        true
    }

    pub fn select(&mut self, tab: ZotTab) -> bool {
        self.show(ZotPage::Section(tab))
    }

    /// Remove a closable tab. Closing the active one returns to the page
    /// shown before it that is still in the strip.
    pub fn close(&mut self, page: &ZotPage) -> bool {
        let Some(index) = self.open.iter().position(|p| p == page) else {
            return false;
        };
        self.open.remove(index);
        self.trail.retain(|p| p != page);
        if &self.active == page {
            self.active = self.trail.pop().unwrap_or(ZotPage::Section(self.section));
            if let ZotPage::Section(tab) = self.active {
                self.section = tab;
            }
        }
        true
    }

    /// A section left the strip (Drive with no documents): show Overview in
    /// its place without disturbing an open tab.
    pub fn hide_section(&mut self, gone: ZotTab) -> bool {
        let hidden = ZotPage::Section(gone);
        self.trail.retain(|p| p != &hidden);
        if self.active == hidden {
            self.active = ZotPage::Section(ZotTab::Overview);
        }
        let changed = self.section == gone;
        if changed {
            self.section = ZotTab::Overview;
        }
        changed
    }

    /// Back to Overview with no tabs, for a different Zapper.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Workers of a Zot belong under it, not in project lists.
pub fn zot_owns_chat(zots: &[crate::zots::Zot], chat_id: &str) -> bool {
    zots.iter().any(|z| {
        z.coordinator_chat_id == chat_id
            || z.previous_coordinator_chats.iter().any(|c| c == chat_id)
            || z.workers.iter().any(|w| w.chat_id == chat_id)
    })
}

/// Whether `chat_id` has the shape the engine gives Zapper conversations:
/// `zot-<zapper>-<n>` for coordinators and `zot-worker-<command>` for the
/// threads they start. Lets clients keep a brand-new thread out of project
/// lists before the Zapper's own list catches up.
pub fn zapper_chat_id(chat_id: &str) -> bool {
    chat_id.starts_with("zot-")
}

/// One thing in the Overview's "Needs you" card: something that really waits
/// on the user. Coordinator events (thread reports, linked pull requests,
/// interruptions) are the Zapper's inbox and never appear here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeedsYouItem {
    /// An open decision with no answer yet.
    Decision { id: String, question: String },
    /// An issue the Zapper handed to the user.
    Issue {
        id: String,
        code: String,
        title: String,
        /// The latest question the Zapper asked on the issue, if any.
        question: Option<String>,
    },
}

/// What Open on a "Needs you" item shows. Never a chat: the items have no
/// chat of their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeedsYouTarget {
    /// The issue's tab.
    IssueTab(String),
    /// The answer form of the decision.
    DecisionForm(String),
}

impl NeedsYouItem {
    pub fn target(&self) -> NeedsYouTarget {
        match self {
            Self::Decision { id, .. } => NeedsYouTarget::DecisionForm(id.clone()),
            Self::Issue { id, .. } => NeedsYouTarget::IssueTab(id.clone()),
        }
    }
    /// The row's title.
    pub fn title(&self) -> String {
        match self {
            Self::Decision { question, .. } => question.clone(),
            Self::Issue {
                code,
                title,
                question,
                ..
            } => format!("{code} · {}", question.as_deref().unwrap_or(title)),
        }
    }
}

/// What waits on the user in a Zapper's `issues` and `decisions`: open issues owned by the user (with the
/// latest `question` activity) first, then decisions with no answer. Both
/// leave the list once answered or handed back, because they are read from
/// the current owner and answer.
pub fn needs_you_items(
    issues: &[crate::zots::ZotIssue],
    decisions: &[crate::zots::ZotDecision],
) -> Vec<NeedsYouItem> {
    use crate::zots::{ZotIssueOwner, ZotIssueStatus};
    let issues = issues
        .iter()
        .filter(|i| {
            i.details.status == ZotIssueStatus::NeedsYou
                && i.details.current_owner() == Some(ZotIssueOwner::User)
        })
        .map(|i| NeedsYouItem::Issue {
            id: i.id.clone(),
            code: i.code(),
            title: i.details.title.clone(),
            question: i
                .activity
                .iter()
                .rev()
                .find(|a| a.kind == "question" && !a.text.trim().is_empty())
                .map(|a| a.text.trim().to_owned()),
        });
    let decisions =
        decisions
            .iter()
            .filter(|d| d.answer.is_none())
            .map(|d| NeedsYouItem::Decision {
                id: d.id.clone(),
                question: d.question.clone(),
            });
    issues.chain(decisions).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selecting_animates_both_tabs() {
        let mut tabs = ZotTabs::default();
        assert_eq!(tabs.reveal(ZotTab::Overview, 0.0), 1.0);
        assert!(!tabs.select(ZotTab::Overview));
        assert!(tabs.select(ZotTab::Drive));
        assert_eq!(tabs.reveal(ZotTab::Drive, 0.25), 0.25);
        assert_eq!(tabs.reveal(ZotTab::Overview, 0.25), 0.75);
        assert_eq!(tabs.reveal(ZotTab::Threads, 0.5), 0.0);
        assert_eq!(tabs.epoch, 1);
    }

    fn issue(id: &str) -> ZotPage {
        ZotPage::Issue(id.into())
    }

    #[test]
    fn issues_and_files_open_as_tabs_after_the_sections() {
        let mut tabs = ZotPanelTabs::default();
        assert_eq!(tabs.active(), &ZotPage::Section(ZotTab::Overview));
        assert!(tabs.select(ZotTab::Issues));
        assert!(tabs.show(issue("a")));
        assert!(tabs.show(ZotPage::File("NOTES.md".into())));
        assert_eq!(
            tabs.open_tabs(),
            [issue("a"), ZotPage::File("NOTES.md".into())]
        );
        // Re-opening focuses the existing tab instead of adding another.
        assert!(tabs.show(issue("a")));
        assert_eq!(tabs.open_tabs().len(), 2);
        assert_eq!(tabs.active_issue(), Some("a"));
        // The section it was opened from keeps its label.
        assert_eq!(tabs.section(), ZotTab::Issues);
        assert!(!tabs.show(issue("a")));
    }

    #[test]
    fn closing_returns_to_the_tab_shown_before() {
        let mut tabs = ZotPanelTabs::default();
        tabs.select(ZotTab::Drive);
        tabs.show(ZotPage::File("NOTES.md".into()));
        tabs.select(ZotTab::Issues);
        tabs.show(issue("a"));
        tabs.show(issue("b"));
        assert!(tabs.close(&issue("b")));
        assert_eq!(tabs.active(), &issue("a"));
        assert!(tabs.close(&issue("a")));
        assert_eq!(tabs.active(), &ZotPage::Section(ZotTab::Issues));
        // Closing a background tab leaves the active page alone.
        assert!(tabs.close(&ZotPage::File("NOTES.md".into())));
        assert_eq!(tabs.active(), &ZotPage::Section(ZotTab::Issues));
        assert!(tabs.open_tabs().is_empty());
        // Sections and unknown pages cannot be closed.
        assert!(!tabs.close(&ZotPage::Section(ZotTab::Issues)));
        assert!(!tabs.close(&issue("z")));
    }

    #[test]
    fn closing_skips_pages_already_closed() {
        let mut tabs = ZotPanelTabs::default();
        tabs.show(issue("a"));
        tabs.show(ZotPage::History);
        tabs.show(issue("b"));
        tabs.close(&ZotPage::History);
        tabs.close(&issue("b"));
        assert_eq!(tabs.active(), &issue("a"));
        tabs.close(&issue("a"));
        assert_eq!(tabs.active(), &ZotPage::Section(ZotTab::Overview));
    }

    #[test]
    fn reset_returns_to_a_bare_overview() {
        let mut tabs = ZotPanelTabs::default();
        tabs.select(ZotTab::Threads);
        tabs.show(issue("a"));
        tabs.reset();
        assert_eq!(tabs, ZotPanelTabs::default());
    }

    #[test]
    fn a_hidden_section_falls_back_to_overview() {
        let mut tabs = ZotPanelTabs::default();
        tabs.select(ZotTab::Drive);
        assert!(tabs.hide_section(ZotTab::Drive));
        assert_eq!(tabs.active(), &ZotPage::Section(ZotTab::Overview));
        tabs.select(ZotTab::Drive);
        tabs.show(issue("a"));
        assert!(tabs.hide_section(ZotTab::Drive));
        assert_eq!(tabs.active(), &issue("a"));
        assert_eq!(tabs.section(), ZotTab::Overview);
        // Closing the tab no longer returns to the hidden section.
        tabs.close(&issue("a"));
        assert_eq!(tabs.active(), &ZotPage::Section(ZotTab::Overview));
        assert!(!tabs.hide_section(ZotTab::Drive));
    }

    #[test]
    fn history_is_not_a_section() {
        assert!(!ZOT_SECTIONS.contains(&ZotTab::History));
        assert!(ZotPage::History.closable());
        assert!(!ZotPage::Section(ZotTab::Drive).closable());
    }

    fn raised(
        number: u32,
        status: crate::zots::ZotIssueStatus,
        owner: Option<crate::zots::ZotIssueOwner>,
        activity: &[(&str, &str)],
    ) -> crate::zots::ZotIssue {
        crate::zots::ZotIssue {
            id: format!("issue-{number}"),
            number,
            details: crate::zots::ZotIssueDetails {
                title: format!("Issue {number}"),
                status,
                owner,
                ..Default::default()
            },
            created_at: 0,
            updated_at: 0,
            threads: Vec::new(),
            pull_requests: Vec::new(),
            activity: activity
                .iter()
                .enumerate()
                .map(|(n, (kind, text))| crate::zots::ZotIssueActivity {
                    actor: None,
                    id: format!("a{n}"),
                    created_at: n as i64,
                    kind: (*kind).into(),
                    text: (*text).into(),
                    reply_to: None,
                })
                .collect(),
        }
    }
    fn decision(id: &str, answer: Option<&str>) -> crate::zots::ZotDecision {
        crate::zots::ZotDecision {
            id: id.into(),
            worker_chat_id: None,
            question: format!("Question {id}?"),
            answer: answer.map(str::to_owned),
        }
    }

    #[test]
    fn needs_you_lists_only_user_asks() {
        use crate::zots::{ZotIssueOwner as O, ZotIssueStatus as S};
        let issues = [
            raised(
                1,
                S::NeedsYou,
                Some(O::User),
                &[("comment", "fyi"), ("question", "Ship it?")],
            ),
            // Not the user's: the Zapper and its threads hold these.
            raised(2, S::InProgress, Some(O::Zapper), &[("question", "old")]),
            raised(
                3,
                S::InProgress,
                Some(O::Thread {
                    chat_id: "c".into(),
                }),
                &[],
            ),
            // Handed back or finished.
            raised(4, S::Todo, Some(O::Zapper), &[("question", "answered")]),
            raised(5, S::Done, Some(O::User), &[]),
        ];
        let decisions = [decision("d1", None), decision("d2", Some("yes"))];
        let items = needs_you_items(&issues, &decisions);
        assert_eq!(
            items,
            [
                NeedsYouItem::Issue {
                    id: "issue-1".into(),
                    code: "ZAP-1".into(),
                    title: "Issue 1".into(),
                    question: Some("Ship it?".into()),
                },
                NeedsYouItem::Decision {
                    id: "d1".into(),
                    question: "Question d1?".into(),
                },
            ]
        );
        assert_eq!(items[0].title(), "ZAP-1 · Ship it?");
    }

    #[test]
    fn needs_you_issue_without_a_question_shows_its_title() {
        use crate::zots::{ZotIssueOwner as O, ZotIssueStatus as S};
        let items = needs_you_items(&[raised(7, S::NeedsYou, Some(O::User), &[])], &[]);
        assert_eq!(items[0].title(), "ZAP-7 · Issue 7");
    }

    #[test]
    fn needs_you_items_leave_once_answered_or_handed_back() {
        use crate::zots::{ZotIssueOwner as O, ZotIssueStatus as S};
        let mut asked = raised(1, S::NeedsYou, Some(O::User), &[("question", "Ok?")]);
        let mut open = decision("d", None);
        assert_eq!(needs_you_items(&[asked.clone()], &[open.clone()]).len(), 2);
        open.answer = Some("yes".into());
        asked.details.owner = Some(O::Zapper);
        asked.details.status = S::InProgress;
        assert!(needs_you_items(&[asked], &[open]).is_empty());
    }

    #[test]
    fn open_routes_to_the_issue_tab_or_the_decision_form() {
        let item = NeedsYouItem::Issue {
            id: "i".into(),
            code: "ZAP-1".into(),
            title: "t".into(),
            question: None,
        };
        assert_eq!(item.target(), NeedsYouTarget::IssueTab("i".into()));
        let ask = NeedsYouItem::Decision {
            id: "d".into(),
            question: "q".into(),
        };
        assert_eq!(ask.target(), NeedsYouTarget::DecisionForm("d".into()));
    }
}
