//! Decision-grade benchmark metric contract.

/// Decision-grade metric tier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionMetricLevel {
    /// Focused dashboard metric that can drive optimization once confidence gates pass.
    FocusedDashboard,
}

impl DecisionMetricLevel {
    /// Return the stable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FocusedDashboard => "focused_dashboard",
        }
    }
}

/// Canonical event endpoint used to derive a metric from a raw trace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricEndpoint {
    /// External API request accepted.
    RequestStart,
    /// Pool lease request starts.
    PoolLeaseRequested,
    /// Pool lease has been acquired.
    PoolLeaseAcquired,
    /// Snapshot restore starts.
    SnapshotRestoreStart,
    /// Virtualization start has been called.
    VzStartCalled,
    /// Readiness probe has passed.
    ReadyProbePassed,
    /// Exec request has been sent.
    ExecRequestSent,
    /// Guest process has started.
    ProcessStarted,
    /// First stdout byte has been observed by the host.
    FirstStdoutByte,
    /// Process has exited.
    ProcessExited,
    /// Cleanup starts.
    CleanupStart,
    /// Fstrim starts.
    FstrimStart,
    /// Fstrim completes.
    FstrimDone,
    /// Cleanup has completed.
    CleanupDone,
}

impl MetricEndpoint {
    /// Return the stable event name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RequestStart => "RequestStart",
            Self::PoolLeaseRequested => "PoolLeaseRequested",
            Self::PoolLeaseAcquired => "PoolLeaseAcquired",
            Self::SnapshotRestoreStart => "SnapshotRestoreStart",
            Self::VzStartCalled => "VzStartCalled",
            Self::ReadyProbePassed => "ReadyProbePassed",
            Self::ExecRequestSent => "ExecRequestSent",
            Self::ProcessStarted => "ProcessStarted",
            Self::FirstStdoutByte => "FirstStdoutByte",
            Self::ProcessExited => "ProcessExited",
            Self::CleanupStart => "CleanupStart",
            Self::FstrimStart => "FstrimStart",
            Self::FstrimDone => "FstrimDone",
            Self::CleanupDone => "CleanupDone",
        }
    }
}

/// Lifecycle class for a benchmark metric.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleClass {
    /// Image/template/rootfs are not ready.
    ColdUnprepared,
    /// Template/rootfs are ready but no VM/pool is ready.
    ColdPrepared,
    /// Warm path that is not already leased as a ready sandbox.
    Warm,
    /// Clean sandbox already pooled.
    Hot,
    /// Snapshot or paused state restored.
    Resumed,
}

impl LifecycleClass {
    /// Return the stable lifecycle label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ColdUnprepared => "cold_unprepared",
            Self::ColdPrepared => "cold_prepared",
            Self::Warm => "warm",
            Self::Hot => "hot",
            Self::Resumed => "resumed",
        }
    }
}

/// Workload class for a benchmark metric.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkloadClass {
    /// Tiny command that writes a known stdout payload.
    TinyExec,
    /// Direct argv tiny command without shell startup.
    DirectExec,
    /// Shell command startup workload.
    ShellExec,
    /// One hundred tiny commands in one ready sandbox.
    Batch100Execs,
    /// Disk bloat and reclaim workload.
    DiskBloatReclaim,
    /// Concurrent sandbox create workload.
    ConcurrentCreate,
    /// Readiness probe workload.
    ReadinessProbe,
}

impl WorkloadClass {
    /// Return the stable workload label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TinyExec => "tiny_exec",
            Self::DirectExec => "direct_exec",
            Self::ShellExec => "shell_exec",
            Self::Batch100Execs => "batch_100_execs",
            Self::DiskBloatReclaim => "disk_bloat_reclaim",
            Self::ConcurrentCreate => "concurrent_create",
            Self::ReadinessProbe => "readiness_probe",
        }
    }
}

/// Runtime profile label for a benchmark metric.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeProfile {
    /// Fast local agent profile.
    FastAgent,
    /// Disk reclaim profile.
    DiskReclaim,
    /// Density sweep profile.
    Density,
}

impl RuntimeProfile {
    /// Return the stable profile label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FastAgent => "fast_agent",
            Self::DiskReclaim => "disk_reclaim",
            Self::Density => "density",
        }
    }
}

/// Percentile confidence policy for one metric.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PercentilePolicy {
    p95_min_samples: usize,
    p99_min_samples: usize,
}

impl PercentilePolicy {
    /// Construct a percentile policy.
    #[must_use]
    pub const fn new(p95_min_samples: usize, p99_min_samples: usize) -> Self {
        Self {
            p95_min_samples,
            p99_min_samples,
        }
    }

    /// Return minimum samples required for decision-grade p95.
    #[must_use]
    pub const fn p95_min_samples(self) -> usize {
        self.p95_min_samples
    }

    /// Return minimum samples required for decision-grade p99.
    #[must_use]
    pub const fn p99_min_samples(self) -> usize {
        self.p99_min_samples
    }
}

/// One decision-grade metric contract row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricContract {
    metric: &'static str,
    start_event: MetricEndpoint,
    end_event: MetricEndpoint,
    lifecycle: LifecycleClass,
    workload: WorkloadClass,
    profile: RuntimeProfile,
    included_phases: &'static str,
    excluded_phases: &'static str,
    owner: &'static str,
    percentile_policy: PercentilePolicy,
    level: DecisionMetricLevel,
}

impl MetricContract {
    /// Construct a metric contract row.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        metric: &'static str,
        start_event: MetricEndpoint,
        end_event: MetricEndpoint,
        lifecycle: LifecycleClass,
        workload: WorkloadClass,
        profile: RuntimeProfile,
        included_phases: &'static str,
        excluded_phases: &'static str,
        owner: &'static str,
        percentile_policy: PercentilePolicy,
        level: DecisionMetricLevel,
    ) -> Self {
        Self {
            metric,
            start_event,
            end_event,
            lifecycle,
            workload,
            profile,
            included_phases,
            excluded_phases,
            owner,
            percentile_policy,
            level,
        }
    }

    /// Return the metric name.
    #[must_use]
    pub const fn metric(self) -> &'static str {
        self.metric
    }

    /// Return the start event.
    #[must_use]
    pub const fn start_event(self) -> MetricEndpoint {
        self.start_event
    }

    /// Return the end event.
    #[must_use]
    pub const fn end_event(self) -> MetricEndpoint {
        self.end_event
    }

    /// Return the lifecycle class.
    #[must_use]
    pub const fn lifecycle(self) -> LifecycleClass {
        self.lifecycle
    }

    /// Return the workload class.
    #[must_use]
    pub const fn workload(self) -> WorkloadClass {
        self.workload
    }

    /// Return the runtime profile.
    #[must_use]
    pub const fn profile(self) -> RuntimeProfile {
        self.profile
    }

    /// Return included phase description.
    #[must_use]
    pub const fn included_phases(self) -> &'static str {
        self.included_phases
    }

    /// Return excluded phase description.
    #[must_use]
    pub const fn excluded_phases(self) -> &'static str {
        self.excluded_phases
    }

    /// Return owner crate or module.
    #[must_use]
    pub const fn owner(self) -> &'static str {
        self.owner
    }

    /// Return percentile policy.
    #[must_use]
    pub const fn percentile_policy(self) -> PercentilePolicy {
        self.percentile_policy
    }

    /// Return decision metric tier.
    #[must_use]
    pub const fn level(self) -> DecisionMetricLevel {
        self.level
    }
}

const FAST_PATH_POLICY: PercentilePolicy = PercentilePolicy::new(100, 500);
const SLOW_PATH_POLICY: PercentilePolicy = PercentilePolicy::new(30, 500);
const FOCUSED: DecisionMetricLevel = DecisionMetricLevel::FocusedDashboard;

/// Focused decision-grade benchmark metrics.
#[rustfmt::skip]
pub const DECISION_GRADE_METRICS: &[MetricContract] = &[
    MetricContract::new("start.hot_to_first_stdout_ms", MetricEndpoint::PoolLeaseAcquired, MetricEndpoint::FirstStdoutByte, LifecycleClass::Hot, WorkloadClass::TinyExec, RuntimeProfile::FastAgent, "readiness check, exec request, process start, stdout wait", "pool lease acquisition, template lookup, snapshot restore, cleanup", "firkin-runtime/firkin-vminitd-client", FAST_PATH_POLICY, FOCUSED),
    MetricContract::new("start.hot_to_ready_ms", MetricEndpoint::PoolLeaseAcquired, MetricEndpoint::ReadyProbePassed, LifecycleClass::Hot, WorkloadClass::ReadinessProbe, RuntimeProfile::FastAgent, "guest ping, workspace probe, exec probe, optional DNS probe", "pool lease acquisition, first user command stdout, cleanup", "firkin-runtime", FAST_PATH_POLICY, FOCUSED),
    MetricContract::new("start.warm_to_first_stdout_ms", MetricEndpoint::RequestStart, MetricEndpoint::FirstStdoutByte, LifecycleClass::Warm, WorkloadClass::TinyExec, RuntimeProfile::FastAgent, "warm start, readiness, exec request, stdout wait", "cold image/template preparation, hot pool lease-only path, cleanup", "firkin-runtime/firkin-single-node", FAST_PATH_POLICY, FOCUSED),
    MetricContract::new("start.agent_task_ready_ms", MetricEndpoint::RequestStart, MetricEndpoint::FirstStdoutByte, LifecycleClass::Hot, WorkloadClass::TinyExec, RuntimeProfile::FastAgent, "external API request, sandbox create, readiness, first useful stdout", "post-first-stdout task wall time and cleanup", "firkin-benchmark/firkin-runtime", FAST_PATH_POLICY, FOCUSED),
    MetricContract::new("pool.lease_ms", MetricEndpoint::PoolLeaseRequested, MetricEndpoint::PoolLeaseAcquired, LifecycleClass::Hot, WorkloadClass::TinyExec, RuntimeProfile::FastAgent, "pool lookup and lease acquisition", "readiness, workspace setup, exec, stdout, cleanup", "firkin-admission/firkin-runtime", FAST_PATH_POLICY, FOCUSED),
    MetricContract::new("exec.direct_command_start_ms", MetricEndpoint::ExecRequestSent, MetricEndpoint::ProcessStarted, LifecycleClass::Hot, WorkloadClass::DirectExec, RuntimeProfile::FastAgent, "direct argv exec RPC dispatch through guest process start", "shell startup, pool lease, readiness, stdout wait, process exit, cleanup", "firkin-vminitd-client/firkin-runtime", FAST_PATH_POLICY, FOCUSED),
    MetricContract::new("exec.direct_first_stdout_byte_ms", MetricEndpoint::ExecRequestSent, MetricEndpoint::FirstStdoutByte, LifecycleClass::Hot, WorkloadClass::DirectExec, RuntimeProfile::FastAgent, "direct argv exec RPC dispatch, process start, stdout wait", "shell startup, pool lease, readiness, process exit, cleanup", "firkin-vminitd-client/firkin-runtime", FAST_PATH_POLICY, FOCUSED),
    MetricContract::new("exec.batch_100_small_commands_ms", MetricEndpoint::ExecRequestSent, MetricEndpoint::ProcessExited, LifecycleClass::Hot, WorkloadClass::Batch100Execs, RuntimeProfile::FastAgent, "one retained shell execution processing 100 tiny command payloads through final process exit", "sandbox startup, pool lease, cleanup, independent process startup per command", "firkin-benchmark/firkin-vminitd-client", FAST_PATH_POLICY, FOCUSED),
    MetricContract::new("density.max_active_before_retained_shell_first_stdout_p95_doubles", MetricEndpoint::ExecRequestSent, MetricEndpoint::FirstStdoutByte, LifecycleClass::Hot, WorkloadClass::ShellExec, RuntimeProfile::Density, "concurrency sweep of retained-shell dispatch first-stdout p95", "sandbox create/start latency, cold/warm/resumed lifecycles, independent process startup per command", "firkin-benchmark/firkin-runtime", SLOW_PATH_POLICY, FOCUSED),
    MetricContract::new("disk.sparse_bloat_after_trim", MetricEndpoint::FstrimDone, MetricEndpoint::FstrimDone, LifecycleClass::Hot, WorkloadClass::DiskBloatReclaim, RuntimeProfile::DiskReclaim, "host allocated bytes and guest used bytes after fstrim", "pre-task and pre-trim bloat states", "firkin-benchmark/firkin-single-node", SLOW_PATH_POLICY, FOCUSED),
    MetricContract::new("disk.host_bytes_reclaimed_after_trim", MetricEndpoint::FstrimStart, MetricEndpoint::FstrimDone, LifecycleClass::Hot, WorkloadClass::DiskBloatReclaim, RuntimeProfile::DiskReclaim, "host allocated-byte delta across fstrim", "guest-reported trim bytes without host allocation delta", "firkin-benchmark/firkin-single-node", SLOW_PATH_POLICY, FOCUSED),
    MetricContract::new("cleanup.leftover_bytes", MetricEndpoint::CleanupStart, MetricEndpoint::CleanupDone, LifecycleClass::Hot, WorkloadClass::TinyExec, RuntimeProfile::FastAgent, "run-scoped Firkin-owned leftover bytes after destroy", "global cache/state roots and unrelated templates", "firkin-hygiene/firkin-runtime", SLOW_PATH_POLICY, FOCUSED),
    MetricContract::new("reliability.unknown_failure_rate", MetricEndpoint::RequestStart, MetricEndpoint::CleanupDone, LifecycleClass::Hot, WorkloadClass::TinyExec, RuntimeProfile::FastAgent, "classified create, readiness, exec, and cleanup attempts", "known boot, agent, DNS, workspace, and OOM failures", "firkin-benchmark/firkin-runtime", SLOW_PATH_POLICY, FOCUSED),
];

/// Return the focused decision-grade metric contract.
#[must_use]
pub const fn decision_grade_metric_contract() -> &'static [MetricContract] {
    DECISION_GRADE_METRICS
}
