//! Machine-minute plans: period terms, the allowance, pay as you go and holds.
//!
//! A plan period counts usage as credit at the list rate (`$20 / 33,600`
//! machine minutes), not as provider cost: awake time × machine rate. The
//! allowance, the pay-as-you-go charge and the cap all use that credit, with
//! no markup, so an extra machine minute always costs the same as an included
//! one. Provider cost is still recorded beside it for margin reporting.
use crate::error::ApiError;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use zuse_protocol::cloud_billing::{
    HoldReason, PlanUsage,
    plans::{self, PlanId},
};

/// `price_catalog_version` of machine-minute periods.
pub(crate) const CATALOG_VERSION: &str = "machine-minutes-v1";
/// Ledger kind of machine time valued at the list rate.
pub(crate) const MACHINE_TIME: &str = "machine-time";
/// Ledger kinds drawing on the shared credit balance. Hosted model usage is
/// added here as its own kind; no schema change is needed.
pub(crate) const CREDIT_USAGE_KINDS: [&str; 1] = [MACHINE_TIME];
/// Default monthly pay-as-you-go cap.
pub(crate) const DEFAULT_CAP_MICROS: i64 = 25_000_000;

/// The plan of a machine-minute period; `None` for periods with original terms.
pub(crate) fn period_plan(period: &Value) -> Option<PlanId> {
    if period["price_catalog_version"] != CATALOG_VERSION {
        return None;
    }
    PlanId::parse(period["plan_id"].as_str()?)
}

/// Terms written for a new period of `plan`: base price, included credit and markup.
pub(crate) fn terms(plan: PlanId) -> (i64, i64, i64) {
    let p = plan.plan();
    (p.price_micros, p.included_credit_micros, 0)
}

/// Whether usage `used` (credit micros) holds the period. A plan without pay
/// as you go stops at its allowance; otherwise the overage stops at the cap.
pub(crate) fn hold(
    plan: PlanId,
    included: i64,
    cap: i64,
    used: i64,
    trial_rejected: bool,
) -> Option<HoldReason> {
    if trial_rejected && plan == PlanId::CloudTrial {
        return Some(HoldReason::TrialUnavailable);
    }
    if !plan.plan().pay_as_you_go {
        return (used >= included).then_some(HoldReason::TrialAllowanceUsed);
    }
    let overage = (used - included).max(0);
    (overage > 0 && overage >= cap).then_some(HoldReason::CapReached)
}

fn reason_str(reason: HoldReason) -> &'static str {
    match reason {
        HoldReason::TrialAllowanceUsed => "trial-allowance-used",
        HoldReason::TrialUnavailable => "trial-unavailable",
        HoldReason::CapReached => "cap-reached",
    }
}
fn parse_reason(value: &str) -> Option<HoldReason> {
    match value {
        "trial-allowance-used" => Some(HoldReason::TrialAllowanceUsed),
        "trial-unavailable" => Some(HoldReason::TrialUnavailable),
        "cap-reached" => Some(HoldReason::CapReached),
        _ => None,
    }
}

/// Credit used and pay-as-you-go charge settled in a period.
pub(crate) async fn usage_totals(
    db: &mut sqlx::PgConnection,
    period: &str,
) -> Result<(i64, i64), ApiError> {
    Ok(sqlx::query_as("SELECT COALESCE(SUM(amount_micros) FILTER(WHERE kind=ANY($2)),0)::bigint,COALESCE(SUM(amount_micros) FILTER(WHERE kind='overage-charge'),0)::bigint FROM api_cloud_billing_ledger WHERE period_id=$1")
        .bind(period).bind(&CREDIT_USAGE_KINDS[..]).fetch_one(db).await?)
}

/// Recompute a machine-minute period's usage hold from its ledger, cap and
/// trial state. Writes only on change; turning a hold on wakes the runtime
/// worker (migration 0054), which pauses the account's computers.
pub(crate) async fn refresh_hold(
    tx: &mut Transaction<'_, Postgres>,
    period_id: &str,
) -> Result<Option<HoldReason>, ApiError> {
    let period: Option<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(p) FROM api_cloud_billing_periods p WHERE period_id=$1",
    )
    .bind(period_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(period) = period else {
        return Ok(None);
    };
    let Some(plan) = period_plan(&period) else {
        return Ok(None);
    };
    if period["shared_credit_account"] == true {
        // Embedded billing holds on its wallet balance instead.
        return Ok(None);
    }
    let (used, _) = usage_totals(&mut **tx, period_id).await?;
    let rejected = if plan == PlanId::CloudTrial {
        let state: Option<String> = sqlx::query_scalar(
            "SELECT state FROM api_cloud_trials WHERE account_id=$1 AND subscription_id=$2",
        )
        .bind(period["account_id"].as_str())
        .bind(period["provider_subscription_id"].as_str())
        .fetch_optional(&mut **tx)
        .await?;
        state.as_deref() == Some("rejected")
    } else {
        false
    };
    let reason = hold(
        plan,
        period["included_provider_cost_micros"]
            .as_i64()
            .unwrap_or(0),
        period["overage_cap_micros"].as_i64().unwrap_or(0),
        used,
        rejected,
    );
    sqlx::query("UPDATE api_cloud_billing_periods SET usage_hold=$2,hold_reason=$3 WHERE period_id=$1 AND (usage_hold IS DISTINCT FROM $2 OR hold_reason IS DISTINCT FROM $3)")
        .bind(period_id)
        .bind(reason.is_some())
        .bind(reason.map(reason_str))
        .execute(&mut **tx)
        .await?;
    Ok(reason)
}

/// The plan, allowance and usage of a machine-minute period, in machine minutes.
pub(crate) fn plan_usage(period: &Value, used: i64, charged: i64) -> Option<PlanUsage> {
    let id = period_plan(period)?;
    let plan = id.plan();
    let included = period["included_provider_cost_micros"].as_i64()?;
    let cap = period["overage_cap_micros"].as_i64()?;
    let end = period["period_end"].as_i64()?;
    let included_minutes = plans::credit_machine_minutes(included);
    let used_minutes = plans::credit_machine_minutes(used);
    let extra = (used - included).max(0);
    let held = period["usage_hold"] == true;
    Some(PlanUsage {
        plan_id: id,
        plan_name: plan.name.into(),
        price_micros: period["base_price_micros"]
            .as_i64()
            .unwrap_or(plan.price_micros),
        trial: id == PlanId::CloudTrial,
        trial_ends_at: (id == PlanId::CloudTrial).then_some(end),
        renews_at: end,
        included_machine_minutes: included_minutes,
        used_machine_minutes: used_minutes,
        remaining_machine_minutes: (included_minutes - used_minutes).max(0),
        extra_machine_minutes: plans::credit_machine_minutes(extra),
        extra_charge_micros: charged.max(0),
        overage_cap_micros: cap,
        pay_as_you_go: plan.pay_as_you_go,
        usage_hold: held,
        hold_reason: held
            .then(|| period["hold_reason"].as_str().and_then(parse_reason))
            .flatten(),
        default_machine_rate: plans::DEFAULT_RATE,
        max_vcpus: plan.max_vcpus,
        max_memory_mib: plan.max_memory_mib,
    })
}

/// The plan of the account's current machine-minute period, if any.
pub(crate) async fn current_plan<'e, E: sqlx::Executor<'e, Database = Postgres>>(
    db: E,
    owner: &str,
    now: i64,
) -> Result<Option<PlanId>, ApiError> {
    let period: Option<Value> = sqlx::query_scalar("SELECT to_jsonb(p) FROM api_cloud_billing_periods p WHERE account_id=$1 AND period_start<=$2 AND period_end>$2 ORDER BY period_start DESC LIMIT 1")
        .bind(owner)
        .bind(now)
        .fetch_optional(db)
        .await?;
    Ok(period.as_ref().and_then(period_plan))
}

/// Size limit for new computers: the current plan's, or Cloud's for accounts
/// on original terms (every paid account gets the default machine).
pub(crate) fn size_allowed(plan: Option<PlanId>, size: &str) -> Result<(), ApiError> {
    let Some((vcpus, memory)) = plans::size_resources(size) else {
        return Ok(());
    };
    if plan.unwrap_or(PlanId::Cloud).plan().allows(vcpus, memory) {
        Ok(())
    } else {
        Err(ApiError::forbidden("cloud_size_requires_cloud_max"))
    }
}

#[cfg(test)]
mod pg_tests;
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn allowance_then_pay_as_you_go_up_to_the_cap() {
        let cloud = PlanId::Cloud;
        assert_eq!(hold(cloud, 20_000_000, 25_000_000, 19_999_999, false), None);
        assert_eq!(hold(cloud, 20_000_000, 25_000_000, 44_999_999, false), None);
        assert_eq!(
            hold(cloud, 20_000_000, 25_000_000, 45_000_000, false),
            Some(HoldReason::CapReached)
        );
        // A zero cap stops at the allowance.
        assert_eq!(
            hold(cloud, 20_000_000, 0, 20_000_001, false),
            Some(HoldReason::CapReached)
        );
        assert_eq!(hold(cloud, 20_000_000, 0, 20_000_000, false), None);
    }
    #[test]
    fn trial_has_no_pay_as_you_go() {
        let trial = PlanId::CloudTrial;
        assert_eq!(hold(trial, 5_000_000, 25_000_000, 4_999_999, false), None);
        assert_eq!(
            hold(trial, 5_000_000, 25_000_000, 5_000_000, false),
            Some(HoldReason::TrialAllowanceUsed)
        );
        assert_eq!(
            hold(trial, 5_000_000, 25_000_000, 0, true),
            Some(HoldReason::TrialUnavailable)
        );
    }
    #[test]
    fn usage_is_reported_in_machine_minutes() {
        let period = json!({"price_catalog_version":CATALOG_VERSION,"plan_id":"cloud","included_provider_cost_micros":20_000_000,"overage_cap_micros":25_000_000,"base_price_micros":20_000_000,"period_end":99,"usage_hold":false,"hold_reason":"cap-reached"});
        let usage = plan_usage(&period, 21_714_286, 1_714_286).unwrap();
        assert_eq!(usage.included_machine_minutes, 33_600);
        assert_eq!(usage.used_machine_minutes, 36_480);
        assert_eq!(usage.remaining_machine_minutes, 0);
        assert_eq!(usage.extra_machine_minutes, 2_880);
        assert_eq!(usage.extra_charge_micros, 1_714_286);
        assert_eq!(usage.hold_reason, None, "a cleared hold has no reason");
        assert!(
            plan_usage(
                &json!({"price_catalog_version":"provider-compute-v1"}),
                0,
                0
            )
            .is_none()
        );
    }
    #[test]
    fn large_machines_need_cloud_max() {
        assert!(size_allowed(Some(PlanId::Cloud), "default").is_ok());
        assert_eq!(
            size_allowed(Some(PlanId::Cloud), "large").unwrap_err().code,
            "cloud_size_requires_cloud_max"
        );
        assert!(size_allowed(None, "large").is_err());
        assert!(size_allowed(Some(PlanId::Legacy), "large").is_err());
        assert!(size_allowed(Some(PlanId::CloudMax), "large").is_ok());
    }
}
