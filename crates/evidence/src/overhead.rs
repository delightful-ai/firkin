//! overhead — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::benchmark::BenchmarkSummary;
#[allow(unused_imports)]
use crate::lifecycle::BenchmarkEvidenceError;
#[allow(unused_imports)]
use crate::slo::BenchmarkSloTarget;
#[allow(unused_imports)]
use firkin_trace::BenchmarkMetricKind;
#[allow(unused_imports)]
use firkin_trace::BenchmarkSample;
#[allow(unused_imports)]
use firkin_trace::BenchmarkUnit;
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::Path;
/// Required Firkin overhead metric shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RequiredFirkinOverheadMetric {
    /// Required metric name.
    pub name: &'static str,
    /// Required measurement unit.
    pub unit: BenchmarkUnit,
    /// Required p95 ceiling for the metric.
    pub max_p95: f64,
    /// Operator-facing target note.
    pub notes: &'static str,
}
/// Required Firkin/Cube overhead metrics for production substrate evidence.
pub const REQUIRED_FIRKIN_OVERHEAD_METRICS: &[RequiredFirkinOverheadMetric] = &[
    RequiredFirkinOverheadMetric {
        name: "control_plane_cpu_idle",
        unit: BenchmarkUnit::Percent,
        max_p95: 1.0,
        notes: "firkin-and-cube-idle-tax-excluding-running-vms",
    },
    RequiredFirkinOverheadMetric {
        name: "control_plane_rss_idle",
        unit: BenchmarkUnit::Mebibytes,
        max_p95: 256.0,
        notes: "firkin-and-cube-resident-memory-excluding-vm-guest-memory",
    },
    RequiredFirkinOverheadMetric {
        name: "per_sandbox_host_rss",
        unit: BenchmarkUnit::Mebibytes,
        max_p95: 128.0,
        notes: "host-side-bookkeeping-excluding-vm-guest-memory-and-rootfs-artifacts-calibrated-for-signed-live-apple-vz",
    },
    RequiredFirkinOverheadMetric {
        name: "disk_metadata_growth",
        unit: BenchmarkUnit::Bytes,
        max_p95: 1_048_576.0,
        notes: "firkin-control-metadata-growth-per-sandbox-excluding-rootfs-and-snapshot-bytes",
    },
    RequiredFirkinOverheadMetric {
        name: "idle_wakeup_rate",
        unit: BenchmarkUnit::Hertz,
        max_p95: 1.0,
        notes: "steady-state-runtime-wakeups-with-warm-pool-maintained",
    },
];
/// Return the default Firkin overhead p95 SLO gate targets.
#[must_use]
pub fn default_firkin_overhead_slo_targets(min_samples: usize) -> Vec<BenchmarkSloTarget> {
    REQUIRED_FIRKIN_OVERHEAD_METRICS
        .iter()
        .map(|metric| {
            BenchmarkSloTarget::new(
                metric.name,
                BenchmarkMetricKind::FirkinOverhead,
                metric.unit,
                metric.max_p95,
                min_samples,
            )
        })
        .collect()
}
/// Validated benchmark evidence for required Firkin overhead metrics.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct BenchmarkOverheadEvidenceReport {
    pub(crate) required_metrics: Vec<String>,
    pub(crate) summaries: Vec<BenchmarkSummary>,
}
impl BenchmarkOverheadEvidenceReport {
    /// Validate samples and summarize all required Firkin overhead metrics.
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
        for metric in REQUIRED_FIRKIN_OVERHEAD_METRICS {
            let Some(samples) = grouped.get(metric.name) else {
                return Err(BenchmarkEvidenceError::MissingFirkinOverhead {
                    metric: metric.name.to_owned(),
                });
            };
            if samples.iter().any(|sample| {
                sample.kind() != BenchmarkMetricKind::FirkinOverhead || sample.unit() != metric.unit
            }) {
                return Err(BenchmarkEvidenceError::WrongFirkinOverheadShape {
                    metric: metric.name.to_owned(),
                });
            }
            let summary = BenchmarkSummary::from_samples(metric.name.to_owned(), samples.clone())
                .map_err(|source| BenchmarkEvidenceError::Summary {
                metric: metric.name.to_owned(),
                source,
            })?;
            summaries.push(summary);
        }
        for (metric, samples) in grouped {
            if REQUIRED_FIRKIN_OVERHEAD_METRICS
                .iter()
                .any(|required| required.name == metric)
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
            required_metrics: REQUIRED_FIRKIN_OVERHEAD_METRICS
                .iter()
                .map(|metric| metric.name.to_owned())
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
    /// Return the overhead summaries.
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
/// Durable Firkin overhead evidence artifact.
pub struct BenchmarkOverheadEvidenceArtifact;
impl BenchmarkOverheadEvidenceArtifact {
    /// Write an overhead evidence report as pretty JSON.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] when serialization or writing fails.
    pub fn write_json(
        path: impl AsRef<Path>,
        report: &BenchmarkOverheadEvidenceReport,
    ) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(report).map_err(io::Error::other)?;
        fs::write(path, bytes)
    }
    /// Read an overhead evidence report from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] when reading or deserialization fails.
    pub fn read_json(path: impl AsRef<Path>) -> io::Result<BenchmarkOverheadEvidenceReport> {
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overhead_report_preserves_extra_p0_summaries_without_requiring_them() {
        let mut samples = REQUIRED_FIRKIN_OVERHEAD_METRICS
            .iter()
            .map(|metric| {
                BenchmarkSample::new(
                    metric.name,
                    BenchmarkMetricKind::FirkinOverhead,
                    metric.unit,
                    1.0,
                )
            })
            .collect::<Vec<_>>();
        samples.push(BenchmarkSample::new(
            "sandbox.mem.idle_host_footprint_bytes",
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Bytes,
            4096.0,
        ));

        let report = BenchmarkOverheadEvidenceReport::from_samples(samples).expect("report");

        assert!(
            report
                .summary_for("sandbox.mem.idle_host_footprint_bytes")
                .is_some()
        );
        assert_eq!(
            report.required_metrics(),
            REQUIRED_FIRKIN_OVERHEAD_METRICS
                .iter()
                .map(|metric| metric.name)
                .collect::<Vec<_>>()
        );
    }
}
