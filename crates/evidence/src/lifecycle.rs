//! lifecycle — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::benchmark::{BenchmarkSummary, BenchmarkSummaryError};
#[allow(unused_imports)]
use crate::slo::BenchmarkSloTarget;
#[allow(unused_imports)]
use firkin_trace::BenchmarkSample;
#[allow(unused_imports)]
use firkin_trace::{BenchmarkMetricKind, BenchmarkUnit};
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Required lifecycle latency metrics for production substrate evidence.
pub const REQUIRED_LIFECYCLE_LATENCY_METRICS: &[&str] = &[
    "start.hot_to_first_stdout_ms",
    "start.hot_to_ready_ms",
    "start.resume_to_first_stdout_ms",
    "start.warm_to_first_stdout_ms",
    "start.agent_task_ready_ms",
    "pool.lease_ms",
    "exec.command_start_ms",
    "exec.first_stdout_byte_ms",
    "exec.batch_100_small_commands_ms",
];
/// Required lifecycle latency target shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequiredLifecycleLatencyTarget {
    /// Required metric name.
    pub name: &'static str,
    /// Initial p50 target in milliseconds.
    pub p50_ms: u32,
    /// Initial p95 target in milliseconds.
    pub p95_ms: u32,
    /// Target rationale.
    pub notes: &'static str,
}
/// Initial lifecycle latency targets for production Apple/VZ substrate evidence.
pub const REQUIRED_LIFECYCLE_LATENCY_TARGETS: &[RequiredLifecycleLatencyTarget] = &[
    RequiredLifecycleLatencyTarget {
        name: "start.hot_to_first_stdout_ms",
        p50_ms: 50,
        p95_ms: 75,
        notes: "hot-lease-acquired-through-first-stdout",
    },
    RequiredLifecycleLatencyTarget {
        name: "start.hot_to_ready_ms",
        p50_ms: 50,
        p95_ms: 75,
        notes: "hot-lease-acquired-through-real-readiness",
    },
    RequiredLifecycleLatencyTarget {
        name: "start.resume_to_first_stdout_ms",
        p50_ms: 35,
        p95_ms: 50,
        notes: "snapshot-restore-start-through-first-stdout",
    },
    RequiredLifecycleLatencyTarget {
        name: "start.warm_to_first_stdout_ms",
        p50_ms: 350,
        p95_ms: 500,
        notes: "warm-request-through-first-stdout",
    },
    RequiredLifecycleLatencyTarget {
        name: "start.agent_task_ready_ms",
        p50_ms: 50,
        p95_ms: 75,
        notes: "external-request-through-first-useful-stdout",
    },
    RequiredLifecycleLatencyTarget {
        name: "pool.lease_ms",
        p50_ms: 1,
        p95_ms: 5,
        notes: "pool-lease-only",
    },
    RequiredLifecycleLatencyTarget {
        name: "exec.command_start_ms",
        p50_ms: 15,
        p95_ms: 25,
        notes: "exec-request-sent-through-process-start",
    },
    RequiredLifecycleLatencyTarget {
        name: "exec.first_stdout_byte_ms",
        p50_ms: 20,
        p95_ms: 35,
        notes: "exec-request-sent-through-first-stdout-byte",
    },
    RequiredLifecycleLatencyTarget {
        name: "exec.batch_100_small_commands_ms",
        p50_ms: 2_000,
        p95_ms: 3_000,
        notes: "batch-100-small-commands",
    },
];
/// Return the default lifecycle latency p95 SLO gate targets.
#[must_use]
pub fn default_lifecycle_latency_slo_targets(min_samples: usize) -> Vec<BenchmarkSloTarget> {
    REQUIRED_LIFECYCLE_LATENCY_TARGETS
        .iter()
        .map(|target| {
            BenchmarkSloTarget::new(
                target.name,
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                f64::from(target.p95_ms),
                min_samples,
            )
        })
        .collect()
}
/// Benchmark evidence validation error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum BenchmarkEvidenceError {
    /// Required lifecycle latency metric is absent.
    #[error("missing required lifecycle latency metric `{metric}`")]
    MissingLifecycleLatency {
        /// Missing metric name.
        metric: String,
    },
    /// Required metric used the wrong kind or unit.
    #[error("required lifecycle latency metric `{metric}` must use lifecycle latency milliseconds")]
    WrongLifecycleLatencyShape {
        /// Metric with the wrong shape.
        metric: String,
    },
    /// Required Firkin overhead metric is absent.
    #[error("missing required Firkin overhead metric `{metric}`")]
    MissingFirkinOverhead {
        /// Missing metric name.
        metric: String,
    },
    /// Required Firkin overhead metric used the wrong kind or unit.
    #[error("required Firkin overhead metric `{metric}` used the wrong shape")]
    WrongFirkinOverheadShape {
        /// Metric with the wrong shape.
        metric: String,
    },
    /// Metric summary could not be built.
    #[error("benchmark summary for `{metric}` is invalid: {source}")]
    Summary {
        /// Metric being summarized.
        metric: String,
        /// Source summary error.
        source: BenchmarkSummaryError,
    },
}
/// Validated benchmark evidence for the required production lifecycle metrics.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct BenchmarkEvidenceReport {
    pub(crate) required_metrics: Vec<String>,
    pub(crate) summaries: Vec<BenchmarkSummary>,
}
impl BenchmarkEvidenceReport {
    /// Validate samples and summarize all required production lifecycle metrics.
    ///
    /// # Errors
    ///
    /// Returns [`BenchmarkEvidenceError`] when a required metric is absent,
    /// uses the wrong shape, or cannot be summarized.
    pub fn from_samples(
        samples: impl IntoIterator<Item = BenchmarkSample>,
    ) -> std::result::Result<Self, BenchmarkEvidenceError> {
        let mut grouped = BTreeMap::<String, Vec<BenchmarkSample>>::new();
        for sample in samples {
            grouped
                .entry(sample.metric().to_owned())
                .or_default()
                .push(sample);
        }
        let mut summaries = Vec::with_capacity(grouped.len());
        for metric in REQUIRED_LIFECYCLE_LATENCY_METRICS {
            let Some(samples) = grouped.get(*metric) else {
                return Err(BenchmarkEvidenceError::MissingLifecycleLatency {
                    metric: (*metric).to_owned(),
                });
            };
            if samples.iter().any(|sample| {
                sample.kind() != BenchmarkMetricKind::LifecycleLatency
                    || sample.unit() != BenchmarkUnit::Milliseconds
            }) {
                return Err(BenchmarkEvidenceError::WrongLifecycleLatencyShape {
                    metric: (*metric).to_owned(),
                });
            }
            let summary = BenchmarkSummary::from_samples((*metric).to_owned(), samples.clone())
                .map_err(|source| BenchmarkEvidenceError::Summary {
                    metric: (*metric).to_owned(),
                    source,
                })?;
            summaries.push(summary);
        }
        for (metric, samples) in grouped {
            if REQUIRED_LIFECYCLE_LATENCY_METRICS
                .iter()
                .any(|required| *required == metric)
            {
                continue;
            }
            let summary =
                BenchmarkSummary::from_samples(metric.clone(), samples).map_err(|source| {
                    BenchmarkEvidenceError::Summary {
                        metric: metric.clone(),
                        source,
                    }
                })?;
            summaries.push(summary);
        }
        Ok(Self {
            required_metrics: REQUIRED_LIFECYCLE_LATENCY_METRICS
                .iter()
                .map(|metric| (*metric).to_owned())
                .collect(),
            summaries,
        })
    }
    /// Return the required metric names covered by this evidence report.
    #[must_use]
    pub fn required_metrics(&self) -> Vec<&str> {
        self.required_metrics
            .iter()
            .map(std::string::String::as_str)
            .collect()
    }
    /// Return the lifecycle latency summaries.
    #[must_use]
    pub fn summaries(&self) -> &[BenchmarkSummary] {
        &self.summaries
    }
    /// Return the summary for one metric.
    #[must_use]
    pub fn summary_for(&self, metric: &str) -> Option<&BenchmarkSummary> {
        self.summaries
            .iter()
            .find(|summary| summary.metric() == metric)
    }
}
/// Durable benchmark evidence artifact.
pub struct BenchmarkEvidenceArtifact;
impl BenchmarkEvidenceArtifact {
    /// Write a benchmark evidence report as pretty JSON.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] when serialization or writing fails.
    pub fn write_json(path: impl AsRef<Path>, report: &BenchmarkEvidenceReport) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(report).map_err(io::Error::other)?;
        fs::write(path, bytes)
    }
    /// Read a benchmark evidence report from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] when reading or deserialization fails.
    pub fn read_json(path: impl AsRef<Path>) -> io::Result<BenchmarkEvidenceReport> {
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_report_preserves_extra_p0_summaries_without_requiring_them() {
        let mut samples = REQUIRED_LIFECYCLE_LATENCY_METRICS
            .iter()
            .map(|metric| {
                BenchmarkSample::new(
                    *metric,
                    BenchmarkMetricKind::LifecycleLatency,
                    BenchmarkUnit::Milliseconds,
                    1.0,
                )
            })
            .collect::<Vec<_>>();
        samples.push(BenchmarkSample::new(
            "sandbox.disk.metadata_create_stat_unlink_ms",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            8.0,
        ));
        samples.push(BenchmarkSample::new(
            "sandbox.disk.fsync_p99_us",
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Microseconds,
            111.0,
        ));

        let report = BenchmarkEvidenceReport::from_samples(samples).expect("report");

        assert!(
            report
                .summary_for("sandbox.disk.metadata_create_stat_unlink_ms")
                .is_some()
        );
        assert!(report.summary_for("sandbox.disk.fsync_p99_us").is_some());
        assert_eq!(
            report.required_metrics(),
            REQUIRED_LIFECYCLE_LATENCY_METRICS.to_vec()
        );
    }
}
