//! Agent benchmark scorecard evidence.
#![allow(missing_docs)]

use std::{collections::BTreeMap, fs, io, path::Path};

use firkin_trace::{BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit};
use thiserror::Error as ThisError;

use crate::{
    AGENT_COMPUTER_DRILLDOWN_METRICS, AGENT_COMPUTER_SCORECARD_METRICS,
    AUTOSCALE_EFFICIENCY_SCORECARD_METRICS, BenchmarkMeasurementStatus, BenchmarkMetricDefinition,
    BenchmarkSummary, BenchmarkSummaryError, P0_SCORECARD_METRICS,
    autoscale_efficiency_measurement_coverage, benchmark_metric_definition,
    required_agent_computer_metric_definitions, required_autoscale_efficiency_metric_definitions,
    required_scorecard_metric_definitions,
};

#[derive(Clone, Debug, PartialEq, ThisError)]
pub enum AgentBenchmarkScorecardError {
    #[error("missing required agent scorecard metric `{metric}`")]
    MissingRequiredMetric { metric: String },
    #[error("agent scorecard metric `{metric}` has wrong shape")]
    WrongRequiredMetricShape {
        metric: String,
        expected_kind: BenchmarkMetricKind,
        actual_kind: BenchmarkMetricKind,
        expected_unit: BenchmarkUnit,
        actual_unit: BenchmarkUnit,
    },
    #[error(
        "agent scorecard metric `{metric}` has too few samples: required {required}, actual {actual}"
    )]
    InsufficientSamples {
        metric: String,
        required: usize,
        actual: usize,
    },
    #[error("agent scorecard summary for `{metric}` is invalid: {source}")]
    Summary {
        metric: String,
        source: BenchmarkSummaryError,
    },
}

#[derive(Clone, Debug, PartialEq, ThisError)]
pub enum AutoscaleEfficiencyScorecardError {
    #[error("missing required autoscale efficiency scorecard metric `{metric}`")]
    MissingRequiredMetric { metric: String },
    #[error("autoscale efficiency scorecard metric `{metric}` has wrong shape")]
    WrongRequiredMetricShape {
        metric: String,
        expected_kind: BenchmarkMetricKind,
        actual_kind: BenchmarkMetricKind,
        expected_unit: BenchmarkUnit,
        actual_unit: BenchmarkUnit,
    },
    #[error(
        "autoscale efficiency scorecard metric `{metric}` has too few samples: required {required}, actual {actual}"
    )]
    InsufficientSamples {
        metric: String,
        required: usize,
        actual: usize,
    },
    #[error("autoscale efficiency scorecard summary for `{metric}` is invalid: {source}")]
    Summary {
        metric: String,
        source: BenchmarkSummaryError,
    },
}

#[derive(Clone, Debug, PartialEq, ThisError)]
pub enum AgentComputerScorecardError {
    #[error("missing required agent-computer scorecard metric `{metric}`")]
    MissingRequiredMetric { metric: String },
    #[error("agent-computer scorecard metric `{metric}` has wrong shape")]
    WrongRequiredMetricShape {
        metric: String,
        expected_kind: BenchmarkMetricKind,
        actual_kind: BenchmarkMetricKind,
        expected_unit: BenchmarkUnit,
        actual_unit: BenchmarkUnit,
    },
    #[error(
        "agent-computer scorecard metric `{metric}` has too few samples: required {required}, actual {actual}"
    )]
    InsufficientSamples {
        metric: String,
        required: usize,
        actual: usize,
    },
    #[error("agent-computer scorecard summary for `{metric}` is invalid: {source}")]
    Summary {
        metric: String,
        source: BenchmarkSummaryError,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScorecardSnappyTargetDirection {
    AtMost,
    AtLeast,
}

impl ScorecardSnappyTargetDirection {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AtMost => "at_most",
            Self::AtLeast => "at_least",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScorecardSnappyTarget {
    metric: &'static str,
    direction: ScorecardSnappyTargetDirection,
    p95_threshold: f64,
}

impl ScorecardSnappyTarget {
    #[must_use]
    pub const fn at_most_p95(metric: &'static str, p95_threshold: f64) -> Self {
        Self {
            metric,
            direction: ScorecardSnappyTargetDirection::AtMost,
            p95_threshold,
        }
    }

    #[must_use]
    pub const fn at_least_p95(metric: &'static str, p95_threshold: f64) -> Self {
        Self {
            metric,
            direction: ScorecardSnappyTargetDirection::AtLeast,
            p95_threshold,
        }
    }

    #[must_use]
    pub const fn metric(self) -> &'static str {
        self.metric
    }

    #[must_use]
    pub const fn direction(self) -> ScorecardSnappyTargetDirection {
        self.direction
    }

    #[must_use]
    pub const fn p95_threshold(self) -> f64 {
        self.p95_threshold
    }

    #[must_use]
    pub fn evaluate(self, summary: &BenchmarkSummary) -> Option<ScorecardSnappyTargetMiss> {
        let actual = summary.p95();
        let passed = match self.direction {
            ScorecardSnappyTargetDirection::AtMost => actual <= self.p95_threshold,
            ScorecardSnappyTargetDirection::AtLeast => actual >= self.p95_threshold,
        };
        (!passed).then(|| ScorecardSnappyTargetMiss {
            metric: self.metric.to_owned(),
            direction: self.direction,
            p95_threshold: self.p95_threshold,
            actual_p95: actual,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScorecardSnappyTargetMiss {
    metric: String,
    direction: ScorecardSnappyTargetDirection,
    p95_threshold: f64,
    actual_p95: f64,
}

impl ScorecardSnappyTargetMiss {
    #[must_use]
    pub fn metric(&self) -> &str {
        &self.metric
    }

    #[must_use]
    pub const fn direction(&self) -> ScorecardSnappyTargetDirection {
        self.direction
    }

    #[must_use]
    pub const fn p95_threshold(&self) -> f64 {
        self.p95_threshold
    }

    #[must_use]
    pub const fn actual_p95(&self) -> f64 {
        self.actual_p95
    }
}

pub const AGENT_SCORECARD_SNAPPY_TARGETS: &[ScorecardSnappyTarget] = &[
    ScorecardSnappyTarget::at_most_p95("start.hot_to_first_stdout_ms", 75.0),
    ScorecardSnappyTarget::at_most_p95("start.resume_to_first_stdout_ms", 35.0),
    ScorecardSnappyTarget::at_most_p95("start.warm_to_first_stdout_ms", 350.0),
    ScorecardSnappyTarget::at_most_p95("start.agent_task_ready_ms", 150.0),
    ScorecardSnappyTarget::at_most_p95("pool.lease_ms", 1.0),
    ScorecardSnappyTarget::at_most_p95("exec.command_start_ms", 20.0),
    ScorecardSnappyTarget::at_most_p95("exec.first_stdout_byte_ms", 25.0),
    ScorecardSnappyTarget::at_most_p95("exec.batch_100_small_commands_ms", 500.0),
    ScorecardSnappyTarget::at_least_p95(
        "density.max_active_before_hot_to_first_stdout_p95_doubles",
        8.0,
    ),
    ScorecardSnappyTarget::at_most_p95("disk.sparse_bloat_after_trim", 1.25),
    ScorecardSnappyTarget::at_most_p95("cleanup.leftover_bytes", 0.0),
    ScorecardSnappyTarget::at_most_p95("reliability.unknown_failure_rate", 0.0),
];

pub const AUTOSCALE_SCORECARD_SNAPPY_TARGETS: &[ScorecardSnappyTarget] = &[
    ScorecardSnappyTarget::at_least_p95("autoscale.ready_queue_hit_rate_pct", 90.0),
    ScorecardSnappyTarget::at_most_p95("product.agent_computer_ready_ms", 250.0),
    ScorecardSnappyTarget::at_most_p95("product.agent_computer_resume_ms", 75.0),
    ScorecardSnappyTarget::at_least_p95("autoscale.safe_spare_limiting_utilization_pct", 70.0),
    ScorecardSnappyTarget::at_most_p95("autoscale.pressure_to_safe_floor_ms", 5_000.0),
    ScorecardSnappyTarget::at_most_p95("autoscale.pressure_clear_to_ready_target_ms", 10_000.0),
    ScorecardSnappyTarget::at_least_p95(
        "density.max_agent_computers_before_ready_p95_doubles",
        4.0,
    ),
    ScorecardSnappyTarget::at_least_p95(
        "density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles",
        4.0,
    ),
    ScorecardSnappyTarget::at_most_p95("autoscale.active_evictions_due_to_pool_pressure", 0.0),
    ScorecardSnappyTarget::at_most_p95("autoscale.reserve_floor_violations", 0.0),
    ScorecardSnappyTarget::at_most_p95("cleanup.leftover_bytes", 0.0),
    ScorecardSnappyTarget::at_most_p95("reliability.unknown_failure_rate", 0.0),
];

pub const AGENT_COMPUTER_SCORECARD_SNAPPY_TARGETS: &[ScorecardSnappyTarget] = &[
    ScorecardSnappyTarget::at_most_p95("product.agent_computer_ready_ms", 250.0),
    ScorecardSnappyTarget::at_most_p95("product.agent_computer_resume_ms", 75.0),
    ScorecardSnappyTarget::at_least_p95(
        "density.max_agent_computers_before_ready_p95_doubles",
        4.0,
    ),
    ScorecardSnappyTarget::at_most_p95("cleanup.leftover_bytes", 0.0),
    ScorecardSnappyTarget::at_most_p95("reliability.unknown_failure_rate", 0.0),
];

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct AgentBenchmarkScorecardReport {
    required_metrics: Vec<String>,
    summaries: Vec<BenchmarkSummary>,
}

impl AgentBenchmarkScorecardReport {
    #[must_use]
    pub fn required_metric_names() -> &'static [&'static str] {
        P0_SCORECARD_METRICS
    }

    pub fn from_samples(
        samples: impl IntoIterator<Item = BenchmarkSample>,
    ) -> Result<Self, AgentBenchmarkScorecardError> {
        Self::from_samples_with_min_samples(samples, 1)
    }

    pub fn from_samples_with_min_samples(
        samples: impl IntoIterator<Item = BenchmarkSample>,
        min_samples: usize,
    ) -> Result<Self, AgentBenchmarkScorecardError> {
        let mut grouped = BTreeMap::<String, Vec<BenchmarkSample>>::new();
        for sample in samples {
            grouped
                .entry(sample.metric().to_owned())
                .or_default()
                .push(sample);
        }

        let definitions = required_scorecard_metric_definitions();
        let mut summaries = Vec::with_capacity(definitions.len());
        for definition in definitions {
            let samples = samples_for_definition(&grouped, definition)?;
            let summary = BenchmarkSummary::from_samples(definition.name, samples.clone())
                .map_err(|source| AgentBenchmarkScorecardError::Summary {
                    metric: definition.name.to_owned(),
                    source,
                })?;
            if summary.count() < min_samples {
                return Err(AgentBenchmarkScorecardError::InsufficientSamples {
                    metric: definition.name.to_owned(),
                    required: min_samples,
                    actual: summary.count(),
                });
            }
            summaries.push(summary);
        }

        Ok(Self {
            required_metrics: P0_SCORECARD_METRICS
                .iter()
                .map(|metric| (*metric).to_owned())
                .collect(),
            summaries,
        })
    }

    pub fn validate_min_samples(
        &self,
        min_samples: usize,
    ) -> Result<(), AgentBenchmarkScorecardError> {
        for definition in required_scorecard_metric_definitions() {
            let Some(summary) = self.summary_for(definition.name) else {
                return Err(AgentBenchmarkScorecardError::MissingRequiredMetric {
                    metric: definition.name.to_owned(),
                });
            };
            if summary.kind() != definition.kind || summary.unit() != definition.unit {
                return Err(AgentBenchmarkScorecardError::WrongRequiredMetricShape {
                    metric: definition.name.to_owned(),
                    expected_kind: definition.kind,
                    actual_kind: summary.kind(),
                    expected_unit: definition.unit,
                    actual_unit: summary.unit(),
                });
            }
            if summary.count() < min_samples {
                return Err(AgentBenchmarkScorecardError::InsufficientSamples {
                    metric: definition.name.to_owned(),
                    required: min_samples,
                    actual: summary.count(),
                });
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn snappy_target_misses(&self) -> Vec<ScorecardSnappyTargetMiss> {
        scorecard_snappy_target_misses(self.summaries(), AGENT_SCORECARD_SNAPPY_TARGETS)
    }

    #[must_use]
    pub fn required_metrics(&self) -> Vec<&str> {
        self.required_metrics
            .iter()
            .map(std::string::String::as_str)
            .collect()
    }

    #[must_use]
    pub fn summaries(&self) -> &[BenchmarkSummary] {
        &self.summaries
    }

    #[must_use]
    pub fn summary_for(&self, metric: &str) -> Option<&BenchmarkSummary> {
        self.summaries
            .iter()
            .find(|summary| summary.metric() == metric)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct AutoscaleEfficiencyScorecardReport {
    required_metrics: Vec<String>,
    summaries: Vec<BenchmarkSummary>,
    promotion_blockers: Vec<AutoscaleScorecardPromotionBlocker>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct AutoscaleScorecardPromotionBlocker {
    metric: String,
    blocker: String,
    next_action: String,
}

impl AutoscaleScorecardPromotionBlocker {
    #[must_use]
    pub fn new(
        metric: impl Into<String>,
        blocker: impl Into<String>,
        next_action: impl Into<String>,
    ) -> Self {
        Self {
            metric: metric.into(),
            blocker: blocker.into(),
            next_action: next_action.into(),
        }
    }

    #[must_use]
    pub fn metric(&self) -> &str {
        &self.metric
    }

    #[must_use]
    pub fn blocker(&self) -> &str {
        &self.blocker
    }

    #[must_use]
    pub fn next_action(&self) -> &str {
        &self.next_action
    }
}

impl AutoscaleEfficiencyScorecardReport {
    #[must_use]
    pub fn required_metric_names() -> &'static [&'static str] {
        AUTOSCALE_EFFICIENCY_SCORECARD_METRICS
    }

    pub fn from_samples(
        samples: impl IntoIterator<Item = BenchmarkSample>,
    ) -> Result<Self, AutoscaleEfficiencyScorecardError> {
        Self::from_samples_with_min_samples(samples, 1)
    }

    pub fn from_samples_with_min_samples(
        samples: impl IntoIterator<Item = BenchmarkSample>,
        min_samples: usize,
    ) -> Result<Self, AutoscaleEfficiencyScorecardError> {
        let mut grouped = BTreeMap::<String, Vec<BenchmarkSample>>::new();
        for sample in samples {
            grouped
                .entry(sample.metric().to_owned())
                .or_default()
                .push(sample);
        }

        let definitions = required_autoscale_efficiency_metric_definitions();
        let mut summaries = Vec::with_capacity(definitions.len());
        for definition in definitions {
            let samples = autoscale_samples_for_definition(&grouped, definition)?;
            let summary = BenchmarkSummary::from_samples(definition.name, samples.clone())
                .map_err(|source| AutoscaleEfficiencyScorecardError::Summary {
                    metric: definition.name.to_owned(),
                    source,
                })?;
            if summary.count() < min_samples {
                return Err(AutoscaleEfficiencyScorecardError::InsufficientSamples {
                    metric: definition.name.to_owned(),
                    required: min_samples,
                    actual: summary.count(),
                });
            }
            summaries.push(summary);
        }

        Ok(Self {
            required_metrics: AUTOSCALE_EFFICIENCY_SCORECARD_METRICS
                .iter()
                .map(|metric| (*metric).to_owned())
                .collect(),
            summaries,
            promotion_blockers: autoscale_promotion_blockers(&grouped),
        })
    }

    pub fn validate_min_samples(
        &self,
        min_samples: usize,
    ) -> Result<(), AutoscaleEfficiencyScorecardError> {
        for definition in required_autoscale_efficiency_metric_definitions() {
            let Some(summary) = self.summary_for(definition.name) else {
                return Err(AutoscaleEfficiencyScorecardError::MissingRequiredMetric {
                    metric: definition.name.to_owned(),
                });
            };
            if summary.kind() != definition.kind || summary.unit() != definition.unit {
                return Err(
                    AutoscaleEfficiencyScorecardError::WrongRequiredMetricShape {
                        metric: definition.name.to_owned(),
                        expected_kind: definition.kind,
                        actual_kind: summary.kind(),
                        expected_unit: definition.unit,
                        actual_unit: summary.unit(),
                    },
                );
            }
            if summary.count() < min_samples {
                return Err(AutoscaleEfficiencyScorecardError::InsufficientSamples {
                    metric: definition.name.to_owned(),
                    required: min_samples,
                    actual: summary.count(),
                });
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn snappy_target_misses(&self) -> Vec<ScorecardSnappyTargetMiss> {
        scorecard_snappy_target_misses(self.summaries(), AUTOSCALE_SCORECARD_SNAPPY_TARGETS)
    }

    #[must_use]
    pub fn required_metrics(&self) -> Vec<&str> {
        self.required_metrics
            .iter()
            .map(std::string::String::as_str)
            .collect()
    }

    #[must_use]
    pub fn summaries(&self) -> &[BenchmarkSummary] {
        &self.summaries
    }

    #[must_use]
    pub fn promotion_blockers(&self) -> &[AutoscaleScorecardPromotionBlocker] {
        &self.promotion_blockers
    }

    #[must_use]
    pub fn summary_for(&self, metric: &str) -> Option<&BenchmarkSummary> {
        self.summaries
            .iter()
            .find(|summary| summary.metric() == metric)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct AgentComputerScorecardReport {
    required_metrics: Vec<String>,
    summaries: Vec<BenchmarkSummary>,
    promotion_blockers: Vec<AgentComputerScorecardPromotionBlocker>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct AgentComputerScorecardPromotionBlocker {
    metric: String,
    blocker: String,
    next_action: String,
}

impl AgentComputerScorecardPromotionBlocker {
    #[must_use]
    pub fn new(
        metric: impl Into<String>,
        blocker: impl Into<String>,
        next_action: impl Into<String>,
    ) -> Self {
        Self {
            metric: metric.into(),
            blocker: blocker.into(),
            next_action: next_action.into(),
        }
    }

    #[must_use]
    pub fn metric(&self) -> &str {
        &self.metric
    }

    #[must_use]
    pub fn blocker(&self) -> &str {
        &self.blocker
    }

    #[must_use]
    pub fn next_action(&self) -> &str {
        &self.next_action
    }
}

impl AgentComputerScorecardReport {
    #[must_use]
    pub fn required_metric_names() -> &'static [&'static str] {
        AGENT_COMPUTER_SCORECARD_METRICS
    }

    pub fn from_samples(
        samples: impl IntoIterator<Item = BenchmarkSample>,
    ) -> Result<Self, AgentComputerScorecardError> {
        Self::from_samples_with_min_samples(samples, 1)
    }

    pub fn from_samples_with_min_samples(
        samples: impl IntoIterator<Item = BenchmarkSample>,
        min_samples: usize,
    ) -> Result<Self, AgentComputerScorecardError> {
        let mut grouped = BTreeMap::<String, Vec<BenchmarkSample>>::new();
        for sample in samples {
            grouped
                .entry(sample.metric().to_owned())
                .or_default()
                .push(sample);
        }

        let definitions = required_agent_computer_metric_definitions();
        let mut summaries = Vec::with_capacity(definitions.len());
        for definition in definitions {
            let samples = agent_computer_samples_for_definition(&grouped, definition)?;
            let summary = BenchmarkSummary::from_samples(definition.name, samples.clone())
                .map_err(|source| AgentComputerScorecardError::Summary {
                    metric: definition.name.to_owned(),
                    source,
                })?;
            if summary.count() < min_samples {
                return Err(AgentComputerScorecardError::InsufficientSamples {
                    metric: definition.name.to_owned(),
                    required: min_samples,
                    actual: summary.count(),
                });
            }
            summaries.push(summary);
        }
        for metric in AGENT_COMPUTER_DRILLDOWN_METRICS {
            let Some(samples) = grouped.get(*metric) else {
                continue;
            };
            let definition = benchmark_metric_definition(metric)
                .expect("agent-computer drilldown metric must exist in benchmark catalog");
            if samples
                .iter()
                .any(|sample| sample.kind() != definition.kind || sample.unit() != definition.unit)
            {
                return Err(AgentComputerScorecardError::WrongRequiredMetricShape {
                    metric: definition.name.to_owned(),
                    expected_kind: definition.kind,
                    actual_kind: samples
                        .iter()
                        .find(|sample| {
                            sample.kind() != definition.kind || sample.unit() != definition.unit
                        })
                        .expect("wrong drilldown shape")
                        .kind(),
                    expected_unit: definition.unit,
                    actual_unit: samples
                        .iter()
                        .find(|sample| {
                            sample.kind() != definition.kind || sample.unit() != definition.unit
                        })
                        .expect("wrong drilldown shape")
                        .unit(),
                });
            }
            let summary = BenchmarkSummary::from_samples(definition.name, samples.clone())
                .map_err(|source| AgentComputerScorecardError::Summary {
                    metric: definition.name.to_owned(),
                    source,
                })?;
            summaries.push(summary);
        }

        Ok(Self {
            required_metrics: AGENT_COMPUTER_SCORECARD_METRICS
                .iter()
                .map(|metric| (*metric).to_owned())
                .collect(),
            summaries,
            promotion_blockers: agent_computer_promotion_blockers(&grouped),
        })
    }

    pub fn validate_min_samples(
        &self,
        min_samples: usize,
    ) -> Result<(), AgentComputerScorecardError> {
        for definition in required_agent_computer_metric_definitions() {
            let Some(summary) = self.summary_for(definition.name) else {
                return Err(AgentComputerScorecardError::MissingRequiredMetric {
                    metric: definition.name.to_owned(),
                });
            };
            if summary.kind() != definition.kind || summary.unit() != definition.unit {
                return Err(AgentComputerScorecardError::WrongRequiredMetricShape {
                    metric: definition.name.to_owned(),
                    expected_kind: definition.kind,
                    actual_kind: summary.kind(),
                    expected_unit: definition.unit,
                    actual_unit: summary.unit(),
                });
            }
            if summary.count() < min_samples {
                return Err(AgentComputerScorecardError::InsufficientSamples {
                    metric: definition.name.to_owned(),
                    required: min_samples,
                    actual: summary.count(),
                });
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn snappy_target_misses(&self) -> Vec<ScorecardSnappyTargetMiss> {
        scorecard_snappy_target_misses(self.summaries(), AGENT_COMPUTER_SCORECARD_SNAPPY_TARGETS)
    }

    #[must_use]
    pub fn required_metrics(&self) -> Vec<&str> {
        self.required_metrics
            .iter()
            .map(std::string::String::as_str)
            .collect()
    }

    #[must_use]
    pub fn summaries(&self) -> &[BenchmarkSummary] {
        &self.summaries
    }

    #[must_use]
    pub fn promotion_blockers(&self) -> &[AgentComputerScorecardPromotionBlocker] {
        &self.promotion_blockers
    }

    #[must_use]
    pub fn summary_for(&self, metric: &str) -> Option<&BenchmarkSummary> {
        self.summaries
            .iter()
            .find(|summary| summary.metric() == metric)
    }
}

fn agent_computer_promotion_blockers(
    grouped: &BTreeMap<String, Vec<BenchmarkSample>>,
) -> Vec<AgentComputerScorecardPromotionBlocker> {
    let mut blockers = Vec::new();
    if grouped
        .get("product.database_ready_ms")
        .into_iter()
        .flatten()
        .any(|sample| {
            sample.tag_value("measurement_boundary") == Some("sqlite_proxy_not_db_sidecar")
        })
    {
        blockers.push(AgentComputerScorecardPromotionBlocker::new(
            "product.database_ready_ms",
            "database readiness is measured by SQLite through code-interpreter, not a real database sidecar health probe",
            "emit DatabaseReady from a real DB sidecar health check before enforcing the database-sidecar SLA",
        ));
    }
    for metric in [
        "product.agent_computer_ready_ms",
        "product.agent_computer_resume_ms",
    ] {
        if grouped.get(metric).into_iter().flatten().any(|sample| {
            sample.tag_value("cli_boundary") != Some("real_cli")
                || sample.tag_value("browser_boundary") != Some("real_browser_sidecar")
                || sample.tag_value("database_boundary") != Some("real_db_sidecar")
        }) {
            blockers.push(AgentComputerScorecardPromotionBlocker::new(
                metric,
                "agent-computer readiness lacks real CLI, browser, or database product health boundaries",
                "tag product readiness with cli_boundary=real_cli, browser_boundary=real_browser_sidecar, and database_boundary=real_db_sidecar only after those checks gate AgentComputerReady",
            ));
        }
    }
    blockers
}

fn scorecard_snappy_target_misses(
    summaries: &[BenchmarkSummary],
    targets: &[ScorecardSnappyTarget],
) -> Vec<ScorecardSnappyTargetMiss> {
    targets
        .iter()
        .filter_map(|target| {
            summaries
                .iter()
                .find(|summary| summary.metric() == target.metric())
                .and_then(|summary| target.evaluate(summary))
        })
        .collect()
}

fn autoscale_promotion_blockers(
    grouped: &BTreeMap<String, Vec<BenchmarkSample>>,
) -> Vec<AutoscaleScorecardPromotionBlocker> {
    autoscale_efficiency_measurement_coverage()
        .iter()
        .filter(|coverage| {
            coverage.status != BenchmarkMeasurementStatus::SignedLiveExact
                && !autoscale_metric_has_promotable_samples(grouped, coverage.metric)
        })
        .map(|coverage| {
            AutoscaleScorecardPromotionBlocker::new(
                coverage.metric,
                format!(
                    "autoscale scorecard metric is {}, not signed_live_exact",
                    coverage.status.as_str()
                ),
                format!(
                    "replace {} with signed-live autoscale harness evidence before enforcing this SLA",
                    coverage.source
                ),
            )
        })
        .collect()
}

fn autoscale_metric_has_promotable_samples(
    grouped: &BTreeMap<String, Vec<BenchmarkSample>>,
    metric: &str,
) -> bool {
    grouped.get(metric).is_some_and(|samples| {
        !samples.is_empty()
            && samples
                .iter()
                .all(|sample| autoscale_sample_promotes_metric(metric, sample))
    })
}

fn autoscale_sample_promotes_metric(metric: &str, sample: &BenchmarkSample) -> bool {
    match metric {
        "autoscale.ready_queue_hit_rate_pct" => {
            sample.tag_value("source") == Some("autoscale-ready-queue-outcomes")
                && sample.tag_value("measurement_boundary") == Some("signed_live_product_path")
                && sample.tag_value("request_classification")
                    == Some("hot_or_resumed_ready_capacity")
                && sample.tag_value("demand_source") == Some("agent_computer_scorecard_harness")
                && sample.tag_value("outcome_source") == Some("observed_product_request_results")
        }
        "autoscale.safe_spare_limiting_utilization_pct" => {
            sample.tag_value("source") == Some("autoscale-safe-spare-utilization")
                && sample.tag_value("measurement_boundary")
                    == Some("signed_live_resource_accounting")
                && sample.tag_value("total_resource_source") == Some("host_capacity_probe")
                && sample.tag_value("active_resource_source")
                    == Some("runtime_active_pod_registry_budget")
                && sample.tag_value("reserved_floor_source") == Some("runtime_reserve_floor_config")
                && sample.tag_value("ready_queue_resource_source")
                    == Some("observed_ready_hit_budget")
                && sample.tag_value("resource_accounting_scope")
                    == Some("agent_computer_scorecard_harness_observation")
                && sample.tag_value("limiting_resource").is_some()
        }
        "product.agent_computer_ready_ms" | "product.agent_computer_resume_ms" => {
            sample.tag_value("trust") == Some("exact_host_event_pair")
                && sample.tag_value("measurement_boundary") == Some("product_path")
                && sample.tag_value("cli_boundary") == Some("real_cli")
                && sample.tag_value("browser_boundary") == Some("real_browser_sidecar")
                && sample.tag_value("database_boundary") == Some("real_db_sidecar")
        }
        "density.max_agent_computers_before_ready_p95_doubles" => {
            sample.tag_value("measurement_boundary") == Some("product_path")
                && sample.tag_value("pod_surface") == Some("product_pod_ready_deck")
                && sample.tag_value("excludes_container_add") == Some("false")
                && sample.tag_value("ready_signal")
                    == Some("agent_computer_ready_after_container_add")
                && sample.tag_value("cli_boundary") == Some("real_cli")
                && sample.tag_value("browser_boundary") == Some("real_browser_sidecar")
                && sample.tag_value("database_boundary") == Some("real_db_sidecar")
        }
        "density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles" => {
            sample.tag_value("measurement_boundary") == Some("prestarted_slot_checkout")
                && sample.tag_value("slot_surface") == Some("prestarted_agent_slot")
                && sample.tag_value("excludes_container_add") == Some("true")
                && sample.tag_value("ready_signal") == Some("request_fifo_acceptance")
                && sample.tag_value("output_wait_preserved") == Some("true")
        }
        "autoscale.active_evictions_due_to_pool_pressure"
        | "autoscale.reserve_floor_violations" => {
            sample.tag_value("source") == Some("autoscale-protection-counts")
                && sample.tag_value("measurement_boundary") == Some("signed_live_product_path")
                && sample.tag_value("eviction_scope") == Some("active_session_protection")
                && sample.tag_value("reserve_scope") == Some("configured_runtime_floor")
                && sample.tag_value("pressure_policy") == Some("no_pool_comfort_eviction")
                && sample.tag_value("protection_count_source")
                    == Some("observed_harness_completion")
        }
        "autoscale.pressure_to_safe_floor_ms" => {
            sample.tag_value("trust") == Some("exact_host_event_pair")
                && sample.tag_value("measurement_boundary")
                    == Some("signed_live_autoscale_scenario")
                && sample.tag_value("pressure_source") == Some("runtime_pressure_signal")
                && sample.tag_value("safe_floor_source") == Some("runtime_reserve_floor_probe")
                && sample.tag_value("start_event") == Some("PressureDetected")
                && sample.tag_value("end_event") == Some("SafeFloorRestored")
        }
        "autoscale.pressure_clear_to_ready_target_ms" => {
            sample.tag_value("trust") == Some("exact_host_event_pair")
                && sample.tag_value("measurement_boundary")
                    == Some("signed_live_autoscale_scenario")
                && sample.tag_value("pressure_clear_source") == Some("runtime_pressure_signal")
                && sample.tag_value("ready_target_source") == Some("runtime_ready_queue_probe")
                && sample.tag_value("start_event") == Some("SafeFloorRestored")
                && sample.tag_value("end_event") == Some("ReadyTargetRestored")
        }
        _ => false,
    }
}

fn samples_for_definition<'a>(
    grouped: &'a BTreeMap<String, Vec<BenchmarkSample>>,
    definition: &BenchmarkMetricDefinition,
) -> Result<&'a Vec<BenchmarkSample>, AgentBenchmarkScorecardError> {
    let Some(samples) = grouped.get(definition.name) else {
        return Err(AgentBenchmarkScorecardError::MissingRequiredMetric {
            metric: definition.name.to_owned(),
        });
    };
    if let Some(sample) = samples
        .iter()
        .find(|sample| sample.kind() != definition.kind || sample.unit() != definition.unit)
    {
        return Err(AgentBenchmarkScorecardError::WrongRequiredMetricShape {
            metric: definition.name.to_owned(),
            expected_kind: definition.kind,
            actual_kind: sample.kind(),
            expected_unit: definition.unit,
            actual_unit: sample.unit(),
        });
    }
    Ok(samples)
}

fn autoscale_samples_for_definition<'a>(
    grouped: &'a BTreeMap<String, Vec<BenchmarkSample>>,
    definition: &BenchmarkMetricDefinition,
) -> Result<&'a Vec<BenchmarkSample>, AutoscaleEfficiencyScorecardError> {
    let Some(samples) = grouped.get(definition.name) else {
        return Err(AutoscaleEfficiencyScorecardError::MissingRequiredMetric {
            metric: definition.name.to_owned(),
        });
    };
    if let Some(sample) = samples
        .iter()
        .find(|sample| sample.kind() != definition.kind || sample.unit() != definition.unit)
    {
        return Err(
            AutoscaleEfficiencyScorecardError::WrongRequiredMetricShape {
                metric: definition.name.to_owned(),
                expected_kind: definition.kind,
                actual_kind: sample.kind(),
                expected_unit: definition.unit,
                actual_unit: sample.unit(),
            },
        );
    }
    Ok(samples)
}

fn agent_computer_samples_for_definition<'a>(
    grouped: &'a BTreeMap<String, Vec<BenchmarkSample>>,
    definition: &BenchmarkMetricDefinition,
) -> Result<&'a Vec<BenchmarkSample>, AgentComputerScorecardError> {
    let Some(samples) = grouped.get(definition.name) else {
        return Err(AgentComputerScorecardError::MissingRequiredMetric {
            metric: definition.name.to_owned(),
        });
    };
    if let Some(sample) = samples
        .iter()
        .find(|sample| sample.kind() != definition.kind || sample.unit() != definition.unit)
    {
        return Err(AgentComputerScorecardError::WrongRequiredMetricShape {
            metric: definition.name.to_owned(),
            expected_kind: definition.kind,
            actual_kind: sample.kind(),
            expected_unit: definition.unit,
            actual_unit: sample.unit(),
        });
    }
    Ok(samples)
}

pub struct AgentBenchmarkScorecardArtifact;

impl AgentBenchmarkScorecardArtifact {
    pub fn write_json(
        path: impl AsRef<Path>,
        report: &AgentBenchmarkScorecardReport,
    ) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(report).map_err(io::Error::other)?;
        fs::write(path, bytes)
    }

    pub fn read_json(path: impl AsRef<Path>) -> io::Result<AgentBenchmarkScorecardReport> {
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }
}

pub struct AutoscaleEfficiencyScorecardArtifact;

impl AutoscaleEfficiencyScorecardArtifact {
    pub fn write_json(
        path: impl AsRef<Path>,
        report: &AutoscaleEfficiencyScorecardReport,
    ) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(report).map_err(io::Error::other)?;
        fs::write(path, bytes)
    }

    pub fn read_json(path: impl AsRef<Path>) -> io::Result<AutoscaleEfficiencyScorecardReport> {
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }
}

pub struct AgentComputerScorecardArtifact;

impl AgentComputerScorecardArtifact {
    pub fn write_json(
        path: impl AsRef<Path>,
        report: &AgentComputerScorecardReport,
    ) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(report).map_err(io::Error::other)?;
        fs::write(path, bytes)
    }

    pub fn read_json(path: impl AsRef<Path>) -> io::Result<AgentComputerScorecardReport> {
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scorecard_samples(values: impl IntoIterator<Item = f64>) -> Vec<BenchmarkSample> {
        let values = values.into_iter().collect::<Vec<_>>();
        required_scorecard_metric_definitions()
            .into_iter()
            .flat_map(|definition| {
                values.iter().copied().map(move |value| {
                    BenchmarkSample::new(definition.name, definition.kind, definition.unit, value)
                })
            })
            .collect()
    }

    fn autoscale_scorecard_samples(values: impl IntoIterator<Item = f64>) -> Vec<BenchmarkSample> {
        let values = values.into_iter().collect::<Vec<_>>();
        required_autoscale_efficiency_metric_definitions()
            .into_iter()
            .flat_map(|definition| {
                values.iter().copied().map(move |value| {
                    BenchmarkSample::new(definition.name, definition.kind, definition.unit, value)
                })
            })
            .collect()
    }

    fn agent_computer_scorecard_samples(
        values: impl IntoIterator<Item = f64>,
    ) -> Vec<BenchmarkSample> {
        let values = values.into_iter().collect::<Vec<_>>();
        required_agent_computer_metric_definitions()
            .into_iter()
            .flat_map(|definition| {
                values.iter().copied().map(move |value| {
                    let sample = BenchmarkSample::new(
                        definition.name,
                        definition.kind,
                        definition.unit,
                        value,
                    );
                    match definition.name {
                        "product.agent_computer_ready_ms" | "product.agent_computer_resume_ms" => {
                            sample
                                .with_static_tag("cli_boundary", "real_cli")
                                .with_static_tag("browser_boundary", "real_browser_sidecar")
                                .with_static_tag("database_boundary", "real_db_sidecar")
                        }
                        _ => sample,
                    }
                })
            })
            .collect()
    }

    fn proxy_database_ready_samples(values: impl IntoIterator<Item = f64>) -> Vec<BenchmarkSample> {
        values
            .into_iter()
            .map(|value| {
                BenchmarkSample::new(
                    "product.database_ready_ms",
                    BenchmarkMetricKind::LifecycleLatency,
                    BenchmarkUnit::Milliseconds,
                    value,
                )
                .with_static_tag("measurement_boundary", "sqlite_proxy_not_db_sidecar")
            })
            .collect()
    }

    fn promotable_product_autoscale_samples(
        values: impl IntoIterator<Item = f64>,
    ) -> Vec<BenchmarkSample> {
        let values = values.into_iter().collect::<Vec<_>>();
        [
            "product.agent_computer_ready_ms",
            "product.agent_computer_resume_ms",
        ]
        .into_iter()
        .flat_map(|metric| {
            values.iter().copied().map(move |value| {
                BenchmarkSample::new(
                    metric,
                    BenchmarkMetricKind::LifecycleLatency,
                    BenchmarkUnit::Milliseconds,
                    value,
                )
                .with_static_tag("trust", "exact_host_event_pair")
                .with_static_tag("measurement_boundary", "product_path")
                .with_static_tag("cli_boundary", "real_cli")
                .with_static_tag("browser_boundary", "real_browser_sidecar")
                .with_static_tag("database_boundary", "real_db_sidecar")
            })
        })
        .collect()
    }

    fn promotable_product_density_samples(
        values: impl IntoIterator<Item = f64>,
    ) -> Vec<BenchmarkSample> {
        values
            .into_iter()
            .map(|value| {
                BenchmarkSample::new(
                    "density.max_agent_computers_before_ready_p95_doubles",
                    BenchmarkMetricKind::WorkloadResource,
                    BenchmarkUnit::Count,
                    value,
                )
                .with_static_tag("measurement_boundary", "product_path")
                .with_static_tag("pod_surface", "product_pod_ready_deck")
                .with_static_tag("excludes_container_add", "false")
                .with_static_tag("ready_signal", "agent_computer_ready_after_container_add")
                .with_static_tag("cli_boundary", "real_cli")
                .with_static_tag("browser_boundary", "real_browser_sidecar")
                .with_static_tag("database_boundary", "real_db_sidecar")
            })
            .collect()
    }

    fn promotable_ready_queue_samples(
        values: impl IntoIterator<Item = f64>,
    ) -> Vec<BenchmarkSample> {
        values
            .into_iter()
            .map(|value| {
                BenchmarkSample::new(
                    "autoscale.ready_queue_hit_rate_pct",
                    BenchmarkMetricKind::WorkloadResource,
                    BenchmarkUnit::Percent,
                    value,
                )
                .with_static_tag("source", "autoscale-ready-queue-outcomes")
                .with_static_tag("measurement_boundary", "signed_live_product_path")
                .with_static_tag("request_classification", "hot_or_resumed_ready_capacity")
                .with_static_tag("demand_source", "agent_computer_scorecard_harness")
                .with_static_tag("outcome_source", "observed_product_request_results")
            })
            .collect()
    }

    fn promotable_safe_spare_samples(
        values: impl IntoIterator<Item = f64>,
    ) -> Vec<BenchmarkSample> {
        values
            .into_iter()
            .map(|value| {
                BenchmarkSample::new(
                    "autoscale.safe_spare_limiting_utilization_pct",
                    BenchmarkMetricKind::WorkloadResource,
                    BenchmarkUnit::Percent,
                    value,
                )
                .with_static_tag("source", "autoscale-safe-spare-utilization")
                .with_static_tag("measurement_boundary", "signed_live_resource_accounting")
                .with_static_tag("total_resource_source", "host_capacity_probe")
                .with_static_tag(
                    "active_resource_source",
                    "runtime_active_pod_registry_budget",
                )
                .with_static_tag("reserved_floor_source", "runtime_reserve_floor_config")
                .with_static_tag("ready_queue_resource_source", "observed_ready_hit_budget")
                .with_static_tag(
                    "resource_accounting_scope",
                    "agent_computer_scorecard_harness_observation",
                )
                .with_static_tag("limiting_resource", "memory")
            })
            .collect()
    }

    fn promotable_protection_samples(
        metric: &'static str,
        values: impl IntoIterator<Item = f64>,
    ) -> Vec<BenchmarkSample> {
        values
            .into_iter()
            .map(|value| {
                BenchmarkSample::new(
                    metric,
                    BenchmarkMetricKind::WorkloadResource,
                    BenchmarkUnit::Count,
                    value,
                )
                .with_static_tag("source", "autoscale-protection-counts")
                .with_static_tag("measurement_boundary", "signed_live_product_path")
                .with_static_tag("eviction_scope", "active_session_protection")
                .with_static_tag("reserve_scope", "configured_runtime_floor")
                .with_static_tag("pressure_policy", "no_pool_comfort_eviction")
                .with_static_tag("protection_count_source", "observed_harness_completion")
            })
            .collect()
    }

    fn promotable_pressure_samples(
        metric: &'static str,
        start_event: &'static str,
        end_event: &'static str,
        values: impl IntoIterator<Item = f64>,
    ) -> Vec<BenchmarkSample> {
        values
            .into_iter()
            .map(|value| {
                let sample = BenchmarkSample::new(
                    metric,
                    BenchmarkMetricKind::LifecycleLatency,
                    BenchmarkUnit::Milliseconds,
                    value,
                )
                .with_static_tag("trust", "exact_host_event_pair")
                .with_static_tag("measurement_boundary", "signed_live_autoscale_scenario")
                .with_static_tag("start_event", start_event)
                .with_static_tag("end_event", end_event);
                match metric {
                    "autoscale.pressure_to_safe_floor_ms" => sample
                        .with_static_tag("pressure_source", "runtime_pressure_signal")
                        .with_static_tag("safe_floor_source", "runtime_reserve_floor_probe"),
                    "autoscale.pressure_clear_to_ready_target_ms" => sample
                        .with_static_tag("pressure_clear_source", "runtime_pressure_signal")
                        .with_static_tag("ready_target_source", "runtime_ready_queue_probe"),
                    _ => sample,
                }
            })
            .collect()
    }

    fn promotable_prestarted_agent_slot_density_samples(
        values: impl IntoIterator<Item = f64>,
    ) -> Vec<BenchmarkSample> {
        values
            .into_iter()
            .map(|value| {
                BenchmarkSample::new(
                    "density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles",
                    BenchmarkMetricKind::WorkloadResource,
                    BenchmarkUnit::Count,
                    value,
                )
                .with_static_tag("measurement_boundary", "prestarted_slot_checkout")
                .with_static_tag("slot_surface", "prestarted_agent_slot")
                .with_static_tag("excludes_container_add", "true")
                .with_static_tag("ready_signal", "request_fifo_acceptance")
                .with_static_tag("output_wait_preserved", "true")
            })
            .collect()
    }

    #[test]
    fn scorecard_summarizes_required_metrics_with_tail_percentiles() {
        let report = AgentBenchmarkScorecardReport::from_samples(scorecard_samples(
            (1..=100_u32).map(f64::from),
        ))
        .expect("scorecard report");

        assert_eq!(report.required_metrics(), P0_SCORECARD_METRICS.to_vec());
        let ready = report
            .summary_for("start.agent_task_ready_ms")
            .expect("agent ready summary");
        assert_eq!(ready.count(), 100);
        assert_eq!(ready.p50(), 50.0);
        assert_eq!(ready.p90(), 90.0);
        assert_eq!(ready.p95(), 95.0);
        assert_eq!(ready.p99(), 99.0);
        assert_eq!(ready.max(), 100.0);
    }

    #[test]
    fn scorecard_rejects_missing_required_metric() {
        let samples = scorecard_samples([1.0])
            .into_iter()
            .filter(|sample| sample.metric() != "exec.command_start_ms")
            .collect::<Vec<_>>();

        let error =
            AgentBenchmarkScorecardReport::from_samples(samples).expect_err("missing metric");

        assert!(matches!(
            error,
            AgentBenchmarkScorecardError::MissingRequiredMetric { metric }
            if metric == "exec.command_start_ms"
        ));
    }

    #[test]
    fn scorecard_rejects_wrong_required_metric_shape() {
        let mut samples = scorecard_samples([1.0]);
        samples.push(BenchmarkSample::new(
            "exec.command_start_ms",
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Bytes,
            1.0,
        ));

        let error = AgentBenchmarkScorecardReport::from_samples(samples).expect_err("wrong shape");

        assert!(matches!(
            error,
            AgentBenchmarkScorecardError::WrongRequiredMetricShape { metric, .. }
            if metric == "exec.command_start_ms"
        ));
    }

    #[test]
    fn scorecard_rejects_insufficient_samples() {
        let error = AgentBenchmarkScorecardReport::from_samples_with_min_samples(
            scorecard_samples([1.0]),
            2,
        )
        .expect_err("too few samples");

        assert!(matches!(
            error,
            AgentBenchmarkScorecardError::InsufficientSamples { metric, required: 2, actual: 1 }
            if metric == "start.hot_to_first_stdout_ms"
        ));
    }

    #[test]
    fn scorecard_reports_snappy_target_misses_separately_from_shape() {
        let report = AgentBenchmarkScorecardReport::from_samples(scorecard_samples([100.0]))
            .expect("scorecard report");

        let misses = report.snappy_target_misses();

        assert!(misses.iter().any(|miss| {
            miss.metric() == "start.hot_to_first_stdout_ms"
                && miss.direction() == ScorecardSnappyTargetDirection::AtMost
                && (miss.p95_threshold() - 75.0).abs() < f64::EPSILON
                && (miss.actual_p95() - 100.0).abs() < f64::EPSILON
        }));
    }

    #[test]
    fn scorecard_artifact_round_trips_json() {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact = temp.path().join("agent-scorecard.json");
        let report = AgentBenchmarkScorecardReport::from_samples(scorecard_samples([1.0, 2.0]))
            .expect("report");

        AgentBenchmarkScorecardArtifact::write_json(&artifact, &report).expect("write");
        let restored = AgentBenchmarkScorecardArtifact::read_json(&artifact).expect("read");

        assert_eq!(restored, report);
        restored.validate_min_samples(2).expect("valid restored");
    }

    #[test]
    fn autoscale_scorecard_summarizes_required_metrics() {
        let report = AutoscaleEfficiencyScorecardReport::from_samples(autoscale_scorecard_samples(
            (1..=100_u32).map(f64::from),
        ))
        .expect("autoscale scorecard report");

        assert_eq!(
            report.required_metrics(),
            AUTOSCALE_EFFICIENCY_SCORECARD_METRICS.to_vec()
        );
        let ready_hit_rate = report
            .summary_for("autoscale.ready_queue_hit_rate_pct")
            .expect("ready queue hit-rate summary");
        assert_eq!(ready_hit_rate.count(), 100);
        assert_eq!(ready_hit_rate.unit(), BenchmarkUnit::Percent);
        assert_eq!(ready_hit_rate.p95(), 95.0);
        assert_eq!(report.promotion_blockers().len(), 11);
        let blocker = report
            .promotion_blockers()
            .iter()
            .find(|blocker| blocker.metric() == "autoscale.ready_queue_hit_rate_pct")
            .expect("ready queue promotion blocker");
        assert!(blocker.blocker().contains("unit_validated_only"));
        assert!(
            blocker
                .next_action()
                .contains("signed-live autoscale harness")
        );
    }

    #[test]
    fn autoscale_scorecard_unblocks_signed_live_product_path_samples() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| {
                sample.metric() != "product.agent_computer_ready_ms"
                    && sample.metric() != "product.agent_computer_resume_ms"
            })
            .collect::<Vec<_>>();
        samples.extend(promotable_product_autoscale_samples(
            (1..=100_u32).map(f64::from),
        ));

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert_eq!(report.promotion_blockers().len(), 9);
        assert!(
            report
                .promotion_blockers()
                .iter()
                .all(
                    |blocker| blocker.metric() != "product.agent_computer_ready_ms"
                        && blocker.metric() != "product.agent_computer_resume_ms"
                )
        );
    }

    #[test]
    fn autoscale_scorecard_unblocks_signed_live_ready_queue_samples() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| sample.metric() != "autoscale.ready_queue_hit_rate_pct")
            .collect::<Vec<_>>();
        samples.extend(promotable_ready_queue_samples((1..=100_u32).map(f64::from)));

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .all(|blocker| blocker.metric() != "autoscale.ready_queue_hit_rate_pct")
        );
    }

    #[test]
    fn autoscale_scorecard_blocks_unobserved_ready_queue_samples() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| sample.metric() != "autoscale.ready_queue_hit_rate_pct")
            .collect::<Vec<_>>();
        samples.extend((1..=100_u32).map(|value| {
            BenchmarkSample::new(
                "autoscale.ready_queue_hit_rate_pct",
                BenchmarkMetricKind::WorkloadResource,
                BenchmarkUnit::Percent,
                f64::from(value),
            )
            .with_static_tag("source", "autoscale-ready-queue-outcomes")
            .with_static_tag("measurement_boundary", "signed_live_product_path")
            .with_static_tag("request_classification", "hot_or_resumed_ready_capacity")
            .with_static_tag("demand_source", "agent_computer_scorecard_harness")
        }));

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric() == "autoscale.ready_queue_hit_rate_pct")
        );
    }

    #[test]
    fn autoscale_scorecard_unblocks_signed_live_safe_spare_samples() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| sample.metric() != "autoscale.safe_spare_limiting_utilization_pct")
            .collect::<Vec<_>>();
        samples.extend(promotable_safe_spare_samples((1..=100_u32).map(f64::from)));

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .all(|blocker| blocker.metric() != "autoscale.safe_spare_limiting_utilization_pct")
        );
    }

    #[test]
    fn autoscale_scorecard_keeps_proxy_safe_spare_samples_blocked() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| sample.metric() != "autoscale.safe_spare_limiting_utilization_pct")
            .collect::<Vec<_>>();
        samples.extend(
            promotable_safe_spare_samples((1..=100_u32).map(f64::from))
                .into_iter()
                .map(|sample| {
                    sample
                        .with_static_tag("measurement_boundary", "configured_capacity_proxy")
                        .with_static_tag("total_resource_source", "single_node_default_config")
                }),
        );

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric() == "autoscale.safe_spare_limiting_utilization_pct")
        );
    }

    #[test]
    fn autoscale_scorecard_blocks_unobserved_safe_spare_samples() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| sample.metric() != "autoscale.safe_spare_limiting_utilization_pct")
            .collect::<Vec<_>>();
        samples.extend((1..=100_u32).map(|value| {
            BenchmarkSample::new(
                "autoscale.safe_spare_limiting_utilization_pct",
                BenchmarkMetricKind::WorkloadResource,
                BenchmarkUnit::Percent,
                f64::from(value),
            )
            .with_static_tag("source", "autoscale-safe-spare-utilization")
            .with_static_tag("measurement_boundary", "signed_live_resource_accounting")
            .with_static_tag("total_resource_source", "host_capacity_probe")
            .with_static_tag("active_resource_source", "runtime_active_accounting")
            .with_static_tag("reserved_floor_source", "runtime_reserve_floor_config")
            .with_static_tag(
                "ready_queue_resource_source",
                "runtime_ready_queue_accounting",
            )
            .with_static_tag("limiting_resource", "memory")
        }));

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric() == "autoscale.safe_spare_limiting_utilization_pct")
        );
    }

    #[test]
    fn autoscale_scorecard_unblocks_real_product_density_samples() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| {
                sample.metric() != "density.max_agent_computers_before_ready_p95_doubles"
            })
            .collect::<Vec<_>>();
        samples.extend(promotable_product_density_samples(
            (1..=100_u32).map(f64::from),
        ));

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .all(|blocker| blocker.metric()
                    != "density.max_agent_computers_before_ready_p95_doubles")
        );
    }

    #[test]
    fn autoscale_scorecard_keeps_proxy_product_density_blocked() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| {
                sample.metric() != "density.max_agent_computers_before_ready_p95_doubles"
            })
            .collect::<Vec<_>>();
        samples.extend(
            promotable_product_density_samples((1..=100_u32).map(f64::from))
                .into_iter()
                .map(|sample| sample.with_static_tag("browser_boundary", "proxy_health")),
        );

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric()
                    == "density.max_agent_computers_before_ready_p95_doubles")
        );
    }

    #[test]
    fn autoscale_scorecard_keeps_checkout_only_density_from_promoting_product_density() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| {
                sample.metric() != "density.max_agent_computers_before_ready_p95_doubles"
            })
            .collect::<Vec<_>>();
        samples.extend(
            promotable_product_density_samples((1..=100_u32).map(f64::from))
                .into_iter()
                .map(|sample| {
                    sample
                        .with_static_tag("excludes_container_add", "true")
                        .with_static_tag("ready_signal", "request_fifo_acceptance")
                }),
        );

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric()
                    == "density.max_agent_computers_before_ready_p95_doubles")
        );
    }

    #[test]
    fn autoscale_scorecard_unblocks_real_prestarted_agent_slot_density_samples() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| {
                sample.metric()
                    != "density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles"
            })
            .collect::<Vec<_>>();
        samples.extend(promotable_prestarted_agent_slot_density_samples(
            (1..=100_u32).map(f64::from),
        ));

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .all(|blocker| blocker.metric()
                    != "density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles")
        );
    }

    #[test]
    fn autoscale_scorecard_unblocks_signed_live_protection_samples() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| {
                sample.metric() != "autoscale.active_evictions_due_to_pool_pressure"
                    && sample.metric() != "autoscale.reserve_floor_violations"
            })
            .collect::<Vec<_>>();
        samples.extend(promotable_protection_samples(
            "autoscale.active_evictions_due_to_pool_pressure",
            (1..=100_u32).map(f64::from),
        ));
        samples.extend(promotable_protection_samples(
            "autoscale.reserve_floor_violations",
            (1..=100_u32).map(f64::from),
        ));

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .all(|blocker| blocker.metric()
                    != "autoscale.active_evictions_due_to_pool_pressure"
                    && blocker.metric() != "autoscale.reserve_floor_violations")
        );
    }

    #[test]
    fn autoscale_scorecard_blocks_unobserved_protection_samples() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| {
                sample.metric() != "autoscale.active_evictions_due_to_pool_pressure"
                    && sample.metric() != "autoscale.reserve_floor_violations"
            })
            .collect::<Vec<_>>();
        for metric in [
            "autoscale.active_evictions_due_to_pool_pressure",
            "autoscale.reserve_floor_violations",
        ] {
            samples.extend((1..=100_u32).map(|value| {
                BenchmarkSample::new(
                    metric,
                    BenchmarkMetricKind::WorkloadResource,
                    BenchmarkUnit::Count,
                    f64::from(value),
                )
                .with_static_tag("source", "autoscale-protection-counts")
                .with_static_tag("measurement_boundary", "signed_live_product_path")
                .with_static_tag("eviction_scope", "active_session_protection")
                .with_static_tag("reserve_scope", "configured_runtime_floor")
                .with_static_tag("pressure_policy", "no_pool_comfort_eviction")
            }));
        }

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report.promotion_blockers().iter().any(
                |blocker| blocker.metric() == "autoscale.active_evictions_due_to_pool_pressure"
            )
        );
        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric() == "autoscale.reserve_floor_violations")
        );
    }

    #[test]
    fn autoscale_scorecard_unblocks_signed_live_pressure_scenario_samples() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| {
                sample.metric() != "autoscale.pressure_to_safe_floor_ms"
                    && sample.metric() != "autoscale.pressure_clear_to_ready_target_ms"
            })
            .collect::<Vec<_>>();
        samples.extend(promotable_pressure_samples(
            "autoscale.pressure_to_safe_floor_ms",
            "PressureDetected",
            "SafeFloorRestored",
            (1..=100_u32).map(f64::from),
        ));
        samples.extend(promotable_pressure_samples(
            "autoscale.pressure_clear_to_ready_target_ms",
            "SafeFloorRestored",
            "ReadyTargetRestored",
            (1..=100_u32).map(f64::from),
        ));

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .all(
                    |blocker| blocker.metric() != "autoscale.pressure_to_safe_floor_ms"
                        && blocker.metric() != "autoscale.pressure_clear_to_ready_target_ms"
                )
        );
    }

    #[test]
    fn autoscale_scorecard_keeps_filler_pressure_scenario_samples_blocked() {
        let report = AutoscaleEfficiencyScorecardReport::from_samples(autoscale_scorecard_samples(
            (1..=100_u32).map(f64::from),
        ))
        .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric() == "autoscale.pressure_to_safe_floor_ms")
        );
        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric() == "autoscale.pressure_clear_to_ready_target_ms")
        );
    }

    #[test]
    fn autoscale_scorecard_keeps_proxy_database_product_path_blocked() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| {
                sample.metric() != "product.agent_computer_ready_ms"
                    && sample.metric() != "product.agent_computer_resume_ms"
            })
            .collect::<Vec<_>>();
        samples.extend(
            promotable_product_autoscale_samples((1..=100_u32).map(f64::from))
                .into_iter()
                .map(|sample| {
                    sample
                        .with_static_tag("cli_boundary", "code_interpreter_exec")
                        .with_static_tag("browser_boundary", "code_interpreter_health")
                        .with_static_tag("database_boundary", "sqlite_proxy_not_db_sidecar")
                }),
        );

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric() == "product.agent_computer_ready_ms")
        );
        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric() == "product.agent_computer_resume_ms")
        );
    }

    #[test]
    fn autoscale_scorecard_keeps_proxy_cli_browser_product_path_blocked() {
        let mut samples = autoscale_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .filter(|sample| {
                sample.metric() != "product.agent_computer_ready_ms"
                    && sample.metric() != "product.agent_computer_resume_ms"
            })
            .collect::<Vec<_>>();
        samples.extend(
            promotable_product_autoscale_samples((1..=100_u32).map(f64::from))
                .into_iter()
                .map(|sample| {
                    sample
                        .with_static_tag("cli_boundary", "code_interpreter_exec")
                        .with_static_tag("browser_boundary", "code_interpreter_health")
                }),
        );

        let report = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect("autoscale scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric() == "product.agent_computer_ready_ms")
        );
        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric() == "product.agent_computer_resume_ms")
        );
    }

    #[test]
    fn autoscale_scorecard_rejects_missing_required_metric() {
        let samples = autoscale_scorecard_samples([1.0])
            .into_iter()
            .filter(|sample| sample.metric() != "autoscale.reserve_floor_violations")
            .collect::<Vec<_>>();

        let error = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect_err("missing autoscale metric");

        assert!(matches!(
            error,
            AutoscaleEfficiencyScorecardError::MissingRequiredMetric { metric }
            if metric == "autoscale.reserve_floor_violations"
        ));
    }

    #[test]
    fn autoscale_scorecard_rejects_wrong_required_metric_shape() {
        let mut samples = autoscale_scorecard_samples([1.0]);
        samples.push(BenchmarkSample::new(
            "autoscale.safe_spare_limiting_utilization_pct",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            1.0,
        ));

        let error = AutoscaleEfficiencyScorecardReport::from_samples(samples)
            .expect_err("wrong autoscale shape");

        assert!(matches!(
            error,
            AutoscaleEfficiencyScorecardError::WrongRequiredMetricShape { metric, .. }
            if metric == "autoscale.safe_spare_limiting_utilization_pct"
        ));
    }

    #[test]
    fn autoscale_scorecard_rejects_insufficient_samples() {
        let error = AutoscaleEfficiencyScorecardReport::from_samples_with_min_samples(
            autoscale_scorecard_samples([1.0, 2.0]),
            3,
        )
        .expect_err("too few autoscale samples");

        assert!(matches!(
            error,
            AutoscaleEfficiencyScorecardError::InsufficientSamples { metric, required: 3, actual: 2 }
            if metric == "autoscale.ready_queue_hit_rate_pct"
        ));
    }

    #[test]
    fn autoscale_scorecard_reports_snappy_target_misses_separately_from_promotion() {
        let report =
            AutoscaleEfficiencyScorecardReport::from_samples(autoscale_scorecard_samples([100.0]))
                .expect("autoscale scorecard report");

        let misses = report.snappy_target_misses();

        assert!(misses.iter().any(|miss| {
            miss.metric() == "autoscale.active_evictions_due_to_pool_pressure"
                && miss.direction() == ScorecardSnappyTargetDirection::AtMost
                && miss.p95_threshold().abs() < f64::EPSILON
                && (miss.actual_p95() - 100.0).abs() < f64::EPSILON
        }));
    }

    #[test]
    fn autoscale_scorecard_artifact_round_trips_json() {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact = temp.path().join("autoscale-scorecard.json");
        let report =
            AutoscaleEfficiencyScorecardReport::from_samples(autoscale_scorecard_samples([
                1.0, 2.0,
            ]))
            .expect("report");

        AutoscaleEfficiencyScorecardArtifact::write_json(&artifact, &report).expect("write");
        let restored = AutoscaleEfficiencyScorecardArtifact::read_json(&artifact).expect("read");

        assert_eq!(restored, report);
        restored.validate_min_samples(2).expect("valid restored");
        assert_eq!(restored.promotion_blockers().len(), 11);
    }

    #[test]
    fn agent_computer_scorecard_summarizes_product_path_subset() {
        let mut samples = agent_computer_scorecard_samples((1..=100_u32).map(f64::from));
        samples.extend((1..=100_u32).map(|value| {
            BenchmarkSample::new(
                "product.cli_ready_ms",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                f64::from(value),
            )
        }));
        let report = AgentComputerScorecardReport::from_samples(samples)
            .expect("agent-computer scorecard report");

        assert_eq!(
            report.required_metrics(),
            AGENT_COMPUTER_SCORECARD_METRICS.to_vec()
        );
        let ready = report
            .summary_for("product.agent_computer_ready_ms")
            .expect("agent-computer ready summary");
        assert_eq!(ready.count(), 100);
        assert_eq!(ready.unit(), BenchmarkUnit::Milliseconds);
        assert_eq!(ready.p95(), 95.0);
        let cli = report
            .summary_for("product.cli_ready_ms")
            .expect("cli drilldown summary");
        assert_eq!(cli.count(), 100);
        assert_eq!(cli.p95(), 95.0);
        assert!(report.promotion_blockers().is_empty());
    }

    #[test]
    fn agent_computer_scorecard_reports_proxy_database_promotion_blocker() {
        let mut samples = agent_computer_scorecard_samples((1..=100_u32).map(f64::from));
        samples.extend(proxy_database_ready_samples((1..=100_u32).map(f64::from)));

        let report = AgentComputerScorecardReport::from_samples(samples)
            .expect("agent-computer scorecard report");

        assert_eq!(report.promotion_blockers().len(), 1);
        let blocker = &report.promotion_blockers()[0];
        assert_eq!(blocker.metric(), "product.database_ready_ms");
        assert!(
            blocker
                .blocker()
                .contains("SQLite through code-interpreter")
        );
        assert!(blocker.next_action().contains("real DB sidecar"));
    }

    #[test]
    fn agent_computer_scorecard_blocks_proxy_database_product_readiness() {
        let samples = agent_computer_scorecard_samples((1..=100_u32).map(f64::from))
            .into_iter()
            .map(|sample| match sample.metric() {
                "product.agent_computer_ready_ms" | "product.agent_computer_resume_ms" => sample
                    .with_static_tag("cli_boundary", "code_interpreter_exec")
                    .with_static_tag("browser_boundary", "code_interpreter_health")
                    .with_static_tag("database_boundary", "sqlite_proxy_not_db_sidecar"),
                _ => sample,
            })
            .collect::<Vec<_>>();

        let report = AgentComputerScorecardReport::from_samples(samples)
            .expect("agent-computer scorecard report");

        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric() == "product.agent_computer_ready_ms")
        );
        assert!(
            report
                .promotion_blockers()
                .iter()
                .any(|blocker| blocker.metric() == "product.agent_computer_resume_ms")
        );
    }

    #[test]
    fn agent_computer_scorecard_rejects_missing_required_metric() {
        let samples = agent_computer_scorecard_samples([1.0])
            .into_iter()
            .filter(|sample| sample.metric() != "product.agent_computer_ready_ms")
            .collect::<Vec<_>>();

        let error = AgentComputerScorecardReport::from_samples(samples)
            .expect_err("missing agent-computer metric");

        assert!(matches!(
            error,
            AgentComputerScorecardError::MissingRequiredMetric { metric }
            if metric == "product.agent_computer_ready_ms"
        ));
    }

    #[test]
    fn agent_computer_scorecard_rejects_wrong_required_metric_shape() {
        let mut samples = agent_computer_scorecard_samples([1.0]);
        samples.push(BenchmarkSample::new(
            "density.max_agent_computers_before_ready_p95_doubles",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            1.0,
        ));

        let error = AgentComputerScorecardReport::from_samples(samples)
            .expect_err("wrong agent-computer shape");

        assert!(matches!(
            error,
            AgentComputerScorecardError::WrongRequiredMetricShape { metric, .. }
            if metric == "density.max_agent_computers_before_ready_p95_doubles"
        ));
    }

    #[test]
    fn agent_computer_scorecard_rejects_insufficient_samples() {
        let error = AgentComputerScorecardReport::from_samples_with_min_samples(
            agent_computer_scorecard_samples([1.0, 2.0]),
            3,
        )
        .expect_err("too few agent-computer samples");

        assert!(matches!(
            error,
            AgentComputerScorecardError::InsufficientSamples { metric, required: 3, actual: 2 }
            if metric == "product.agent_computer_ready_ms"
        ));
    }

    #[test]
    fn agent_computer_scorecard_reports_snappy_misses_without_promotion_blockers() {
        let report =
            AgentComputerScorecardReport::from_samples(agent_computer_scorecard_samples([100.0]))
                .expect("agent-computer scorecard report");

        let misses = report.snappy_target_misses();

        assert!(report.promotion_blockers().is_empty());
        assert!(misses.iter().any(|miss| {
            miss.metric() == "product.agent_computer_resume_ms"
                && miss.direction() == ScorecardSnappyTargetDirection::AtMost
                && (miss.p95_threshold() - 75.0).abs() < f64::EPSILON
                && (miss.actual_p95() - 100.0).abs() < f64::EPSILON
        }));
    }

    #[test]
    fn agent_computer_scorecard_artifact_round_trips_json() {
        let temp = tempfile::tempdir().expect("tempdir");
        let artifact = temp.path().join("agent-computer-scorecard.json");
        let report =
            AgentComputerScorecardReport::from_samples(agent_computer_scorecard_samples([
                1.0, 2.0,
            ]))
            .expect("report");

        AgentComputerScorecardArtifact::write_json(&artifact, &report).expect("write");
        let restored = AgentComputerScorecardArtifact::read_json(&artifact).expect("read");

        assert_eq!(restored, report);
        restored.validate_min_samples(2).expect("valid restored");
        assert!(restored.promotion_blockers().is_empty());
    }
}
