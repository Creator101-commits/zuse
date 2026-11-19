//! Self-contained browser pages for sign-in, integration, and checkout
//! callbacks. Every page shares one look: a dithered backdrop behind a single
//! printed artifact (a ticket for account sign-in, a stamp for an integration,
//! a receipt for a payment) or a compact card when the browser still has a
//! choice to make.
//!
//! Pages carry no external resources. The only scripts are the inline
//! backdrop and form-feedback scripts, admitted by hash in
//! [`content_security_policy`]; serve every page with it.

use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

const BASE_CSS: &str = include_str!("../assets/base.css");
const TICKET_CSS: &str = include_str!("../assets/ticket.css");
const STAMP_CSS: &str = include_str!("../assets/stamp.css");
const RECEIPT_CSS: &str = include_str!("../assets/receipt.css");
const CARD_CSS: &str = include_str!("../assets/card.css");
const DITHER_JS: &str = include_str!("../assets/dither.js");
const SUBMIT_JS: &str = include_str!("../assets/submit.js");

const SUCCESS_TONE: &str = "#caff00";
const ERROR_TONE: &str = "#ff7a6b";
const INK: &str = "#181713";
const MESSAGE_MAX: usize = 220;
const NAME_MAX: usize = 40;
const VALUE_MAX: usize = 100;

/// Escape a value for HTML text or a quoted attribute.
pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Trim and truncate untrusted text on a character boundary.
fn truncate(value: &str, max: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(max - 1).collect();
    format!("{}…", cut.trim_end())
}

/// [`truncate`], then [`escape`].
fn clamp(value: &str, max: usize) -> String {
    escape(&truncate(value, max))
}

fn script_source(script: &str) -> String {
    let digest = Sha256::digest(script.as_bytes());
    format!(
        "'sha256-{}'",
        base64::engine::general_purpose::STANDARD.encode(digest)
    )
}

/// CSP for every page in this crate: inline styles, the two hashed inline
/// scripts, same-origin form posts, and nothing else.
pub fn content_security_policy() -> &'static str {
    static CSP: OnceLock<String> = OnceLock::new();
    CSP.get_or_init(|| {
        format!(
            "default-src 'none'; style-src 'unsafe-inline'; script-src {} {}; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
            script_source(DITHER_JS),
            script_source(SUBMIT_JS),
        )
    })
}

/// A complete `HTTP/1.1` response for loopback listeners that write raw
/// sockets instead of going through a web framework.
pub fn http_response(status: &str, html: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncache-control: no-store\r\n\
         referrer-policy: no-referrer\r\nx-content-type-options: nosniff\r\n\
         content-security-policy: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{html}",
        content_security_policy(),
        html.len()
    )
}

/// Whether the flow behind a page finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Failure,
}

impl Outcome {
    fn ok(self) -> bool {
        self == Outcome::Success
    }
    fn tone(self) -> &'static str {
        if self.ok() { SUCCESS_TONE } else { ERROR_TONE }
    }
}

struct Stamp {
    date: String,
    issued: String,
    reference: String,
}

fn stamp_now() -> Stamp {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let minute_of_day = secs.rem_euclid(86_400) / 60;
    let (hour, minute) = (minute_of_day / 60, minute_of_day % 60);
    let date = format!("{year}.{month:02}.{day:02}");
    Stamp {
        issued: format!("{date} {hour:02}:{minute:02}"),
        reference: format!("ZS-{year}{month:02}{day:02}-{hour:02}{minute:02}"),
        date,
    }
}

/// Days since 1970-01-01 to a proleptic Gregorian date (H. Hinnant).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn document(title: &str, styles: &str, body: &str, forms: bool) -> String {
    document_with_head(title, styles, body, forms, "")
}

/// [`document`] with extra `<head>` markup the caller already escaped.
fn document_with_head(title: &str, styles: &str, body: &str, forms: bool, head: &str) -> String {
    let submit = if forms {
        format!("<script>{SUBMIT_JS}</script>")
    } else {
        String::new()
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{} · Zuse</title>{head}<style>{BASE_CSS}{styles}</style></head>\
         <body>{body}<script>{DITHER_JS}</script>{submit}</body></html>",
        escape(title)
    )
}

fn piece(width: &str, artwork: &str, note: &str) -> String {
    format!(
        "<main class=\"stage\"><div class=\"piece\" style=\"--w:{width}\">{artwork}\
         <div class=\"footer\" role=\"status\"><span>{note}</span><strong>Return to Zuse</strong></div>\
         </div></main>"
    )
}

const DITHER_ART: &str = "<i></i><i></i><i></i><i></i>";

/// The Zuse account sign-in result, printed as an admission ticket.
/// `message` replaces the default line under the headline.
pub fn sign_in(outcome: Outcome, message: Option<&str>) -> String {
    let ok = outcome.ok();
    let Stamp {
        issued, reference, ..
    } = stamp_now();
    let copy = match message.filter(|m| !m.trim().is_empty()) {
        Some(message) => clamp(message, MESSAGE_MAX),
        None if ok => "Your Zuse account is ready. You can close this tab.".into(),
        None => "The sign-in was cancelled or rejected. Return to Zuse and try again.".into(),
    };
    let artwork = format!(
        "<div class=\"ticket-shell\"><article class=\"ticket\" style=\"--paper:{tone};--ink:{INK}\" aria-label=\"Zuse sign-in ticket\">\
         <div class=\"ticket-body halftone\">\
         <div class=\"ticket-top\"><span>Zuse · Account</span><span>{admitted}</span></div>\
         <div class=\"ticket-pattern dither-art\" aria-hidden=\"true\">{DITHER_ART}</div>\
         <div><h1>{headline}</h1><p>{copy}</p></div>\
         <dl class=\"ticket-facts\"><div><dt>Issued (UTC)</dt><dd>{issued}</dd></div>\
         <div><dt>Status</dt><dd>{status}</dd></div></dl>\
         </div>\
         <div class=\"ticket-stub\"><div class=\"ticket-stub-row\">\
         <div><span class=\"micro\">Admit</span><strong>{admit}</strong></div>\
         <div><span class=\"micro\">Reference</span><span class=\"reference\">{reference}</span></div>\
         </div><div class=\"barcode\" aria-hidden=\"true\"></div></div>\
         </article></div>",
        tone = outcome.tone(),
        admitted = if ok { "Admitted" } else { "Void" },
        headline = if ok {
            "You’re<br>signed in"
        } else {
            "Sign-in<br>didn’t finish"
        },
        status = if ok { "Signed in" } else { "Not signed in" },
        admit = if ok { "One" } else { "None" },
    );
    document(
        if ok { "Signed in" } else { "Sign-in failed" },
        TICKET_CSS,
        &piece(
            "18rem",
            &artwork,
            if ok {
                "Close this tab"
            } else {
                "Nothing changed"
            },
        ),
        false,
    )
}

/// An integration result (an agent subscription, a plugin, Slack), printed
/// as a postage stamp. `message` replaces the default line under the name.
pub fn integration(name: &str, outcome: Outcome, message: Option<&str>) -> String {
    let ok = outcome.ok();
    let Stamp {
        date, reference, ..
    } = stamp_now();
    let raw = match truncate(name, NAME_MAX) {
        n if n.is_empty() => "Plugin".to_string(),
        n => n,
    };
    let name = escape(&raw);
    let copy = match message.filter(|m| !m.trim().is_empty()) {
        Some(message) => clamp(message, MESSAGE_MAX),
        None if ok => format!("{name} is connected to Zuse. You can close this tab."),
        None => format!("{name} did not finish connecting. Return to Zuse and try again."),
    };
    let artwork = format!(
        "<div class=\"stamp-shell\"><article class=\"stamp\" style=\"--ink:{INK}\" aria-label=\"{name} integration stamp\">\
         <div class=\"stamp-face halftone\" style=\"--paper:{tone}\">\
         <div class=\"stamp-head\"><span class=\"micro\">Zuse · Integration</span><strong>{count}</strong></div>\
         <div><h1>{name}</h1><p>{copy}</p></div>\
         <div class=\"stamp-art dither-art\" aria-hidden=\"true\">{DITHER_ART}</div>\
         <span class=\"micro\">{reference}</span>\
         <div class=\"postmark\" aria-hidden=\"true\"><span>{mark}<br>{date}</span></div>\
         </div></article></div>",
        tone = outcome.tone(),
        count = if ok { "01" } else { "00" },
        mark = if ok { "Connected" } else { "Declined" },
    );
    let title = if ok {
        format!("{raw} connected")
    } else {
        format!("{raw} not connected")
    };
    document(
        &title,
        STAMP_CSS,
        &piece(
            "15rem",
            &artwork,
            if ok {
                "Close this tab"
            } else {
                "Nothing changed"
            },
        ),
        false,
    )
}

/// Where a payment stands when the browser returns from checkout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payment {
    Confirmed,
    Pending,
    Failed,
}

pub struct Amount<'a> {
    pub cents: i64,
    pub currency: &'a str,
}

/// A checkout return, printed as a receipt feeding out of a printer.
pub struct Receipt<'a> {
    pub payment: Payment,
    /// Short status shown on the printer screen and receipt tag.
    pub status: &'a str,
    /// The purchased line item.
    pub product: &'a str,
    pub amount: Option<Amount<'a>>,
    /// Human-quotable order reference.
    pub reference: Option<&'a str>,
    pub message: &'a str,
    /// One line under the printer.
    pub hint: &'a str,
}

fn format_amount(amount: &Amount<'_>) -> String {
    let currency: String = amount
        .currency
        .trim()
        .chars()
        .filter(char::is_ascii_alphabetic)
        .take(3)
        .collect::<String>()
        .to_ascii_uppercase();
    let sign = if amount.cents < 0 { "-" } else { "" };
    let cents = amount.cents.unsigned_abs();
    format!("{currency} {sign}{}.{:02}", cents / 100, cents % 100)
}

pub fn receipt(receipt: &Receipt<'_>) -> String {
    let Stamp { issued, .. } = stamp_now();
    let failed = receipt.payment == Payment::Failed;
    let status = clamp(receipt.status, NAME_MAX);
    let indicator = match receipt.payment {
        Payment::Pending => "<span class=\"spinner\" aria-hidden=\"true\"></span>",
        Payment::Failed => "<span class=\"dot failed\" aria-hidden=\"true\"></span>",
        Payment::Confirmed => "<span class=\"dot\" aria-hidden=\"true\"></span>",
    };
    let reference = receipt
        .reference
        .filter(|r| !r.trim().is_empty())
        .map_or_else(
            || "Zuse billing".to_string(),
            |r| format!("Order {}", clamp(r, 24)),
        );
    let product = clamp(receipt.product, VALUE_MAX);
    let lines = match &receipt.amount {
        Some(amount) => {
            let amount = escape(&format_amount(amount));
            format!(
                "<div class=\"line\"><span class=\"label\">{product}</span><span class=\"value\">{amount}</span></div>\
                 <div class=\"rule\"></div>\
                 <div class=\"line total\"><span class=\"label\">Total</span><span class=\"value\">{amount}</span></div>"
            )
        }
        None => format!("<div class=\"line\"><span class=\"label\">{product}</span></div>"),
    };
    let body = format!(
        "<main class=\"stage\"><div class=\"printer\">\
         <div class=\"printer-head\"><span class=\"micro\">Zuse · Billing</span><span class=\"micro\">{issued} UTC</span></div>\
         <div class=\"screen\"><div class=\"screen-row\">{indicator}<span role=\"status\">{status}</span></div></div>\
         <div class=\"slot\" aria-hidden=\"true\"></div></div>\
         <div class=\"output\"><article class=\"receipt\" aria-label=\"Zuse purchase receipt\">\
         <h1>Zuse</h1>\
         <div class=\"receipt-meta\"><span>{reference}</span><span>{issued}</span></div>\
         <div class=\"rule\"></div>{lines}\
         <span class=\"status{tag}\">{status}</span>\
         <p>{message}</p>\
         <div class=\"barcode\" aria-hidden=\"true\"></div>\
         </article></div>\
         <p class=\"hint\">{hint}</p></main>",
        tag = if failed { " failed" } else { "" },
        message = clamp(receipt.message, MESSAGE_MAX),
        hint = clamp(receipt.hint, MESSAGE_MAX),
    );
    document(
        if failed {
            "Checkout not completed"
        } else {
            "Checkout complete"
        },
        RECEIPT_CSS,
        &body,
        false,
    )
}

/// A browser action on a [`Card`].
pub enum Action<'a> {
    /// A navigation; `external` opens a new tab.
    Link {
        label: &'a str,
        href: &'a str,
        external: bool,
    },
    /// A same-origin `POST` with hidden fields. `pending` is announced while
    /// the request is in flight.
    Form {
        label: &'a str,
        action: &'a str,
        fields: Vec<(&'a str, String)>,
        pending: &'a str,
    },
}

/// A selectable account or installation in a [`Card`] list.
pub struct Row<'a> {
    pub name: &'a str,
    pub meta: Option<&'a str>,
    pub action: Action<'a>,
}

/// The tone of the dot next to a card's status badge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Success,
    Waiting,
    Failure,
}

/// A compact card for steps that still need the browser: choose an account,
/// confirm a connection, wait for approval, or a plain notice.
pub struct Card<'a> {
    /// Eyebrow after "Zuse /", such as "GitHub".
    pub integration: &'a str,
    pub status: &'a str,
    pub tone: Tone,
    pub title: &'a str,
    pub description: &'a str,
    pub facts: Vec<(&'a str, String)>,
    pub rows: Vec<Row<'a>>,
    /// Shown inside the list when `rows` is empty but a list was expected.
    pub empty: Option<&'a str>,
    pub actions: Vec<Action<'a>>,
    pub hint: Option<&'a str>,
}

impl<'a> Card<'a> {
    /// A plain notice: eyebrow, title, description, nothing to choose.
    pub fn notice(
        integration: &'a str,
        status: &'a str,
        tone: Tone,
        title: &'a str,
        description: &'a str,
    ) -> Self {
        Self {
            integration,
            status,
            tone,
            title,
            description,
            facts: Vec::new(),
            rows: Vec::new(),
            empty: None,
            actions: Vec::new(),
            hint: None,
        }
    }
}

const EXTERNAL: &str = "<svg viewBox=\"0 0 12 12\" fill=\"none\" aria-hidden=\"true\" focusable=\"false\"><path d=\"M3.5 8.5l5-5M4.5 3.5h4v4\" stroke=\"currentColor\" stroke-width=\"1.4\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/></svg>";

fn action_html(action: &Action<'_>, class: &str, aria: Option<&str>) -> String {
    let aria = aria
        .map(|a| format!(" aria-label=\"{}\"", escape(a)))
        .unwrap_or_default();
    match action {
        Action::Link {
            label,
            href,
            external,
        } => {
            let (target, icon) = if *external {
                (
                    " target=\"_blank\" rel=\"noopener noreferrer\"",
                    format!("{EXTERNAL}<span class=\"sr-only\"> (opens in a new tab)</span>"),
                )
            } else {
                ("", String::new())
            };
            format!(
                "<a class=\"{class}\" href=\"{}\"{target}{aria}>{}{icon}</a>",
                escape(href),
                escape(label)
            )
        }
        Action::Form {
            label,
            action,
            fields,
            pending,
        } => {
            let inputs: String = fields
                .iter()
                .map(|(name, value)| {
                    format!(
                        "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
                        escape(name),
                        escape(value)
                    )
                })
                .collect();
            format!(
                "<form method=\"post\" action=\"{}\" data-pending=\"{}\" data-pending-label=\"Working…\">{inputs}\
                 <button class=\"{class}\" type=\"submit\"{aria}>{}</button></form>",
                escape(action),
                escape(pending),
                escape(label)
            )
        }
    }
}

pub fn card(card: &Card<'_>) -> String {
    card_with_head(card, "")
}

/// A [`card`] that also asks the browser to open a `zuse://` link as soon as
/// it loads. A meta refresh needs no script, so the policy stays as strict.
/// Put the same link in the card's actions for browsers that want a click.
/// Anything other than a `zuse://` link renders the plain card.
pub fn card_opening_zuse(card: &Card<'_>, link: &str) -> String {
    if !link.starts_with("zuse://") {
        return self::card(card);
    }
    let refresh = format!(
        "<meta http-equiv=\"refresh\" content=\"0;url={}\">",
        escape(link)
    );
    card_with_head(card, &refresh)
}

fn card_with_head(card: &Card<'_>, head: &str) -> String {
    let forms = card
        .rows
        .iter()
        .map(|row| &row.action)
        .chain(&card.actions)
        .any(|action| matches!(action, Action::Form { .. }));
    let facts = if card.facts.is_empty() {
        String::new()
    } else {
        let items: String = card
            .facts
            .iter()
            .map(|(label, value)| {
                format!(
                    "<div><dt>{}</dt><dd title=\"{value}\">{value}</dd></div>",
                    escape(label),
                    value = clamp(value, VALUE_MAX)
                )
            })
            .collect();
        format!("<dl class=\"facts\">{items}</dl>")
    };
    let list = if !card.rows.is_empty() {
        let rows: String = card
            .rows
            .iter()
            .map(|row| {
                let initial = row
                    .name
                    .chars()
                    .next()
                    .map(|c| escape(&c.to_string()))
                    .unwrap_or_default();
                let meta = row
                    .meta
                    .map(|m| format!("<span class=\"meta\">{}</span>", clamp(m, VALUE_MAX)))
                    .unwrap_or_default();
                let label = match &row.action {
                    Action::Link { label, .. } | Action::Form { label, .. } => *label,
                };
                format!(
                    "<div class=\"row\"><span class=\"mark\" aria-hidden=\"true\">{initial}</span>\
                     <div><span class=\"name\">{}</span>{meta}</div>{}</div>",
                    clamp(row.name, VALUE_MAX),
                    action_html(
                        &row.action,
                        "button",
                        Some(&format!("{label} {}", row.name.trim()))
                    )
                )
            })
            .collect();
        format!("<div class=\"list\">{rows}</div>")
    } else if let Some(empty) = card.empty {
        format!(
            "<div class=\"list\"><p class=\"empty\">{}</p></div>",
            escape(empty)
        )
    } else {
        String::new()
    };
    let buttons = if card.actions.is_empty() {
        String::new()
    } else {
        let items: String = card
            .actions
            .iter()
            .enumerate()
            .map(|(index, action)| {
                let class = if index == 0 && card.rows.is_empty() {
                    "button"
                } else {
                    "button secondary"
                };
                action_html(action, class, None)
            })
            .collect();
        format!("<div class=\"buttons\">{items}</div>")
    };
    let tone = match card.tone {
        Tone::Success => SUCCESS_TONE,
        Tone::Waiting => "#f5b54a",
        Tone::Failure => ERROR_TONE,
    };
    let status_line = if forms {
        "<p data-submit-status role=\"status\" hidden></p>"
    } else {
        ""
    };
    let hint = card
        .hint
        .map(|h| format!("<p class=\"hint\">{}</p>", escape(h)))
        .unwrap_or_default();
    let body = format!(
        "<main class=\"stage\"><section class=\"card\">\
         <p class=\"eyebrow\"><strong>Zuse</strong><span aria-hidden=\"true\">/</span>{integration}\
         <span class=\"badge\" style=\"--tone:{tone}\">{status}</span></p>\
         <h1>{title}</h1><p class=\"description\">{description}</p>\
         {facts}{list}{buttons}{status_line}{hint}</section></main>",
        integration = escape(card.integration),
        status = escape(card.status),
        title = escape(card.title),
        description = escape(card.description),
    );
    document_with_head(card.title, CARD_CSS, &body, forms, head)
}

/// A plain Zuse notice for stray or invalid requests to a callback listener.
pub fn message(title: &str, description: &str) -> String {
    card(&Card::notice(
        "Callback",
        "Closed",
        Tone::Failure,
        title,
        description,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_follow_the_gregorian_calendar() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn untrusted_text_is_escaped_and_clamped() {
        let page = sign_in(Outcome::Failure, Some("<script>alert(1)</script>"));
        assert!(!page.contains("<script>alert"));
        assert!(page.contains("&lt;script&gt;"));
        let long = "x".repeat(500);
        let page = integration("ChatGPT", Outcome::Failure, Some(&long));
        assert!(!page.contains(&long));
        assert!(page.contains('…'));
    }

    #[test]
    fn policy_admits_exactly_the_inline_scripts() {
        let csp = content_security_policy();
        assert!(csp.contains(&script_source(DITHER_JS)));
        assert!(csp.contains(&script_source(SUBMIT_JS)));
        assert!(csp.starts_with("default-src 'none'"));
        // The emitted script bodies must match the hashed text byte for byte.
        let page = card(&Card {
            rows: vec![Row {
                name: "team",
                meta: None,
                action: Action::Form {
                    label: "Use",
                    action: "/select",
                    fields: vec![("state", "s".into())],
                    pending: "Connecting…",
                },
            }],
            ..Card::notice("GitHub", "Choose", Tone::Waiting, "Choose", "d")
        });
        assert!(page.contains(&format!("<script>{DITHER_JS}</script>")));
        assert!(page.contains(&format!("<script>{SUBMIT_JS}</script>")));
        assert!(page.contains("aria-label=\"Use team\""));
        assert!(!sign_in(Outcome::Success, None).contains(SUBMIT_JS));
    }

    #[test]
    fn opening_zuse_adds_only_a_refresh_and_keeps_the_policy() {
        let before = content_security_policy().to_owned();
        let notice = || Card::notice("GitHub", "Confirm", Tone::Waiting, "Confirm", "d");
        let page = card_opening_zuse(&notice(), "zuse://github/confirm?operation=a&code=B");
        assert!(page.contains(
            "<meta http-equiv=\"refresh\" content=\"0;url=zuse://github/confirm?operation=a&amp;code=B\">"
        ));
        // No new script: the hashed backdrop script is still the only one.
        assert_eq!(page.matches("<script>").count(), 1);
        assert_eq!(content_security_policy(), before);
        let scripts = before
            .split("; ")
            .find(|directive| directive.starts_with("script-src"))
            .unwrap();
        assert!(!scripts.contains("unsafe"));
        // Only Zuse links are opened automatically.
        let other = card_opening_zuse(&notice(), "https://example.com");
        assert!(!other.contains("http-equiv=\"refresh\""));
        assert_eq!(other, card(&notice()));
    }

    #[test]
    fn receipts_format_amounts() {
        let page = receipt(&Receipt {
            payment: Payment::Confirmed,
            status: "Payment confirmed",
            product: "Zuse Cloud",
            amount: Some(Amount {
                cents: 4000,
                currency: "usd",
            }),
            reference: Some("AB12CD34"),
            message: "m",
            hint: "h",
        });
        assert!(page.contains("USD 40.00") && page.contains("Payment confirmed"));
    }

    #[test]
    #[ignore = "writes preview pages to target/browser-pages"]
    fn write_previews() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/browser-pages");
        std::fs::create_dir_all(&dir).unwrap();
        let pages = [
            ("sign-in-success", sign_in(Outcome::Success, None)),
            ("sign-in-failure", sign_in(Outcome::Failure, None)),
            (
                "integration-success",
                integration("ChatGPT", Outcome::Success, None),
            ),
            (
                "integration-failure",
                integration("Plugin", Outcome::Failure, None),
            ),
            (
                "receipt",
                receipt(&Receipt {
                    payment: Payment::Confirmed,
                    status: "Payment confirmed",
                    product: "Zuse Cloud subscription",
                    amount: Some(Amount {
                        cents: 4000,
                        currency: "usd",
                    }),
                    reference: Some("AB12CD34"),
                    message: "Return to Zuse and refresh Cloud to see your subscription.",
                    hint: "Open the billing portal from Zuse for invoices and taxes.",
                }),
            ),
            (
                "card",
                card(&Card {
                    facts: vec![
                        ("GitHub account", "octocat".into()),
                        ("Zuse workspace", "Personal".into()),
                    ],
                    rows: vec![Row {
                        name: "team",
                        meta: Some("Organization"),
                        action: Action::Form {
                            label: "Use",
                            action: "/select",
                            fields: vec![("state", "s".into())],
                            pending: "Connecting…",
                        },
                    }],
                    actions: vec![Action::Link {
                        label: "Install GitHub App",
                        href: "https://github.com",
                        external: true,
                    }],
                    hint: Some(
                        "Continue only if you started this connection from your own Zuse app.",
                    ),
                    ..Card::notice(
                        "GitHub",
                        "Choose",
                        Tone::Waiting,
                        "Choose a GitHub installation",
                        "Pick where Zuse should work.",
                    )
                }),
            ),
        ];
        for (name, html) in pages {
            let csp = content_security_policy();
            let html = html.replacen(
                "<head>",
                &format!(
                    "<head><meta http-equiv=\"Content-Security-Policy\" content=\"{}\">",
                    csp.replace("; frame-ancestors 'none'", "")
                ),
                1,
            );
            std::fs::write(dir.join(format!("{name}.html")), html).unwrap();
        }
    }
}
