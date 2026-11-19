//! The account's Cloud plan as onboarding and the Machine page show it: the
//! plan name, whether it is the first-week trial, and machine minutes. Read
//! from the billing summary's plan (the same numbers Billing shows); never
//! names a provider.
use serde_json::Value;
use zuse_protocol::cloud_billing::{
    PlanUsage,
    plans::{self, PlanId},
};

/// The machine every plan starts chats on.
pub(crate) const DEFAULT_MACHINE: &str = "4 vCPU · 16 GB";
/// The largest machine, on Cloud Max.
pub(crate) const LARGE_MACHINE: &str = "8 vCPU · 32 GB";
/// Machine minutes in one hour of the default machine.
pub(crate) const MINUTES_PER_HOUR: i64 = plans::DEFAULT_RATE as i64 * 60;
/// How long a quiet cloud computer stays awake before it sleeps.
pub(crate) const SLEEP_AFTER_MINUTES: i64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tier {
    Cloud,
    CloudMax,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlanGlance {
    pub tier: Tier,
    pub trial: bool,
    pub included_minutes: i64,
    pub used_minutes: i64,
}

impl Tier {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Tier::Cloud => "Cloud",
            Tier::CloudMax => "Cloud Max",
        }
    }
}

/// Minutes a plan includes each cycle (the trial: its first week).
pub(crate) fn plan_minutes(tier: Tier, trial: bool) -> i64 {
    match (tier, trial) {
        (_, true) => PlanId::CloudTrial,
        (Tier::Cloud, false) => PlanId::Cloud,
        (Tier::CloudMax, false) => PlanId::CloudMax,
    }
    .plan()
    .included_machine_minutes()
}

/// The plan an entitlement offer grants.
fn tier_for_offer(offer: &str) -> Option<(Tier, bool)> {
    match offer {
        plans::CLOUD_MAX_OFFER => Some((Tier::CloudMax, false)),
        plans::CLOUD_TRIAL_OFFER => Some((Tier::Cloud, true)),
        plans::CLOUD_OFFER | plans::LEGACY_OFFER => Some((Tier::Cloud, false)),
        _ => None,
    }
}

/// The active plan in a `CloudBillingEntitlements` answer. Cloud Max wins
/// over Cloud when both are briefly active during a plan change.
pub(crate) fn tier_from_entitlements(data: &Value, now: i64) -> Option<(Tier, bool)> {
    data["entitlements"]
        .as_array()?
        .iter()
        .filter(|ent| {
            matches!(
                ent["status"].as_str(),
                Some("pending" | "active" | "grace" | "trialing")
            ) && ent["paidThrough"].as_i64().is_none_or(|end| end > now)
        })
        .filter_map(|ent| {
            let (tier, trial) = tier_for_offer(ent["offerId"].as_str()?)?;
            Some((tier, trial || ent["status"] == "trialing"))
        })
        .max_by_key(|(tier, _)| *tier == Tier::CloudMax)
}

/// The plan a `CloudBillingSummary` reports, when its period is on
/// machine-minute terms.
pub(crate) fn summary_plan(summary: &Value) -> Option<PlanUsage> {
    serde_json::from_value(summary.get("plan")?.clone()).ok()
}
/// Included and used minutes from a `CloudBillingSummary`. Prefers the
/// summary's plan; otherwise scales the plan's minutes by how much of the
/// included allowance is used (periods billed before machine minutes).
pub(crate) fn minutes_from_summary(summary: &Value, included: i64) -> (i64, i64) {
    if let Some(plan) = summary_plan(summary) {
        let total = plan.included_machine_minutes.max(0);
        return (total, plan.used_machine_minutes.clamp(0, total));
    }
    let allowance = summary["includedProviderCostMicros"].as_i64().unwrap_or(0);
    let spent = summary["includedUsedMicros"].as_i64().unwrap_or(0);
    if allowance <= 0 {
        return (included, 0);
    }
    let used = (included as i128 * spent.clamp(0, allowance) as i128 / allowance as i128) as i64;
    (included, used)
}

impl PlanGlance {
    pub(crate) fn new(tier: Tier, trial: bool, summary: Option<&Value>) -> Self {
        // The summary's plan is authoritative: it knows a trial and Cloud Max
        // even when entitlements only name the subscription's offer.
        let (tier, trial) = match summary.and_then(summary_plan) {
            Some(plan) => (
                if plan.plan_id == PlanId::CloudMax {
                    Tier::CloudMax
                } else {
                    Tier::Cloud
                },
                plan.trial,
            ),
            None => (tier, trial),
        };
        let included = plan_minutes(tier, trial);
        let (included_minutes, used_minutes) = summary
            .map(|s| minutes_from_summary(s, included))
            .unwrap_or((included, 0));
        Self {
            tier,
            trial,
            included_minutes,
            used_minutes,
        }
    }
    pub(crate) fn remaining_minutes(&self) -> i64 {
        (self.included_minutes - self.used_minutes).max(0)
    }
    /// Share of the included minutes still left, for the meter.
    pub(crate) fn fraction_left(&self) -> f32 {
        if self.included_minutes <= 0 {
            return 0.;
        }
        self.remaining_minutes() as f32 / self.included_minutes as f32
    }
    /// "Cloud", "Cloud · trial", "Cloud Max".
    pub(crate) fn label(&self) -> String {
        if self.trial {
            format!("{} · first-week trial", self.tier.name())
        } else {
            self.tier.name().to_owned()
        }
    }
    /// "33,600 of 33,600 minutes left".
    pub(crate) fn minutes_line(&self) -> String {
        format!(
            "{} of {} minutes left",
            crate::cloud_settings::kit::thousands(self.remaining_minutes()),
            crate::cloud_settings::kit::thousands(self.included_minutes)
        )
    }
    /// "About 140 hours on your cloud computer this month."
    pub(crate) fn hours_line(&self) -> String {
        let hours = (self.remaining_minutes() + MINUTES_PER_HOUR / 2) / MINUTES_PER_HOUR;
        format!(
            "About {hours} hour{} on your {DEFAULT_MACHINE} cloud computer {}. Asleep computers use none.",
            if hours == 1 { "" } else { "s" },
            if self.trial {
                "this week"
            } else {
                "this cycle"
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plans_carry_their_minutes() {
        assert_eq!(plan_minutes(Tier::Cloud, false), 33_600);
        assert_eq!(plan_minutes(Tier::Cloud, true), 8_400);
        assert_eq!(plan_minutes(Tier::CloudMax, false), 84_000);
        assert_eq!(33_600 / MINUTES_PER_HOUR, 140);
        assert_eq!(84_000 / MINUTES_PER_HOUR, 350);
    }

    #[test]
    fn entitlements_name_the_plan() {
        let now = 1_000;
        let active = |offer: &str, status: &str| json!({"entitlements":[{"offerId":offer,"status":status,"paidThrough":2_000}]});
        assert_eq!(
            tier_from_entitlements(&active("cloud-starter-v1", "active"), now),
            Some((Tier::Cloud, false))
        );
        assert_eq!(
            tier_from_entitlements(&active("cloud-starter-v1", "trialing"), now),
            Some((Tier::Cloud, true))
        );
        assert_eq!(
            tier_from_entitlements(&active("cloud-starter-trial-v1", "active"), now),
            Some((Tier::Cloud, true))
        );
        assert_eq!(
            tier_from_entitlements(&active("cloud-pro-v1", "active"), now),
            Some((Tier::CloudMax, false))
        );
        assert_eq!(
            tier_from_entitlements(&active("cloud-workspace-standard-v1", "grace"), now),
            Some((Tier::Cloud, false))
        );
        assert_eq!(
            tier_from_entitlements(&active("cloud-pro-v1", "ended"), now),
            None
        );
        let expired =
            json!({"entitlements":[{"offerId":"cloud-pro-v1","status":"active","paidThrough":10}]});
        assert_eq!(tier_from_entitlements(&expired, now), None);
        let both = json!({"entitlements":[
            {"offerId":"cloud-starter-v1","status":"active"},
            {"offerId":"cloud-pro-v1","status":"pending"}
        ]});
        assert_eq!(
            tier_from_entitlements(&both, now),
            Some((Tier::CloudMax, false))
        );
    }

    #[test]
    fn minutes_come_from_the_summary() {
        // Machine-minute periods report the plan directly, as Billing shows it.
        let plan = json!({"plan":{"planId":"cloud-trial","planName":"Cloud trial","priceMicros":1_000_000,"trial":true,"trialEndsAt":5,"renewsAt":5,"includedMachineMinutes":8_400,"usedMachineMinutes":1_200,"remainingMachineMinutes":7_200,"extraMachineMinutes":0,"extraChargeMicros":0,"overageCapMicros":0,"payAsYouGo":false,"usageHold":false,"defaultMachineRate":4,"maxVcpus":4,"maxMemoryMib":16384}});
        assert_eq!(minutes_from_summary(&plan, 1), (8_400, 1_200));
        let glance = PlanGlance::new(Tier::Cloud, false, Some(&plan));
        assert!(glance.trial, "the summary knows the trial");
        assert_eq!(glance.remaining_minutes(), 7_200);
        // Older summaries scale the plan's minutes by allowance spent.
        let money = json!({"includedProviderCostMicros":20_000_000,"includedUsedMicros":5_000_000});
        assert_eq!(minutes_from_summary(&money, 33_600), (33_600, 8_400));
        let none = json!({});
        assert_eq!(minutes_from_summary(&none, 8_400), (8_400, 0));
    }

    #[test]
    fn glance_copy_reads_in_minutes_and_hours() {
        let fresh = PlanGlance::new(Tier::Cloud, false, None);
        assert_eq!(fresh.label(), "Cloud");
        assert_eq!(fresh.minutes_line(), "33,600 of 33,600 minutes left");
        assert!(
            fresh
                .hours_line()
                .starts_with("About 140 hours on your 4 vCPU · 16 GB")
        );
        assert_eq!(fresh.fraction_left(), 1.0);
        let trial = PlanGlance::new(Tier::Cloud, true, None);
        assert_eq!(trial.label(), "Cloud · first-week trial");
        assert!(trial.hours_line().contains("About 35 hours"));
        assert!(trial.hours_line().contains("this week"));
        let max = PlanGlance::new(Tier::CloudMax, false, None);
        assert_eq!(max.label(), "Cloud Max");
        assert_eq!(max.minutes_line(), "84,000 of 84,000 minutes left");
        assert_eq!(
            (DEFAULT_MACHINE, LARGE_MACHINE),
            ("4 vCPU · 16 GB", "8 vCPU · 32 GB")
        );
    }
}
