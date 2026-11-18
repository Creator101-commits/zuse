//! Daily compute-cost anomaly check over each provider's own usage report.
//!
//! Runs once per closed UTC day (two hours after it ends, so the report is
//! complete) on the same schedule table as the machine pass. It reads the
//! provider report directly rather than `api_cloud_billing_usage`, so it works
//! whether or not billing settlement is enabled in an environment. Only
//! machines this environment owns (see the module docs) are considered.
use super::*;
use std::collections::BTreeMap;

const DAY: i64 = 86_400_000;
/// A closed day's report is read this long after the day ends.
const REPORT_DELAY_MS: i64 = 2 * 60 * 60_000;
/// Running at least this long in one day is flagged even with activity.
const ALL_DAY_SECONDS: u64 = 22 * 60 * 60;
const TRAILING_DAYS: i64 = 7;

/// The closed UTC day `[start, end)` whose report is read at `now`.
pub(super) fn report_day(now: i64) -> (i64, i64) {
    let end = now.saturating_sub(REPORT_DELAY_MS).div_euclid(DAY) * DAY;
    (end - DAY, end)
}
/// When the next day's report is due.
pub(super) fn next_report_at(now: i64) -> i64 {
    report_day(now).1 + DAY + REPORT_DELAY_MS
}

/// One owned machine's usage for the day, with its attribution.
#[derive(Clone, Debug)]
pub struct MachineDay {
    pub machine_id: String,
    pub running_seconds: u64,
    pub cost_micros: i64,
    pub account: Option<String>,
    /// The latest recorded user activity on its workspace, if any.
    pub last_activity_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finding {
    /// Awake for hours in a day with no activity since before it began.
    AwakeWithoutActivity {
        machine_id: String,
        running_seconds: u64,
    },
    /// Awake nearly the whole day, activity or not.
    AwakeAllDay {
        machine_id: String,
        running_seconds: u64,
    },
    /// An account's day cost far above its trailing daily average.
    Spike {
        account: String,
        cost_micros: i64,
        trailing_average_micros: i64,
    },
}

/// Pure evaluation of one day. `trailing` is each account's total cost over
/// the previous seven days.
pub fn evaluate(
    day_start: i64,
    today: &[MachineDay],
    trailing: &BTreeMap<String, i64>,
    config: &Config,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let awake_seconds = u64::try_from(config.awake_alert_ms / 1000).unwrap_or(u64::MAX);
    let mut accounts: BTreeMap<&str, i64> = BTreeMap::new();
    for machine in today {
        if machine.running_seconds >= ALL_DAY_SECONDS {
            findings.push(Finding::AwakeAllDay {
                machine_id: machine.machine_id.clone(),
                running_seconds: machine.running_seconds,
            });
        } else if machine.running_seconds >= awake_seconds
            && machine.last_activity_at.is_none_or(|at| at < day_start)
        {
            findings.push(Finding::AwakeWithoutActivity {
                machine_id: machine.machine_id.clone(),
                running_seconds: machine.running_seconds,
            });
        }
        if let Some(account) = &machine.account {
            *accounts.entry(account).or_default() += machine.cost_micros;
        }
    }
    for (account, cost) in accounts {
        let average = trailing.get(account).copied().unwrap_or(0) / TRAILING_DAYS;
        if cost >= config.spike_floor_micros && cost > average.saturating_mul(config.spike_factor) {
            findings.push(Finding::Spike {
                account: account.to_owned(),
                cost_micros: cost,
                trailing_average_micros: average,
            });
        }
    }
    findings
}

/// One cost check's outcome, saved on the schedule row.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostReport {
    pub day_start: i64,
    pub machines: usize,
    pub findings: usize,
    pub errors: Vec<String>,
}

impl Facts {
    /// Whether this environment owns the machine (never true for foreign ones).
    pub(super) fn owned(&self) -> bool {
        self.workspace.is_some()
            || self.infrastructure.is_some()
            || self.in_flight.is_some()
            || self.builder.is_some()
            || self.owner.is_some()
    }
}

impl RuntimeControl {
    /// The daily cost check when due. `Ok(None)`: disabled, not installed,
    /// not due or held by another worker.
    pub async fn cost_anomalies_once(
        &self,
        registry: &Registry,
        config: &Config,
        now: i64,
    ) -> Result<Option<CostReport>, ApiError> {
        if config.mode == Mode::Off
            || !crate::db::schema::present(&self.pool, SCHEDULE_PRESENT).await?
        {
            return Ok(None);
        }
        let Some(token) = self.claim_reconciliation(COST_TASK, now).await? else {
            return Ok(None);
        };
        match self.check_costs(registry, config, now).await {
            Ok(report) => {
                self.finish_reconciliation(
                    COST_TASK,
                    &token,
                    next_report_at(now),
                    serde_json::to_value(&report).ok(),
                    clock(),
                )
                .await?;
                Ok(Some(report))
            }
            Err(error) => {
                let retry = clock().max(now).saturating_add(60 * 60_000);
                self.finish_reconciliation(COST_TASK, &token, retry, None, clock())
                    .await?;
                Err(error)
            }
        }
    }

    async fn check_costs(
        &self,
        registry: &Registry,
        config: &Config,
        now: i64,
    ) -> Result<CostReport, ApiError> {
        let (start, end) = report_day(now);
        let mut report = CostReport {
            day_start: start,
            ..CostReport::default()
        };
        for id in registry.configured_ids() {
            let Some(provider) = registry.get(id) else {
                continue;
            };
            let usage = |from, to| {
                let provider = provider.clone();
                async move { tokio::time::timeout(LIST_TIMEOUT, provider.usage_report(from, to)).await }
            };
            let today = match usage(start, end).await {
                Ok(Ok(Some(rows))) => rows,
                Ok(Ok(None)) => continue,
                _ => {
                    tracing::warn!(provider = id.as_str(), "Provider usage report deferred");
                    report.errors.push(format!("{}:usage", id.as_str()));
                    continue;
                }
            };
            let trailing = match usage(start - TRAILING_DAYS * DAY, start).await {
                Ok(Ok(Some(rows))) => rows,
                _ => {
                    report
                        .errors
                        .push(format!("{}:trailing-usage", id.as_str()));
                    Vec::new()
                }
            };
            let mut ids: Vec<String> = today
                .iter()
                .chain(&trailing)
                .map(|row| row.machine_id.clone())
                .collect();
            ids.sort();
            ids.dedup();
            let pairs: Vec<(String, String)> =
                ids.iter().map(|id| (id.clone(), String::new())).collect();
            let facts: HashMap<String, Facts> = ids
                .iter()
                .cloned()
                .zip(self.machine_facts(id, &pairs, config, now).await?)
                .collect();
            let owned = |machine: &str| facts.get(machine).filter(|f| f.owned());
            let days: Vec<MachineDay> = today
                .iter()
                .filter_map(|row| {
                    let facts = owned(&row.machine_id)?;
                    let attribution = facts.attribution();
                    Some(MachineDay {
                        machine_id: row.machine_id.clone(),
                        running_seconds: row.running_seconds,
                        cost_micros: row.cost_micros,
                        account: attribution.as_ref().and_then(|a| a.1.clone()),
                        last_activity_at: attribution.and_then(|a| a.2),
                    })
                })
                .collect();
            let mut previous: BTreeMap<String, i64> = BTreeMap::new();
            for row in &trailing {
                if let Some(account) = owned(&row.machine_id)
                    .and_then(Facts::attribution)
                    .and_then(|a| a.1)
                {
                    *previous.entry(account).or_default() += row.cost_micros;
                }
            }
            report.machines += days.len();
            let day = chrono::DateTime::from_timestamp_millis(start)
                .map(|at| at.format("%Y-%m-%d").to_string())
                .unwrap_or_default();
            for finding in evaluate(start, &days, &previous, config) {
                report.findings += 1;
                let (kind, subject, summary, details) = match &finding {
                    Finding::AwakeWithoutActivity {
                        machine_id,
                        running_seconds,
                    } => (
                        "cost:awake-without-activity",
                        format!("{}:{machine_id}:{day}", id.as_str()),
                        format!(
                            "{} machine {machine_id} ran {:.1} h on {day} with no activity",
                            id.as_str(),
                            *running_seconds as f64 / 3600.0
                        ),
                        json!({"runningSeconds":running_seconds,"day":day}),
                    ),
                    Finding::AwakeAllDay {
                        machine_id,
                        running_seconds,
                    } => (
                        "cost:awake-all-day",
                        format!("{}:{machine_id}:{day}", id.as_str()),
                        format!(
                            "{} machine {machine_id} ran {:.1} h on {day}",
                            id.as_str(),
                            *running_seconds as f64 / 3600.0
                        ),
                        json!({"runningSeconds":running_seconds,"day":day}),
                    ),
                    Finding::Spike {
                        account,
                        cost_micros,
                        trailing_average_micros,
                    } => (
                        "cost:account-spike",
                        format!("{}:{account}:{day}", id.as_str()),
                        format!(
                            "Account {account} spent ${:.2} on {} compute on {day} (7-day daily average ${:.2})",
                            *cost_micros as f64 / 1e6,
                            id.as_str(),
                            *trailing_average_micros as f64 / 1e6
                        ),
                        json!({"costMicros":cost_micros,"trailingAverageMicros":trailing_average_micros,"day":day}),
                    ),
                };
                self.record_alert(kind, &subject, &summary, details, now)
                    .await?;
            }
        }
        tracing::info!(
            day_start = report.day_start,
            machines = report.machines,
            findings = report.findings,
            "Compute cost check finished"
        );
        Ok(report)
    }
}
