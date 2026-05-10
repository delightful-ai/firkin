//! Derive decision-grade metric samples from raw sandbox event traces.

use firkin_trace::{
    BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit, LifecycleClass as TraceLifecycleClass,
    RuntimeProfile, SandboxEventName, SandboxEventTrace, SandboxEventTraceError, SandboxTraceEvent,
    WorkloadClass as TraceWorkloadClass,
};
use thiserror::Error as ThisError;

use crate::{
    LifecycleClass as ContractLifecycleClass, MetricContract, MetricEndpoint,
    RuntimeProfile as ContractRuntimeProfile, WorkloadClass as ContractWorkloadClass,
    benchmark_metric_definition,
};

/// Trust label for one derived metric sample.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DerivedMetricTrust {
    /// Value was derived from one host monotonic event pair.
    ExactHostEventPair,
}

impl DerivedMetricTrust {
    /// Return stable trust label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExactHostEventPair => "exact_host_event_pair",
        }
    }
}

/// Confidence label for one derived metric sample.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DerivedMetricConfidence {
    /// One trace sample is smoke evidence until a sample floor is met.
    SmokeOnly,
}

impl DerivedMetricConfidence {
    /// Return stable confidence label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SmokeOnly => "smoke_only",
        }
    }
}

/// One metric sample derived from a raw sandbox event trace.
#[derive(Clone, Debug, PartialEq)]
pub struct DerivedMetricSample {
    metric: &'static str,
    value: f64,
    kind: BenchmarkMetricKind,
    unit: BenchmarkUnit,
    lifecycle: TraceLifecycleClass,
    workload: TraceWorkloadClass,
    profile: RuntimeProfile,
    start_event: SandboxEventName,
    end_event: SandboxEventName,
    trust: DerivedMetricTrust,
    confidence: DerivedMetricConfidence,
}

/// Product/autoscale metric that can be derived exactly from a raw event pair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductAutoscaleDurationMetric {
    /// External request to full browser + database + CLI readiness.
    AgentComputerReady,
    /// Pressure-suspended agent computer back to full product readiness.
    AgentComputerResume,
    /// External request to first useful CLI stdout.
    AgentComputerCliReady,
    /// External request to browser/control sidecar readiness.
    AgentComputerBrowserReady,
    /// External request to database readiness.
    AgentComputerDatabaseReady,
    /// Pressure detected to reserve floors restored.
    PressureToSafeFloor,
    /// Safe floor restored to ready target restored.
    PressureClearToReadyTarget,
}

impl ProductAutoscaleDurationMetric {
    /// Return the stable metric name.
    #[must_use]
    pub const fn metric(self) -> &'static str {
        match self {
            Self::AgentComputerReady => "product.agent_computer_ready_ms",
            Self::AgentComputerResume => "product.agent_computer_resume_ms",
            Self::AgentComputerCliReady => "product.cli_ready_ms",
            Self::AgentComputerBrowserReady => "product.browser_ready_ms",
            Self::AgentComputerDatabaseReady => "product.database_ready_ms",
            Self::PressureToSafeFloor => "autoscale.pressure_to_safe_floor_ms",
            Self::PressureClearToReadyTarget => "autoscale.pressure_clear_to_ready_target_ms",
        }
    }

    const fn start_event(self) -> SandboxEventName {
        match self {
            Self::AgentComputerReady
            | Self::AgentComputerCliReady
            | Self::AgentComputerBrowserReady
            | Self::AgentComputerDatabaseReady => SandboxEventName::AgentComputerRequestStart,
            Self::AgentComputerResume => SandboxEventName::AgentComputerResumed,
            Self::PressureToSafeFloor => SandboxEventName::PressureDetected,
            Self::PressureClearToReadyTarget => SandboxEventName::SafeFloorRestored,
        }
    }

    const fn end_event(self) -> SandboxEventName {
        match self {
            Self::AgentComputerReady | Self::AgentComputerResume => {
                SandboxEventName::AgentComputerReady
            }
            Self::AgentComputerCliReady => SandboxEventName::CliFirstUsefulStdout,
            Self::AgentComputerBrowserReady => SandboxEventName::BrowserReady,
            Self::AgentComputerDatabaseReady => SandboxEventName::DatabaseReady,
            Self::PressureToSafeFloor => SandboxEventName::SafeFloorRestored,
            Self::PressureClearToReadyTarget => SandboxEventName::ReadyTargetRestored,
        }
    }

    const fn lifecycle(self) -> TraceLifecycleClass {
        match self {
            Self::AgentComputerReady
            | Self::AgentComputerCliReady
            | Self::AgentComputerBrowserReady
            | Self::AgentComputerDatabaseReady
            | Self::PressureToSafeFloor
            | Self::PressureClearToReadyTarget => TraceLifecycleClass::Hot,
            Self::AgentComputerResume => TraceLifecycleClass::Resumed,
        }
    }

    const fn workload(self) -> TraceWorkloadClass {
        match self {
            Self::AgentComputerReady
            | Self::AgentComputerResume
            | Self::AgentComputerCliReady
            | Self::AgentComputerBrowserReady
            | Self::AgentComputerDatabaseReady => TraceWorkloadClass::AgentComputer,
            Self::PressureToSafeFloor | Self::PressureClearToReadyTarget => {
                TraceWorkloadClass::AutoscaleScenario
            }
        }
    }

    const fn profile(self) -> RuntimeProfile {
        RuntimeProfile::BrowserDbCli
    }
}

/// Product/autoscale duration metrics with event-pair derivation rules.
pub const PRODUCT_AUTOSCALE_DURATION_METRICS: &[ProductAutoscaleDurationMetric] = &[
    ProductAutoscaleDurationMetric::AgentComputerReady,
    ProductAutoscaleDurationMetric::AgentComputerResume,
    ProductAutoscaleDurationMetric::AgentComputerCliReady,
    ProductAutoscaleDurationMetric::AgentComputerBrowserReady,
    ProductAutoscaleDurationMetric::AgentComputerDatabaseReady,
    ProductAutoscaleDurationMetric::PressureToSafeFloor,
    ProductAutoscaleDurationMetric::PressureClearToReadyTarget,
];

impl DerivedMetricSample {
    /// Return metric name.
    #[must_use]
    pub const fn metric(&self) -> &'static str {
        self.metric
    }

    /// Return metric value.
    #[must_use]
    pub const fn value(&self) -> f64 {
        self.value
    }

    /// Return metric kind.
    #[must_use]
    pub const fn kind(&self) -> BenchmarkMetricKind {
        self.kind
    }

    /// Return metric unit.
    #[must_use]
    pub const fn unit(&self) -> BenchmarkUnit {
        self.unit
    }

    /// Return lifecycle class.
    #[must_use]
    pub const fn lifecycle(&self) -> TraceLifecycleClass {
        self.lifecycle
    }

    /// Return workload class.
    #[must_use]
    pub const fn workload(&self) -> TraceWorkloadClass {
        self.workload
    }

    /// Return runtime profile.
    #[must_use]
    pub const fn profile(&self) -> RuntimeProfile {
        self.profile
    }

    /// Return source start event.
    #[must_use]
    pub const fn start_event(&self) -> SandboxEventName {
        self.start_event
    }

    /// Return source end event.
    #[must_use]
    pub const fn end_event(&self) -> SandboxEventName {
        self.end_event
    }

    /// Return trust label.
    #[must_use]
    pub const fn trust(&self) -> DerivedMetricTrust {
        self.trust
    }

    /// Return confidence label.
    #[must_use]
    pub const fn confidence(&self) -> DerivedMetricConfidence {
        self.confidence
    }

    /// Consume the derived metric into a benchmark sample with source tags.
    #[must_use]
    pub fn into_benchmark_sample(self) -> BenchmarkSample {
        let sample = BenchmarkSample::from_static(self.metric, self.kind, self.unit, self.value)
            .with_static_tag("trust", self.trust.as_str())
            .with_static_tag("confidence", self.confidence.as_str())
            .with_dynamic_tag("start_event", format!("{:?}", self.start_event))
            .with_dynamic_tag("end_event", format!("{:?}", self.end_event));
        with_product_probe_boundary_tags(sample, self.metric)
    }
}

fn with_product_probe_boundary_tags(sample: BenchmarkSample, metric: &str) -> BenchmarkSample {
    match metric {
        "product.agent_computer_ready_ms" | "product.agent_computer_resume_ms" => sample
            .with_static_tag("probe_surface", "browser_db_cli_readiness")
            .with_static_tag("measurement_boundary", "product_path"),
        "product.cli_ready_ms" => sample
            .with_static_tag("probe_surface", "code_interpreter_exec")
            .with_static_tag("measurement_boundary", "cli_proxy"),
        "product.browser_ready_ms" => sample
            .with_static_tag("probe_surface", "code_interpreter_health")
            .with_static_tag("measurement_boundary", "browser_proxy"),
        "product.database_ready_ms" => sample
            .with_static_tag("probe_surface", "code_interpreter_exec")
            .with_static_tag("measurement_boundary", "sqlite_proxy_not_db_sidecar"),
        _ => sample,
    }
}

/// Metric derivation error.
#[derive(Clone, Debug, PartialEq, ThisError)]
pub enum MetricDerivationError {
    /// Metric has no catalog definition.
    #[error("metric `{metric}` has no catalog definition")]
    MissingCatalogDefinition {
        /// Metric name.
        metric: &'static str,
    },
    /// Metric requires a non-duration value source not present in event pairs.
    #[error("metric `{metric}` requires a non-duration value source")]
    UnsupportedValueSource {
        /// Metric name.
        metric: &'static str,
    },
    /// Required endpoint is missing.
    #[error("metric `{metric}` is missing event {event:?}")]
    MissingEvent {
        /// Metric name.
        metric: &'static str,
        /// Missing event.
        event: SandboxEventName,
    },
    /// Required readiness prerequisite is missing.
    #[error("metric `{metric}` is missing prerequisite event {event:?}")]
    MissingPrerequisite {
        /// Metric name.
        metric: &'static str,
        /// Missing prerequisite event.
        event: SandboxEventName,
    },
    /// Event has the wrong lifecycle class.
    #[error("metric `{metric}` expected lifecycle {expected:?}, found {actual:?}")]
    WrongLifecycle {
        /// Metric name.
        metric: &'static str,
        /// Expected lifecycle.
        expected: TraceLifecycleClass,
        /// Actual lifecycle.
        actual: TraceLifecycleClass,
    },
    /// Event has the wrong workload class.
    #[error("metric `{metric}` expected workload {expected:?}, found {actual:?}")]
    WrongWorkload {
        /// Metric name.
        metric: &'static str,
        /// Expected workload.
        expected: TraceWorkloadClass,
        /// Actual workload.
        actual: TraceWorkloadClass,
    },
    /// Event has the wrong runtime profile.
    #[error("metric `{metric}` expected profile {expected:?}, found {actual:?}")]
    WrongProfile {
        /// Metric name.
        metric: &'static str,
        /// Expected profile.
        expected: RuntimeProfile,
        /// Actual profile.
        actual: RuntimeProfile,
    },
    /// Event pair was invalid.
    #[error("metric `{metric}` has invalid event pair: {source}")]
    EventPair {
        /// Metric name.
        metric: &'static str,
        /// Source trace error.
        source: SandboxEventTraceError,
    },
}

/// Derive one metric sample from one raw event trace and metric contract row.
///
/// # Errors
///
/// Returns [`MetricDerivationError`] when the metric is not duration-derived,
/// either endpoint is missing, a readiness prerequisite is missing, or the
/// trace labels do not match the contract.
pub fn derive_contract_metric_sample(
    trace: &SandboxEventTrace,
    contract: &MetricContract,
) -> Result<DerivedMetricSample, MetricDerivationError> {
    let metric = contract.metric();
    let definition = benchmark_metric_definition(metric)
        .ok_or(MetricDerivationError::MissingCatalogDefinition { metric })?;
    if definition.kind != BenchmarkMetricKind::LifecycleLatency
        || definition.unit != BenchmarkUnit::Milliseconds
    {
        return Err(MetricDerivationError::UnsupportedValueSource { metric });
    }

    let start_event = endpoint_to_event(contract.start_event());
    let end_event = endpoint_to_event(contract.end_event());
    require_readiness_prerequisites(trace, metric)?;

    let start = trace
        .headline_event(start_event)
        .ok_or(MetricDerivationError::MissingEvent {
            metric,
            event: start_event,
        })?;
    let end = trace
        .headline_event(end_event)
        .ok_or(MetricDerivationError::MissingEvent {
            metric,
            event: end_event,
        })?;

    let expected_lifecycle = lifecycle_to_trace(contract.lifecycle());
    let expected_workload = workload_to_trace(contract.workload());
    let expected_profile = profile_to_trace(contract.profile());
    validate_event_labels(
        metric,
        start,
        expected_lifecycle,
        expected_workload,
        expected_profile,
    )?;
    validate_event_labels(
        metric,
        end,
        expected_lifecycle,
        expected_workload,
        expected_profile,
    )?;

    let duration = trace
        .duration_between(start_event, end_event)
        .map_err(|source| MetricDerivationError::EventPair { metric, source })?;
    Ok(DerivedMetricSample {
        metric,
        value: duration.as_secs_f64() * 1000.0,
        kind: definition.kind,
        unit: definition.unit,
        lifecycle: expected_lifecycle,
        workload: expected_workload,
        profile: expected_profile,
        start_event,
        end_event,
        trust: DerivedMetricTrust::ExactHostEventPair,
        confidence: DerivedMetricConfidence::SmokeOnly,
    })
}

/// Derive every duration-backed contract metric available from raw traces.
///
/// Non-duration contracts and non-matching trace classes are skipped. Missing
/// required metrics are still caught by the report writer that consumes the
/// returned samples.
#[must_use]
pub fn derive_available_contract_metric_samples(
    traces: impl IntoIterator<Item = SandboxEventTrace>,
) -> Vec<BenchmarkSample> {
    let contracts = crate::decision_grade_metric_contract();
    traces
        .into_iter()
        .flat_map(|trace| {
            contracts.iter().filter_map(move |contract| {
                derive_contract_metric_sample(&trace, contract)
                    .ok()
                    .map(DerivedMetricSample::into_benchmark_sample)
            })
        })
        .collect()
}

/// Derive one product/autoscale duration metric from a raw event trace.
///
/// # Errors
///
/// Returns [`MetricDerivationError`] when the metric is missing from the
/// catalog, either endpoint is missing, required product readiness probes are
/// absent, or the event labels do not match the product/autoscale contract.
pub fn derive_product_autoscale_metric_sample(
    trace: &SandboxEventTrace,
    metric: ProductAutoscaleDurationMetric,
) -> Result<DerivedMetricSample, MetricDerivationError> {
    let metric_name = metric.metric();
    let definition = benchmark_metric_definition(metric_name).ok_or(
        MetricDerivationError::MissingCatalogDefinition {
            metric: metric_name,
        },
    )?;
    if definition.kind != BenchmarkMetricKind::LifecycleLatency
        || definition.unit != BenchmarkUnit::Milliseconds
    {
        return Err(MetricDerivationError::UnsupportedValueSource {
            metric: metric_name,
        });
    }

    require_product_autoscale_prerequisites(trace, metric)?;

    let start_event = metric.start_event();
    let end_event = metric.end_event();
    let start = trace
        .headline_event(start_event)
        .ok_or(MetricDerivationError::MissingEvent {
            metric: metric_name,
            event: start_event,
        })?;
    let end = trace
        .headline_event(end_event)
        .ok_or(MetricDerivationError::MissingEvent {
            metric: metric_name,
            event: end_event,
        })?;
    let expected_lifecycle = metric.lifecycle();
    let expected_workload = metric.workload();
    let expected_profile = metric.profile();
    validate_event_labels(
        metric_name,
        start,
        expected_lifecycle,
        expected_workload,
        expected_profile,
    )?;
    validate_event_labels(
        metric_name,
        end,
        expected_lifecycle,
        expected_workload,
        expected_profile,
    )?;

    let duration = trace
        .duration_between(start_event, end_event)
        .map_err(|source| MetricDerivationError::EventPair {
            metric: metric_name,
            source,
        })?;
    Ok(DerivedMetricSample {
        metric: metric_name,
        value: duration.as_secs_f64() * 1000.0,
        kind: definition.kind,
        unit: definition.unit,
        lifecycle: expected_lifecycle,
        workload: expected_workload,
        profile: expected_profile,
        start_event,
        end_event,
        trust: DerivedMetricTrust::ExactHostEventPair,
        confidence: DerivedMetricConfidence::SmokeOnly,
    })
}

/// Derive every available product/autoscale duration sample from raw traces.
#[must_use]
pub fn derive_available_product_autoscale_metric_samples(
    traces: impl IntoIterator<Item = SandboxEventTrace>,
) -> Vec<BenchmarkSample> {
    traces
        .into_iter()
        .flat_map(|trace| {
            PRODUCT_AUTOSCALE_DURATION_METRICS
                .iter()
                .filter_map(move |metric| {
                    derive_product_autoscale_metric_sample(&trace, *metric)
                        .ok()
                        .map(DerivedMetricSample::into_benchmark_sample)
                })
        })
        .collect()
}

fn require_readiness_prerequisites(
    trace: &SandboxEventTrace,
    metric: &'static str,
) -> Result<(), MetricDerivationError> {
    if metric != "start.hot_to_ready_ms" {
        return Ok(());
    }
    for event in [
        SandboxEventName::GuestAgentPingPassed,
        SandboxEventName::WorkspaceReady,
    ] {
        if trace.headline_event(event).is_none() {
            return Err(MetricDerivationError::MissingPrerequisite { metric, event });
        }
    }
    Ok(())
}

fn require_product_autoscale_prerequisites(
    trace: &SandboxEventTrace,
    metric: ProductAutoscaleDurationMetric,
) -> Result<(), MetricDerivationError> {
    let metric_name = metric.metric();
    let prerequisites: &[SandboxEventName] = match metric {
        ProductAutoscaleDurationMetric::AgentComputerReady
        | ProductAutoscaleDurationMetric::AgentComputerResume => &[
            SandboxEventName::CliFirstUsefulStdout,
            SandboxEventName::BrowserReady,
            SandboxEventName::DatabaseReady,
        ],
        ProductAutoscaleDurationMetric::AgentComputerCliReady
        | ProductAutoscaleDurationMetric::AgentComputerBrowserReady
        | ProductAutoscaleDurationMetric::AgentComputerDatabaseReady => &[],
        ProductAutoscaleDurationMetric::PressureToSafeFloor => &[
            SandboxEventName::AutoscaleDecisionMade,
            SandboxEventName::AutoscaleActionStarted,
        ],
        ProductAutoscaleDurationMetric::PressureClearToReadyTarget => &[],
    };
    for event in prerequisites {
        if trace.headline_event(*event).is_none() {
            return Err(MetricDerivationError::MissingPrerequisite {
                metric: metric_name,
                event: *event,
            });
        }
    }
    Ok(())
}

fn validate_event_labels(
    metric: &'static str,
    event: &SandboxTraceEvent,
    expected_lifecycle: TraceLifecycleClass,
    expected_workload: TraceWorkloadClass,
    expected_profile: RuntimeProfile,
) -> Result<(), MetricDerivationError> {
    if event.lifecycle() != expected_lifecycle {
        return Err(MetricDerivationError::WrongLifecycle {
            metric,
            expected: expected_lifecycle,
            actual: event.lifecycle(),
        });
    }
    if event.workload() != expected_workload {
        return Err(MetricDerivationError::WrongWorkload {
            metric,
            expected: expected_workload,
            actual: event.workload(),
        });
    }
    if event.profile() != expected_profile {
        return Err(MetricDerivationError::WrongProfile {
            metric,
            expected: expected_profile,
            actual: event.profile(),
        });
    }
    Ok(())
}

const fn endpoint_to_event(endpoint: MetricEndpoint) -> SandboxEventName {
    match endpoint {
        MetricEndpoint::RequestStart => SandboxEventName::RequestStart,
        MetricEndpoint::PoolLeaseRequested => SandboxEventName::PoolLeaseRequested,
        MetricEndpoint::PoolLeaseAcquired => SandboxEventName::PoolLeaseAcquired,
        MetricEndpoint::SnapshotRestoreStart => SandboxEventName::SnapshotRestoreStart,
        MetricEndpoint::VzStartCalled => SandboxEventName::VzStartCalled,
        MetricEndpoint::ReadyProbePassed => SandboxEventName::ReadyProbePassed,
        MetricEndpoint::ExecRequestSent => SandboxEventName::ExecRequestSent,
        MetricEndpoint::ProcessStarted => SandboxEventName::ProcessStarted,
        MetricEndpoint::FirstStdoutByte => SandboxEventName::FirstStdoutByte,
        MetricEndpoint::ProcessExited => SandboxEventName::ProcessExited,
        MetricEndpoint::CleanupStart => SandboxEventName::CleanupStart,
        MetricEndpoint::FstrimStart => SandboxEventName::FstrimStart,
        MetricEndpoint::FstrimDone => SandboxEventName::FstrimDone,
        MetricEndpoint::CleanupDone => SandboxEventName::CleanupDone,
    }
}

const fn lifecycle_to_trace(lifecycle: ContractLifecycleClass) -> TraceLifecycleClass {
    match lifecycle {
        ContractLifecycleClass::ColdUnprepared => TraceLifecycleClass::ColdUnprepared,
        ContractLifecycleClass::ColdPrepared => TraceLifecycleClass::ColdPrepared,
        ContractLifecycleClass::Warm => TraceLifecycleClass::Warm,
        ContractLifecycleClass::Hot => TraceLifecycleClass::Hot,
        ContractLifecycleClass::Resumed => TraceLifecycleClass::Resumed,
    }
}

const fn workload_to_trace(workload: ContractWorkloadClass) -> TraceWorkloadClass {
    match workload {
        ContractWorkloadClass::TinyExec => TraceWorkloadClass::TinyExec,
        ContractWorkloadClass::ShellExec => TraceWorkloadClass::ShellExec,
        ContractWorkloadClass::Batch100Execs => TraceWorkloadClass::Batch100Execs,
        ContractWorkloadClass::DiskBloatReclaim => TraceWorkloadClass::DiskBloatReclaim,
        ContractWorkloadClass::ConcurrentCreate => TraceWorkloadClass::ConcurrentCreate,
        ContractWorkloadClass::ReadinessProbe => TraceWorkloadClass::ReadinessProbe,
    }
}

const fn profile_to_trace(profile: ContractRuntimeProfile) -> RuntimeProfile {
    match profile {
        ContractRuntimeProfile::FastAgent => RuntimeProfile::FastAgent,
        ContractRuntimeProfile::DiskReclaim => RuntimeProfile::DiskReclaim,
        ContractRuntimeProfile::Density => RuntimeProfile::Density,
    }
}
