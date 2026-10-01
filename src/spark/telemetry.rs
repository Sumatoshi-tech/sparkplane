//! Fixed read-only host telemetry. Never participates in resource admission.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelTelemetry {
    pub health: super::wire::ExecutorHealth,
    pub resources: Option<super::resources::HostResourceSnapshot>,
    pub cpu_total_ticks: Option<u64>,
    pub cpu_idle_ticks: Option<u64>,
    pub gpu: Option<GpuTelemetry>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpuTelemetry {
    pub name: String,
    pub utilization_percent: Option<f64>,
    pub temperature_celsius: Option<f64>,
    pub power_watts: Option<f64>,
    pub dedicated_memory_total_mib: Option<f64>,
    pub dedicated_memory_used_mib: Option<f64>,
}
pub(crate) fn cpu_ticks(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let values = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() < 4 {
        return None;
    }
    Some((
        values.iter().try_fold(0u64, |sum, v| sum.checked_add(*v))?,
        values[3].checked_add(values.get(4).copied().unwrap_or(0))?,
    ))
}
pub(crate) fn gpu(text: &str) -> Option<GpuTelemetry> {
    let parts = text
        .lines()
        .next()?
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    if parts.len() != 6 || parts[0].len() > 128 {
        return None;
    }
    let number = |index: usize| {
        parts[index]
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0)
    };
    Some(GpuTelemetry {
        name: parts[0].into(),
        utilization_percent: number(1).filter(|v| *v <= 100.0),
        temperature_celsius: number(2),
        power_watts: number(3),
        dedicated_memory_total_mib: number(4),
        dedicated_memory_used_mib: number(5),
    })
}
pub(crate) fn gpu_command() -> [&'static str; 6] {
    [
        "/usr/bin/timeout",
        "--signal=KILL",
        "2s",
        "/usr/bin/nvidia-smi",
        "--query-gpu=name,utilization.gpu,temperature.gpu,power.draw,memory.total,memory.used",
        "--format=csv,noheader,nounits",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gb10_reports_unavailable_dedicated_memory() {
        let sample = gpu("NVIDIA GB10, 93, 70, 39.35, [N/A], [N/A]\n").unwrap();
        assert_eq!(sample.dedicated_memory_total_mib, None);
        assert_eq!(sample.utilization_percent, Some(93.0));
        assert_eq!(cpu_ticks("cpu  1 2 3 4 5 6 7 8 99 99\n"), Some((36, 9)));
    }
}
