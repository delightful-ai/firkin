//! Density benchmark helper calculations.
#![allow(missing_docs)]

use firkin_trace::{BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit};
use thiserror::Error as ThisError;

pub const MAX_ACTIVE_BEFORE_P95_DOUBLES_METRIC: &str =
    "density.max_active_before_hot_to_first_stdout_p95_doubles";
pub const MAX_RETAINED_SHELLS_BEFORE_FIRST_STDOUT_P95_DOUBLES_METRIC: &str =
    "density.max_active_before_retained_shell_first_stdout_p95_doubles";
pub const MAX_AGENT_COMPUTERS_BEFORE_READY_P95_DOUBLES_METRIC: &str =
    "density.max_agent_computers_before_ready_p95_doubles";
pub const MAX_PRESTARTED_AGENT_SLOTS_BEFORE_CHECKOUT_READY_P95_DOUBLES_METRIC: &str =
    "density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles";
pub const PRESTARTED_AGENT_SLOT_FIFO_ACCEPTANCE_P95_MS_METRIC: &str =
    "density.prestarted_agent_slot_fifo_acceptance_p95_ms";
const PRESTARTED_AGENT_SLOT_FIFO_ACCEPTANCE_SNAPPY_TARGET_MS: u64 = 5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DensityP95Point {
    concurrency: u64,
    p95_latency_ms: f64,
}

impl DensityP95Point {
    #[must_use]
    pub const fn new(concurrency: u64, p95_latency_ms: f64) -> Self {
        Self {
            concurrency,
            p95_latency_ms,
        }
    }

    #[must_use]
    pub const fn concurrency(self) -> u64 {
        self.concurrency
    }

    #[must_use]
    pub const fn p95_latency_ms(self) -> f64 {
        self.p95_latency_ms
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DensityLimit {
    max_active_before_p95_doubles: u64,
    baseline_p95_latency_ms: f64,
    threshold_p95_latency_ms: f64,
}

impl DensityLimit {
    #[must_use]
    pub const fn max_active_before_p95_doubles(self) -> u64 {
        self.max_active_before_p95_doubles
    }

    #[must_use]
    pub const fn baseline_p95_latency_ms(self) -> f64 {
        self.baseline_p95_latency_ms
    }

    #[must_use]
    pub const fn threshold_p95_latency_ms(self) -> f64 {
        self.threshold_p95_latency_ms
    }

    #[must_use]
    pub fn into_sample(self) -> BenchmarkSample {
        BenchmarkSample::from_static(
            MAX_ACTIVE_BEFORE_P95_DOUBLES_METRIC,
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Count,
            self.max_active_before_p95_doubles as f64,
        )
        .with_static_tag("source", "density-p95-threshold")
    }

    #[must_use]
    pub fn into_agent_computer_sample(self) -> BenchmarkSample {
        BenchmarkSample::from_static(
            MAX_AGENT_COMPUTERS_BEFORE_READY_P95_DOUBLES_METRIC,
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Count,
            self.max_active_before_p95_doubles as f64,
        )
        .with_static_tag("source", "density-agent-computer-ready-p95-threshold")
    }

    #[must_use]
    pub fn into_retained_shell_sample(self) -> BenchmarkSample {
        BenchmarkSample::from_static(
            MAX_RETAINED_SHELLS_BEFORE_FIRST_STDOUT_P95_DOUBLES_METRIC,
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Count,
            self.max_active_before_p95_doubles as f64,
        )
        .with_static_tag(
            "source",
            "density-retained-shell-first-stdout-p95-threshold",
        )
    }

    #[must_use]
    pub fn into_prestarted_agent_slot_sample(self) -> BenchmarkSample {
        BenchmarkSample::from_static(
            MAX_PRESTARTED_AGENT_SLOTS_BEFORE_CHECKOUT_READY_P95_DOUBLES_METRIC,
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Count,
            self.max_active_before_p95_doubles as f64,
        )
        .with_static_tag(
            "source",
            "density-prestarted-agent-slot-ready-p95-threshold",
        )
    }
}

#[derive(Debug, ThisError, PartialEq)]
pub enum DensityLimitError {
    #[error("density benchmark has no single-sandbox baseline")]
    MissingSingleSandboxBaseline,
    #[error("density benchmark concurrency must be greater than zero")]
    ZeroConcurrency,
    #[error("density benchmark p95 latency must be finite and positive")]
    InvalidP95Latency,
    #[error("density benchmark single-sandbox baseline overflows 2x threshold")]
    ThresholdOverflow,
}

pub fn max_active_before_p95_doubles(
    points: impl IntoIterator<Item = DensityP95Point>,
) -> Result<DensityLimit, DensityLimitError> {
    let mut baseline = None;
    let mut accepted = 0_u64;
    let mut captured = Vec::new();

    for point in points {
        validate_point(point)?;
        if point.concurrency() == 1 && baseline.is_none() {
            baseline = Some(point.p95_latency_ms());
        }
        captured.push(point);
    }

    let baseline_p95_latency_ms =
        baseline.ok_or(DensityLimitError::MissingSingleSandboxBaseline)?;
    let threshold_p95_latency_ms = baseline_p95_latency_ms * 2.0;
    if !threshold_p95_latency_ms.is_finite() {
        return Err(DensityLimitError::ThresholdOverflow);
    }

    for point in captured {
        if point.p95_latency_ms() <= threshold_p95_latency_ms {
            accepted = accepted.max(point.concurrency());
        }
    }

    Ok(DensityLimit {
        max_active_before_p95_doubles: accepted,
        baseline_p95_latency_ms,
        threshold_p95_latency_ms,
    })
}

pub fn prestarted_agent_slot_fifo_acceptance_p95_sample(
    points: impl IntoIterator<Item = DensityP95Point>,
) -> Result<BenchmarkSample, DensityLimitError> {
    let mut max_point: Option<DensityP95Point> = None;
    for point in points {
        validate_point(point)?;
        max_point = match max_point {
            Some(current) if point.p95_latency_ms() <= current.p95_latency_ms() => Some(current),
            _ => Some(point),
        };
    }
    let max_point = max_point.ok_or(DensityLimitError::MissingSingleSandboxBaseline)?;
    Ok(BenchmarkSample::from_static(
        PRESTARTED_AGENT_SLOT_FIFO_ACCEPTANCE_P95_MS_METRIC,
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        max_point.p95_latency_ms(),
    )
    .with_static_tag("source", "prestarted-agent-slot-fifo-acceptance-p95")
    .with_static_tag("measurement_boundary", "prestarted_slot_checkout")
    .with_static_tag("slot_surface", "prestarted_agent_slot")
    .with_static_tag("capacity_source", "already_prestarted_slot")
    .with_static_tag("autoscale_refill_observed", "false")
    .with_static_tag("excludes_container_add", "true")
    .with_static_tag("ready_signal", "request_fifo_acceptance")
    .with_dynamic_tag(
        "snappy_target_ms",
        PRESTARTED_AGENT_SLOT_FIFO_ACCEPTANCE_SNAPPY_TARGET_MS.to_string(),
    )
    .with_dynamic_tag("max_concurrency_level", max_point.concurrency().to_string()))
}

fn validate_point(point: DensityP95Point) -> Result<(), DensityLimitError> {
    if point.concurrency() == 0 {
        return Err(DensityLimitError::ZeroConcurrency);
    }
    if !point.p95_latency_ms().is_finite() || point.p95_latency_ms() <= 0.0 {
        return Err(DensityLimitError::InvalidP95Latency);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_largest_concurrency_within_double_single_sandbox_p95() {
        let limit = max_active_before_p95_doubles([
            DensityP95Point::new(1, 100.0),
            DensityP95Point::new(2, 175.0),
            DensityP95Point::new(4, 200.0),
            DensityP95Point::new(8, 201.0),
        ])
        .unwrap();

        assert_eq!(limit.max_active_before_p95_doubles(), 4);
        assert_eq!(limit.baseline_p95_latency_ms(), 100.0);
        assert_eq!(limit.threshold_p95_latency_ms(), 200.0);
    }

    #[test]
    fn emits_workload_resource_count_sample() {
        let sample = max_active_before_p95_doubles([
            DensityP95Point::new(1, 10.0),
            DensityP95Point::new(3, 19.0),
        ])
        .unwrap()
        .into_sample();

        assert_eq!(sample.metric(), MAX_ACTIVE_BEFORE_P95_DOUBLES_METRIC);
        assert_eq!(sample.kind(), BenchmarkMetricKind::WorkloadResource);
        assert_eq!(sample.unit(), BenchmarkUnit::Count);
        assert_eq!(sample.value(), 3.0);
        assert_eq!(sample.tag_value("source"), Some("density-p95-threshold"));
    }

    #[test]
    fn emits_agent_computer_density_breakpoint_sample() {
        let sample = max_active_before_p95_doubles([
            DensityP95Point::new(1, 100.0),
            DensityP95Point::new(4, 190.0),
            DensityP95Point::new(5, 230.0),
        ])
        .unwrap()
        .into_agent_computer_sample();

        assert_eq!(
            sample.metric(),
            MAX_AGENT_COMPUTERS_BEFORE_READY_P95_DOUBLES_METRIC
        );
        assert_eq!(sample.kind(), BenchmarkMetricKind::WorkloadResource);
        assert_eq!(sample.unit(), BenchmarkUnit::Count);
        assert_eq!(sample.value(), 4.0);
        assert_eq!(
            sample.tag_value("source"),
            Some("density-agent-computer-ready-p95-threshold")
        );
    }

    #[test]
    fn emits_retained_shell_density_breakpoint_sample() {
        let sample = max_active_before_p95_doubles([
            DensityP95Point::new(1, 4.25),
            DensityP95Point::new(4, 0.90),
            DensityP95Point::new(8, 1.43),
        ])
        .unwrap()
        .into_retained_shell_sample();

        assert_eq!(
            sample.metric(),
            MAX_RETAINED_SHELLS_BEFORE_FIRST_STDOUT_P95_DOUBLES_METRIC
        );
        assert_eq!(sample.kind(), BenchmarkMetricKind::WorkloadResource);
        assert_eq!(sample.unit(), BenchmarkUnit::Count);
        assert_eq!(sample.value(), 8.0);
        assert_eq!(
            sample.tag_value("source"),
            Some("density-retained-shell-first-stdout-p95-threshold")
        );
    }

    #[test]
    fn emits_prestarted_agent_slot_density_breakpoint_sample() {
        let sample = max_active_before_p95_doubles([
            DensityP95Point::new(1, 8.0),
            DensityP95Point::new(4, 14.0),
            DensityP95Point::new(8, 19.0),
        ])
        .unwrap()
        .into_prestarted_agent_slot_sample();

        assert_eq!(
            sample.metric(),
            MAX_PRESTARTED_AGENT_SLOTS_BEFORE_CHECKOUT_READY_P95_DOUBLES_METRIC
        );
        assert_eq!(sample.kind(), BenchmarkMetricKind::WorkloadResource);
        assert_eq!(sample.unit(), BenchmarkUnit::Count);
        assert_eq!(sample.value(), 4.0);
        assert_eq!(
            sample.tag_value("source"),
            Some("density-prestarted-agent-slot-ready-p95-threshold")
        );
    }

    #[test]
    fn emits_prestarted_agent_slot_fifo_acceptance_snappy_guard_sample() {
        let sample = prestarted_agent_slot_fifo_acceptance_p95_sample([
            DensityP95Point::new(1, 0.96),
            DensityP95Point::new(2, 1.33),
            DensityP95Point::new(4, 2.64),
        ])
        .unwrap();

        assert_eq!(
            sample.metric(),
            PRESTARTED_AGENT_SLOT_FIFO_ACCEPTANCE_P95_MS_METRIC
        );
        assert_eq!(sample.kind(), BenchmarkMetricKind::LifecycleLatency);
        assert_eq!(sample.unit(), BenchmarkUnit::Milliseconds);
        assert_eq!(sample.value(), 2.64);
        assert_eq!(
            sample.tag_value("source"),
            Some("prestarted-agent-slot-fifo-acceptance-p95")
        );
        assert_eq!(
            sample.tag_value("capacity_source"),
            Some("already_prestarted_slot")
        );
        assert_eq!(sample.tag_value("autoscale_refill_observed"), Some("false"));
        assert_eq!(sample.tag_value("snappy_target_ms"), Some("5"));
        assert_eq!(sample.tag_value("max_concurrency_level"), Some("4"));
    }

    #[test]
    fn accepts_points_in_any_order() {
        let limit = max_active_before_p95_doubles([
            DensityP95Point::new(8, 90.0),
            DensityP95Point::new(1, 50.0),
            DensityP95Point::new(16, 101.0),
        ])
        .unwrap();

        assert_eq!(limit.max_active_before_p95_doubles(), 8);
    }

    #[test]
    fn rejects_missing_single_sandbox_baseline() {
        assert_eq!(
            max_active_before_p95_doubles([DensityP95Point::new(2, 100.0)]).unwrap_err(),
            DensityLimitError::MissingSingleSandboxBaseline
        );
    }

    #[test]
    fn rejects_invalid_measurements() {
        assert_eq!(
            max_active_before_p95_doubles([DensityP95Point::new(0, 100.0)]).unwrap_err(),
            DensityLimitError::ZeroConcurrency
        );
        assert_eq!(
            max_active_before_p95_doubles([DensityP95Point::new(1, f64::NAN)]).unwrap_err(),
            DensityLimitError::InvalidP95Latency
        );
    }
}
