//! The repository's `CHANGELOG.md`, bundled into the binary and parsed for the
//! in-app "What's new" view. Release headings look like
//! `## 0.26.1 - 2026-11-15` (a bracketed `## [0.26.1]` also parses);
//! `## Unreleased` is skipped.

use crate::{ReleaseNotesSection, parse_release_notes};

/// The changelog this binary was built with.
pub const BUNDLED: &str = include_str!("../../../CHANGELOG.md");

/// One released version from the changelog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// `YYYY-MM-DD`, when the heading carries one.
    pub date: Option<String>,
    pub sections: Vec<ReleaseNotesSection>,
}

/// Which kind of change a changelog section lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Changed,
    Fixed,
    Other,
}

impl ChangeKind {
    pub fn of_section(title: &str) -> Self {
        let title = title.trim().to_ascii_lowercase();
        if title.starts_with("add") || title.starts_with("new") {
            Self::Added
        } else if title.starts_with("change") || title.starts_with("improve") {
            Self::Changed
        } else if title.starts_with("fix") {
            Self::Fixed
        } else {
            Self::Other
        }
    }
}

/// A changelog bullet split into a short title and the rest of its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Highlight {
    pub kind: ChangeKind,
    pub title: String,
    pub detail: Option<String>,
}

/// Changes of each kind in a release.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChangeCounts {
    pub added: usize,
    pub changed: usize,
    pub fixed: usize,
}

impl Release {
    /// Every bullet, new features first, then improvements, fixes and the rest.
    pub fn highlights(&self) -> Vec<Highlight> {
        let mut highlights: Vec<Highlight> = self
            .sections
            .iter()
            .flat_map(|section| {
                let kind = ChangeKind::of_section(&section.title);
                section
                    .items
                    .iter()
                    .map(move |item| split_highlight(kind, item))
            })
            .collect();
        highlights.sort_by_key(|h| match h.kind {
            ChangeKind::Added => 0,
            ChangeKind::Changed => 1,
            ChangeKind::Fixed => 2,
            ChangeKind::Other => 3,
        });
        highlights
    }

    pub fn counts(&self) -> ChangeCounts {
        let mut counts = ChangeCounts::default();
        for section in &self.sections {
            let n = section.items.len();
            match ChangeKind::of_section(&section.title) {
                ChangeKind::Added => counts.added += n,
                ChangeKind::Changed => counts.changed += n,
                ChangeKind::Fixed => counts.fixed += n,
                ChangeKind::Other => {}
            }
        }
        counts
    }
}

/// Released versions, newest first, in changelog order. Repeated section
/// headings within one release are merged.
pub fn parse_changelog(source: &str) -> Vec<Release> {
    let mut releases = Vec::new();
    let mut current: Option<(String, Option<String>, String)> = None;
    let finish = |entry: Option<(String, Option<String>, String)>, out: &mut Vec<Release>| {
        if let Some((version, date, body)) = entry {
            let sections = merge_sections(parse_release_notes(&body));
            if !sections.is_empty() && !version.eq_ignore_ascii_case("unreleased") {
                out.push(Release {
                    version,
                    date,
                    sections,
                });
            }
        }
    };
    for line in source.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            finish(current.take(), &mut releases);
            let (version, date) = parse_heading(heading);
            current = Some((version, date, String::new()));
        } else if let Some((_, _, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    finish(current.take(), &mut releases);
    releases
}

/// The bundled changelog's releases.
pub fn bundled() -> Vec<Release> {
    parse_changelog(BUNDLED)
}

fn parse_heading(heading: &str) -> (String, Option<String>) {
    let heading = heading.trim();
    let (version, rest) = match heading.split_once(char::is_whitespace) {
        Some((version, rest)) => (version, rest.trim()),
        None => (heading, ""),
    };
    let version = version.trim_matches(|c| c == '[' || c == ']').to_owned();
    let date = rest
        .trim_start_matches(['-', '–', '—'])
        .trim()
        .get(..10)
        .filter(|date| {
            date.len() == 10
                && date.chars().enumerate().all(|(i, c)| {
                    if i == 4 || i == 7 {
                        c == '-'
                    } else {
                        c.is_ascii_digit()
                    }
                })
        })
        .map(str::to_owned);
    (version, date)
}

fn merge_sections(sections: Vec<ReleaseNotesSection>) -> Vec<ReleaseNotesSection> {
    let mut merged: Vec<ReleaseNotesSection> = Vec::new();
    for section in sections {
        match merged.iter_mut().find(|s| s.title == section.title) {
            Some(existing) => existing.items.extend(section.items),
            None => merged.push(section),
        }
    }
    merged
}

/// "Do X in Y. It also does Z." → title "Do X in Y", detail "It also does Z.".
/// A bullet without a sentence break stays whole as the title.
pub fn split_highlight(kind: ChangeKind, item: &str) -> Highlight {
    let item = item.trim();
    let split = item.char_indices().find_map(|(i, c)| {
        if c != '.' {
            return None;
        }
        let after = &item[i + 1..];
        let next = after.strip_prefix(' ')?.chars().next()?;
        // Only break before a capitalised sentence, so "e.g. foo" and
        // version numbers stay intact.
        next.is_uppercase().then_some(i)
    });
    match split {
        Some(i) => Highlight {
            kind,
            title: item[..i].trim().to_owned(),
            detail: Some(item[i + 1..].trim().to_owned()),
        },
        None => Highlight {
            kind,
            title: item.trim_end_matches('.').to_owned(),
            detail: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Changelog\n\n## Unreleased\n\n### Added\n\n- Not shipped yet.\n\n\
        ## 0.26.1 - 2026-11-15\n\n### Fixed\n\n- Correct sizing.\n\n### Added\n\n\
        - Choose **Stable** or Preview updates. Your selection is saved.\n- Saved `zoom` controls.\n\n\
        ### Fixed\n\n- Avoid idle redraw loops.\n\n\
        ## [0.26.0-preview.5] - 2026-11-11\n\n### Changed\n\n- Refined composer.\n\n\
        ## 0.25.0\n\n### Notes\n\n";

    #[test]
    fn parses_released_versions_newest_first() {
        let releases = parse_changelog(SAMPLE);
        let versions: Vec<_> = releases.iter().map(|r| r.version.as_str()).collect();
        assert_eq!(versions, ["0.26.1", "0.26.0-preview.5"]);
        assert_eq!(releases[0].date.as_deref(), Some("2026-11-15"));
        assert_eq!(releases[1].date.as_deref(), Some("2026-11-11"));
    }

    #[test]
    fn merges_repeated_sections_and_counts_kinds() {
        let release = &parse_changelog(SAMPLE)[0];
        let fixed = release
            .sections
            .iter()
            .find(|s| s.title == "Fixed")
            .unwrap();
        assert_eq!(fixed.items.len(), 2);
        assert_eq!(
            release.counts(),
            ChangeCounts {
                added: 2,
                changed: 0,
                fixed: 2
            }
        );
    }

    #[test]
    fn highlights_put_new_features_first_and_split_sentences() {
        let highlights = parse_changelog(SAMPLE)[0].highlights();
        assert_eq!(highlights[0].kind, ChangeKind::Added);
        assert_eq!(highlights[0].title, "Choose Stable or Preview updates");
        assert_eq!(
            highlights[0].detail.as_deref(),
            Some("Your selection is saved.")
        );
        assert_eq!(highlights[1].title, "Saved zoom controls");
        assert_eq!(highlights[1].detail, None);
        assert_eq!(highlights.last().unwrap().kind, ChangeKind::Fixed);
    }

    #[test]
    fn keeps_abbreviations_and_versions_in_titles() {
        let h = split_highlight(
            ChangeKind::Fixed,
            "Updating from 0.26.1 works, e.g. on KDE.",
        );
        assert_eq!(h.title, "Updating from 0.26.1 works, e.g. on KDE");
        assert_eq!(h.detail, None);
    }

    #[test]
    fn bundled_changelog_parses() {
        let releases = bundled();
        assert!(!releases.is_empty());
        assert!(releases.iter().all(|r| r.version != "Unreleased"));
    }
}
