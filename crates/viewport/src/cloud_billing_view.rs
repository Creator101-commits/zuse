//! Shared Cloud hosting and billing presentation. Hosts retain account-bound RPC ownership.
use crate::cloud_settings::style::{badge, button, group, note, setting_row};
use crate::localization::message as tr;
use crate::{
    palette, settings_style as s,
    shell::{BORDER, MUTED, TEXT},
};
use gpui::{prelude::*, *};
use serde_json::Value;
use zuse_i18n::messages as msg;
use zuse_model::cloud_billing::{about_hours, cents, machine_minutes, per_minute, usd};
mod pages;
pub mod plans;
pub use pages::{Machine, billing, compute};
use zuse_protocol::cloud_billing::plans::PlanCatalog;
use zuse_protocol::cloud_billing::{
    BillingStatus, BillingSummary, BillingUsageItem, PrepaidBalance,
};

#[derive(Clone, Copy)]
pub enum Action {
    Refresh,
    RefreshCredits,
    /// Check out a cloud plan offer. While subscribed the server answers
    /// with the portal link that switches plans, or converts a trial.
    Checkout(&'static str),
    ManageSubscription,
    /// Show and focus the pay-as-you-go cap editor.
    RaiseCap,
    SaveCap,
    SelectCredit(i64),
    BuyCredits,
    MoreUsage,
    ToggleUsageHistory,
    /// Desktop pages link to each other; the browser ignores these.
    OpenBilling,
    OpenUsage,
    OpenAgents,
}
#[derive(Clone, Copy, Default)]
pub struct Pending {
    pub loading: bool,
    pub subscription: bool,
    pub credits: bool,
    pub cap: bool,
}
pub struct Data<'a> {
    pub subscribed: Option<bool>,
    pub pending: Pending,
    pub summary: Option<&'a BillingSummary>,
    /// Plans from the entitlements read; absent on older servers.
    pub catalog: Option<&'a PlanCatalog>,
    pub usage: &'a [BillingUsageItem],
    pub credits: Option<&'a PrepaidBalance>,
    pub credit_amount: i64,
    pub error: Option<&'a str>,
    pub credit_error: Option<&'a str>,
    pub more: bool,
    pub history_open: bool,
    pub workspaces: &'a [Value],
    pub personal: bool,
}
pub fn hosting<T: 'static>(
    data: &Data<'_>,
    workspace: &str,
    cx: &mut Context<T>,
    act: impl Fn(&mut T, Action, &mut Window, &mut Context<T>) + Copy + 'static,
) -> Div {
    let busy = data.pending.subscription;
    let disabled = data.subscribed.is_none() || busy;
    let data_subscribed = data.subscribed == Some(true);
    let mut controls = div().flex().items_center().gap(px(6.));
    if data.subscribed == Some(true) {
        controls = controls.child(badge("In use", false, false));
    }
    let label = if data.subscribed == Some(true) {
        "Manage subscription"
    } else {
        "Subscribe · from $20/month"
    };
    if data.subscribed.is_none() {
        controls = controls.child(badge(
            if data.pending.loading {
                "Loading…"
            } else {
                "Could not load billing"
            },
            false,
            true,
        ));
        if !data.pending.loading {
            controls = controls.child(
                button("retry-cloud-subscription", "Retry", false, false, false).on_click(
                    cx.listener(move |this, _, window, cx| act(this, Action::Refresh, window, cx)),
                ),
            );
        }
    } else {
        controls = controls.child(
            button(
                "open-cloud-billing-portal",
                label,
                data.subscribed == Some(false),
                disabled,
                busy,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if !disabled {
                    act(
                        this,
                        if data_subscribed {
                            Action::ManageSubscription
                        } else {
                            Action::Checkout(zuse_protocol::cloud_billing::plans::CLOUD_OFFER)
                        },
                        window,
                        cx,
                    )
                }
            })),
        );
    }
    let mut identity = div()
        .h(px(28.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .child(s::title_text(SharedString::from(format!(
            "Cloud · {workspace}"
        ))));
    if data.subscribed == Some(true) {
        identity = identity.child(badge("Ready", false, false));
    }
    let body = div().child(setting_row(
        true,
        "Zuse Cloud",
        if data.subscribed == Some(true) {
            "Subscribed. Zuse runs and bills the machines."
        } else {
            "No setup. Zuse runs and bills the machines."
        },
        Some(controls.into_any_element()),
    ));
    let mut root = div().flex().flex_col().gap(px(16.)).child(identity);
    if let Some(error) = data.error {
        root = root.child(
            div()
                .id("cloud-billing-error")
                .role(Role::Alert)
                .rounded(px(7.))
                .px(px(12.))
                .py(px(8.))
                .bg(palette::tone(
                    hsla(349.3 / 360., 0.311, 0.176, 1.),
                    hsla(0., 1., 0.973, 1.),
                ))
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(s::destructive_text())
                .child(error.to_owned()),
        );
    }
    root = root.child(group("Hosting",Some("Hosting pays for the machines. Agents and GitHub use your own accounts, connected through Zuse or already signed in on your snapshot."),None,body));
    if data.subscribed == Some(false) {
        root = root.child(plans::plans_group(data, cx, act));
    }
    root
}

pub fn usage<T: 'static>(
    data: &Data<'_>,
    cap: AnyElement,
    focus: &[FocusHandle; 4],
    cx: &mut Context<T>,
    act: impl Fn(&mut T, Action, &mut Window, &mut Context<T>) + Copy + 'static,
) -> Div {
    let mut root = div().flex().flex_col().gap(px(16.));
    if data.subscribed == Some(true) {
        let body = if let Some(summary) = data.summary {
            let percent = if summary.overage_cap_micros <= 0 {
                if summary.overage_provider_cost_micros > 0 {
                    100
                } else {
                    0
                }
            } else {
                ((i128::from(summary.overage_charge_micros) * 100
                    / i128::from(summary.overage_cap_micros))
                .clamp(0, 100)) as i64
            };
            let mut rows = div().flex().flex_col();
            let mut first = true;
            let by_minutes = summary.plan.as_ref();
            if let Some(p) = by_minutes {
                if let Some(banner) =
                    plans::hold_banner(summary, data.pending.subscription, cx, act)
                {
                    rows = rows.child(div().p(px(12.)).child(banner));
                    first = false;
                }
                rows = rows.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .px(px(12.))
                        .py(px(12.))
                        .when(!first, |v| v.border_t_1().border_color(palette::rgb(BORDER)))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(12.))
                                .child(s::details(plans::plan_title(p), plans::allowance_line(p)))
                                .when(p.trial, |row| {
                                    let busy = data.pending.subscription;
                                    row.child(
                                        button("cloud-trial-start-plan", "Start Cloud now", true, busy, busy)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                if !busy {
                                                    act(
                                                        this,
                                                        Action::Checkout(
                                                            zuse_protocol::cloud_billing::plans::CLOUD_OFFER,
                                                        ),
                                                        window,
                                                        cx,
                                                    )
                                                }
                                            })),
                                    )
                                }),
                        )
                        .child(plans::minutes_block(p, None)),
                );
                if let Some(extra) = plans::extra_line(p) {
                    rows = rows.child(setting_row(false, "Pay as you go", extra, None));
                }
                first = false;
            } else if percent >= 50 {
                rows=rows.child(setting_row(first,if percent>=100{"Billing hold"}else{"Usage warning"},if percent>=100{"The overage cap is exhausted. New billable operations are blocked and running compute is being paused.".to_owned()}else{format!("You have used {percent}% of this period's overage cap.")},Some(badge(SharedString::from(format!("{percent}%")),true,false).into_any_element())));
                first = false;
            }
            if by_minutes.is_none() {
                rows = rows.child(setting_row(
                    first,
                    SharedString::from(if summary.shared_credits {
                        format!(
                            "{} credits remaining",
                            usd(summary.included_remaining_micros)
                        )
                    } else {
                        format!(
                            "{} of {} included used",
                            usd(summary.included_used_micros),
                            usd(summary.included_provider_cost_micros)
                        )
                    }),
                    SharedString::from(if summary.shared_credits {
                        format!(
                            "{} monthly credits · {} subscription before tax",
                            usd(summary.included_provider_cost_micros),
                            usd(summary.base_price_micros)
                        )
                    } else {
                        format!(
                            "{} remaining · {} overage · {} invoice estimate before tax",
                            usd(summary.included_remaining_micros),
                            usd(summary.overage_charge_micros),
                            usd(summary.current_invoice_estimate_micros)
                        )
                    }),
                    None,
                ));
            }
            if by_minutes.map_or(!summary.shared_credits, |h| h.pay_as_you_go) {
                let busy = data.pending.cap;
                let disabled = data.pending.loading || busy;
                rows = rows.child(setting_row(
                    false,
                    "Pay-as-you-go cap",
                    "Monthly limit on usage beyond your plan, before tax. Cloud computers pause when it is reached.",
                    Some(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .w(px(80.))
                                    .h(px(28.))
                                    .flex()
                                    .items_center()
                                    .px(px(9.))
                                    .rounded(px(7.))
                                    .border_1()
                                    .border_color(palette::rgb(BORDER))
                                    .child(cap),
                            )
                            .child(
                                button("save-cloud-cap", "Save", true, disabled, busy).on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        if !disabled {
                                            act(this, Action::SaveCap, window, cx)
                                        }
                                    }),
                                ),
                            )
                            .into_any_element(),
                    ),
                ));
            }
            let recent = if data.usage.is_empty() {
                "No completed cloud computer runs in this billing period.".to_owned()
            } else {
                data.usage
                    .iter()
                    .take(3)
                    .map(|item| {
                        format!(
                            "{} {}: {}{}",
                            item.usage_kind.as_deref().unwrap_or(&item.resource_kind),
                            item.resource_id,
                            usage_amount(item, by_minutes.is_some()),
                            if item.status == "provisional" {
                                " (provisional)"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" · ")
            };
            rows = rows.child(setting_row(
                false,
                "Recent usage",
                SharedString::from(recent),
                None,
            ));
            if data.usage.len() > 3 || data.more {
                rows = rows.child(
                    div().px(px(12.)).pb(px(10.)).child(
                        button(
                            "toggle-cloud-usage-history",
                            if data.history_open {
                                "Hide history"
                            } else {
                                "View history"
                            },
                            false,
                            false,
                            false,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                act(this, Action::ToggleUsageHistory, window, cx)
                            },
                        )),
                    ),
                );
            }
            if data.history_open {
                for item in data.usage.iter().skip(3) {
                    rows = rows.child(setting_row(
                        false,
                        SharedString::from(format!(
                            "{} {}",
                            item.usage_kind.as_deref().unwrap_or(&item.resource_kind),
                            item.resource_id
                        )),
                        SharedString::from(format!(
                            "{} · {}",
                            usage_amount(item, by_minutes.is_some()),
                            item.status
                        )),
                        None,
                    ));
                }
                if data.more {
                    let disabled = data.pending.loading;
                    rows = rows.child(div().px(px(12.)).pb(px(10.)).child(
                        button("more-cloud-usage", "Load more", false, disabled, false).on_click(
                            cx.listener(move |this, _, window, cx| {
                                if !disabled {
                                    act(this, Action::MoreUsage, window, cx)
                                }
                            }),
                        ),
                    ));
                }
            }
            let paused = summary.status == BillingStatus::BillingHold;
            group(
                "Usage and billing",
                Some(if by_minutes.is_some() {
                    "Plans include machine minutes. A 4 vCPU / 16 GB cloud computer uses 4 per awake minute; asleep computers and their storage use none."
                } else if summary.shared_credits {
                    "Your plan includes shared credits for compute, storage and Zuse-billed models."
                } else {
                    "Your plan includes cloud compute and image storage. Additional usage has a 5% markup."
                }),
                Some(
                    badge(if paused { "Paused" } else { "Active" }, paused, false)
                        .into_any_element(),
                ),
                rows,
            )
        } else {
            let disabled = data.pending.loading;
            group(
                "Usage and billing",
                Some("Usage details are temporarily unavailable."),
                None,
                div().child(setting_row(
                    true,
                    if disabled {
                        "Loading billing…"
                    } else {
                        "Could not load billing"
                    },
                    "Refresh cloud settings to try again. Existing workspaces are unaffected.",
                    Some(
                        button("retry-cloud-billing", "Retry", false, disabled, false)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if !disabled {
                                    act(this, Action::Refresh, window, cx)
                                }
                            }))
                            .into_any_element(),
                    ),
                )),
            )
        };
        root = root.child(body);
    }
    let credits = credits_body(data, focus, true, cx, act);
    let portal_busy = data.pending.subscription;
    let disabled = portal_busy;
    let billing=div().flex().flex_col().child(credits).child(setting_row(false,"Manage subscription","Manage or cancel your Zuse subscription, payment details, and invoices in the billing portal.",Some(button("manage-cloud-subscription","Manage subscription",false,disabled,portal_busy).on_click(cx.listener(move|this,_,window,cx|if !disabled{act(this,Action::ManageSubscription,window,cx)})).into_any_element())));
    root = root.child(group("Billing", None, None, billing));
    if data.subscribed == Some(true) && data.personal {
        let mut activity = div().flex().flex_col();
        if data.workspaces.is_empty() {
            activity = activity.child(setting_row(
                true,
                "No cloud workspaces yet",
                "Start a cloud chat and its workspace will appear here.",
                None,
            ));
        } else {
            for (i, workspace) in data.workspaces.iter().enumerate() {
                let state = workspace["state"].as_str().unwrap_or("unknown");
                activity = activity.child(setting_row(
                    i == 0,
                    SharedString::from(
                        workspace["branch"]
                            .as_str()
                            .unwrap_or("Workspace")
                            .to_owned(),
                    ),
                    SharedString::from(workspace["statusCode"].as_str().unwrap_or("").to_owned()),
                    Some(
                        badge(
                            SharedString::from(state.to_owned()),
                            !matches!(state, "ready" | "deleted"),
                            state == "deleted",
                        )
                        .into_any_element(),
                    ),
                ));
            }
        }
        root = root.child(group(
            "Workspace activity",
            Some("Current and recent cloud workspaces for this account."),
            Some(
                badge(
                    SharedString::from(data.workspaces.len().to_string()),
                    false,
                    true,
                )
                .into_any_element(),
            ),
            activity,
        ));
    }
    root
}

/// One usage entry: machine minutes (with the hours hint and, for larger
/// computers, how fast they use them) when the plan counts them, otherwise
/// its cost.
fn usage_amount(item: &BillingUsageItem, by_minutes: bool) -> String {
    match item.machine_minutes.filter(|_| by_minutes) {
        Some(minutes) => {
            let rate = item
                .machine_rate
                .unwrap_or(zuse_protocol::cloud_billing::plans::DEFAULT_RATE);
            let mut text = format!(
                "{} ({})",
                machine_minutes(minutes),
                about_hours(minutes, rate)
            );
            if rate != zuse_protocol::cloud_billing::plans::DEFAULT_RATE {
                text = format!("{text} · {}", per_minute(rate));
            }
            text
        }
        None => usd(item.provider_cost_micros),
    }
}

/// Machine minutes a credit amount in cents buys at the plan rate.
fn credit_minutes(cents: i64) -> i64 {
    zuse_protocol::cloud_billing::plans::credit_machine_minutes(cents.saturating_mul(10_000))
}
/// "21,000 minutes of credit", or why the balance is missing.
fn credit_balance_line(data: &Data<'_>) -> String {
    data.credits
        .map(|b| {
            format!(
                "{} minutes of credit",
                crate::cloud_settings::kit::thousands(credit_minutes(b.credit_cents))
            )
        })
        .unwrap_or_else(|| "Balance unavailable".into())
}
/// The "Buy more minutes" header meta: the balance as a quiet line and a
/// refresh icon.
pub(super) fn credits_header<T: 'static>(
    data: &Data<'_>,
    cx: &mut Context<T>,
    act: impl Fn(&mut T, Action, &mut Window, &mut Context<T>) + Copy + 'static,
) -> AnyElement {
    let busy = data.pending.credits;
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(
            div()
                .text_size(px(11.))
                .text_color(palette::rgb(MUTED))
                .child(credit_balance_line(data)),
        )
        .child(
            crate::cloud_settings::style::icon_button(
                "refresh-prepaid-credits",
                "refresh",
                "Refresh balance",
                busy,
                busy,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if !busy {
                    act(this, Action::RefreshCredits, window, cx)
                }
            })),
        )
        .into_any_element()
}
/// "Buy more minutes": four equal packs (price first, minutes under it),
/// the purchase button and one quiet line. `balance` puts the balance line
/// in the body for layouts without a header meta.
fn credits_body<T: 'static>(
    data: &Data<'_>,
    focus: &[FocusHandle; 4],
    balance: bool,
    cx: &mut Context<T>,
    act: impl Fn(&mut T, Action, &mut Window, &mut Context<T>) + Copy + 'static,
) -> Div {
    use crate::cloud_settings::kit;
    let busy = data.pending.credits;
    let enabled = data.credits.is_some_and(|b| b.available) && !busy;
    let mut credits = div().flex().flex_col().gap(px(12.)).p(px(kit::INSET));
    if balance {
        credits = credits.child(s::description_text(credit_balance_line(data)));
    }
    let mut options = div()
        .flex()
        .gap(px(8.))
        .w_full()
        .id("cloud-credit-amounts")
        .role(Role::RadioGroup)
        .aria_label(tr(cx, msg::settings::prepaid_amount));
    for (i, amount) in [1000, 2500, 5000, 10000].into_iter().enumerate() {
        let handles = focus.clone();
        let selected = data.credit_amount == amount;
        let minutes = format!("{} minutes", kit::thousands(credit_minutes(amount)));
        let price = format!("${}", amount / 100);
        let label = format!("{price} · {minutes}");
        options = options.child(
            div()
                .id(("credit-amount", i))
                .role(Role::RadioButton)
                .aria_label(SharedString::from(label))
                .aria_selected(selected)
                .track_focus(&focus[i].clone().tab_stop(enabled && selected))
                .tab_index(if enabled && selected { 0 } else { -1 })
                .flex_1()
                .flex_basis(px(0.))
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(px(kit::INSET))
                .rounded(px(10.))
                .border_1()
                .border_color(if selected {
                    kit::accent()
                } else {
                    palette::rgb(BORDER).into()
                })
                .when(selected, |v| v.bg(kit::accent().opacity(0.06)))
                .when(!enabled, |v| v.opacity(0.5))
                .when(enabled && !selected, |v| {
                    v.cursor_pointer()
                        .hover(|v| v.bg(s::frame_bg().opacity(0.5)))
                })
                .focus_visible(|v| v.border_color(kit::accent()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(14.))
                                .line_height(px(18.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(palette::rgb(TEXT))
                                .child(SharedString::from(price)),
                        )
                        .when(selected, |v| {
                            v.child(
                                crate::assets::icon("check")
                                    .size(px(12.))
                                    .text_color(kit::accent()),
                            )
                        }),
                )
                .child(s::description_text(minutes))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                    if !enabled {
                        return;
                    }
                    let next = match event.keystroke.key.as_str() {
                        "left" | "up" => (i + 3) % 4,
                        "right" | "down" => (i + 1) % 4,
                        "home" => 0,
                        "end" => 3,
                        "enter" | "space" => {
                            cx.stop_propagation();
                            return;
                        }
                        _ => return,
                    };
                    act(
                        this,
                        Action::SelectCredit([1000, 2500, 5000, 10000][next]),
                        window,
                        cx,
                    );
                    window.focus(&handles[next], cx);
                    cx.stop_propagation();
                }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if enabled {
                        act(this, Action::SelectCredit(amount), window, cx)
                    }
                })),
        );
    }
    credits = credits.child(options).child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(11.))
                    .line_height(px(16.))
                    .text_color(palette::rgb(MUTED))
                    .child("Credit rolls over and applies to future Zuse invoices."),
            )
            .child(
                button("buy-prepaid-credits", "Buy minutes", true, !enabled, busy).on_click(
                    cx.listener(move |this, _, window, cx| {
                        if enabled {
                            act(this, Action::BuyCredits, window, cx)
                        }
                    }),
                ),
            ),
    );
    if let Some(balance) = data.credits.filter(|b| b.debit_cents > 0) {
        credits = credits.child(s::description_text(format!(
            "{} owed on future invoices",
            cents(balance.debit_cents)
        )));
    }
    if let Some(credit) = data.credits.and_then(|b| b.invoice_credit_cents) {
        credits = credits.child(s::description_text(format!(
            "{} in earlier invoice credit, applied to subscription invoices.",
            cents(credit)
        )));
    }
    if data.credits.is_some_and(|b| !b.available) {
        credits = credits.child(s::description_text(tr(cx, msg::settings::prepaid_disabled)));
    }
    if let Some(error) = data.credit_error {
        credits = credits.child(note(error, true));
    }
    credits
}
