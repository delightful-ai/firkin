//! First-class benchmark metric catalog.
#![allow(missing_docs)]

use firkin_trace::{BenchmarkMetricKind, BenchmarkUnit};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum BenchmarkMetricGroup {
    Product,
    Startup,
    Exec,
    AgentTask,
    Autoscale,
    Demand,
    Capacity,
    Disk,
    Memory,
    Cpu,
    Pressure,
    Network,
    Pids,
    Pod,
    Cache,
    Isolation,
    Cleanup,
    Reliability,
    Density,
    Power,
}

impl BenchmarkMetricGroup {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Startup => "startup",
            Self::Exec => "exec",
            Self::AgentTask => "agent_task",
            Self::Autoscale => "autoscale",
            Self::Demand => "demand",
            Self::Capacity => "capacity",
            Self::Disk => "disk",
            Self::Memory => "memory",
            Self::Cpu => "cpu",
            Self::Pressure => "pressure",
            Self::Network => "network",
            Self::Pids => "pids",
            Self::Pod => "pod",
            Self::Cache => "cache",
            Self::Isolation => "isolation",
            Self::Cleanup => "cleanup",
            Self::Reliability => "reliability",
            Self::Density => "density",
            Self::Power => "power",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum BenchmarkRequirementLevel {
    P0Dashboard,
    AutoscaleDashboard,
    Core,
    Drilldown,
    FutureHarness,
}

impl BenchmarkRequirementLevel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::P0Dashboard => "p0_dashboard",
            Self::AutoscaleDashboard => "autoscale_dashboard",
            Self::Core => "core",
            Self::Drilldown => "drilldown",
            Self::FutureHarness => "future_harness",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum BenchmarkMeasurementStatus {
    SignedLiveExact,
    SignedLiveProxy,
    UnitValidatedOnly,
    NeedsLiveHarness,
}

impl BenchmarkMeasurementStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SignedLiveExact => "signed_live_exact",
            Self::SignedLiveProxy => "signed_live_proxy",
            Self::UnitValidatedOnly => "unit_validated_only",
            Self::NeedsLiveHarness => "needs_live_harness",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct BenchmarkMetricDefinition {
    pub name: &'static str,
    pub group: BenchmarkMetricGroup,
    pub kind: BenchmarkMetricKind,
    pub unit: BenchmarkUnit,
    pub requirement: BenchmarkRequirementLevel,
    pub notes: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct BenchmarkMeasurementCoverage {
    pub metric: &'static str,
    pub status: BenchmarkMeasurementStatus,
    pub source: &'static str,
    pub notes: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum BenchmarkMetricMatch {
    Exact(&'static str),
    Prefix(&'static str),
    Fallback,
}

impl BenchmarkMetricMatch {
    #[must_use]
    pub fn matches(self, metric: &str) -> bool {
        match self {
            Self::Exact(expected) => metric == expected,
            Self::Prefix(prefix) => metric.starts_with(prefix),
            Self::Fallback => true,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact(value) | Self::Prefix(value) => value,
            Self::Fallback => "*",
        }
    }

    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Exact(_) => "exact",
            Self::Prefix(_) => "prefix",
            Self::Fallback => "fallback",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct BenchmarkMetricOwnership {
    pub metric_match: BenchmarkMetricMatch,
    pub phase_label: &'static str,
    pub owner: &'static str,
    pub next_action_hint: &'static str,
}

const fn metric(
    name: &'static str,
    group: BenchmarkMetricGroup,
    kind: BenchmarkMetricKind,
    unit: BenchmarkUnit,
    requirement: BenchmarkRequirementLevel,
    notes: &'static str,
) -> BenchmarkMetricDefinition {
    BenchmarkMetricDefinition {
        name,
        group,
        kind,
        unit,
        requirement,
        notes,
    }
}

const fn coverage(
    metric: &'static str,
    status: BenchmarkMeasurementStatus,
    source: &'static str,
    notes: &'static str,
) -> BenchmarkMeasurementCoverage {
    BenchmarkMeasurementCoverage {
        metric,
        status,
        source,
        notes,
    }
}

const fn ownership(
    metric_match: BenchmarkMetricMatch,
    phase_label: &'static str,
    owner: &'static str,
    next_action_hint: &'static str,
) -> BenchmarkMetricOwnership {
    BenchmarkMetricOwnership {
        metric_match,
        phase_label,
        owner,
        next_action_hint,
    }
}

pub const P0_SCORECARD_METRICS: &[&str] = &[
    "start.hot_to_first_stdout_ms",
    "start.hot_to_ready_ms",
    "start.resume_to_first_stdout_ms",
    "start.warm_to_first_stdout_ms",
    "start.agent_task_ready_ms",
    "pool.lease_ms",
    "exec.command_start_ms",
    "exec.first_stdout_byte_ms",
    "exec.batch_100_small_commands_ms",
    "density.max_active_before_hot_to_first_stdout_p95_doubles",
    "disk.sparse_bloat_after_trim",
    "disk.host_bytes_reclaimed_after_trim",
    "cleanup.leftover_bytes",
    "reliability.unknown_failure_rate",
];

pub const AUTOSCALE_EFFICIENCY_SCORECARD_METRICS: &[&str] = &[
    "autoscale.ready_queue_hit_rate_pct",
    "product.agent_computer_ready_ms",
    "product.agent_computer_resume_ms",
    "autoscale.safe_spare_limiting_utilization_pct",
    "autoscale.pressure_to_safe_floor_ms",
    "autoscale.pressure_clear_to_ready_target_ms",
    "density.max_agent_computers_before_ready_p95_doubles",
    "density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles",
    "density.prestarted_agent_slot_fifo_acceptance_p95_ms",
    "autoscale.active_evictions_due_to_pool_pressure",
    "autoscale.reserve_floor_violations",
    "cleanup.leftover_bytes",
    "reliability.unknown_failure_rate",
];

pub const AGENT_COMPUTER_SCORECARD_METRICS: &[&str] = &[
    "product.agent_computer_ready_ms",
    "product.agent_computer_resume_ms",
    "density.max_agent_computers_before_ready_p95_doubles",
    "cleanup.leftover_bytes",
    "reliability.unknown_failure_rate",
];

pub const AGENT_COMPUTER_DRILLDOWN_METRICS: &[&str] = &[
    "product.cli_ready_ms",
    "product.browser_ready_ms",
    "product.database_ready_ms",
];

pub const P0_SCORECARD_MEASUREMENT_COVERAGE: &[BenchmarkMeasurementCoverage] = &[
    coverage(
        "start.hot_to_first_stdout_ms",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:start.hot_to_first_stdout_ms",
        "derived from PoolLeaseAcquired through FirstStdoutByte on the hot tiny-exec path",
    ),
    coverage(
        "start.hot_to_ready_ms",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:start.hot_to_ready_ms",
        "derived from PoolLeaseAcquired through ReadyProbePassed on the hot readiness path",
    ),
    coverage(
        "start.resume_to_first_stdout_ms",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:start.resume_to_first_stdout_ms",
        "derived from SnapshotRestoreStart through FirstStdoutByte on the resumed tiny-exec path",
    ),
    coverage(
        "start.warm_to_first_stdout_ms",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:start.warm_to_first_stdout_ms",
        "derived from RequestStart through FirstStdoutByte on the warm tiny-exec path",
    ),
    coverage(
        "start.agent_task_ready_ms",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:start.agent_task_ready_ms",
        "derived from external RequestStart through FirstStdoutByte for first useful stdout",
    ),
    coverage(
        "pool.lease_ms",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:pool.lease_ms",
        "derived from PoolLeaseRequested through PoolLeaseAcquired and excludes readiness, workspace, exec, stdout, and cleanup",
    ),
    coverage(
        "exec.command_start_ms",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:exec.command_start_ms",
        "derived from ExecRequestSent through ProcessStarted",
    ),
    coverage(
        "exec.first_stdout_byte_ms",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:exec.first_stdout_byte_ms",
        "derived from ExecRequestSent through FirstStdoutByte",
    ),
    coverage(
        "exec.batch_100_small_commands_ms",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:exec.batch_100_small_commands_ms",
        "derived from the first batch ExecRequestSent through the final ProcessExited",
    ),
    coverage(
        "density.max_active_before_hot_to_first_stdout_p95_doubles",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:density.max_active_before_hot_to_first_stdout_p95_doubles",
        "derived from hot-to-first-stdout p95 points in the concurrent-create workload only",
    ),
    coverage(
        "disk.sparse_bloat_after_trim",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:disk.sparse_bloat_after_trim",
        "derived from paired host allocated bytes and guest used bytes after fstrim",
    ),
    coverage(
        "disk.host_bytes_reclaimed_after_trim",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:disk.host_bytes_reclaimed_after_trim",
        "derived from host allocated-byte delta across FstrimStart and FstrimDone",
    ),
    coverage(
        "cleanup.leftover_bytes",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:cleanup.leftover_bytes",
        "derived from run-scoped Firkin-owned cleanup scan at CleanupDone",
    ),
    coverage(
        "reliability.unknown_failure_rate",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:reliability.unknown_failure_rate",
        "derived from classified attempts after known boot, agent, DNS, workspace, and OOM failures are excluded",
    ),
];

pub const AUTOSCALE_EFFICIENCY_MEASUREMENT_COVERAGE: &[BenchmarkMeasurementCoverage] = &[
    coverage(
        "autoscale.ready_queue_hit_rate_pct",
        BenchmarkMeasurementStatus::UnitValidatedOnly,
        "autoscale_trace:ready_queue_hit_rate_pct",
        "unit-validated ready-hit/miss calculation; still requires signed-live request outcome classification between ready hot/resumed capacity and warm/cold creation",
    ),
    coverage(
        "product.agent_computer_ready_ms",
        BenchmarkMeasurementStatus::UnitValidatedOnly,
        "autoscale_trace:product.agent_computer_ready_ms",
        "unit-validated event-pair derivation; still requires signed-live browser + database + CLI product readiness harness",
    ),
    coverage(
        "product.agent_computer_resume_ms",
        BenchmarkMeasurementStatus::UnitValidatedOnly,
        "autoscale_trace:product.agent_computer_resume_ms",
        "unit-validated event-pair derivation; still requires signed-live pressure-suspended agent-computer resume harness",
    ),
    coverage(
        "autoscale.safe_spare_limiting_utilization_pct",
        BenchmarkMeasurementStatus::UnitValidatedOnly,
        "autoscale_trace:safe_spare_limiting_utilization_pct",
        "unit-validated limiting-resource derivation; still requires signed-live per-resource safe spare accounting",
    ),
    coverage(
        "autoscale.pressure_to_safe_floor_ms",
        BenchmarkMeasurementStatus::UnitValidatedOnly,
        "autoscale_trace:pressure_to_safe_floor_ms",
        "unit-validated event-pair derivation; still requires signed-live pressure-state and reserve-floor harness",
    ),
    coverage(
        "autoscale.pressure_clear_to_ready_target_ms",
        BenchmarkMeasurementStatus::UnitValidatedOnly,
        "autoscale_trace:pressure_clear_to_ready_target_ms",
        "unit-validated event-pair derivation; still requires signed-live pressure-clear and ready-target harness",
    ),
    coverage(
        "density.max_agent_computers_before_ready_p95_doubles",
        BenchmarkMeasurementStatus::UnitValidatedOnly,
        "autoscale_trace:density.max_agent_computers_before_ready_p95_doubles",
        "unit-validated p95-doubling breakpoint; still requires browser + database + CLI density sweep, not shell-only density",
    ),
    coverage(
        "density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles",
        BenchmarkMeasurementStatus::UnitValidatedOnly,
        "autoscale_trace:density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles",
        "unit-validated p95-doubling breakpoint for already-running slots; signed-live smoke exists but still needs decision-grade repeats and non-serialized completion observation",
    ),
    coverage(
        "density.prestarted_agent_slot_fifo_acceptance_p95_ms",
        BenchmarkMeasurementStatus::UnitValidatedOnly,
        "autoscale_trace:density.prestarted_agent_slot_fifo_acceptance_p95_ms",
        "unit-validated FIFO acceptance snappy guard for prestarted slots; still requires signed-live request ordering under real product load",
    ),
    coverage(
        "autoscale.active_evictions_due_to_pool_pressure",
        BenchmarkMeasurementStatus::UnitValidatedOnly,
        "autoscale_trace:active_evictions_due_to_pool_pressure",
        "unit-validated protection-count sample; still requires signed-live eviction cause classification that separates active protection from pool comfort",
    ),
    coverage(
        "autoscale.reserve_floor_violations",
        BenchmarkMeasurementStatus::UnitValidatedOnly,
        "autoscale_trace:reserve_floor_violations",
        "unit-validated protection-count sample; still requires signed-live controller decisions to record configured reserves and post-decision resource state",
    ),
    coverage(
        "cleanup.leftover_bytes",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:cleanup.leftover_bytes",
        "shared guardrail from the P0 scorecard; must remain exact for autoscale runs",
    ),
    coverage(
        "reliability.unknown_failure_rate",
        BenchmarkMeasurementStatus::SignedLiveExact,
        "event_trace:reliability.unknown_failure_rate",
        "shared guardrail from the P0 scorecard; must remain exact for autoscale runs",
    ),
];

pub const BENCHMARK_METRIC_OWNERSHIP: &[BenchmarkMetricOwnership] = &[
    ownership(
        BenchmarkMetricMatch::Exact("product.agent_computer_ready_ms"),
        "product_agent_computer_ready",
        "firkin-benchmark/firkin-runtime",
        "inspect external request through browser, database, and CLI readiness probes",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("product.agent_computer_resume_ms"),
        "product_agent_computer_resume",
        "firkin-runtime/firkin-single-node",
        "inspect pressure-suspended resume through browser, database, and CLI readiness probes",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("autoscale.ready_queue_hit_rate_pct"),
        "autoscale_ready_queue",
        "firkin-admission/firkin-runtime",
        "inspect demand classification and whether requests were served from hot or resumed ready capacity",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("autoscale.safe_spare_limiting_utilization_pct"),
        "autoscale_safe_spare",
        "firkin-admission/firkin-benchmark",
        "inspect per-resource safe spare accounting and limiting-resource selection",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("autoscale.pressure_to_safe_floor_ms"),
        "autoscale_pressure_shrink",
        "firkin-admission/firkin-hygiene",
        "inspect pressure detection, shrink decisions, reclaim, and reserve-floor satisfaction",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("autoscale.pressure_clear_to_ready_target_ms"),
        "autoscale_pressure_refill",
        "firkin-admission/firkin-runtime",
        "inspect pressure-clear detection, refill decisions, and restored ready target",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("density.max_agent_computers_before_ready_p95_doubles"),
        "density_agent_computer_ready",
        "firkin-benchmark/firkin-runtime",
        "inspect browser + database + CLI density sweep for product-readiness p95 breakpoint",
    ),
    ownership(
        BenchmarkMetricMatch::Exact(
            "density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles",
        ),
        "density_prestarted_agent_slot_ready",
        "firkin-benchmark/firkin-runtime/firkin-single-node",
        "inspect already-running agent-slot checkout density and completion observation",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("density.prestarted_agent_slot_fifo_acceptance_p95_ms"),
        "density_prestarted_agent_slot_fifo",
        "firkin-benchmark/firkin-runtime/firkin-single-node",
        "inspect FIFO request acceptance for already-running prestarted agent slots",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("autoscale.active_evictions_due_to_pool_pressure"),
        "autoscale_active_protection",
        "firkin-admission/firkin-runtime",
        "inspect eviction causes and reject active-session eviction for pool comfort",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("autoscale.reserve_floor_violations"),
        "autoscale_reserve_floor",
        "firkin-admission/firkin-hygiene",
        "inspect reserve-floor configuration, controller decisions, and post-decision resource state",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("start.hot_to_first_stdout_ms"),
        "startup_hot_to_stdout",
        "firkin-runtime/firkin-vminitd-client",
        "inspect readiness, exec dispatch, process start, and first stdout wait after a hot pool lease",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("start.hot_to_ready_ms"),
        "startup_hot_to_ready",
        "firkin-runtime",
        "inspect guest ping, workspace probe, exec probe, and optional DNS readiness after a hot pool lease",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("start.resume_to_first_stdout_ms"),
        "startup_resume_to_stdout",
        "firkin-runtime/firkin-single-node",
        "inspect snapshot restore, readiness, exec dispatch, and first stdout on the resumed path",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("start.warm_to_first_stdout_ms"),
        "startup_warm_to_stdout",
        "firkin-runtime/firkin-single-node",
        "inspect warm start, readiness, exec dispatch, and first stdout from request start",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("start.agent_task_ready_ms"),
        "agent_task_ready",
        "firkin-benchmark/firkin-runtime",
        "inspect external request handling through first useful stdout",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("pool.lease_ms"),
        "pool_lease",
        "firkin-admission/firkin-runtime",
        "inspect pool lookup and lease acquisition without readiness, workspace, exec, stdout, or cleanup",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("exec.command_start_ms"),
        "exec_first_process",
        "firkin-vminitd-client/firkin-runtime",
        "inspect exec RPC dispatch, vsock handoff, and guest process start",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("exec.first_stdout_byte_ms"),
        "exec_first_output",
        "firkin-vminitd-client/firkin-runtime",
        "inspect stdout transport buffering and first-byte propagation",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("exec.batch_100_small_commands_ms"),
        "exec_batch_100",
        "firkin-benchmark/firkin-vminitd-client",
        "inspect small-command batch dispatch and final process exit",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("density.max_active_before_hot_to_first_stdout_p95_doubles"),
        "density_hot_to_stdout",
        "firkin-benchmark/firkin-runtime",
        "inspect concurrent-create sweep for the hot-to-first-stdout p95 breakpoint",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("disk.sparse_bloat_after_trim"),
        "disk_trim_bloat",
        "firkin-benchmark/firkin-single-node",
        "inspect paired host allocation and guest used bytes after fstrim",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("disk.host_bytes_reclaimed_after_trim"),
        "disk_trim_reclaim",
        "firkin-benchmark/firkin-single-node",
        "inspect host allocation delta across fstrim",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("cleanup.leftover_bytes"),
        "cleanup_leftover",
        "firkin-hygiene/firkin-runtime",
        "inspect run-scoped Firkin-owned leftover bytes after destroy",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("reliability.unknown_failure_rate"),
        "runtime_reliability",
        "firkin-benchmark/firkin-runtime",
        "inspect classified attempts and unknown failure bucketing",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("agent_task_ready_ms"),
        "agent_task_ready",
        "firkin-runtime/firkin-benchmark",
        "inspect lifecycle span from sandbox create request through first useful stdout",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("command_start"),
        "exec_first_process",
        "firkin-vminitd-client/firkin-runtime",
        "inspect command dispatch, vsock handoff, and guest process spawn timing",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("first_stdout_byte"),
        "exec_first_output",
        "firkin-vminitd-client/firkin-runtime",
        "inspect stdout transport buffering and first-byte propagation",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("warm_snapshot_restore"),
        "startup_warm_restore",
        "firkin-runtime/firkin-single-node",
        "inspect warm snapshot restore and ready-template checkout path",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("sandbox.start.resume_snapshot_to_first_stdout_ms"),
        "startup_resume_to_stdout",
        "firkin-runtime/firkin-single-node",
        "inspect composed restore, readiness, envd routing, command dispatch, and first stdout path",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("cold_template_build"),
        "startup_cold_template",
        "firkin-runtime/firkin-single-node",
        "inspect cold template materialization and product create path",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("template.build_ms"),
        "template_build",
        "firkin-template/firkin-runtime",
        "inspect template command execution and rootfs bake path",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("template.snapshot_save_ms"),
        "template_snapshot",
        "firkin-template/firkin-runtime",
        "inspect template snapshot capture path",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("kill_delete"),
        "cleanup_delete",
        "firkin-runtime/firkin-hygiene",
        "inspect sandbox kill, delete, and managed-root cleanup",
    ),
    ownership(
        BenchmarkMetricMatch::Exact("concurrent_create"),
        "density_concurrent_create",
        "firkin-benchmark/firkin-runtime",
        "inspect concurrent create sweep and runtime capacity blockers",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("sandbox.disk."),
        "disk_io",
        "firkin-oci/firkin-template/firkin-core",
        "inspect rootfs, pod-store, filesystem, and trim measurement path",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("sandbox.mem."),
        "memory_attribution",
        "firkin-vmm/firkin-runtime",
        "inspect VM task attribution, guest reclaim, and host residual accounting",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("sandbox.net."),
        "network",
        "firkin-vmm/firkin-vminitd-client",
        "inspect guest networking, vsock, and policy-denial path",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("sandbox.exec."),
        "exec_control",
        "firkin-vminitd-client/firkin-runtime",
        "inspect exec RPC, stream transport, signal, and log backpressure path",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("sandbox.start."),
        "startup",
        "firkin-runtime/firkin-single-node",
        "inspect runtime create, template restore, VZ start, and guest readiness path",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("sandbox.cleanup."),
        "cleanup",
        "firkin-runtime/firkin-hygiene",
        "inspect managed-root scan, artifact deletion, and cleanup accounting",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("sandbox.density."),
        "density",
        "firkin-benchmark/firkin-runtime",
        "inspect benchmark concurrency sweep and runtime capacity policy",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("sandbox.pressure."),
        "guest_pressure",
        "firkin-vminitd-client/firkin-benchmark",
        "inspect guest PSI collection, kernel support, and pressure workload trigger",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("sandbox.reliability."),
        "runtime_reliability",
        "firkin-runtime/firkin-benchmark",
        "inspect classified runtime attempts, boot failures, and unknown failure buckets",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("product."),
        "product_path",
        "firkin-benchmark/firkin-runtime",
        "inspect product-level readiness probes and user-visible task path spans",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("autoscale."),
        "autoscale_controller",
        "firkin-admission/firkin-runtime",
        "inspect autoscale demand, supply, pressure, controller, and actuator traces",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("demand."),
        "autoscale_demand",
        "firkin-admission/firkin-benchmark",
        "inspect request arrival, priority, queueing, timeout, and cancellation traces",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("capacity."),
        "autoscale_capacity",
        "firkin-admission/firkin-runtime",
        "inspect active, ready, hot, resumed, warm, and reserve capacity accounting",
    ),
    ownership(
        BenchmarkMetricMatch::Prefix("pressure."),
        "autoscale_pressure",
        "firkin-admission/firkin-hygiene",
        "inspect host pressure sampling, reserve floors, thermal state, and sample freshness",
    ),
    ownership(
        BenchmarkMetricMatch::Fallback,
        "benchmark",
        "firkin-benchmark",
        "inspect suite definition and metric emission before assigning a narrower owner",
    ),
];

#[rustfmt::skip]
pub const BENCHMARK_METRIC_CATALOG: &[BenchmarkMetricDefinition] = &[
    metric("product.agent_computer_ready_ms", BenchmarkMetricGroup::Product, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::AutoscaleDashboard, "external-request-to-browser-database-cli-ready"),
    metric("product.agent_computer_resume_ms", BenchmarkMetricGroup::Product, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::AutoscaleDashboard, "pressure-suspended-agent-computer-to-product-ready"),
    metric("product.browser_ready_ms", BenchmarkMetricGroup::Product, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "browser-sidecar-ready-probe"),
    metric("product.database_ready_ms", BenchmarkMetricGroup::Product, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "sqlite-through-code-interpreter-proxy-until-db-sidecar"),
    metric("product.cli_ready_ms", BenchmarkMetricGroup::Product, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "cli-exec-ready-probe"),

    metric("autoscale.ready_queue_hit_rate_pct", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::AutoscaleDashboard, "requests-served-from-hot-or-resumed-ready-capacity"),
    metric("autoscale.safe_spare_utilization_pct", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "active-plus-ready-resource-over-safe-spare-resource"),
    metric("autoscale.safe_spare_cpu_utilization_pct", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "cpu-safe-spare-utilization"),
    metric("autoscale.safe_spare_memory_utilization_pct", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "memory-safe-spare-utilization"),
    metric("autoscale.safe_spare_disk_utilization_pct", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "disk-safe-spare-utilization"),
    metric("autoscale.safe_spare_limiting_utilization_pct", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::AutoscaleDashboard, "limiting-resource-safe-spare-utilization"),
    metric("autoscale.pressure_to_safe_floor_ms", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::AutoscaleDashboard, "pressure-detected-to-reserve-floors-satisfied"),
    metric("autoscale.pressure_clear_to_ready_target_ms", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::AutoscaleDashboard, "pressure-clear-or-demand-rise-to-ready-target-restored"),
    metric("autoscale.active_evictions_due_to_pool_pressure", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::AutoscaleDashboard, "active-sessions-evicted-only-for-pool-comfort"),
    metric("autoscale.reserve_floor_violations", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::AutoscaleDashboard, "controller-drove-resource-below-configured-reserve-floor"),
    metric("autoscale.tick_ms", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "autoscale-control-loop-tick-duration"),
    metric("autoscale.decision_latency_ms", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "demand-supply-pressure-sample-to-controller-decision"),
    metric("autoscale.decision", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Drilldown, "controller-decision-classification-count"),
    metric("autoscale.decision_reason", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Drilldown, "controller-decision-reason-count"),
    metric("autoscale.desired_ready_agent_computers", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "computed-agent-computer-ready-target"),
    metric("autoscale.desired_warm_cli_sandboxes", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "computed-cli-warm-pool-target"),
    metric("autoscale.desired_reserved_disk_bytes", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "computed-disk-reserve-floor"),
    metric("autoscale.desired_reserved_memory_bytes", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "computed-memory-reserve-floor"),
    metric("autoscale.actions_requested", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "controller-actions-requested"),
    metric("autoscale.actions_completed", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "controller-actions-completed"),
    metric("autoscale.action_failure_count", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "controller-action-failures"),
    metric("autoscale.stop_sidecars_ms", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "stop-agent-computer-sidecars-duration"),
    metric("autoscale.restart_sidecars_ms", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "restart-agent-computer-sidecars-duration"),
    metric("autoscale.suspend_agent_computer_ms", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "suspend-agent-computer-duration"),
    metric("autoscale.resume_agent_computer_ms", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "resume-agent-computer-duration"),
    metric("autoscale.destroy_idle_sandbox_ms", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "destroy-idle-sandbox-duration"),
    metric("autoscale.create_ready_sandbox_ms", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "create-ready-sandbox-duration"),
    metric("autoscale.reclaim_disk_bytes", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "disk-bytes-reclaimed-by-autoscale-action"),
    metric("autoscale.reclaim_memory_bytes", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "memory-bytes-reclaimed-by-autoscale-action"),
    metric("autoscale.action_timeout_count", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "autoscale-action-timeouts"),
    metric("autoscale.idle_ready_agent_computer_ms", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::FirkinOverhead, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "idle-ready-agent-computer-retention-time"),
    metric("autoscale.ready_agent_computer_used_before_eviction_pct", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::FirkinOverhead, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "ready-agent-computers-used-before-eviction"),
    metric("autoscale.wasted_ready_ms_per_served_request", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::FirkinOverhead, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "idle-ready-time-per-served-request"),
    metric("autoscale.wasted_disk_byte_ms", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::FirkinOverhead, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "idle-ready-disk-byte-time"),
    metric("autoscale.wasted_memory_byte_ms", BenchmarkMetricGroup::Autoscale, BenchmarkMetricKind::FirkinOverhead, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "idle-ready-memory-byte-time"),

    metric("demand.request_arrival_rate_per_s", BenchmarkMetricGroup::Demand, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::CountPerSecond, BenchmarkRequirementLevel::Core, "autoscale-request-arrival-rate"),
    metric("demand.burst_size", BenchmarkMetricGroup::Demand, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "requests-in-burst-window"),
    metric("demand.pending_agent_computers", BenchmarkMetricGroup::Demand, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "queued-agent-computer-requests"),
    metric("demand.pending_cli_sandboxes", BenchmarkMetricGroup::Demand, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "queued-cli-sandbox-requests"),
    metric("demand.queue_wait_ms", BenchmarkMetricGroup::Demand, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "request-queue-wait-duration"),
    metric("demand.queue_timeout_rate_pct", BenchmarkMetricGroup::Demand, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "queued-requests-that-time-out"),
    metric("demand.cancelled_while_queued_rate_pct", BenchmarkMetricGroup::Demand, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "queued-requests-cancelled-before-admission"),
    metric("demand.priority_class", BenchmarkMetricGroup::Demand, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Drilldown, "request-priority-classification-count"),

    metric("capacity.active_agent_computers", BenchmarkMetricGroup::Capacity, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "currently-active-agent-computers"),
    metric("capacity.ready_agent_computers", BenchmarkMetricGroup::Capacity, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "ready-agent-computers"),
    metric("capacity.hot_agent_computers", BenchmarkMetricGroup::Capacity, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "hot-agent-computers"),
    metric("capacity.resumed_agent_computers", BenchmarkMetricGroup::Capacity, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "resumed-agent-computers"),
    metric("capacity.warm_pool_entries", BenchmarkMetricGroup::Capacity, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "warm-cli-or-template-pool-entries"),
    metric("capacity.prepared_templates", BenchmarkMetricGroup::Capacity, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "prepared-template-count"),
    metric("capacity.cold_templates_available", BenchmarkMetricGroup::Capacity, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "available-cold-template-count"),
    metric("capacity.ready_target", BenchmarkMetricGroup::Capacity, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "controller-ready-target"),
    metric("capacity.ready_deficit", BenchmarkMetricGroup::Capacity, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "ready-target-minus-ready-capacity"),
    metric("capacity.ready_surplus", BenchmarkMetricGroup::Capacity, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "ready-capacity-above-ready-target"),
    metric("capacity.active_sessions", BenchmarkMetricGroup::Capacity, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "currently-active-sessions"),
    metric("capacity.pending_active_sessions", BenchmarkMetricGroup::Capacity, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "requests-accepted-but-not-yet-active"),
    metric("capacity.max_configured_agent_computers", BenchmarkMetricGroup::Capacity, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "configured-agent-computer-cap"),

    metric("pressure.state", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "autoscale-pressure-state-classification"),
    metric("pressure.reason", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "autoscale-pressure-reason-classification"),
    metric("pressure.disk_available_bytes", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "host-disk-available-to-autoscale"),
    metric("pressure.disk_reserved_floor_bytes", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "configured-disk-reserve-floor"),
    metric("pressure.memory_available_bytes", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "host-memory-available-to-autoscale"),
    metric("pressure.memory_reserved_floor_bytes", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "configured-memory-reserve-floor"),
    metric("pressure.cpu_load_pct", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "host-cpu-load-seen-by-autoscale"),
    metric("pressure.cpu_pressure_pct", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "cpu-pressure-seen-by-autoscale"),
    metric("pressure.io_pressure_pct", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "io-pressure-seen-by-autoscale"),
    metric("pressure.thermal_state", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "host-thermal-state-classification"),
    metric("pressure.power_state", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "host-power-state-classification"),
    metric("pressure.low_power_mode", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "low-power-mode-state"),
    metric("pressure.sample_age_ms", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "age-of-pressure-sample-at-decision-time"),

    metric("start.agent_task_ready_ms", BenchmarkMetricGroup::AgentTask, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::P0Dashboard, "external-request-to-first-useful-stdout"),
    metric("agent_task_ready_ms", BenchmarkMetricGroup::AgentTask, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "legacy-host-api-call-to-first-useful-tool-output"),
    metric("sandbox.task.time_to_first_tool_ms", BenchmarkMetricGroup::AgentTask, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "first-meaningful-tool-result"),
    metric("sandbox.task.time_to_first_test_ms", BenchmarkMetricGroup::AgentTask, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "first-test-output"),
    metric("sandbox.task.wall_ms", BenchmarkMetricGroup::AgentTask, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "full-agent-task-wall-clock"),
    metric("sandbox.task.success", BenchmarkMetricGroup::AgentTask, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "successful-agent-task-count"),
    metric("sandbox.task.failure_reason", BenchmarkMetricGroup::AgentTask, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "classified-agent-task-failures"),
    metric("sandbox.task.repo_import_ms", BenchmarkMetricGroup::AgentTask, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "repo-materialization-time"),
    metric("sandbox.task.repo_index_ms", BenchmarkMetricGroup::AgentTask, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "repo-index-time"),
    metric("sandbox.task.patch_apply_ms", BenchmarkMetricGroup::AgentTask, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "patch-application-time"),
    metric("sandbox.task.artifact_export_ms", BenchmarkMetricGroup::AgentTask, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "artifact-export-time"),

    metric("sandbox.start.total_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "request-to-ready-total"),
    metric("start.hot_to_first_stdout_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::P0Dashboard, "pool-lease-acquired-to-first-stdout"),
    metric("start.hot_to_ready_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::P0Dashboard, "pool-lease-acquired-to-exec-proven-ready"),
    metric("start.resume_to_first_stdout_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::P0Dashboard, "snapshot-restore-start-to-first-stdout"),
    metric("start.warm_to_first_stdout_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::P0Dashboard, "warm-request-start-to-first-stdout"),
    metric("pool.lease_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::P0Dashboard, "pool-lease-requested-to-acquired"),
    metric("sandbox.start.warm_ready_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "legacy-warm-local-image-ready"),
    metric("sandbox.start.cold_ready_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "legacy-cold-local-image-ready"),
    metric("sandbox.start.resume_snapshot_to_first_stdout_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "legacy-snapshot-restore-to-first-command-stdout"),
    metric("sandbox.start.hot_pool_checkout_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "legacy-hot-pool-checkout-to-ready"),
    metric("sandbox.start.request_received_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "host-request-accepted"),
    metric("sandbox.start.config_built_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "runtime-config-build"),
    metric("sandbox.start.image_resolve_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "oci-manifest-resolve"),
    metric("sandbox.start.image_pull_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "remote-image-fetch"),
    metric("sandbox.start.image_unpack_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "image-layer-unpack"),
    metric("sandbox.start.rootfs_prepare_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "rootfs-materialization"),
    metric("template.build_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "template-command-build"),
    metric("template.snapshot_save_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "template-snapshot-save"),
    metric("sandbox.start.rootfs_clone_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "rootfs-clone"),
    metric("sandbox.start.overlay_create_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "overlay-create"),
    metric("sandbox.start.workspace_create_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "workspace-materialization"),
    metric("sandbox.start.disk_attach_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "disk-device-attach"),
    metric("sandbox.start.vm_create_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "vm-object-create"),
    metric("sandbox.start.vz_validate_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "virtualization-config-validation"),
    metric("sandbox.start.vm_start_call_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "vz-start-call"),
    metric("sandbox.start.vm_boot_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "vm-start-to-guest-agent-listening"),
    metric("sandbox.start.kernel_boot_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "kernel-to-userspace"),
    metric("sandbox.start.init_started_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "guest-init-start"),
    metric("sandbox.start.guest_agent_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "guest-agent-readiness"),
    metric("sandbox.start.agent_handshake_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "vsock-agent-handshake"),
    metric("sandbox.start.network_device_attach_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "network-device-attach"),
    metric("sandbox.start.ip_assignment_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "guest-ip-assignment"),
    metric("sandbox.start.network_ready_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "network-usable"),
    metric("sandbox.start.dns_ready_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "dns-usable"),
    metric("sandbox.start.mounts_ready_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "workspace-and-volume-mounts-ready"),
    metric("sandbox.start.cgroups_ready_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "cgroup-limits-installed"),
    metric("sandbox.start.first_exec_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "first-command-accepted"),
    metric("sandbox.start.first_stdout_ms", BenchmarkMetricGroup::Startup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "first-command-output"),

    metric("sandbox.exec.count", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "executed-command-count"),
    metric("sandbox.exec.latency_ms", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "exec-request-roundtrip"),
    metric("exec.command_start_ms", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::P0Dashboard, "exec-request-sent-to-process-started"),
    metric("exec.first_stdout_byte_ms", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::P0Dashboard, "exec-request-sent-to-first-stdout-byte"),
    metric("exec.batch_100_small_commands_ms", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::P0Dashboard, "retained-shell-batch-100-small-commands-through-final-process-exit"),
    metric("sandbox.exec.first_latency_ms", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "legacy-first-exec-latency"),
    metric("sandbox.exec.first_stdout_ms", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "legacy-exec-request-to-first-stdout"),
    metric("sandbox.exec.exit_collection_ms", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "exit-status-collection"),
    metric("sandbox.exec.exit_code", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Drilldown, "process-exit-code"),
    metric("sandbox.exec.timeout_count", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "process-timeouts"),
    metric("sandbox.exec.signal_latency_ms", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "signal-delivery-latency"),
    metric("sandbox.exec.stdin_write_latency_ms", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "stdin-write-latency"),
    metric("sandbox.exec.stdout_stream_lag_ms", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "stdout-stream-lag"),
    metric("sandbox.exec.stderr_stream_lag_ms", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "stderr-stream-lag"),
    metric("sandbox.exec.max_log_backpressure_ms", BenchmarkMetricGroup::Exec, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "log-stream-backpressure"),

    metric("sandbox.cpu.host_percent", BenchmarkMetricGroup::Cpu, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "host-cpu-percent"),
    metric("sandbox.cpu.guest_usage_usec", BenchmarkMetricGroup::Cpu, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Microseconds, BenchmarkRequirementLevel::Core, "guest-cpu-usage"),
    metric("sandbox.cpu.cgroup_usage_usec", BenchmarkMetricGroup::Cpu, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Microseconds, BenchmarkRequirementLevel::Core, "cgroup-cpu-usage"),
    metric("sandbox.cpu.throttled_usec", BenchmarkMetricGroup::Cpu, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Microseconds, BenchmarkRequirementLevel::Core, "cgroup-throttled-time"),
    metric("sandbox.cpu.throttle_count", BenchmarkMetricGroup::Cpu, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "cgroup-throttle-events"),
    metric("sandbox.cpu.idle_tax_percent", BenchmarkMetricGroup::Cpu, BenchmarkMetricKind::FirkinOverhead, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "idle-cpu-per-hot-sandbox"),
    metric("sandbox.cpu.context_switches_per_sec", BenchmarkMetricGroup::Cpu, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::CountPerSecond, BenchmarkRequirementLevel::Drilldown, "context-switch-rate"),

    metric("sandbox.pressure.cpu_some_avg10", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "cpu-psi-some-avg10"),
    metric("sandbox.pressure.cpu_full_avg10", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "cpu-psi-full-avg10"),
    metric("sandbox.pressure.memory_some_avg10", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "memory-psi-some-avg10"),
    metric("sandbox.pressure.memory_full_avg10", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "memory-psi-full-avg10"),
    metric("sandbox.pressure.io_some_avg10", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "io-psi-some-avg10"),
    metric("sandbox.pressure.io_full_avg10", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "io-psi-full-avg10"),
    metric("sandbox.pressure.events_per_task", BenchmarkMetricGroup::Pressure, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "pressure-events-per-task"),

    metric("sandbox.mem.host_footprint_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "host-vm-footprint"),
    metric("sandbox.mem.idle_host_footprint_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "idle-host-footprint-per-sandbox"),
    metric("sandbox.mem.host_rss_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "host-rss"),
    metric("sandbox.mem.host_private_dirty_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "host-private-dirty"),
    metric("sandbox.mem.host_compressed_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Drilldown, "host-compressed-estimate"),
    metric("sandbox.mem.guest_available_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "guest-memavailable"),
    metric("sandbox.mem.guest_used_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "guest-used-memory"),
    metric("sandbox.mem.guest_page_cache_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "guest-page-cache"),
    metric("sandbox.mem.guest_slab_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Drilldown, "guest-slab"),
    metric("sandbox.mem.cgroup_current_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "cgroup-memory-current"),
    metric("sandbox.mem.cgroup_peak_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "cgroup-memory-peak"),
    metric("sandbox.mem.cgroup_oom_count", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "cgroup-oom-count"),
    metric("sandbox.mem.cgroup_oom_kill_count", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "cgroup-oom-kill-count"),
    metric("sandbox.mem.balloon_target_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "balloon-target"),
    metric("sandbox.mem.reclaimed_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "host-memory-reclaimed"),
    metric("sandbox.mem.retention_ratio", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Ratio, BenchmarkRequirementLevel::Core, "memory-retention-after-guest-free"),
    metric("sandbox.mem.post_task_residual_bytes", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "post-task-host-residual-memory"),
    metric("sandbox.mem.reclaim_effectiveness_ratio", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Ratio, BenchmarkRequirementLevel::Core, "host-memory-reclaim-effectiveness"),
    metric("sandbox.mem.recycle_required", BenchmarkMetricGroup::Memory, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "recycle-required-after-memory-spike"),

    metric("sandbox.disk.read_bytes", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "disk-read-bytes"),
    metric("sandbox.disk.write_bytes", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "disk-write-bytes"),
    metric("sandbox.disk.read_iops", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Iops, BenchmarkRequirementLevel::Core, "read-iops"),
    metric("sandbox.disk.write_iops", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Iops, BenchmarkRequirementLevel::Core, "write-iops"),
    metric("sandbox.disk.seq_read_mib_s", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::MebibytesPerSecond, BenchmarkRequirementLevel::Drilldown, "sequential-read-throughput"),
    metric("sandbox.disk.seq_write_mib_s", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::MebibytesPerSecond, BenchmarkRequirementLevel::Drilldown, "sequential-write-throughput"),
    metric("sandbox.disk.rand_read_iops_4k_qd1", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Iops, BenchmarkRequirementLevel::Drilldown, "4k-random-read-qd1"),
    metric("sandbox.disk.rand_write_iops_4k_qd1", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Iops, BenchmarkRequirementLevel::Drilldown, "4k-random-write-qd1"),
    metric("sandbox.disk.rand_read_iops_4k_qd32", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Iops, BenchmarkRequirementLevel::Drilldown, "4k-random-read-qd32"),
    metric("sandbox.disk.rand_write_iops_4k_qd32", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Iops, BenchmarkRequirementLevel::Drilldown, "4k-random-write-qd32"),
    metric("sandbox.disk.read_latency_p99_us", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Microseconds, BenchmarkRequirementLevel::Drilldown, "read-latency-p99"),
    metric("sandbox.disk.write_latency_p99_us", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Microseconds, BenchmarkRequirementLevel::Drilldown, "write-latency-p99"),
    metric("sandbox.disk.fsync_p99_us", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Microseconds, BenchmarkRequirementLevel::Core, "fsync-latency-p99"),
    metric("sandbox.disk.fdatasync_p99_us", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Microseconds, BenchmarkRequirementLevel::Core, "fdatasync-latency-p99"),
    metric("sandbox.disk.metadata_create_stat_unlink_ms", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "small-file-create-stat-unlink-bundle"),
    metric("sandbox.disk.create_10k_files_ms", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "create-10k-files"),
    metric("sandbox.disk.stat_10k_files_ms", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "stat-10k-files"),
    metric("sandbox.disk.unlink_10k_files_ms", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "unlink-10k-files"),
    metric("sandbox.disk.rename_10k_files_ms", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "rename-10k-files"),
    metric("sandbox.disk.git_checkout_ms", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "git-checkout-workload"),
    metric("sandbox.disk.git_status_ms", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "git-status-workload"),
    metric("sandbox.disk.cargo_build_ms", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "cargo-build-workload"),
    metric("sandbox.disk.npm_install_ms", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "npm-install-workload"),
    metric("sandbox.disk.sqlite_txn_s", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::OperationsPerSecond, BenchmarkRequirementLevel::Drilldown, "sqlite-transaction-rate"),
    metric("sandbox.disk.guest_used_bytes", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "guest-df-used"),
    metric("sandbox.disk.guest_available_bytes", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "guest-df-available"),
    metric("sandbox.disk.host_allocated_bytes", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "host-allocated-bytes"),
    metric("sandbox.disk.host_logical_bytes", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "host-logical-bytes"),
    metric("disk.sparse_bloat_after_delete", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Ratio, BenchmarkRequirementLevel::Core, "host-allocated-to-guest-used-ratio-after-delete-before-fstrim"),
    metric("disk.sparse_bloat_after_trim", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Ratio, BenchmarkRequirementLevel::P0Dashboard, "host-allocated-to-guest-used-ratio-after-fstrim"),
    metric("sandbox.disk.sparse_bloat_ratio", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Ratio, BenchmarkRequirementLevel::Core, "legacy-host-allocated-to-guest-used-ratio"),
    metric("sandbox.disk.trim_ms", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "fstrim-duration"),
    metric("sandbox.disk.trim_reclaimed_bytes", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "host-bytes-reclaimed-after-trim"),
    metric("disk.host_bytes_reclaimed_after_trim", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::P0Dashboard, "host-bytes-reclaimed-after-fstrim"),
    metric("sandbox.disk.trim_reclaim_bytes_per_sec", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::BytesPerSecond, BenchmarkRequirementLevel::Core, "trim-reclaim-throughput"),
    metric("sandbox.disk.host_bytes_reclaimed_after_destroy", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "host-bytes-reclaimed-after-destroy"),
    metric("sandbox.disk.volume_delete_ms", BenchmarkMetricGroup::Disk, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "volume-delete-duration"),

    metric("sandbox.net.rx_bytes", BenchmarkMetricGroup::Network, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "network-rx-bytes"),
    metric("sandbox.net.tx_bytes", BenchmarkMetricGroup::Network, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "network-tx-bytes"),
    metric("sandbox.net.dns_ms", BenchmarkMetricGroup::Network, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "first-dns-lookup"),
    metric("sandbox.net.connect_ms", BenchmarkMetricGroup::Network, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "first-tcp-connect"),
    metric("sandbox.net.tls_handshake_ms", BenchmarkMetricGroup::Network, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "tls-handshake"),
    metric("sandbox.net.http_get_small_ms", BenchmarkMetricGroup::Network, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "small-http-get"),
    metric("sandbox.net.download_mib_s", BenchmarkMetricGroup::Network, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::MebibytesPerSecond, BenchmarkRequirementLevel::Drilldown, "http-download-throughput"),
    metric("sandbox.net.rtt_us", BenchmarkMetricGroup::Network, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Microseconds, BenchmarkRequirementLevel::Core, "network-rtt"),
    metric("sandbox.net.vsock_rtt_us", BenchmarkMetricGroup::Network, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Microseconds, BenchmarkRequirementLevel::Core, "vsock-rtt"),
    metric("sandbox.net.vsock_throughput_mib_s", BenchmarkMetricGroup::Network, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::MebibytesPerSecond, BenchmarkRequirementLevel::Core, "vsock-throughput"),
    metric("sandbox.net.policy_denied_count", BenchmarkMetricGroup::Network, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "network-policy-denials"),
    metric("sandbox.net.dns_denied_count", BenchmarkMetricGroup::Network, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "dns-policy-denials"),
    metric("sandbox.net.packet_loss_percent", BenchmarkMetricGroup::Network, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Drilldown, "packet-loss"),
    metric("sandbox.net.connection_failure_rate", BenchmarkMetricGroup::Network, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "connection-failure-rate"),

    metric("sandbox.pids.current", BenchmarkMetricGroup::Pids, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "pids-current"),
    metric("sandbox.pids.peak", BenchmarkMetricGroup::Pids, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "pids-peak"),
    metric("sandbox.pids.limit_hits", BenchmarkMetricGroup::Pids, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "pids-limit-hits"),
    metric("sandbox.pids.zombie_count", BenchmarkMetricGroup::Pids, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Drilldown, "zombie-process-count"),
    metric("sandbox.pids.reap_latency_ms", BenchmarkMetricGroup::Pids, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "process-reap-latency"),

    metric("sandbox.pod.vm_ready_ms", BenchmarkMetricGroup::Pod, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "pod-vm-ready"),
    metric("sandbox.pod.container_spawn_ms", BenchmarkMetricGroup::Pod, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "container-spawn-inside-pod"),
    metric("sandbox.pod.additional_container_memory_bytes", BenchmarkMetricGroup::Pod, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "marginal-container-memory"),
    metric("sandbox.pod.localhost_rtt_us", BenchmarkMetricGroup::Pod, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Microseconds, BenchmarkRequirementLevel::Core, "pod-localhost-rtt"),
    metric("sandbox.pod.shared_volume_latency_us", BenchmarkMetricGroup::Pod, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Microseconds, BenchmarkRequirementLevel::Drilldown, "shared-volume-latency"),
    metric("sandbox.pod.amortization_savings_ratio", BenchmarkMetricGroup::Pod, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Ratio, BenchmarkRequirementLevel::Core, "pod-vs-vm-memory-savings"),

    metric("sandbox.cache.hit_ratio", BenchmarkMetricGroup::Cache, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Ratio, BenchmarkRequirementLevel::Core, "cache-hit-ratio"),
    metric("sandbox.cache.lookup_ms", BenchmarkMetricGroup::Cache, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "cache-lookup"),
    metric("sandbox.cache.restore_ms", BenchmarkMetricGroup::Cache, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "cache-restore"),
    metric("sandbox.cache.save_ms", BenchmarkMetricGroup::Cache, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "cache-save"),
    metric("sandbox.cache.bytes_retained", BenchmarkMetricGroup::Cache, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "cache-retained-bytes"),
    metric("sandbox.cache.evictions", BenchmarkMetricGroup::Cache, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "cache-eviction-count"),
    metric("sandbox.cache.corruption_count", BenchmarkMetricGroup::Cache, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "cache-corruption-count"),

    metric("sandbox.isolation.host_mount_count", BenchmarkMetricGroup::Isolation, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "host-mount-count"),
    metric("sandbox.isolation.host_mount_writable_count", BenchmarkMetricGroup::Isolation, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "writable-host-mount-count"),
    metric("sandbox.isolation.host_writable_bytes_exposed", BenchmarkMetricGroup::Isolation, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "host-writable-byte-exposure"),
    metric("sandbox.isolation.capabilities_granted_count", BenchmarkMetricGroup::Isolation, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "linux-capability-count"),
    metric("sandbox.isolation.network_profile", BenchmarkMetricGroup::Isolation, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "network-profile-classification"),
    metric("sandbox.isolation.risk_flags", BenchmarkMetricGroup::Isolation, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "security-risk-flag-count"),
    metric("sandbox.isolation.secret_exposure_duration_ms", BenchmarkMetricGroup::Isolation, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Drilldown, "secret-mount-exposure-duration"),

    metric("sandbox.cleanup.total_ms", BenchmarkMetricGroup::Cleanup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "cleanup-total"),
    metric("sandbox.cleanup.fstrim_ms", BenchmarkMetricGroup::Cleanup, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "cleanup-fstrim"),
    metric("sandbox.cleanup.host_disk_reclaimed_bytes", BenchmarkMetricGroup::Cleanup, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "disk-reclaimed-during-cleanup"),
    metric("sandbox.cleanup.host_mem_reclaimed_bytes", BenchmarkMetricGroup::Cleanup, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "memory-reclaimed-during-cleanup"),
    metric("cleanup.leftover_bytes", BenchmarkMetricGroup::Cleanup, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::P0Dashboard, "run-scoped-firkin-leftover-bytes-after-destroy"),
    metric("sandbox.cleanup.leftover_bytes", BenchmarkMetricGroup::Cleanup, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "legacy-bytes-left-after-cleanup"),
    metric("sandbox.cleanup.orphan_count", BenchmarkMetricGroup::Cleanup, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "orphaned-resource-count"),
    metric("sandbox.cleanup.leaked_bytes_per_sandbox", BenchmarkMetricGroup::Cleanup, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Bytes, BenchmarkRequirementLevel::Core, "leak-test-byte-slope"),

    metric("sandbox.reliability.boot_failure_rate", BenchmarkMetricGroup::Reliability, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "boot-failure-rate"),
    metric("sandbox.reliability.agent_handshake_failure_rate", BenchmarkMetricGroup::Reliability, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "agent-handshake-failure-rate"),
    metric("sandbox.reliability.network_ready_failure_rate", BenchmarkMetricGroup::Reliability, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "network-readiness-failure-rate"),
    metric("sandbox.reliability.first_exec_failure_rate", BenchmarkMetricGroup::Reliability, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "first-exec-failure-rate"),
    metric("reliability.unknown_failure_rate", BenchmarkMetricGroup::Reliability, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::P0Dashboard, "unknown-failure-rate-after-classified-failure-exclusion"),
    metric("sandbox.reliability.unknown_failure_rate", BenchmarkMetricGroup::Reliability, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "legacy-unclassified-failure-rate"),
    metric("sandbox.reliability.forced_kill_count", BenchmarkMetricGroup::Reliability, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "forced-kill-count"),

    metric("sandbox.density.ready_p95_by_concurrency_ms", BenchmarkMetricGroup::Density, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "ready-p95-under-concurrency"),
    metric("sandbox.density.task_wall_p95_by_concurrency_ms", BenchmarkMetricGroup::Density, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::Core, "task-wall-p95-under-concurrency"),
    metric("sandbox.density.max_idle_hot_sandboxes_before_pressure", BenchmarkMetricGroup::Density, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "max-idle-hot-pool-before-pressure"),
    metric("density.max_active_before_hot_to_first_stdout_p95_doubles", BenchmarkMetricGroup::Density, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::P0Dashboard, "max-active-before-hot-to-first-stdout-p95-doubles"),
    metric("density.max_agent_computers_before_ready_p95_doubles", BenchmarkMetricGroup::Density, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::AutoscaleDashboard, "max-browser-database-cli-agent-computers-before-product-ready-p95-doubles"),
    metric("density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles", BenchmarkMetricGroup::Density, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::AutoscaleDashboard, "max-prestarted-agent-slots-before-checkout-ready-p95-doubles"),
    metric("density.prestarted_agent_slot_fifo_acceptance_p95_ms", BenchmarkMetricGroup::Density, BenchmarkMetricKind::LifecycleLatency, BenchmarkUnit::Milliseconds, BenchmarkRequirementLevel::AutoscaleDashboard, "prestarted-agent-slot-fifo-acceptance-p95-snappy-guard"),
    metric("sandbox.density.max_active_before_p95_doubles", BenchmarkMetricGroup::Density, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "legacy-max-active-before-unqualified-p95-doubles"),
    metric("sandbox.density.sandboxes_per_gb_idle", BenchmarkMetricGroup::Density, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Count, BenchmarkRequirementLevel::Core, "idle-density-per-gb"),
    metric("sandbox.density.tasks_per_minute", BenchmarkMetricGroup::Density, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::CountPerSecond, BenchmarkRequirementLevel::Core, "successful-task-throughput"),
    metric("sandbox.density.p95_degradation_per_doubling", BenchmarkMetricGroup::Density, BenchmarkMetricKind::WorkloadResource, BenchmarkUnit::Ratio, BenchmarkRequirementLevel::Core, "tail-degradation-slope"),

    metric("sandbox.power.idle_cpu_percent_with_hot_pool", BenchmarkMetricGroup::Power, BenchmarkMetricKind::FirkinOverhead, BenchmarkUnit::Percent, BenchmarkRequirementLevel::Core, "host-idle-cpu-with-hot-pool"),
    metric("sandbox.power.wakeups_per_sec", BenchmarkMetricGroup::Power, BenchmarkMetricKind::FirkinOverhead, BenchmarkUnit::CountPerSecond, BenchmarkRequirementLevel::Core, "host-wakeup-rate"),
    metric("sandbox.power.battery_drain_percent_per_hour", BenchmarkMetricGroup::Power, BenchmarkMetricKind::FirkinOverhead, BenchmarkUnit::Percent, BenchmarkRequirementLevel::FutureHarness, "battery-drain-estimate"),
    metric("sandbox.power.thermal_pressure_state", BenchmarkMetricGroup::Power, BenchmarkMetricKind::FirkinOverhead, BenchmarkUnit::Count, BenchmarkRequirementLevel::FutureHarness, "thermal-pressure-classification"),
];

#[must_use]
pub const fn benchmark_metric_catalog() -> &'static [BenchmarkMetricDefinition] {
    BENCHMARK_METRIC_CATALOG
}

#[must_use]
pub fn benchmark_metric_definition(name: &str) -> Option<&'static BenchmarkMetricDefinition> {
    BENCHMARK_METRIC_CATALOG
        .iter()
        .find(|definition| definition.name == name)
}

#[must_use]
pub fn required_scorecard_metric_definitions() -> Vec<&'static BenchmarkMetricDefinition> {
    P0_SCORECARD_METRICS
        .iter()
        .map(|name| {
            benchmark_metric_definition(name)
                .expect("P0 scorecard metric must exist in benchmark catalog")
        })
        .collect()
}

#[must_use]
pub fn required_autoscale_efficiency_metric_definitions() -> Vec<&'static BenchmarkMetricDefinition>
{
    AUTOSCALE_EFFICIENCY_SCORECARD_METRICS
        .iter()
        .map(|name| {
            benchmark_metric_definition(name)
                .expect("autoscale efficiency scorecard metric must exist in benchmark catalog")
        })
        .collect()
}

#[must_use]
pub fn required_agent_computer_metric_definitions() -> Vec<&'static BenchmarkMetricDefinition> {
    AGENT_COMPUTER_SCORECARD_METRICS
        .iter()
        .map(|name| {
            benchmark_metric_definition(name)
                .expect("agent-computer scorecard metric must exist in benchmark catalog")
        })
        .collect()
}

#[must_use]
pub const fn p0_scorecard_measurement_coverage() -> &'static [BenchmarkMeasurementCoverage] {
    P0_SCORECARD_MEASUREMENT_COVERAGE
}

#[must_use]
pub const fn autoscale_efficiency_measurement_coverage() -> &'static [BenchmarkMeasurementCoverage]
{
    AUTOSCALE_EFFICIENCY_MEASUREMENT_COVERAGE
}

#[must_use]
pub const fn benchmark_metric_ownership_table() -> &'static [BenchmarkMetricOwnership] {
    BENCHMARK_METRIC_OWNERSHIP
}

#[must_use]
pub fn benchmark_metric_ownership(metric: &str) -> &'static BenchmarkMetricOwnership {
    BENCHMARK_METRIC_OWNERSHIP
        .iter()
        .find(|ownership| ownership.metric_match.matches(metric))
        .expect("benchmark metric ownership table must include fallback")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn catalog_metric_names_are_unique() {
        let mut names = BTreeSet::new();
        for metric in BENCHMARK_METRIC_CATALOG {
            assert!(
                names.insert(metric.name),
                "duplicate metric {}",
                metric.name
            );
        }
        assert!(names.len() > 130);
    }

    #[test]
    fn p0_scorecard_metrics_are_defined_and_marked_p0() {
        let catalog_p0 = BENCHMARK_METRIC_CATALOG
            .iter()
            .filter(|definition| definition.requirement == BenchmarkRequirementLevel::P0Dashboard)
            .map(|definition| definition.name)
            .collect::<BTreeSet<_>>();
        let scorecard_p0 = P0_SCORECARD_METRICS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        assert_eq!(scorecard_p0, catalog_p0);

        for name in P0_SCORECARD_METRICS {
            let definition = benchmark_metric_definition(name).expect("defined P0 metric");
            assert_eq!(
                definition.requirement,
                BenchmarkRequirementLevel::P0Dashboard
            );
        }
    }

    #[test]
    fn p0_scorecard_metrics_have_explicit_measurement_coverage() {
        let covered = P0_SCORECARD_MEASUREMENT_COVERAGE
            .iter()
            .map(|coverage| coverage.metric)
            .collect::<std::collections::BTreeSet<_>>();
        let scorecard_p0 = P0_SCORECARD_METRICS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        assert_eq!(covered.len(), P0_SCORECARD_METRICS.len());
        assert_eq!(covered, scorecard_p0);
        for metric in P0_SCORECARD_METRICS {
            assert!(
                covered.contains(metric),
                "P0 metric lacks measurement coverage: {metric}"
            );
        }
    }

    #[test]
    fn focused_dashboard_excludes_proxy_memory_metrics() {
        for metric in [
            "sandbox.mem.idle_host_footprint_bytes",
            "sandbox.mem.post_task_residual_bytes",
            "sandbox.mem.reclaim_effectiveness_ratio",
        ] {
            assert!(
                !P0_SCORECARD_METRICS.contains(&metric),
                "proxy memory metric {metric} must not be a focused dashboard metric"
            );
            let definition =
                benchmark_metric_definition(metric).expect("memory metric remains cataloged");
            assert_ne!(
                definition.requirement,
                BenchmarkRequirementLevel::P0Dashboard
            );
            assert!(
                P0_SCORECARD_MEASUREMENT_COVERAGE
                    .iter()
                    .all(|coverage| coverage.metric != metric),
                "proxy memory metric {metric} must not have P0 coverage"
            );
        }
    }

    #[test]
    fn p0_exact_metrics_do_not_cite_proxy_evidence() {
        for coverage in P0_SCORECARD_MEASUREMENT_COVERAGE {
            if coverage.status != BenchmarkMeasurementStatus::SignedLiveExact {
                continue;
            }

            assert!(
                coverage.source.starts_with("event_trace:"),
                "exact metric {} must cite event trace evidence, not {}",
                coverage.metric,
                coverage.source
            );
            assert!(
                !coverage.source.contains("overhead") && !coverage.notes.contains("proxy"),
                "exact metric {} must not cite proxy evidence",
                coverage.metric
            );
        }
    }

    #[test]
    fn ownership_exact_metrics_win_before_prefix_rows() {
        let command_start = benchmark_metric_ownership("command_start");
        assert_eq!(
            command_start.metric_match,
            BenchmarkMetricMatch::Exact("command_start")
        );
        assert_eq!(command_start.phase_label, "exec_first_process");
        assert_eq!(command_start.owner, "firkin-vminitd-client/firkin-runtime");

        let kill_delete = benchmark_metric_ownership("kill_delete");
        assert_eq!(
            kill_delete.metric_match,
            BenchmarkMetricMatch::Exact("kill_delete")
        );
        assert_eq!(kill_delete.phase_label, "cleanup_delete");
        assert_eq!(kill_delete.owner, "firkin-runtime/firkin-hygiene");
    }

    #[test]
    fn ownership_prefix_metrics_return_phase_owner_and_hint() {
        let memory = benchmark_metric_ownership("sandbox.mem.idle_host_footprint_bytes");
        assert_eq!(
            memory.metric_match,
            BenchmarkMetricMatch::Prefix("sandbox.mem.")
        );
        assert_eq!(memory.phase_label, "memory_attribution");
        assert_eq!(memory.owner, "firkin-vmm/firkin-runtime");
        assert!(memory.next_action_hint.contains("VM task attribution"));

        let disk = benchmark_metric_ownership("sandbox.disk.fsync_p99_us");
        assert_eq!(disk.phase_label, "disk_io");
        assert_eq!(disk.owner, "firkin-oci/firkin-template/firkin-core");
        assert!(disk.next_action_hint.contains("trim measurement path"));
    }

    #[test]
    fn ownership_fallback_keeps_unknown_metrics_actionable() {
        let ownership = benchmark_metric_ownership("foreign.metric");
        assert_eq!(ownership.metric_match, BenchmarkMetricMatch::Fallback);
        assert_eq!(ownership.phase_label, "benchmark");
        assert_eq!(ownership.owner, "firkin-benchmark");
        assert!(
            ownership
                .next_action_hint
                .contains("assigning a narrower owner")
        );
    }

    #[test]
    fn p0_metrics_have_non_fallback_ownership() {
        for metric in P0_SCORECARD_METRICS {
            let ownership = benchmark_metric_ownership(metric);
            assert_ne!(
                ownership.metric_match,
                BenchmarkMetricMatch::Fallback,
                "P0 metric {metric} must have a specific phase owner"
            );
            assert!(!ownership.phase_label.is_empty());
            assert!(!ownership.owner.is_empty());
            assert!(!ownership.next_action_hint.is_empty());
        }
    }

    #[test]
    fn ownership_table_has_one_fallback_at_the_end() {
        let fallback_count = BENCHMARK_METRIC_OWNERSHIP
            .iter()
            .filter(|ownership| ownership.metric_match == BenchmarkMetricMatch::Fallback)
            .count();

        assert_eq!(fallback_count, 1);
        assert_eq!(
            BENCHMARK_METRIC_OWNERSHIP
                .last()
                .expect("ownership fallback")
                .metric_match,
            BenchmarkMetricMatch::Fallback
        );
    }

    #[test]
    fn catalog_covers_requested_metric_families() {
        for group in [
            BenchmarkMetricGroup::Product,
            BenchmarkMetricGroup::Startup,
            BenchmarkMetricGroup::Exec,
            BenchmarkMetricGroup::AgentTask,
            BenchmarkMetricGroup::Autoscale,
            BenchmarkMetricGroup::Demand,
            BenchmarkMetricGroup::Capacity,
            BenchmarkMetricGroup::Disk,
            BenchmarkMetricGroup::Memory,
            BenchmarkMetricGroup::Cpu,
            BenchmarkMetricGroup::Pressure,
            BenchmarkMetricGroup::Network,
            BenchmarkMetricGroup::Pids,
            BenchmarkMetricGroup::Pod,
            BenchmarkMetricGroup::Cache,
            BenchmarkMetricGroup::Isolation,
            BenchmarkMetricGroup::Cleanup,
            BenchmarkMetricGroup::Reliability,
            BenchmarkMetricGroup::Density,
            BenchmarkMetricGroup::Power,
        ] {
            assert!(
                BENCHMARK_METRIC_CATALOG
                    .iter()
                    .any(|metric| metric.group == group),
                "missing group {}",
                group.as_str()
            );
        }
    }

    #[test]
    fn autoscale_scorecard_metrics_are_defined_and_marked_for_autoscale() {
        let catalog_autoscale = BENCHMARK_METRIC_CATALOG
            .iter()
            .filter(|definition| {
                definition.requirement == BenchmarkRequirementLevel::AutoscaleDashboard
            })
            .map(|definition| definition.name)
            .collect::<BTreeSet<_>>();
        let scorecard_autoscale_only = AUTOSCALE_EFFICIENCY_SCORECARD_METRICS
            .iter()
            .copied()
            .filter(|metric| {
                !matches!(
                    *metric,
                    "cleanup.leftover_bytes" | "reliability.unknown_failure_rate"
                )
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(scorecard_autoscale_only, catalog_autoscale);

        for name in AUTOSCALE_EFFICIENCY_SCORECARD_METRICS {
            let definition = benchmark_metric_definition(name).expect("defined autoscale metric");
            if matches!(
                *name,
                "cleanup.leftover_bytes" | "reliability.unknown_failure_rate"
            ) {
                assert_eq!(
                    definition.requirement,
                    BenchmarkRequirementLevel::P0Dashboard
                );
            } else {
                assert_eq!(
                    definition.requirement,
                    BenchmarkRequirementLevel::AutoscaleDashboard
                );
            }
        }
    }

    #[test]
    fn agent_computer_scorecard_is_product_subset_of_autoscale_scorecard() {
        let agent_computer = AGENT_COMPUTER_SCORECARD_METRICS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let autoscale = AUTOSCALE_EFFICIENCY_SCORECARD_METRICS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(
            agent_computer,
            [
                "product.agent_computer_ready_ms",
                "product.agent_computer_resume_ms",
                "density.max_agent_computers_before_ready_p95_doubles",
                "cleanup.leftover_bytes",
                "reliability.unknown_failure_rate",
            ]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
        );
        assert_eq!(
            AGENT_COMPUTER_DRILLDOWN_METRICS,
            &[
                "product.cli_ready_ms",
                "product.browser_ready_ms",
                "product.database_ready_ms",
            ]
        );
        assert!(agent_computer.is_subset(&autoscale));
        for metric in AGENT_COMPUTER_SCORECARD_METRICS {
            assert!(
                benchmark_metric_definition(metric).is_some(),
                "agent-computer metric {metric} must exist in benchmark catalog"
            );
        }
        for metric in AGENT_COMPUTER_DRILLDOWN_METRICS {
            assert!(
                benchmark_metric_definition(metric).is_some(),
                "agent-computer drilldown metric {metric} must exist in benchmark catalog"
            );
        }
    }

    #[test]
    fn autoscale_scorecard_metrics_have_explicit_measurement_coverage() {
        let covered = AUTOSCALE_EFFICIENCY_MEASUREMENT_COVERAGE
            .iter()
            .map(|coverage| coverage.metric)
            .collect::<BTreeSet<_>>();
        let scorecard_autoscale = AUTOSCALE_EFFICIENCY_SCORECARD_METRICS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        assert_eq!(covered.len(), AUTOSCALE_EFFICIENCY_SCORECARD_METRICS.len());
        assert_eq!(covered, scorecard_autoscale);
    }

    #[test]
    fn autoscale_calculation_metrics_are_unit_validated_not_live_yet() {
        let coverage = AUTOSCALE_EFFICIENCY_MEASUREMENT_COVERAGE
            .iter()
            .map(|coverage| (coverage.metric, coverage.status))
            .collect::<std::collections::BTreeMap<_, _>>();

        for metric in [
            "product.agent_computer_ready_ms",
            "product.agent_computer_resume_ms",
            "autoscale.ready_queue_hit_rate_pct",
            "autoscale.safe_spare_limiting_utilization_pct",
            "autoscale.pressure_to_safe_floor_ms",
            "autoscale.pressure_clear_to_ready_target_ms",
            "density.max_agent_computers_before_ready_p95_doubles",
            "autoscale.active_evictions_due_to_pool_pressure",
            "autoscale.reserve_floor_violations",
            "density.prestarted_agent_slot_fifo_acceptance_p95_ms",
        ] {
            assert_eq!(
                coverage.get(metric).copied(),
                Some(BenchmarkMeasurementStatus::UnitValidatedOnly),
                "{metric} should advertise unit-validated derivation without claiming signed-live evidence"
            );
        }

        assert!(
            coverage
                .values()
                .all(|status| *status != BenchmarkMeasurementStatus::NeedsLiveHarness)
        );
    }

    #[test]
    fn autoscale_scorecard_metrics_have_non_fallback_ownership() {
        for metric in AUTOSCALE_EFFICIENCY_SCORECARD_METRICS {
            let ownership = benchmark_metric_ownership(metric);
            assert_ne!(
                ownership.metric_match,
                BenchmarkMetricMatch::Fallback,
                "autoscale metric {metric} must have a specific phase owner"
            );
            assert!(!ownership.phase_label.is_empty());
            assert!(!ownership.owner.is_empty());
            assert!(!ownership.next_action_hint.is_empty());
        }
    }

    #[test]
    fn autoscale_scorecard_doc_matches_catalog_names() {
        let docs_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/specs/firkin-dummy-fast-slas.md");
        let docs = std::fs::read_to_string(&docs_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", docs_path.display()));
        let doc_names = docs
            .lines()
            .skip_while(|line| *line != "### Autoscale Scorecard")
            .skip(1)
            .take_while(|line| *line != "### Demand Metrics")
            .filter(|line| line.starts_with("| "))
            .filter_map(|line| line.split('|').nth(2))
            .map(str::trim)
            .filter_map(|cell| {
                cell.strip_prefix('`')
                    .and_then(|value| value.strip_suffix('`'))
            })
            .collect::<BTreeSet<_>>();
        let code_names = AUTOSCALE_EFFICIENCY_SCORECARD_METRICS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        assert_eq!(doc_names, code_names);
    }
}
