//! How loaded a cloud computer is, from its last CPU, memory and disk sample:
//! the fractions the dithered load bars draw.
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::shell) struct MachineLoad {
    pub cpu: Option<f32>,
    pub memory: Option<f32>,
    pub disk: Option<f32>,
}

/// Each resource's fraction in use, `0..=1`; absent when not sampled.
pub(in crate::shell) fn machine_load(resources: &Value) -> MachineLoad {
    let used = |used: &str, total: &str| {
        let used = resources[used].as_f64()?;
        let total = resources[total].as_f64().filter(|total| *total > 0.0)?;
        Some((used / total).clamp(0.0, 1.0) as f32)
    };
    MachineLoad {
        cpu: resources["cpuPercent"]
            .as_f64()
            .map(|percent| (percent / 100.0).clamp(0.0, 1.0) as f32),
        memory: used("memoryUsedBytes", "memoryTotalBytes"),
        disk: used("diskUsedBytes", "diskTotalBytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fractions_come_from_the_sample() {
        let load = machine_load(&json!({
            "cpuPercent": 25.0,
            "memoryUsedBytes": 4_000_000_000u64, "memoryTotalBytes": 16_000_000_000u64,
            "diskUsedBytes": 90u64, "diskTotalBytes": 0u64,
        }));
        assert_eq!(load.cpu, Some(0.25));
        assert_eq!(load.memory, Some(0.25));
        assert_eq!(load.disk, None, "no total, no fraction");
        assert_eq!(machine_load(&json!({})), MachineLoad::default());
        assert_eq!(machine_load(&json!({"cpuPercent": 180.0})).cpu, Some(1.0));
    }
}
