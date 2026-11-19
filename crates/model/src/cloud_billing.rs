//! Exact money conversion shared by the billing controls on every client.
use zuse_protocol::cloud_billing::plans::CLOUD_OFFERS;
pub const MAX_SAFE_MICROS: i64 = 9_007_199_254_740_991;
pub fn cap_micros(input: &str) -> Option<i64> {
    let mut parts = input.trim().split('.');
    let whole = parts.next()?;
    let fraction = parts.next().unwrap_or("");
    if parts.next().is_some()
        || whole.is_empty()
        || fraction.len() > 6
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let whole = whole.parse::<i64>().ok()?.checked_mul(1_000_000)?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<i64>()
            .ok()?
            .checked_mul(10_i64.pow(6 - fraction.len() as u32))?
    };
    whole
        .checked_add(fraction)
        .filter(|n| *n <= MAX_SAFE_MICROS)
}
pub fn cap_dollars(micros: i64) -> String {
    let whole = micros / 1_000_000;
    let fraction = micros % 1_000_000;
    if fraction == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{:06}", fraction)
            .trim_end_matches('0')
            .into()
    }
}
pub fn usd(micros: i64) -> String {
    let value = i128::from(micros);
    let cents = (value.abs() + 5000) / 10000;
    format!(
        "{}${}.{:02}",
        if value < 0 { "−" } else { "" },
        cents / 100,
        cents % 100
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn monthly_cap_uses_exact_decimal_micros() {
        assert_eq!(cap_micros("25.01"), Some(25_010_000));
        assert_eq!(cap_micros("9007199254.740991"), Some(MAX_SAFE_MICROS));
        for invalid in [
            "-1",
            "1e3",
            "NaN",
            "1.0000001",
            "9007199254.740992",
            "999999999999999999999999",
        ] {
            assert_eq!(cap_micros(invalid), None, "{invalid}");
        }
        for n in [0, 1, 25_010_000, MAX_SAFE_MICROS] {
            assert_eq!(cap_micros(&cap_dollars(n)), Some(n));
        }
        assert_eq!(usd(1_005_000), "$1.01");
    }
    #[test]
    fn machine_minutes_are_the_headline_and_hours_a_hint() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(-5), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(33_600), "33,600");
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(machine_minutes(1), "1 machine minute");
        assert_eq!(machine_minutes(33_600), "33,600 machine minutes");
        assert_eq!(about_hours(33_600, 4), "about 140 h");
        assert_eq!(about_hours(84_000, 4), "about 350 h");
        assert_eq!(about_hours(8_400, 4), "about 35 h");
        assert_eq!(about_hours(21_120, 4), "about 88 h");
        assert_eq!(about_hours(1_240, 8), "about 2.5 h");
        assert_eq!(about_hours(10, 4), "under 0.1 h");
        assert_eq!(about_hours(0, 4), "0 h");
        assert_eq!(machine_size(4, 16_384), "4 vCPU / 16 GB");
        assert_eq!(machine_size(8, 32_768), "8 vCPU / 32 GB");
        assert_eq!(per_minute(8), "8 per minute");
        assert_eq!(monthly(20_000_000), "$20/month");
        assert_eq!(monthly(1_500_000), "$1.50/month");
        assert_eq!(dollars(25_000_000), "$25");
        assert_eq!(dollars(1_000_000), "$1");
        assert_eq!(dollars(1_714_286), "$1.71");
    }
    #[test]
    fn every_cloud_plan_counts_as_subscribed() {
        for offer in CLOUD_OFFERS {
            let value = serde_json::json!({"entitlements":[{"offerId":offer,"status":"active","paidThrough":null}]});
            assert_eq!(cloud_subscribed(&value, 10), Some(true), "{offer}");
        }
        let ended = serde_json::json!({"entitlements":[{"offerId":"cloud-pro-v1","status":"active","paidThrough":5}]});
        assert_eq!(cloud_subscribed(&ended, 10), Some(false));
    }
}

/// Catalog presentation only; checkout independently reconciles canonical provider state.
pub fn cloud_subscribed(value: &serde_json::Value, now: i64) -> Option<bool> {
    Some(value["entitlements"].as_array()?.iter().any(|ent| {
        ent["offerId"]
            .as_str()
            .is_some_and(|offer| CLOUD_OFFERS.contains(&offer))
            && matches!(ent["status"].as_str(), Some("pending" | "active" | "grace"))
            && ent["paidThrough"].as_i64().is_none_or(|end| end > now)
    }))
}

/// A whole number with thousands separators: "33,600". Negatives read 0.
pub fn grouped(value: i64) -> String {
    let digits = value.max(0).to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}
/// The headline unit: "33,600 machine minutes".
pub fn machine_minutes(count: i64) -> String {
    format!(
        "{} machine minute{}",
        grouped(count),
        if count == 1 { "" } else { "s" }
    )
}
/// Awake hours `machine_minutes` cover on a machine that uses `rate` per
/// minute, as a hint: "about 140 h", one decimal below 10 h.
pub fn about_hours(machine_minutes: i64, rate: u32) -> String {
    let tenths = zuse_protocol::cloud_billing::plans::hours_tenths(machine_minutes, rate);
    if machine_minutes <= 0 {
        "0 h".into()
    } else if tenths == 0 {
        "under 0.1 h".into()
    } else if tenths >= 100 {
        format!("about {} h", tenths / 10)
    } else {
        format!("about {}.{} h", tenths / 10, tenths % 10)
    }
}
/// "4 vCPU / 16 GB".
pub fn machine_size(vcpus: u32, memory_mib: u32) -> String {
    format!("{vcpus} vCPU / {} GB", (memory_mib + 512) / 1024)
}
/// How fast a machine uses machine minutes: "8 per minute".
pub fn per_minute(rate: u32) -> String {
    format!("{rate} per minute")
}
/// Whole dollars without cents, otherwise rounded to cents: "$25", "$1.71".
pub fn dollars(micros: i64) -> String {
    if micros % 1_000_000 == 0 {
        format!("${}", micros / 1_000_000)
    } else {
        usd(micros)
    }
}
/// A plan price: "$20/month".
pub fn monthly(micros: i64) -> String {
    format!("{}/month", dollars(micros))
}

pub fn cents(value: i64) -> String {
    let amount = value.unsigned_abs();
    format!(
        "{}${}.{:02}",
        if value < 0 { "-" } else { "" },
        amount / 100,
        amount % 100
    )
}
