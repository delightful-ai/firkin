//! benchmark — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_trace::BenchmarkSample;
#[allow(unused_imports)]
use firkin_trace::{BenchmarkMetricKind, BenchmarkUnit};
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Benchmark summary construction error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum BenchmarkSummaryError {
    /// No samples were provided.
    #[error("benchmark summary requires at least one sample")]
    Empty,
    /// At least one sample does not match the requested metric.
    #[error("benchmark summary samples must all use metric `{expected}`")]
    MetricMismatch {
        /// Expected metric.
        expected: String,
    },
    /// Samples for one summary used multiple kinds or units.
    #[error("benchmark summary samples must use one kind and unit")]
    MixedShape,
}

/// Percentile trust tier derived from sample count and metric policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum PercentileAvailability {
    /// Fewer than three samples. Values are smoke evidence only.
    #[default]
    SmokeOnly,
    /// At least three samples. Values are useful for the fastest harness check.
    SuperfastIteration,
    /// At least five samples. Values are useful for fast local iteration.
    FastIteration,
    /// At least ten samples. Values are useful as a before/after checkpoint.
    BaselineCheckpoint,
    /// At least thirty samples. p50 and p90 are suitable for development iteration.
    P50P90DecisionGrade,
    /// At least the metric p95 sample floor. p95 is suitable for decision-grade comparison.
    P95DecisionGrade,
    /// At least the metric p99 sample floor. p99 is suitable for decision-grade comparison.
    P99DecisionGrade,
}

impl PercentileAvailability {
    /// Compute percentile availability from a sample count.
    #[must_use]
    pub const fn for_sample_count(count: usize) -> Self {
        Self::for_sample_policy(count, 100, 500)
    }

    /// Compute percentile availability from an explicit sample policy.
    #[must_use]
    pub const fn for_sample_policy(
        count: usize,
        p95_min_samples: usize,
        p99_min_samples: usize,
    ) -> Self {
        if count >= p99_min_samples {
            Self::P99DecisionGrade
        } else if count >= p95_min_samples {
            Self::P95DecisionGrade
        } else if count >= 30 {
            Self::P50P90DecisionGrade
        } else if count >= 10 {
            Self::BaselineCheckpoint
        } else if count >= 5 {
            Self::FastIteration
        } else if count >= 3 {
            Self::SuperfastIteration
        } else {
            Self::SmokeOnly
        }
    }

    /// Return stable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SmokeOnly => "smoke_only",
            Self::SuperfastIteration => "superfast_iteration",
            Self::FastIteration => "fast_iteration",
            Self::BaselineCheckpoint => "baseline_checkpoint",
            Self::P50P90DecisionGrade => "p50_p90_decision_grade",
            Self::P95DecisionGrade => "p95_decision_grade",
            Self::P99DecisionGrade => "p99_decision_grade",
        }
    }

    /// Return whether p95/p99 must be treated as unstable.
    #[must_use]
    pub const fn unstable_percentile(self) -> bool {
        matches!(
            self,
            Self::SmokeOnly
                | Self::SuperfastIteration
                | Self::FastIteration
                | Self::BaselineCheckpoint
                | Self::P50P90DecisionGrade
        )
    }

    /// Return p95 status label.
    #[must_use]
    pub const fn p95_status(self) -> &'static str {
        match self {
            Self::P95DecisionGrade | Self::P99DecisionGrade => "decision_grade",
            Self::SmokeOnly
            | Self::SuperfastIteration
            | Self::FastIteration
            | Self::BaselineCheckpoint
            | Self::P50P90DecisionGrade => "unstable",
        }
    }

    /// Return p99 status label.
    #[must_use]
    pub const fn p99_status(self) -> &'static str {
        match self {
            Self::P99DecisionGrade => "decision_grade",
            Self::SmokeOnly
            | Self::SuperfastIteration
            | Self::FastIteration
            | Self::BaselineCheckpoint
            | Self::P50P90DecisionGrade
            | Self::P95DecisionGrade => "experimental",
        }
    }
}

/// Aggregated benchmark metric summary.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct BenchmarkSummary {
    pub(crate) metric: String,
    #[allow(missing_docs)]
    pub kind: BenchmarkMetricKind,
    pub(crate) unit: BenchmarkUnit,
    count: usize,
    #[serde(default)]
    min: f64,
    #[serde(default)]
    mean: f64,
    #[serde(default)]
    median_absolute_deviation: f64,
    #[serde(default)]
    coefficient_of_variation: f64,
    p50: f64,
    p90: f64,
    p95: f64,
    p99: f64,
    max: f64,
    #[serde(default)]
    percentile_availability: PercentileAvailability,
}
impl BenchmarkSummary {
    /// Construct a summary from homogeneous samples for one metric.
    ///
    /// # Errors
    ///
    /// Returns [`BenchmarkSummaryError`] when samples are empty or mixed across
    /// metric names, kinds, or units.
    pub fn from_samples(
        metric: impl Into<String>,
        samples: impl IntoIterator<Item = BenchmarkSample>,
    ) -> std::result::Result<Self, BenchmarkSummaryError> {
        let metric = metric.into();
        let samples = samples.into_iter().collect::<Vec<_>>();
        let Some(first) = samples.first() else {
            return Err(BenchmarkSummaryError::Empty);
        };
        if samples
            .iter()
            .any(|sample| sample.metric() != metric.as_str())
        {
            return Err(BenchmarkSummaryError::MetricMismatch { expected: metric });
        }
        if samples
            .iter()
            .any(|sample| sample.kind() != first.kind() || sample.unit() != first.unit())
        {
            return Err(BenchmarkSummaryError::MixedShape);
        }
        let mut values = samples
            .iter()
            .map(BenchmarkSample::value)
            .collect::<Vec<_>>();
        values.sort_by(f64::total_cmp);
        let p50 = percentile_nearest_rank(&values, 50);
        let mut absolute_deviations = values
            .iter()
            .map(|value| (value - p50).abs())
            .collect::<Vec<_>>();
        absolute_deviations.sort_by(f64::total_cmp);
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values
            .iter()
            .map(|value| {
                let delta = value - mean;
                delta * delta
            })
            .sum::<f64>()
            / values.len() as f64;
        let coefficient_of_variation = if mean.abs() < f64::EPSILON {
            0.0
        } else {
            variance.sqrt() / mean.abs()
        };
        let percentile_availability = percentile_availability_for_metric(&metric, values.len());
        Ok(Self {
            metric,
            kind: first.kind(),
            unit: first.unit(),
            count: values.len(),
            min: values[0],
            mean,
            median_absolute_deviation: percentile_nearest_rank(&absolute_deviations, 50),
            coefficient_of_variation,
            p50,
            p90: percentile_nearest_rank(&values, 90),
            p95: percentile_nearest_rank(&values, 95),
            p99: percentile_nearest_rank(&values, 99),
            max: values[values.len() - 1],
            percentile_availability,
        })
    }
    /// Return the metric name.
    #[must_use]
    pub fn metric(&self) -> &str {
        &self.metric
    }
    /// Return the metric category.
    #[must_use]
    pub const fn kind(&self) -> BenchmarkMetricKind {
        self.kind
    }
    /// Return the measurement unit.
    #[must_use]
    pub const fn unit(&self) -> BenchmarkUnit {
        self.unit
    }
    /// Return sample count.
    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }
    /// Return the minimum observed value.
    #[must_use]
    pub const fn min(&self) -> f64 {
        self.min
    }
    /// Return arithmetic mean.
    #[must_use]
    pub const fn mean(&self) -> f64 {
        self.mean
    }
    /// Return median absolute deviation.
    #[must_use]
    pub const fn median_absolute_deviation(&self) -> f64 {
        self.median_absolute_deviation
    }
    /// Return population coefficient of variation.
    #[must_use]
    pub const fn coefficient_of_variation(&self) -> f64 {
        self.coefficient_of_variation
    }
    /// Return p50 by nearest-rank percentile.
    #[must_use]
    pub const fn p50(&self) -> f64 {
        self.p50
    }
    /// Return p90 by nearest-rank percentile.
    #[must_use]
    pub const fn p90(&self) -> f64 {
        self.p90
    }
    /// Return p95 by nearest-rank percentile.
    #[must_use]
    pub const fn p95(&self) -> f64 {
        self.p95
    }
    /// Return p99 by nearest-rank percentile.
    #[must_use]
    pub const fn p99(&self) -> f64 {
        self.p99
    }
    /// Return the maximum observed value.
    #[must_use]
    pub const fn max(&self) -> f64 {
        self.max
    }
    /// Return percentile availability for this sample count.
    #[must_use]
    pub const fn percentile_availability(&self) -> PercentileAvailability {
        self.percentile_availability
    }
}
fn percentile_nearest_rank(sorted_values: &[f64], percentile: usize) -> f64 {
    let rank = sorted_values.len().saturating_mul(percentile).div_ceil(100);
    let index = rank.saturating_sub(1).min(sorted_values.len() - 1);
    sorted_values[index]
}

fn percentile_availability_for_metric(metric: &str, count: usize) -> PercentileAvailability {
    crate::metric_contract::decision_grade_metric_contract()
        .iter()
        .find(|contract| contract.metric() == metric)
        .map_or_else(
            || PercentileAvailability::for_sample_count(count),
            |contract| {
                let policy = contract.percentile_policy();
                PercentileAvailability::for_sample_policy(
                    count,
                    policy.p95_min_samples(),
                    policy.p99_min_samples(),
                )
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(value: f64) -> BenchmarkSample {
        BenchmarkSample::new(
            "start.hot_to_first_stdout_ms",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            value,
        )
    }

    #[test]
    fn benchmark_summary_marks_superfast_samples_without_overclaiming_percentiles() {
        let summary = BenchmarkSummary::from_samples(
            "start.hot_to_first_stdout_ms",
            [1.0, 2.0, 3.0].map(sample),
        )
        .expect("summary");

        assert_eq!(summary.count(), 3);
        assert_eq!(summary.min(), 1.0);
        assert_eq!(summary.mean(), 2.0);
        assert_eq!(summary.median_absolute_deviation(), 1.0);
        assert!(summary.coefficient_of_variation() > 0.4);
        assert_eq!(
            summary.percentile_availability(),
            PercentileAvailability::SuperfastIteration
        );
        assert!(summary.percentile_availability().unstable_percentile());
        assert_eq!(summary.percentile_availability().p95_status(), "unstable");
    }

    #[test]
    fn benchmark_summary_marks_fast_iteration_sample_floor() {
        let summary = BenchmarkSummary::from_samples(
            "start.hot_to_first_stdout_ms",
            (1..=5_u32).map(|value| sample(f64::from(value))),
        )
        .expect("summary");

        assert_eq!(
            summary.percentile_availability(),
            PercentileAvailability::FastIteration
        );
        assert_eq!(summary.percentile_availability().as_str(), "fast_iteration");
        assert!(summary.percentile_availability().unstable_percentile());
    }

    #[test]
    fn benchmark_summary_marks_baseline_checkpoint_sample_floor() {
        let summary = BenchmarkSummary::from_samples(
            "start.hot_to_first_stdout_ms",
            (1..=10_u32).map(|value| sample(f64::from(value))),
        )
        .expect("summary");

        assert_eq!(
            summary.percentile_availability(),
            PercentileAvailability::BaselineCheckpoint
        );
        assert_eq!(
            summary.percentile_availability().as_str(),
            "baseline_checkpoint"
        );
        assert!(summary.percentile_availability().unstable_percentile());
        assert_eq!(summary.percentile_availability().p95_status(), "unstable");
    }

    #[test]
    fn benchmark_summary_marks_smoke_only_below_superfast_floor() {
        let summary =
            BenchmarkSummary::from_samples("start.hot_to_first_stdout_ms", [1.0, 2.0].map(sample))
                .expect("summary");

        assert_eq!(
            summary.percentile_availability(),
            PercentileAvailability::SmokeOnly
        );
    }

    #[test]
    fn benchmark_summary_marks_p95_and_p99_sample_floors() {
        let p95 = BenchmarkSummary::from_samples(
            "start.hot_to_first_stdout_ms",
            (1..=100_u32).map(|value| sample(f64::from(value))),
        )
        .expect("summary");
        assert_eq!(
            p95.percentile_availability(),
            PercentileAvailability::P95DecisionGrade
        );
        assert_eq!(p95.percentile_availability().p95_status(), "decision_grade");
        assert_eq!(p95.percentile_availability().p99_status(), "experimental");

        let p99_not_ready = BenchmarkSummary::from_samples(
            "start.hot_to_first_stdout_ms",
            (1..=499_u32).map(|value| sample(f64::from(value))),
        )
        .expect("summary");
        assert_eq!(
            p99_not_ready.percentile_availability(),
            PercentileAvailability::P95DecisionGrade
        );
        assert_eq!(
            p99_not_ready.percentile_availability().p99_status(),
            "experimental"
        );

        let p99 = BenchmarkSummary::from_samples(
            "start.hot_to_first_stdout_ms",
            (1..=500_u32).map(|value| sample(f64::from(value))),
        )
        .expect("summary");
        assert_eq!(
            p99.percentile_availability(),
            PercentileAvailability::P99DecisionGrade
        );
        assert_eq!(p99.percentile_availability().p99_status(), "decision_grade");
    }

    #[test]
    fn benchmark_summary_uses_metric_specific_slow_path_p95_floor() {
        let summary = BenchmarkSummary::from_samples(
            "disk.sparse_bloat_after_trim",
            (1..=30_u32).map(|value| {
                BenchmarkSample::new(
                    "disk.sparse_bloat_after_trim",
                    BenchmarkMetricKind::WorkloadResource,
                    BenchmarkUnit::Ratio,
                    f64::from(value),
                )
            }),
        )
        .expect("summary");

        assert_eq!(
            summary.percentile_availability(),
            PercentileAvailability::P95DecisionGrade
        );
    }
}
