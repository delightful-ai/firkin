//! slo — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::benchmark::BenchmarkSummary;
#[allow(unused_imports)]
use crate::lifecycle::BenchmarkEvidenceReport;
#[allow(unused_imports)]
use crate::overhead::BenchmarkOverheadEvidenceReport;
#[allow(unused_imports)]
use firkin_trace::{BenchmarkMetricKind, BenchmarkUnit};
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// One p95 benchmark SLO target for a production evidence report.
#[derive(Clone, Debug, PartialEq)]
pub struct BenchmarkSloTarget {
    pub(crate) metric: String,
    #[allow(missing_docs)]
    pub kind: BenchmarkMetricKind,
    pub(crate) unit: BenchmarkUnit,
    pub(crate) max_p95: f64,
    min_samples: usize,
}
impl BenchmarkSloTarget {
    /// Construct a p95 SLO target.
    #[must_use]
    pub fn new(
        metric: impl Into<String>,
        kind: BenchmarkMetricKind,
        unit: BenchmarkUnit,
        max_p95: f64,
        min_samples: usize,
    ) -> Self {
        Self {
            metric: metric.into(),
            kind,
            unit,
            max_p95,
            min_samples,
        }
    }
    /// Return the metric name.
    #[must_use]
    pub fn metric(&self) -> &str {
        &self.metric
    }
    /// Return the expected metric kind.
    #[must_use]
    pub const fn kind(&self) -> BenchmarkMetricKind {
        self.kind
    }
    /// Return the expected measurement unit.
    #[must_use]
    pub const fn unit(&self) -> BenchmarkUnit {
        self.unit
    }
    /// Return the maximum allowed p95 value.
    #[must_use]
    pub const fn max_p95(&self) -> f64 {
        self.max_p95
    }
    /// Return the minimum required sample count.
    #[must_use]
    pub const fn min_samples(&self) -> usize {
        self.min_samples
    }
}
/// Benchmark SLO gate failure.
#[derive(Clone, Debug, PartialEq, ThisError)]
pub enum BenchmarkSloGateError {
    /// Target metric is absent from the evidence report.
    #[error("benchmark SLO target metric `{metric}` is missing from evidence")]
    MissingMetric {
        /// Missing metric.
        metric: String,
    },
    /// Target metric has a different kind or unit than required.
    #[error("benchmark SLO target metric `{metric}` has the wrong shape")]
    WrongShape {
        /// Metric with the wrong shape.
        metric: String,
        /// Expected kind.
        expected_kind: BenchmarkMetricKind,
        /// Actual kind.
        actual_kind: BenchmarkMetricKind,
        /// Expected unit.
        expected_unit: BenchmarkUnit,
        /// Actual unit.
        actual_unit: BenchmarkUnit,
    },
    /// Target metric does not include enough representative samples.
    #[error(
        "benchmark SLO target metric `{metric}` has too few samples: required {required}, actual {actual}"
    )]
    InsufficientSamples {
        /// Metric with too few samples.
        metric: String,
        /// Required sample count.
        required: usize,
        /// Actual sample count.
        actual: usize,
    },
    /// Target metric p95 exceeds the allowed threshold.
    #[error(
        "benchmark SLO target metric `{metric}` p95 exceeded: allowed {allowed}, actual {actual}"
    )]
    P95Exceeded {
        /// Metric exceeding the target.
        metric: String,
        /// Allowed p95 threshold.
        allowed: f64,
        /// Actual p95 value.
        actual: f64,
    },
}
/// Benchmark evidence report after all configured SLO targets passed.
#[derive(Clone, Debug, PartialEq)]
pub struct BenchmarkSloGateReport {
    passed_targets: Vec<BenchmarkSloTarget>,
}
impl BenchmarkSloGateReport {
    /// Validate lifecycle benchmark evidence against p95 targets.
    ///
    /// # Errors
    ///
    /// Returns [`BenchmarkSloGateError`] when a target metric is absent, uses
    /// the wrong metric shape, has too few samples, or exceeds its p95 target.
    pub fn from_lifecycle_report(
        report: &BenchmarkEvidenceReport,
        targets: impl IntoIterator<Item = BenchmarkSloTarget>,
    ) -> std::result::Result<Self, BenchmarkSloGateError> {
        Self::from_summaries(report.summaries(), targets)
    }
    /// Validate Firkin overhead evidence against p95 targets.
    ///
    /// # Errors
    ///
    /// Returns [`BenchmarkSloGateError`] when a target metric is absent, uses
    /// the wrong metric shape, has too few samples, or exceeds its p95 target.
    pub fn from_overhead_report(
        report: &BenchmarkOverheadEvidenceReport,
        targets: impl IntoIterator<Item = BenchmarkSloTarget>,
    ) -> std::result::Result<Self, BenchmarkSloGateError> {
        Self::from_summaries(report.summaries(), targets)
    }
    fn from_summaries(
        summaries: &[BenchmarkSummary],
        targets: impl IntoIterator<Item = BenchmarkSloTarget>,
    ) -> std::result::Result<Self, BenchmarkSloGateError> {
        let mut passed_targets = Vec::new();
        for target in targets {
            let summary = summaries
                .iter()
                .find(|summary| summary.metric() == target.metric())
                .ok_or_else(|| BenchmarkSloGateError::MissingMetric {
                    metric: target.metric.clone(),
                })?;
            if summary.kind() != target.kind() || summary.unit() != target.unit() {
                return Err(BenchmarkSloGateError::WrongShape {
                    metric: target.metric.clone(),
                    expected_kind: target.kind(),
                    actual_kind: summary.kind(),
                    expected_unit: target.unit(),
                    actual_unit: summary.unit(),
                });
            }
            if summary.count() < target.min_samples() {
                return Err(BenchmarkSloGateError::InsufficientSamples {
                    metric: target.metric.clone(),
                    required: target.min_samples(),
                    actual: summary.count(),
                });
            }
            if summary.p95() > target.max_p95() {
                return Err(BenchmarkSloGateError::P95Exceeded {
                    metric: target.metric.clone(),
                    allowed: target.max_p95(),
                    actual: summary.p95(),
                });
            }
            passed_targets.push(target);
        }
        Ok(Self { passed_targets })
    }
    /// Return targets that passed the gate.
    #[must_use]
    pub fn passed_targets(&self) -> &[BenchmarkSloTarget] {
        &self.passed_targets
    }
}
