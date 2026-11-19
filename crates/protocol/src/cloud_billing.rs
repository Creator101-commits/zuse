//! Existing cloud billing wire contracts. Amounts are USD millionths, never floats.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BillingStatus {
    Active,
    Grace,
    BillingHold,
    Ended,
    Manual,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BillingSummary {
    pub currency: String,
    pub status: BillingStatus,
    pub period_start: i64,
    pub period_end: i64,
    pub base_price_micros: i64,
    pub included_provider_cost_micros: i64,
    pub provider_cost_micros: i64,
    pub storage_cost_micros: i64,
    pub included_used_micros: i64,
    pub included_remaining_micros: i64,
    pub overage_provider_cost_micros: i64,
    pub overage_charge_micros: i64,
    pub overage_cap_micros: i64,
    pub markup_basis_points: i64,
    pub current_invoice_estimate_micros: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_provider_reconciled_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_billing_reconciled_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_polar_reconciled_at: Option<i64>,
    pub usage_provisional: bool,
    #[serde(default)]
    pub shared_credits: bool,
    /// Plan allowance and usage in machine minutes. Absent for periods
    /// billed before machine-minute plans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanUsage>,
}
/// What a billing period includes and how much of it is used, in machine
/// minutes (one minute of a 1 vCPU / 4 GB machine). Money stays in micros.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanUsage {
    pub plan_id: plans::PlanId,
    pub plan_name: String,
    pub price_micros: i64,
    pub trial: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trial_ends_at: Option<i64>,
    /// The plan renews (or the trial converts) at the end of the period.
    pub renews_at: i64,
    pub included_machine_minutes: i64,
    pub used_machine_minutes: i64,
    pub remaining_machine_minutes: i64,
    /// Machine minutes beyond the allowance, billed as pay as you go.
    pub extra_machine_minutes: i64,
    pub extra_charge_micros: i64,
    pub overage_cap_micros: i64,
    pub pay_as_you_go: bool,
    /// Cloud computers are paused until the hold clears.
    pub usage_hold: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_reason: Option<HoldReason>,
    /// Machine minutes the default machine uses per awake minute (4).
    pub default_machine_rate: u32,
    pub max_vcpus: u32,
    pub max_memory_mib: u32,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HoldReason {
    /// The trial allowance is used up; starting the plan restores access.
    TrialAllowanceUsed,
    /// This card or account already had a trial.
    TrialUnavailable,
    /// Pay-as-you-go usage reached the monthly cap.
    CapReached,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BillingUsageItem {
    pub entry_id: String,
    pub resource_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_kind: Option<String>,
    pub resource_id: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_execution_id: Option<String>,
    pub started_at: i64,
    pub ended_at: i64,
    pub vcpu_count: i64,
    pub memory_mib: i64,
    pub provider_cost_micros: i64,
    pub status: String,
    /// Time the computer was awake in this entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awake_ms: Option<i64>,
    /// Machine minutes the computer used per awake minute (4 for 4 vCPU / 16 GB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_rate: Option<u32>,
    /// Machine minutes this entry used from the allowance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_minutes: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BillingUsagePage {
    pub items: Vec<BillingUsageItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingCapRequest {
    pub overage_cap_micros: i64,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BillingUsageRequest {
    pub cursor: Option<String>,
    pub limit: Option<u16>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingCheckoutRequest {
    pub offer_id: String,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

pub const PREPAID_CREDIT_AMOUNTS: [i64; 4] = [1000, 2500, 5000, 10000];
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepaidBalance {
    pub available: bool,
    pub credit_cents: i64,
    pub debit_cents: i64,
    pub currency: String,
    /// Empty for legacy invoice credit; shared-credits denotes the compute/model USD wallet.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub funding: String,
    /// Legacy invoice credit remains spendable only by invoices, never copied into the shared wallet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_credit_cents: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepaidCheckoutRequest {
    pub amount_cents: i64,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// Cloud plans are sold in machine minutes: one minute of a 1 vCPU / 4 GB
/// machine. A machine uses one more machine minute per awake minute for each
/// additional 4 GB of memory, so the default 4 vCPU / 16 GB machine uses 4
/// and an 8 vCPU / 32 GB machine 8. Asleep machines use none. One credit
/// balance in dollars backs every plan at `$20 / 33,600` machine minutes.
pub mod plans {
    use serde::{Deserialize, Serialize};

    pub const DEFAULT_VCPUS: u32 = 4;
    pub const DEFAULT_MEMORY_MIB: u32 = 16 * 1024;
    pub const LARGE_VCPUS: u32 = 8;
    pub const LARGE_MEMORY_MIB: u32 = 32 * 1024;
    /// Memory that one machine minute per minute pays for.
    pub const UNIT_MEMORY_MIB: u32 = 4 * 1024;
    /// The rate: `RATE_MICROS` buys `RATE_MACHINE_MINUTES`.
    pub const RATE_MICROS: i64 = 20_000_000;
    pub const RATE_MACHINE_MINUTES: i64 = 33_600;
    /// Machine minutes per awake minute of the default machine.
    pub const DEFAULT_RATE: u32 = 4;
    const MINUTE_MS: i128 = 60_000;

    /// Offer ids sold at checkout.
    pub const CLOUD_OFFER: &str = "cloud-starter-v1";
    pub const CLOUD_MAX_OFFER: &str = "cloud-pro-v1";
    /// Cloud with the first week for $1; the subscription itself is Cloud.
    pub const CLOUD_TRIAL_OFFER: &str = "cloud-starter-trial-v1";
    /// The original $40 plan. Existing subscribers get Cloud until migrated.
    pub const LEGACY_OFFER: &str = "cloud-workspace-standard-v1";
    /// Every offer whose subscription grants cloud computers.
    pub const CLOUD_OFFERS: [&str; 3] = [CLOUD_OFFER, CLOUD_MAX_OFFER, LEGACY_OFFER];

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub enum PlanId {
        Cloud,
        CloudTrial,
        CloudMax,
        /// The original $40 subscription, with Cloud's allowance.
        Legacy,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Plan {
        pub id: PlanId,
        pub name: &'static str,
        pub price_micros: i64,
        pub included_credit_micros: i64,
        pub max_vcpus: u32,
        pub max_memory_mib: u32,
        /// Usage past the allowance is billed up to the monthly cap.
        pub pay_as_you_go: bool,
    }
    impl PlanId {
        pub const ALL: [Self; 4] = [Self::Cloud, Self::CloudTrial, Self::CloudMax, Self::Legacy];
        pub fn as_str(self) -> &'static str {
            match self {
                Self::Cloud => "cloud",
                Self::CloudTrial => "cloud-trial",
                Self::CloudMax => "cloud-max",
                Self::Legacy => "legacy",
            }
        }
        pub fn parse(value: &str) -> Option<Self> {
            Self::ALL.into_iter().find(|id| id.as_str() == value)
        }
        /// The plan an offer grants. A trialing Cloud subscription is the trial.
        pub fn for_offer(offer: &str, trialing: bool) -> Option<Self> {
            match offer {
                CLOUD_OFFER if trialing => Some(Self::CloudTrial),
                CLOUD_OFFER => Some(Self::Cloud),
                CLOUD_MAX_OFFER => Some(Self::CloudMax),
                LEGACY_OFFER => Some(Self::Legacy),
                _ => None,
            }
        }
        pub fn plan(self) -> Plan {
            let (name, price, credit, large, overage) = match self {
                Self::Cloud => ("Cloud", 20_000_000, 20_000_000, false, true),
                // $1 for the first week, a quarter of Cloud's allowance, no overage.
                Self::CloudTrial => ("Cloud trial", 1_000_000, 5_000_000, false, false),
                Self::CloudMax => ("Cloud Max", 50_000_000, 50_000_000, true, true),
                Self::Legacy => ("Cloud", 40_000_000, 20_000_000, false, true),
            };
            Plan {
                id: self,
                name,
                price_micros: price,
                included_credit_micros: credit,
                max_vcpus: if large { LARGE_VCPUS } else { DEFAULT_VCPUS },
                max_memory_mib: if large {
                    LARGE_MEMORY_MIB
                } else {
                    DEFAULT_MEMORY_MIB
                },
                pay_as_you_go: overage,
            }
        }
    }
    impl Plan {
        pub fn included_machine_minutes(&self) -> i64 {
            credit_machine_minutes(self.included_credit_micros)
        }
        pub fn allows(&self, vcpus: u32, memory_mib: u32) -> bool {
            vcpus <= self.max_vcpus && memory_mib <= self.max_memory_mib
        }
    }
    /// Machine minutes per awake minute: one per 4 GB of memory, rounded up,
    /// at least one. 4 vCPU / 16 GB is 4, 8 vCPU / 32 GB is 8.
    pub fn machine_rate(memory_mib: u32) -> u32 {
        memory_mib.div_ceil(UNIT_MEMORY_MIB).max(1)
    }
    /// Machine minutes used by awake time, rounded down to whole minutes for display.
    pub fn machine_minutes(awake_ms: i64, rate: u32) -> i64 {
        let value = i128::from(awake_ms.max(0)) * i128::from(rate) / MINUTE_MS;
        i64::try_from(value).unwrap_or(i64::MAX)
    }
    /// Credit used by awake time at `awake × rate × $20 / 33,600`, rounded to
    /// the nearest micro. This is what the allowance and overage are counted in.
    pub fn machine_time_micros(awake_ms: i64, rate: u32) -> i64 {
        let numerator = i128::from(awake_ms.max(0)) * i128::from(rate) * i128::from(RATE_MICROS);
        let denominator = MINUTE_MS * i128::from(RATE_MACHINE_MINUTES);
        i64::try_from((numerator + denominator / 2) / denominator).unwrap_or(i64::MAX)
    }
    /// Machine minutes a credit amount buys, rounded to the nearest minute.
    pub fn credit_machine_minutes(micros: i64) -> i64 {
        let value = i128::from(micros.max(0)) * i128::from(RATE_MACHINE_MINUTES);
        let rate = i128::from(RATE_MICROS);
        i64::try_from((value + rate / 2) / rate).unwrap_or(i64::MAX)
    }
    /// Price of `count` machine minutes in micros, rounded.
    pub fn machine_minutes_micros(count: i64) -> i64 {
        let value = i128::from(count.max(0)) * i128::from(RATE_MICROS);
        let rate = i128::from(RATE_MACHINE_MINUTES);
        i64::try_from((value + rate / 2) / rate).unwrap_or(i64::MAX)
    }
    /// Awake hours of a machine with `rate` that `machine_minutes` cover, in tenths.
    pub fn hours_tenths(machine_minutes: i64, rate: u32) -> i64 {
        machine_minutes.max(0) / i64::from(rate.max(1)) / 6
    }
    /// A size option for new cloud computers.
    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "camelCase")]
    pub struct SizeOption {
        pub id: String,
        pub label: String,
        pub vcpus: u32,
        pub memory_mib: u32,
        /// Machine minutes per awake minute.
        pub machine_rate: u32,
        /// The account's plan allows this size.
        pub available: bool,
        /// Plan to upgrade to when not available.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub requires_plan: Option<PlanId>,
    }
    /// Size ids the server allocates, with their resources.
    pub const DEFAULT_SIZE: &str = "default";
    pub const LARGE_SIZE: &str = "large";
    pub fn size_resources(id: &str) -> Option<(u32, u32)> {
        match id {
            DEFAULT_SIZE => Some((DEFAULT_VCPUS, DEFAULT_MEMORY_MIB)),
            LARGE_SIZE => Some((LARGE_VCPUS, LARGE_MEMORY_MIB)),
            _ => None,
        }
    }
    pub fn size_label(id: &str) -> Option<&'static str> {
        match id {
            DEFAULT_SIZE => Some("Standard · 4 vCPU · 16 GB"),
            LARGE_SIZE => Some("Large · 8 vCPU · 32 GB"),
            _ => None,
        }
    }
    /// A plan shown before subscribing or when changing plans.
    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "camelCase")]
    pub struct PlanOffer {
        pub offer_id: String,
        pub plan_id: PlanId,
        pub name: String,
        pub price_micros: i64,
        pub included_machine_minutes: i64,
        pub max_vcpus: u32,
        pub max_memory_mib: u32,
        pub pay_as_you_go: bool,
        /// Checkout can sell this offer right now.
        pub available: bool,
    }
    pub fn offer(offer_id: &str, plan: PlanId, available: bool) -> PlanOffer {
        let p = plan.plan();
        PlanOffer {
            offer_id: offer_id.into(),
            plan_id: plan,
            name: p.name.into(),
            price_micros: p.price_micros,
            included_machine_minutes: p.included_machine_minutes(),
            max_vcpus: p.max_vcpus,
            max_memory_mib: p.max_memory_mib,
            pay_as_you_go: p.pay_as_you_go,
            available,
        }
    }
    /// Plans, trial eligibility and the enterprise contact, returned with
    /// entitlements so clients can show plans before subscribing.
    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "camelCase")]
    pub struct PlanCatalog {
        pub plans: Vec<PlanOffer>,
        pub trial_available: bool,
        pub trial_days: u32,
        pub trial_price_micros: i64,
        pub trial_machine_minutes: i64,
        pub enterprise_contact_url: String,
    }
    pub const TRIAL_DAYS: u32 = 7;
    /// Used when no enterprise contact is configured. Placeholder until the
    /// owner provides a booking link (`ZUSE_ENTERPRISE_CONTACT_URL`).
    pub const ENTERPRISE_CONTACT_FALLBACK: &str = "mailto:hello@zuse.sh?subject=Zuse%20Enterprise";
    pub fn catalog(
        trial_available: bool,
        available: &dyn Fn(&str) -> bool,
        contact: &str,
    ) -> PlanCatalog {
        PlanCatalog {
            plans: vec![
                offer(CLOUD_OFFER, PlanId::Cloud, available(CLOUD_OFFER)),
                offer(
                    CLOUD_MAX_OFFER,
                    PlanId::CloudMax,
                    available(CLOUD_MAX_OFFER),
                ),
            ],
            trial_available: trial_available && available(CLOUD_TRIAL_OFFER),
            trial_days: TRIAL_DAYS,
            trial_price_micros: PlanId::CloudTrial.plan().price_micros,
            trial_machine_minutes: PlanId::CloudTrial.plan().included_machine_minutes(),
            enterprise_contact_url: contact.into(),
        }
    }
    /// User-visible text never names the infrastructure behind cloud computers.
    pub fn names_infrastructure(text: &str) -> bool {
        let lower = text.to_ascii_lowercase();
        ["boxd", "boat", "prized", "e2b", "daytona"]
            .iter()
            .any(|name| {
                lower.match_indices(name).any(|(at, _)| {
                    let before = lower[..at].chars().next_back();
                    let after = lower[at + name.len()..].chars().next();
                    !before.is_some_and(|c| c.is_ascii_alphanumeric())
                        && !after.is_some_and(|c| c.is_ascii_alphanumeric())
                })
            })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn plans_are_sold_in_machine_minutes() {
            assert_eq!(PlanId::Cloud.plan().included_machine_minutes(), 33_600);
            assert_eq!(PlanId::CloudMax.plan().included_machine_minutes(), 84_000);
            assert_eq!(PlanId::CloudTrial.plan().included_machine_minutes(), 8_400);
            assert_eq!(PlanId::Legacy.plan().included_machine_minutes(), 33_600);
            // 33,600 machine minutes are 140 hours on the default machine.
            assert_eq!(hours_tenths(33_600, DEFAULT_RATE), 1400);
            assert_eq!(hours_tenths(84_000, DEFAULT_RATE), 3500);
            assert_eq!(hours_tenths(8_400, DEFAULT_RATE), 350);
            assert!(!PlanId::CloudTrial.plan().pay_as_you_go);
            assert!(PlanId::Cloud.plan().allows(4, 16384));
            assert!(!PlanId::Cloud.plan().allows(8, 32768));
            assert!(PlanId::CloudMax.plan().allows(8, 32768));
            assert!(!PlanId::Legacy.plan().allows(8, 32768));
        }
        #[test]
        fn awake_time_times_machine_rate() {
            assert_eq!(machine_rate(16384), 4);
            assert_eq!(machine_rate(32768), 8);
            assert_eq!(machine_rate(4096), 1);
            assert_eq!(machine_rate(2048), 1);
            assert_eq!(machine_rate(6144), 2);
            // 140 awake hours of the default machine use exactly $20.
            assert_eq!(machine_time_micros(140 * 3_600_000, 4), 20_000_000);
            assert_eq!(machine_minutes(140 * 3_600_000, 4), 33_600);
            // The large machine uses machine minutes twice as fast.
            assert_eq!(machine_time_micros(70 * 3_600_000, 8), 20_000_000);
            assert_eq!(machine_time_micros(60_000, 1), 595);
            assert_eq!(machine_time_micros(0, 8), 0);
            assert_eq!(credit_machine_minutes(20_000_000), 33_600);
            assert_eq!(machine_minutes_micros(33_600), 20_000_000);
            assert_eq!(machine_minutes_micros(2_880), 1_714_286);
        }
        #[test]
        fn offers_map_to_plans() {
            assert_eq!(
                PlanId::for_offer(CLOUD_OFFER, true),
                Some(PlanId::CloudTrial)
            );
            assert_eq!(PlanId::for_offer(LEGACY_OFFER, false), Some(PlanId::Legacy));
            assert_eq!(PlanId::for_offer("persistent-standard-v1", false), None);
            for id in PlanId::ALL {
                assert_eq!(PlanId::parse(id.as_str()), Some(id));
                assert!(!names_infrastructure(id.plan().name));
            }
            let c = catalog(true, &|_| true, ENTERPRISE_CONTACT_FALLBACK);
            assert_eq!(c.trial_machine_minutes, 8_400);
            assert_eq!(c.plans[1].included_machine_minutes, 84_000);
            assert!(!catalog(true, &|o| o != CLOUD_TRIAL_OFFER, "").trial_available);
        }
        #[test]
        fn infrastructure_names_are_detected_as_words() {
            assert!(names_infrastructure("Boxd is not enabled"));
            assert!(names_infrastructure("cloud-boxd"));
            assert!(!names_infrastructure("Keyboard and dashboard"));
            assert!(!names_infrastructure("Cloud · Running"));
        }
    }
}
