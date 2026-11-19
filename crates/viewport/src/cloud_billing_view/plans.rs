//! Cloud plans in machine minutes: plan copy, the plan comparison, the trial
//! offer, the minutes meter and holds. Machine minutes are the headline,
//! hours a hint; dollars appear only on prices and pay as you go.
use super::*;
use crate::tooltip::TooltipExt as _;
use zuse_model::cloud_billing::{
    about_hours, dollars, grouped, machine_minutes, machine_size, monthly,
};
use zuse_protocol::cloud_billing::plans::{self, PlanCatalog, PlanId, PlanOffer};
use zuse_protocol::cloud_billing::{HoldReason, PlanUsage};

pub const TRIAL_COPY: &str = "Try Cloud for $1 for a week — 8,400 machine minutes (about 35 h on a 4 vCPU / 16 GB computer), then $20/month for 33,600.";
pub const ENTERPRISE_COPY: &str =
    "More machine minutes, larger computers, invoicing and support for your team.";

/// The server's catalog, or the published plans from older servers.
pub fn catalog(catalog: Option<&PlanCatalog>) -> PlanCatalog {
    catalog
        .cloned()
        .unwrap_or_else(|| plans::catalog(false, &|_| true, plans::ENTERPRISE_CONTACT_FALLBACK))
}
pub(super) fn date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local).format("%b %-d").to_string())
        .unwrap_or_default()
}
pub fn plan_id(summary: Option<&BillingSummary>) -> Option<PlanId> {
    Some(summary?.plan.as_ref()?.plan_id)
}
/// The machine a plan's hours are counted on: "4 vCPU / 16 GB".
pub fn default_machine(p: &PlanUsage) -> String {
    let rate = p.default_machine_rate.max(1);
    machine_size(rate, rate * plans::UNIT_MEMORY_MIB)
}
/// "Cloud · $20/month".
pub fn plan_title(p: &PlanUsage) -> String {
    if p.trial {
        format!(
            "Cloud trial · {} for the first week",
            dollars(p.price_micros)
        )
    } else {
        format!("{} · {}", p.plan_name, monthly(p.price_micros))
    }
}
/// "33,600 machine minutes · about 140 h on your 4 vCPU / 16 GB machine".
pub fn allowance_line(p: &PlanUsage) -> String {
    format!(
        "{} · {} on your {} machine",
        machine_minutes(p.included_machine_minutes),
        about_hours(p.included_machine_minutes, p.default_machine_rate),
        default_machine(p)
    )
}
/// "Renews Dec 1", or when a trial becomes Cloud.
pub fn renews_line(p: &PlanUsage) -> String {
    if p.trial {
        format!(
            "Cloud starts {} at $20/month",
            date(p.trial_ends_at.unwrap_or(p.renews_at))
        )
    } else {
        format!("Renews {}", date(p.renews_at))
    }
}
/// "12,480 of 33,600 machine minutes used · 21,120 left".
pub fn meter_line(p: &PlanUsage) -> String {
    format!(
        "{} of {} used · {} left",
        grouped(p.used_machine_minutes.min(p.included_machine_minutes)),
        machine_minutes(p.included_machine_minutes),
        grouped(p.remaining_machine_minutes)
    )
}
/// "about 88 h left on 4 vCPU / 16 GB".
pub fn meter_hint(p: &PlanUsage) -> String {
    format!(
        "{} left on {}",
        about_hours(p.remaining_machine_minutes, p.default_machine_rate),
        default_machine(p)
    )
}
/// "2,880 extra machine minutes this month ($1.71) · cap $25"; none when zero.
pub fn extra_line(p: &PlanUsage) -> Option<String> {
    (p.extra_machine_minutes > 0).then(|| {
        format!(
            "{} extra machine minutes this month ({}) · cap {}",
            grouped(p.extra_machine_minutes),
            usd(p.extra_charge_micros),
            dollars(p.overage_cap_micros)
        )
    })
}
/// "Trial · ends Nov 26 · 3,200 of 8,400 machine minutes left".
pub fn trial_line(p: &PlanUsage) -> String {
    let mut parts = vec!["Trial".to_owned()];
    if let Some(end) = p.trial_ends_at {
        parts.push(format!("ends {}", date(end)));
    }
    parts.push(format!(
        "{} of {} left",
        grouped(p.remaining_machine_minutes),
        machine_minutes(p.included_machine_minutes)
    ));
    parts.join(" · ")
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldButton {
    StartCloud,
    RaiseCap,
    UpgradeMax,
}
impl HoldButton {
    pub fn label(self) -> &'static str {
        match self {
            Self::StartCloud => "Start Cloud now",
            Self::RaiseCap => "Raise cap",
            Self::UpgradeMax => "Upgrade to Cloud Max",
        }
    }
    fn action(self) -> Action {
        match self {
            Self::StartCloud => Action::Checkout(plans::CLOUD_OFFER),
            Self::RaiseCap => Action::RaiseCap,
            Self::UpgradeMax => Action::Checkout(plans::CLOUD_MAX_OFFER),
        }
    }
}
/// The hold message and the buttons that clear it.
pub fn hold(p: &PlanUsage) -> Option<(String, Vec<HoldButton>)> {
    if !p.usage_hold {
        return None;
    }
    Some(match p.hold_reason? {
        HoldReason::TrialAllowanceUsed => (
            "Your trial minutes are used up. Cloud computers are paused.".into(),
            vec![HoldButton::StartCloud],
        ),
        HoldReason::TrialUnavailable => (
            "This card or account has already used a trial.".into(),
            vec![HoldButton::StartCloud],
        ),
        HoldReason::CapReached => {
            let mut buttons = vec![HoldButton::RaiseCap];
            if p.plan_id != PlanId::CloudMax {
                buttons.push(HoldButton::UpgradeMax);
            }
            (
                format!(
                    "You reached your {} pay-as-you-go cap. Cloud computers are paused until {}.",
                    dollars(p.overage_cap_micros),
                    date(p.renews_at)
                ),
                buttons,
            )
        }
    })
}
fn warning_tone() -> Hsla {
    hsla(21.6 / 360., 0.982, 0.567, 1.)
}
fn lime() -> Hsla {
    palette::tone(
        hsla(83. / 360., 0.72, 0.46, 1.),
        hsla(83. / 360., 0.74, 0.40, 1.),
    )
}
pub(super) fn hold_banner<T: 'static>(
    summary: &BillingSummary,
    busy: bool,
    cx: &mut Context<T>,
    act: impl Fn(&mut T, Action, &mut Window, &mut Context<T>) + Copy + 'static,
) -> Option<Stateful<Div>> {
    let (text, buttons) = hold(summary.plan.as_ref()?)?;
    let mut controls = div().flex().items_center().gap(px(6.));
    for (i, kind) in buttons.into_iter().enumerate() {
        let action = kind.action();
        let checkout = matches!(action, Action::Checkout(_));
        let disabled = checkout && busy;
        controls = controls.child(
            button(
                ("cloud-hold-action", i),
                kind.label(),
                i == 0,
                disabled,
                disabled,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if !disabled {
                    act(this, action, window, cx)
                }
            })),
        );
    }
    Some(
        div()
            .id("cloud-usage-hold")
            .role(Role::Alert)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(12.))
            .rounded(px(s::FRAME_RADIUS))
            .border_1()
            .border_color(s::destructive_text().opacity(0.3))
            .bg(palette::tone(
                hsla(349.3 / 360., 0.311, 0.14, 1.),
                hsla(0., 1., 0.975, 1.),
            ))
            .px(px(14.))
            .py(px(12.))
            .child(
                div()
                    .size(px(8.))
                    .flex_none()
                    .rounded_full()
                    .bg(s::destructive_text()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(200.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(16.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette::rgb(TEXT))
                            .child("Cloud computers are paused"),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .line_height(px(16.))
                            .text_color(s::destructive_text())
                            .child(text),
                    ),
            )
            .child(controls),
    )
}
/// A dithered meter `height` tall filled to `fraction`; amber from 90%.
pub(super) fn meter(id: &'static str, fraction: f32, height: f32, label: String) -> Stateful<Div> {
    let fill = if fraction >= 0.9 {
        warning_tone()
    } else {
        lime()
    };
    div()
        .id(id)
        .role(Role::ProgressIndicator)
        .aria_label(SharedString::from(label))
        .relative()
        .w_full()
        .h(px(height))
        .child(crate::dither::meter(
            fraction.clamp(0., 1.),
            fill,
            palette::rgb(MUTED).opacity(0.35).into(),
        ))
}
/// The big number, the dithered meter and the used/left line, with an
/// optional control at the line's end.
pub(super) fn minutes_block(p: &PlanUsage, trailing: Option<AnyElement>) -> Div {
    let included = p.included_machine_minutes.max(1);
    let fraction = p.used_machine_minutes as f32 / included as f32;
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            div()
                .flex()
                .items_end()
                .justify_between()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.))
                        .child(
                            div()
                                .text_size(px(28.))
                                .line_height(px(32.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette::rgb(TEXT))
                                .child(grouped(p.remaining_machine_minutes)),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .line_height(px(16.))
                                .text_color(palette::rgb(MUTED))
                                .child("machine minutes left"),
                        ),
                )
                .child(
                    div()
                        .id("cloud-minutes-hint")
                        .text_size(px(11.))
                        .line_height(px(16.))
                        .text_color(palette::rgb(MUTED))
                        .child(meter_hint(p))
                        .tip(
                            "Hours are awake time on your default computer. Larger computers use machine minutes faster; asleep computers use none.",
                        ),
                ),
        )
        .child(meter("cloud-minutes-meter", fraction, 12., meter_line(p)))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .min_h(px(16.))
                .child(
                    div()
                        .min_w_0()
                        .text_size(px(11.))
                        .line_height(px(16.))
                        .text_color(palette::rgb(MUTED))
                        .child(if p.trial { trial_line(p) } else { meter_line(p) }),
                )
                .children(trailing),
        )
}
/// The $1 trial: shown only when the catalog offers it.
pub(super) fn trial_offer<T: 'static>(
    data: &Data<'_>,
    cx: &mut Context<T>,
    act: impl Fn(&mut T, Action, &mut Window, &mut Context<T>) + Copy + 'static,
) -> Option<Stateful<Div>> {
    let catalog = catalog(data.catalog);
    if !catalog.trial_available || data.subscribed != Some(false) {
        return None;
    }
    let disabled = data.pending.subscription;
    Some(
        div()
            .id("cloud-trial-offer")
            .relative()
            .overflow_hidden()
            .rounded(px(s::FRAME_RADIUS))
            .border_1()
            .border_color(lime().opacity(0.45))
            .bg(s::card_bg())
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .px(px(16.))
                    .py(px(14.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .text_size(px(14.))
                                            .line_height(px(18.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(palette::rgb(TEXT))
                                            .child("Try Cloud for $1"),
                                    )
                                    .child(badge("First week", false, false)),
                            )
                            .child(s::description_text(TRIAL_COPY)),
                    )
                    .child(
                        crate::cloud_settings::kit::primary_button(
                            "cloud-plan-trial",
                            "Try for $1",
                            !disabled,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                if !disabled {
                                    act(
                                        this,
                                        Action::Checkout(plans::CLOUD_TRIAL_OFFER),
                                        window,
                                        cx,
                                    )
                                }
                            },
                        )),
                    ),
            ),
    )
}
/// What a plan card's control does for this account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanControl {
    Current(&'static str),
    Checkout {
        label: &'static str,
        offer: &'static str,
        primary: bool,
    },
}
pub fn plan_control(offer: &PlanOffer, current: Option<PlanId>, subscribed: bool) -> PlanControl {
    let id = if offer.plan_id == PlanId::CloudMax {
        plans::CLOUD_MAX_OFFER
    } else {
        plans::CLOUD_OFFER
    };
    let current = current.filter(|_| subscribed);
    match (current, offer.plan_id) {
        (Some(PlanId::CloudTrial), PlanId::Cloud) => PlanControl::Checkout {
            label: "Start Cloud now",
            offer: id,
            primary: true,
        },
        (Some(PlanId::Cloud | PlanId::Legacy), PlanId::Cloud) => {
            PlanControl::Current("Current plan")
        }
        (Some(PlanId::CloudMax), PlanId::CloudMax) => PlanControl::Current("Current plan"),
        (Some(PlanId::CloudMax), _) => PlanControl::Checkout {
            label: "Switch to Cloud",
            offer: id,
            primary: false,
        },
        (Some(_), PlanId::CloudMax) => PlanControl::Checkout {
            label: "Upgrade to Cloud Max",
            offer: id,
            primary: true,
        },
        // Subscribed before plans were reported: let the portal decide.
        (None, _) if subscribed => PlanControl::Checkout {
            label: "Switch plan",
            offer: id,
            primary: false,
        },
        _ => PlanControl::Checkout {
            label: "Subscribe",
            offer: id,
            primary: offer.plan_id == PlanId::Cloud,
        },
    }
}
/// What each plan includes, one line each.
pub fn plan_features(offer: &PlanOffer) -> Vec<String> {
    let rate = plans::DEFAULT_RATE;
    let mut lines = vec![format!(
        "{} on 4 vCPU / 16 GB",
        about_hours(offer.included_machine_minutes, rate)
    )];
    lines.push(if offer.max_vcpus > plans::DEFAULT_VCPUS {
        format!(
            "Computers up to {}",
            machine_size(offer.max_vcpus, offer.max_memory_mib)
        )
    } else {
        "Standard 4 vCPU / 16 GB computers".into()
    });
    if offer.pay_as_you_go {
        lines.push("Pay as you go after, up to your cap".into());
    }
    lines.push("Asleep computers use nothing".into());
    lines
}
fn plan_card(
    name: String,
    price: (String, &'static str),
    allowance: String,
    features: Vec<String>,
    highlight: bool,
    control: AnyElement,
) -> Div {
    let mut list = div().flex().flex_col().gap(px(6.));
    for line in features {
        list = list.child(
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .child(div().mt(px(6.)).size(px(4.)).flex_none().bg(if highlight {
                    lime()
                } else {
                    palette::rgb(MUTED).into()
                }))
                .child(div().flex_1().min_w_0().child(s::description_text(line))),
        );
    }
    div()
        .flex_1()
        .min_w(px(180.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(14.))
        .rounded(px(s::CARD_RADIUS))
        .border_1()
        .border_color(if highlight {
            lime().opacity(0.6)
        } else {
            s::card_border()
        })
        .bg(s::card_bg())
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(s::title_text(name))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(3.))
                        .child(
                            div()
                                .text_size(px(22.))
                                .line_height(px(28.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette::rgb(TEXT))
                                .child(price.0),
                        )
                        .child(s::description_text(price.1)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .text_color(palette::rgb(TEXT))
                        .child(allowance),
                ),
        )
        .child(div().h(px(1.)).bg(s::card_border()))
        .child(list.flex_1())
        .child(div().flex().child(control))
}
/// Cloud, Cloud Max and Enterprise side by side, with the trial offer above
/// them when available.
pub(super) fn plans_group<T: 'static>(
    data: &Data<'_>,
    cx: &mut Context<T>,
    act: impl Fn(&mut T, Action, &mut Window, &mut Context<T>) + Copy + 'static,
) -> Div {
    let catalog = catalog(data.catalog);
    let current = plan_id(data.summary);
    let subscribed = data.subscribed == Some(true);
    let busy = data.pending.subscription;
    let mut cards = div().flex().flex_wrap().gap(px(8.)).p(px(4.));
    for offer in catalog
        .plans
        .iter()
        .filter(|o| matches!(o.plan_id, PlanId::Cloud | PlanId::CloudMax))
    {
        let control = plan_control(offer, current, subscribed);
        let highlight = match &control {
            PlanControl::Current(_) => true,
            PlanControl::Checkout { primary, .. } => *primary && !subscribed,
        };
        let element = match control {
            PlanControl::Current(label) => badge(label, false, false).into_any_element(),
            PlanControl::Checkout {
                label,
                offer: offer_id,
                primary,
            } => {
                let disabled = busy || !offer.available || data.subscribed.is_none();
                button(
                    SharedString::from(format!("cloud-plan-{offer_id}")),
                    label,
                    primary,
                    disabled,
                    busy,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if !disabled {
                        act(this, Action::Checkout(offer_id), window, cx)
                    }
                }))
                .into_any_element()
            }
        };
        cards = cards.child(plan_card(
            offer.name.clone(),
            (dollars(offer.price_micros), "/month"),
            machine_minutes(offer.included_machine_minutes),
            plan_features(offer),
            highlight,
            element,
        ));
    }
    let url = catalog.enterprise_contact_url.clone();
    let valid = url.starts_with("https://") || url.starts_with("mailto:");
    cards = cards.child(plan_card(
        "Enterprise".into(),
        ("Custom".into(), ""),
        "Machine minutes for your team".into(),
        vec![ENTERPRISE_COPY.into(), "Larger computers on request".into()],
        false,
        button("cloud-plan-enterprise", "Contact us", false, !valid, false)
            .on_click(cx.listener(move |_, _, _, cx| {
                if valid {
                    cx.open_url(&url)
                }
            }))
            .into_any_element(),
    ));
    let mut root = div().flex().flex_col().gap(px(12.));
    if let Some(trial) = trial_offer(data, cx, act) {
        root = root.child(trial);
    }
    root.child(
        s::frame()
            .child(s::header(
                "Plans",
                "A machine minute is one minute of a 1 vCPU / 4 GB machine. A 4 vCPU / 16 GB computer uses 4 per awake minute.",
            ))
            .child(cards),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    fn usage(plan: PlanId) -> PlanUsage {
        PlanUsage {
            plan_id: plan,
            plan_name: "Cloud".into(),
            price_micros: 20_000_000,
            trial: false,
            trial_ends_at: None,
            renews_at: 0,
            included_machine_minutes: 33_600,
            used_machine_minutes: 12_480,
            remaining_machine_minutes: 21_120,
            extra_machine_minutes: 2_880,
            extra_charge_micros: 1_714_286,
            overage_cap_micros: 25_000_000,
            pay_as_you_go: true,
            usage_hold: false,
            hold_reason: None,
            default_machine_rate: 4,
            max_vcpus: 4,
            max_memory_mib: 16_384,
        }
    }
    #[test]
    fn plan_lines_follow_the_published_copy() {
        let mut p = usage(PlanId::Cloud);
        assert_eq!(plan_title(&p), "Cloud · $20/month");
        assert_eq!(
            allowance_line(&p),
            "33,600 machine minutes · about 140 h on your 4 vCPU / 16 GB machine"
        );
        assert!(renews_line(&p).starts_with("Renews "));
        assert_eq!(
            meter_line(&p),
            "12,480 of 33,600 machine minutes used · 21,120 left"
        );
        assert_eq!(meter_hint(&p), "about 88 h left on 4 vCPU / 16 GB");
        assert_eq!(
            extra_line(&p).unwrap(),
            "2,880 extra machine minutes this month ($1.71) · cap $25"
        );
        p.extra_machine_minutes = 0;
        assert_eq!(extra_line(&p), None);
        p.trial = true;
        p.included_machine_minutes = 8_400;
        p.remaining_machine_minutes = 3_200;
        assert_eq!(
            trial_line(&p),
            "Trial · 3,200 of 8,400 machine minutes left"
        );
        p.trial_ends_at = Some(1_700_000_000_000);
        assert!(trial_line(&p).starts_with("Trial · ends "));
    }
    #[test]
    fn holds_offer_the_way_out() {
        let mut p = usage(PlanId::Cloud);
        assert_eq!(hold(&p), None);
        p.usage_hold = true;
        p.hold_reason = Some(HoldReason::CapReached);
        let (text, buttons) = hold(&p).unwrap();
        assert!(text.starts_with(
            "You reached your $25 pay-as-you-go cap. Cloud computers are paused until "
        ));
        assert_eq!(buttons, [HoldButton::RaiseCap, HoldButton::UpgradeMax]);
        p.plan_id = PlanId::CloudMax;
        assert_eq!(hold(&p).unwrap().1, [HoldButton::RaiseCap]);
        p.hold_reason = Some(HoldReason::TrialAllowanceUsed);
        assert_eq!(
            hold(&p).unwrap(),
            (
                "Your trial minutes are used up. Cloud computers are paused.".into(),
                vec![HoldButton::StartCloud]
            )
        );
        p.hold_reason = Some(HoldReason::TrialUnavailable);
        assert_eq!(
            hold(&p).unwrap().0,
            "This card or account has already used a trial."
        );
        let labels = [
            HoldButton::StartCloud,
            HoldButton::RaiseCap,
            HoldButton::UpgradeMax,
        ]
        .map(HoldButton::label);
        assert_eq!(
            labels,
            ["Start Cloud now", "Raise cap", "Upgrade to Cloud Max"]
        );
    }
    #[test]
    fn plan_cards_offer_the_right_change() {
        let c = catalog(None);
        let (cloud, max) = (&c.plans[0], &c.plans[1]);
        assert_eq!(
            (cloud.name.as_str(), max.name.as_str()),
            ("Cloud", "Cloud Max")
        );
        assert_eq!(
            machine_minutes(max.included_machine_minutes),
            "84,000 machine minutes"
        );
        let label = |offer, current, subscribed| match plan_control(offer, current, subscribed) {
            PlanControl::Current(l) | PlanControl::Checkout { label: l, .. } => l,
        };
        assert_eq!(label(cloud, None, false), "Subscribe");
        assert_eq!(label(cloud, Some(PlanId::Legacy), true), "Current plan");
        assert_eq!(
            label(max, Some(PlanId::Cloud), true),
            "Upgrade to Cloud Max"
        );
        assert_eq!(
            label(cloud, Some(PlanId::CloudTrial), true),
            "Start Cloud now"
        );
        assert_eq!(
            label(cloud, Some(PlanId::CloudMax), true),
            "Switch to Cloud"
        );
        assert_eq!(label(max, Some(PlanId::CloudMax), true), "Current plan");
        assert_eq!(
            plan_features(max),
            [
                "about 350 h on 4 vCPU / 16 GB",
                "Computers up to 8 vCPU / 32 GB",
                "Pay as you go after, up to your cap",
                "Asleep computers use nothing"
            ]
        );
        assert!(!c.trial_available);
        let p = usage(PlanId::Cloud);
        let texts = c.plans.iter().flat_map(plan_features).chain([
            TRIAL_COPY.into(),
            ENTERPRISE_COPY.into(),
            plan_title(&p),
            allowance_line(&p),
            meter_line(&p),
            meter_hint(&p),
            trial_line(&p),
            extra_line(&p).unwrap(),
        ]);
        for text in texts {
            assert!(!plans::names_infrastructure(&text), "{text}");
        }
    }
}
