//! Autoscale efficiency benchmark sample laws.
#![allow(missing_docs)]

use firkin_trace::{BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit};
use firkin_types::Size;
use thiserror::Error as ThisError;

pub const READY_QUEUE_HIT_RATE_METRIC: &str = "autoscale.ready_queue_hit_rate_pct";
pub const SAFE_SPARE_LIMITING_UTILIZATION_METRIC: &str =
    "autoscale.safe_spare_limiting_utilization_pct";
pub const ACTIVE_EVICTIONS_DUE_TO_POOL_PRESSURE_METRIC: &str =
    "autoscale.active_evictions_due_to_pool_pressure";
pub const RESERVE_FLOOR_VIOLATIONS_METRIC: &str = "autoscale.reserve_floor_violations";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadyQueueOutcomes {
    ready_hits: u64,
    misses: u64,
}

impl ReadyQueueOutcomes {
    #[must_use]
    pub const fn new(ready_hits: u64, misses: u64) -> Self {
        Self { ready_hits, misses }
    }

    #[must_use]
    pub const fn ready_hits(self) -> u64 {
        self.ready_hits
    }

    #[must_use]
    pub const fn misses(self) -> u64 {
        self.misses
    }

    pub fn validate(self) -> Result<ValidatedReadyQueueOutcomes, AutoscaleSampleError> {
        let total = self
            .ready_hits
            .checked_add(self.misses)
            .ok_or(AutoscaleSampleError::ReadyQueueOutcomeOverflow)?;
        if total == 0 {
            return Err(AutoscaleSampleError::EmptyReadyQueueOutcomes);
        }
        Ok(ValidatedReadyQueueOutcomes {
            outcomes: self,
            total,
        })
    }

    pub fn into_sample(self) -> Result<BenchmarkSample, AutoscaleSampleError> {
        self.validate()
            .map(ValidatedReadyQueueOutcomes::into_sample)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidatedReadyQueueOutcomes {
    outcomes: ReadyQueueOutcomes,
    total: u64,
}

impl ValidatedReadyQueueOutcomes {
    #[must_use]
    pub const fn outcomes(self) -> ReadyQueueOutcomes {
        self.outcomes
    }

    #[must_use]
    pub const fn total(self) -> u64 {
        self.total
    }

    #[must_use]
    pub fn hit_rate_percent(self) -> f64 {
        percent(self.outcomes.ready_hits, self.total)
    }

    #[must_use]
    pub fn into_sample(self) -> BenchmarkSample {
        BenchmarkSample::from_static(
            READY_QUEUE_HIT_RATE_METRIC,
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Percent,
            self.hit_rate_percent(),
        )
        .with_static_tag("source", "autoscale-ready-queue-outcomes")
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SafeSpareUtilization {
    resource: &'static str,
    utilization_pct: f64,
}

impl SafeSpareUtilization {
    #[must_use]
    pub const fn new(resource: &'static str, utilization_pct: f64) -> Self {
        Self {
            resource,
            utilization_pct,
        }
    }

    #[must_use]
    pub const fn resource(self) -> &'static str {
        self.resource
    }

    #[must_use]
    pub const fn utilization_pct(self) -> f64 {
        self.utilization_pct
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutoscaleResourceBudget {
    cpus: u32,
    memory: Size,
    disk: Size,
}

impl AutoscaleResourceBudget {
    #[must_use]
    pub const fn new(cpus: u32, memory: Size, disk: Size) -> Self {
        Self { cpus, memory, disk }
    }

    #[must_use]
    pub const fn cpus(self) -> u32 {
        self.cpus
    }

    #[must_use]
    pub const fn memory(self) -> Size {
        self.memory
    }

    #[must_use]
    pub const fn disk(self) -> Size {
        self.disk
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SafeSpareResourceSnapshot {
    total: AutoscaleResourceBudget,
    active: AutoscaleResourceBudget,
    reserved_floor: AutoscaleResourceBudget,
    ready_queue: AutoscaleResourceBudget,
}

impl SafeSpareResourceSnapshot {
    #[must_use]
    pub const fn new(
        total: AutoscaleResourceBudget,
        active: AutoscaleResourceBudget,
        reserved_floor: AutoscaleResourceBudget,
        ready_queue: AutoscaleResourceBudget,
    ) -> Self {
        Self {
            total,
            active,
            reserved_floor,
            ready_queue,
        }
    }

    pub fn limiting_utilization(
        self,
    ) -> Result<LimitingSafeSpareUtilization, AutoscaleSampleError> {
        limiting_safe_spare_utilization(self.utilizations()?)
    }

    pub fn utilizations(self) -> Result<[SafeSpareUtilization; 3], AutoscaleSampleError> {
        Ok([
            SafeSpareUtilization::new(
                "cpu",
                utilization_pct(
                    self.ready_queue.cpus.into(),
                    safe_spare_u64(
                        self.total.cpus.into(),
                        self.active.cpus.into(),
                        self.reserved_floor.cpus.into(),
                        "cpu",
                    )?,
                    "cpu",
                )?,
            ),
            SafeSpareUtilization::new(
                "memory",
                utilization_pct(
                    self.ready_queue.memory.as_bytes(),
                    safe_spare_u64(
                        self.total.memory.as_bytes(),
                        self.active.memory.as_bytes(),
                        self.reserved_floor.memory.as_bytes(),
                        "memory",
                    )?,
                    "memory",
                )?,
            ),
            SafeSpareUtilization::new(
                "disk",
                utilization_pct(
                    self.ready_queue.disk.as_bytes(),
                    safe_spare_u64(
                        self.total.disk.as_bytes(),
                        self.active.disk.as_bytes(),
                        self.reserved_floor.disk.as_bytes(),
                        "disk",
                    )?,
                    "disk",
                )?,
            ),
        ])
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LimitingSafeSpareUtilization {
    resource: &'static str,
    utilization_pct: f64,
}

impl LimitingSafeSpareUtilization {
    #[must_use]
    pub const fn resource(self) -> &'static str {
        self.resource
    }

    #[must_use]
    pub const fn utilization_pct(self) -> f64 {
        self.utilization_pct
    }

    #[must_use]
    pub fn into_sample(self) -> BenchmarkSample {
        BenchmarkSample::from_static(
            SAFE_SPARE_LIMITING_UTILIZATION_METRIC,
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Percent,
            self.utilization_pct,
        )
        .with_static_tag("source", "autoscale-safe-spare-utilization")
        .with_static_tag("limiting_resource", self.resource)
    }
}

pub fn limiting_safe_spare_utilization(
    resources: impl IntoIterator<Item = SafeSpareUtilization>,
) -> Result<LimitingSafeSpareUtilization, AutoscaleSampleError> {
    let mut limiting = None;
    for resource in resources {
        if resource.resource().is_empty() {
            return Err(AutoscaleSampleError::EmptySafeSpareResource);
        }
        if !resource.utilization_pct().is_finite() || resource.utilization_pct() < 0.0 {
            return Err(AutoscaleSampleError::InvalidSafeSpareUtilization {
                resource: resource.resource(),
                utilization_pct: resource.utilization_pct(),
            });
        }
        if limiting
            .as_ref()
            .is_none_or(|current: &LimitingSafeSpareUtilization| {
                resource.utilization_pct() > current.utilization_pct()
            })
        {
            limiting = Some(LimitingSafeSpareUtilization {
                resource: resource.resource(),
                utilization_pct: resource.utilization_pct(),
            });
        }
    }
    limiting.ok_or(AutoscaleSampleError::MissingSafeSpareResources)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AutoscaleProtectionCounts {
    active_evictions_due_to_pool_pressure: u64,
    reserve_floor_violations: u64,
}

impl AutoscaleProtectionCounts {
    #[must_use]
    pub const fn new(
        active_evictions_due_to_pool_pressure: u64,
        reserve_floor_violations: u64,
    ) -> Self {
        Self {
            active_evictions_due_to_pool_pressure,
            reserve_floor_violations,
        }
    }

    #[must_use]
    pub const fn active_evictions_due_to_pool_pressure(self) -> u64 {
        self.active_evictions_due_to_pool_pressure
    }

    #[must_use]
    pub const fn reserve_floor_violations(self) -> u64 {
        self.reserve_floor_violations
    }

    #[must_use]
    pub fn into_samples(self) -> [BenchmarkSample; 2] {
        [
            count_sample(
                ACTIVE_EVICTIONS_DUE_TO_POOL_PRESSURE_METRIC,
                self.active_evictions_due_to_pool_pressure,
            ),
            count_sample(
                RESERVE_FLOOR_VIOLATIONS_METRIC,
                self.reserve_floor_violations,
            ),
        ]
    }
}

#[derive(Debug, ThisError, PartialEq)]
pub enum AutoscaleSampleError {
    #[error("autoscale ready queue outcomes are empty")]
    EmptyReadyQueueOutcomes,
    #[error("autoscale ready queue outcomes overflow u64")]
    ReadyQueueOutcomeOverflow,
    #[error("autoscale safe spare utilization requires at least one resource")]
    MissingSafeSpareResources,
    #[error("autoscale safe spare resource name is empty")]
    EmptySafeSpareResource,
    #[error(
        "autoscale safe spare utilization for `{resource}` must be finite and non-negative, got {utilization_pct}"
    )]
    InvalidSafeSpareUtilization {
        resource: &'static str,
        utilization_pct: f64,
    },
    #[error(
        "autoscale safe spare resource `{resource}` underflowed: total={total} active={active} reserved_floor={reserved_floor}"
    )]
    SafeSpareResourceUnderflow {
        resource: &'static str,
        total: u64,
        active: u64,
        reserved_floor: u64,
    },
    #[error("autoscale safe spare resource `{resource}` is zero after active and reserve floors")]
    ZeroSafeSpareResource { resource: &'static str },
}

fn percent(count: u64, total: u64) -> f64 {
    (count as f64 / total as f64) * 100.0
}

fn utilization_pct(
    ready_queue: u64,
    safe_spare: u64,
    resource: &'static str,
) -> Result<f64, AutoscaleSampleError> {
    if safe_spare == 0 {
        return Err(AutoscaleSampleError::ZeroSafeSpareResource { resource });
    }
    Ok(percent(ready_queue, safe_spare))
}

fn safe_spare_u64(
    total: u64,
    active: u64,
    reserved_floor: u64,
    resource: &'static str,
) -> Result<u64, AutoscaleSampleError> {
    active
        .checked_add(reserved_floor)
        .and_then(|used| total.checked_sub(used))
        .ok_or(AutoscaleSampleError::SafeSpareResourceUnderflow {
            resource,
            total,
            active,
            reserved_floor,
        })
}

fn count_sample(metric: &'static str, value: u64) -> BenchmarkSample {
    BenchmarkSample::from_static(
        metric,
        BenchmarkMetricKind::WorkloadResource,
        BenchmarkUnit::Count,
        value as f64,
    )
    .with_static_tag("source", "autoscale-protection-counts")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_queue_hit_rate_requires_non_empty_classified_outcomes() {
        let sample = ReadyQueueOutcomes::new(7, 3)
            .into_sample()
            .expect("ready hit-rate sample");

        assert_eq!(sample.metric(), READY_QUEUE_HIT_RATE_METRIC);
        assert_eq!(sample.kind(), BenchmarkMetricKind::WorkloadResource);
        assert_eq!(sample.unit(), BenchmarkUnit::Percent);
        assert_eq!(sample.value(), 70.0);
        assert_eq!(
            sample.tag_value("source"),
            Some("autoscale-ready-queue-outcomes")
        );

        assert_eq!(
            ReadyQueueOutcomes::new(0, 0).validate().unwrap_err(),
            AutoscaleSampleError::EmptyReadyQueueOutcomes
        );
        assert_eq!(
            ReadyQueueOutcomes::new(u64::MAX, 1).validate().unwrap_err(),
            AutoscaleSampleError::ReadyQueueOutcomeOverflow
        );
    }

    #[test]
    fn safe_spare_limiting_utilization_selects_highest_resource() {
        let limiting = limiting_safe_spare_utilization([
            SafeSpareUtilization::new("cpu", 10.0),
            SafeSpareUtilization::new("memory", 66.5),
            SafeSpareUtilization::new("disk", 44.0),
        ])
        .expect("limiting resource");
        let sample = limiting.into_sample();

        assert_eq!(limiting.resource(), "memory");
        assert_eq!(limiting.utilization_pct(), 66.5);
        assert_eq!(sample.metric(), SAFE_SPARE_LIMITING_UTILIZATION_METRIC);
        assert_eq!(sample.kind(), BenchmarkMetricKind::WorkloadResource);
        assert_eq!(sample.unit(), BenchmarkUnit::Percent);
        assert_eq!(sample.value(), 66.5);
        assert_eq!(sample.tag_value("limiting_resource"), Some("memory"));
    }

    #[test]
    fn safe_spare_limiting_utilization_rejects_missing_and_invalid_resources() {
        assert_eq!(
            limiting_safe_spare_utilization([]).unwrap_err(),
            AutoscaleSampleError::MissingSafeSpareResources
        );
        assert_eq!(
            limiting_safe_spare_utilization([SafeSpareUtilization::new("", 1.0)]).unwrap_err(),
            AutoscaleSampleError::EmptySafeSpareResource
        );
        assert!(matches!(
            limiting_safe_spare_utilization([SafeSpareUtilization::new("memory", f64::NAN)])
                .unwrap_err(),
            AutoscaleSampleError::InvalidSafeSpareUtilization {
                resource: "memory",
                utilization_pct
            } if utilization_pct.is_nan()
        ));
    }

    #[test]
    fn safe_spare_resource_snapshot_computes_limiting_utilization() {
        let snapshot = SafeSpareResourceSnapshot::new(
            AutoscaleResourceBudget::new(16, Size::gib(64), Size::gib(512)),
            AutoscaleResourceBudget::new(4, Size::gib(16), Size::gib(128)),
            AutoscaleResourceBudget::new(4, Size::gib(16), Size::gib(128)),
            AutoscaleResourceBudget::new(4, Size::gib(16), Size::gib(192)),
        );

        let utilizations = snapshot.utilizations().expect("safe spare utilization");
        assert_eq!(utilizations[0].resource(), "cpu");
        assert_eq!(utilizations[0].utilization_pct(), 50.0);
        assert_eq!(utilizations[1].resource(), "memory");
        assert_eq!(utilizations[1].utilization_pct(), 50.0);
        assert_eq!(utilizations[2].resource(), "disk");
        assert_eq!(utilizations[2].utilization_pct(), 75.0);

        let limiting = snapshot
            .limiting_utilization()
            .expect("limiting utilization");
        assert_eq!(limiting.resource(), "disk");
        assert_eq!(limiting.utilization_pct(), 75.0);
    }

    #[test]
    fn safe_spare_resource_snapshot_rejects_underflow_and_zero_spare() {
        let underflow = SafeSpareResourceSnapshot::new(
            AutoscaleResourceBudget::new(4, Size::gib(8), Size::gib(10)),
            AutoscaleResourceBudget::new(3, Size::gib(6), Size::gib(6)),
            AutoscaleResourceBudget::new(2, Size::gib(1), Size::gib(1)),
            AutoscaleResourceBudget::new(1, Size::gib(1), Size::gib(1)),
        )
        .utilizations()
        .unwrap_err();
        assert!(matches!(
            underflow,
            AutoscaleSampleError::SafeSpareResourceUnderflow {
                resource: "cpu",
                total: 4,
                active: 3,
                reserved_floor: 2
            }
        ));

        let zero_spare = SafeSpareResourceSnapshot::new(
            AutoscaleResourceBudget::new(4, Size::gib(8), Size::gib(10)),
            AutoscaleResourceBudget::new(2, Size::gib(4), Size::gib(5)),
            AutoscaleResourceBudget::new(2, Size::gib(2), Size::gib(1)),
            AutoscaleResourceBudget::new(1, Size::gib(1), Size::gib(1)),
        )
        .utilizations()
        .unwrap_err();
        assert_eq!(
            zero_spare,
            AutoscaleSampleError::ZeroSafeSpareResource { resource: "cpu" }
        );
    }

    #[test]
    fn protection_counts_emit_separate_guardrail_samples() {
        let samples = AutoscaleProtectionCounts::new(0, 2).into_samples();

        assert_eq!(
            samples[0].metric(),
            ACTIVE_EVICTIONS_DUE_TO_POOL_PRESSURE_METRIC
        );
        assert_eq!(samples[0].unit(), BenchmarkUnit::Count);
        assert_eq!(samples[0].value(), 0.0);
        assert_eq!(samples[1].metric(), RESERVE_FLOOR_VIOLATIONS_METRIC);
        assert_eq!(samples[1].unit(), BenchmarkUnit::Count);
        assert_eq!(samples[1].value(), 2.0);
    }
}
