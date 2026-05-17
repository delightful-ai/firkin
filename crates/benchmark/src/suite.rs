//! Benchmark suite catalog.
#![allow(missing_docs)]

use firkin_evidence::{
    AUTOSCALE_EFFICIENCY_SCORECARD_METRICS, P0_SCORECARD_METRICS, benchmark_metric_definition,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BenchmarkExecutionMode {
    HostRunnable,
    LiveVmRequired,
    GuestAgentRequired,
    ExternalToolRequired,
    Manual,
}

impl BenchmarkExecutionMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HostRunnable => "host_runnable",
            Self::LiveVmRequired => "live_vm_required",
            Self::GuestAgentRequired => "guest_agent_required",
            Self::ExternalToolRequired => "external_tool_required",
            Self::Manual => "manual",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BenchmarkCaseDefinition {
    pub id: &'static str,
    pub metric: &'static str,
    pub execution: BenchmarkExecutionMode,
    pub notes: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BenchmarkSuiteDefinition {
    pub id: &'static str,
    pub title: &'static str,
    pub purpose: &'static str,
    pub cases: &'static [BenchmarkCaseDefinition],
}

const fn case(
    id: &'static str,
    metric: &'static str,
    execution: BenchmarkExecutionMode,
    notes: &'static str,
) -> BenchmarkCaseDefinition {
    BenchmarkCaseDefinition {
        id,
        metric,
        execution,
        notes,
    }
}

#[rustfmt::skip]
pub const AGENT_CORE_CASES: &[BenchmarkCaseDefinition] = &[
    case("hot_to_first_stdout", "start.hot_to_first_stdout_ms", BenchmarkExecutionMode::GuestAgentRequired, "hot-lease-to-first-stdout"),
    case("hot_to_ready", "start.hot_to_ready_ms", BenchmarkExecutionMode::GuestAgentRequired, "hot-lease-to-exec-proven-ready"),
    case("warm_to_first_stdout", "start.warm_to_first_stdout_ms", BenchmarkExecutionMode::GuestAgentRequired, "warm-request-start-to-first-stdout"),
    case("agent_task_ready", "start.agent_task_ready_ms", BenchmarkExecutionMode::GuestAgentRequired, "external-request-to-first-useful-stdout"),
    case("pool_lease", "pool.lease_ms", BenchmarkExecutionMode::LiveVmRequired, "pool-lease-only-latency"),
    case("direct_exec_command_start", "exec.direct_command_start_ms", BenchmarkExecutionMode::GuestAgentRequired, "direct-exec-request-to-process-start"),
    case("direct_exec_first_stdout", "exec.direct_first_stdout_byte_ms", BenchmarkExecutionMode::GuestAgentRequired, "direct-exec-request-to-first-stdout-byte"),
    case("exec_batch_100", "exec.batch_100_small_commands_ms", BenchmarkExecutionMode::GuestAgentRequired, "retained-shell-batch-100-small-command-wall-clock"),
    case("density_retained_shell_stdout", "density.max_active_before_retained_shell_first_stdout_p95_doubles", BenchmarkExecutionMode::LiveVmRequired, "retained-shell-first-stdout-concurrency-breakpoint"),
    case("sparse_bloat_after_trim", "disk.sparse_bloat_after_trim", BenchmarkExecutionMode::GuestAgentRequired, "post-fstrim-sparse-bloat-ratio"),
    case("trim_reclaim_bytes", "disk.host_bytes_reclaimed_after_trim", BenchmarkExecutionMode::GuestAgentRequired, "host-bytes-reclaimed-by-fstrim"),
    case("cleanup_leftover", "cleanup.leftover_bytes", BenchmarkExecutionMode::HostRunnable, "run-scoped-leftover-bytes-after-destroy"),
    case("unknown_failure_rate", "reliability.unknown_failure_rate", BenchmarkExecutionMode::LiveVmRequired, "unknown-failure-rate-after-classification"),
];

#[rustfmt::skip]
pub const STARTUP_CASES: &[BenchmarkCaseDefinition] = &[
    case("total", "sandbox.start.total_ms", BenchmarkExecutionMode::LiveVmRequired, "request-to-ready-total"),
    case("raw_snapshot_resume_to_first_stdout", "start.resume_to_first_stdout_ms", BenchmarkExecutionMode::GuestAgentRequired, "raw-snapshot-restore-start-to-first-stdout"),
    case("image_resolve", "sandbox.start.image_resolve_ms", BenchmarkExecutionMode::HostRunnable, "oci-manifest-resolve"),
    case("image_pull", "sandbox.start.image_pull_ms", BenchmarkExecutionMode::ExternalToolRequired, "registry-fetch"),
    case("rootfs_prepare", "sandbox.start.rootfs_prepare_ms", BenchmarkExecutionMode::HostRunnable, "rootfs-materialization"),
    case("disk_attach", "sandbox.start.disk_attach_ms", BenchmarkExecutionMode::LiveVmRequired, "disk-attach"),
    case("vm_create", "sandbox.start.vm_create_ms", BenchmarkExecutionMode::LiveVmRequired, "vm-object-create"),
    case("vm_boot", "sandbox.start.vm_boot_ms", BenchmarkExecutionMode::LiveVmRequired, "vm-start-to-agent-listening"),
    case("agent_handshake", "sandbox.start.agent_handshake_ms", BenchmarkExecutionMode::GuestAgentRequired, "vsock-agent-handshake"),
    case("network_ready", "sandbox.start.network_ready_ms", BenchmarkExecutionMode::GuestAgentRequired, "network-ready"),
    case("dns_ready", "sandbox.start.dns_ready_ms", BenchmarkExecutionMode::GuestAgentRequired, "dns-ready"),
    case("mounts_ready", "sandbox.start.mounts_ready_ms", BenchmarkExecutionMode::GuestAgentRequired, "mounts-ready"),
    case("first_exec", "sandbox.start.first_exec_ms", BenchmarkExecutionMode::GuestAgentRequired, "first-command-accepted"),
    case("first_stdout", "sandbox.start.first_stdout_ms", BenchmarkExecutionMode::GuestAgentRequired, "first-command-output"),
];

#[rustfmt::skip]
pub const DISK_CASES: &[BenchmarkCaseDefinition] = &[
    case("seq_read", "sandbox.disk.seq_read_mib_s", BenchmarkExecutionMode::GuestAgentRequired, "sequential-read-throughput"),
    case("seq_write", "sandbox.disk.seq_write_mib_s", BenchmarkExecutionMode::GuestAgentRequired, "sequential-write-throughput"),
    case("rand_read_qd1", "sandbox.disk.rand_read_iops_4k_qd1", BenchmarkExecutionMode::GuestAgentRequired, "random-read-4k-qd1"),
    case("rand_write_qd1", "sandbox.disk.rand_write_iops_4k_qd1", BenchmarkExecutionMode::GuestAgentRequired, "random-write-4k-qd1"),
    case("rand_read_qd32", "sandbox.disk.rand_read_iops_4k_qd32", BenchmarkExecutionMode::GuestAgentRequired, "random-read-4k-qd32"),
    case("rand_write_qd32", "sandbox.disk.rand_write_iops_4k_qd32", BenchmarkExecutionMode::GuestAgentRequired, "random-write-4k-qd32"),
    case("fsync", "sandbox.disk.fsync_p99_us", BenchmarkExecutionMode::GuestAgentRequired, "fsync-p99"),
    case("fdatasync", "sandbox.disk.fdatasync_p99_us", BenchmarkExecutionMode::GuestAgentRequired, "fdatasync-p99"),
    case("create_10k", "sandbox.disk.create_10k_files_ms", BenchmarkExecutionMode::GuestAgentRequired, "create-10k-small-files"),
    case("stat_10k", "sandbox.disk.stat_10k_files_ms", BenchmarkExecutionMode::GuestAgentRequired, "stat-10k-files"),
    case("unlink_10k", "sandbox.disk.unlink_10k_files_ms", BenchmarkExecutionMode::GuestAgentRequired, "unlink-10k-files"),
    case("git_checkout", "sandbox.disk.git_checkout_ms", BenchmarkExecutionMode::ExternalToolRequired, "git-checkout-workload"),
    case("git_status", "sandbox.disk.git_status_ms", BenchmarkExecutionMode::ExternalToolRequired, "git-status-workload"),
    case("cargo_build", "sandbox.disk.cargo_build_ms", BenchmarkExecutionMode::ExternalToolRequired, "cargo-build-workload"),
    case("npm_install", "sandbox.disk.npm_install_ms", BenchmarkExecutionMode::ExternalToolRequired, "npm-install-workload"),
    case("sparse_bloat_after_delete", "disk.sparse_bloat_after_delete", BenchmarkExecutionMode::HostRunnable, "post-delete-pre-fstrim-sparse-bloat-ratio"),
    case("sparse_bloat_after_trim", "disk.sparse_bloat_after_trim", BenchmarkExecutionMode::HostRunnable, "post-fstrim-sparse-bloat-ratio"),
    case("trim_reclaim_bytes", "disk.host_bytes_reclaimed_after_trim", BenchmarkExecutionMode::GuestAgentRequired, "host-bytes-reclaimed-by-fstrim"),
    case("trim", "sandbox.disk.trim_ms", BenchmarkExecutionMode::GuestAgentRequired, "fstrim-duration"),
    case("trim_reclaim", "sandbox.disk.trim_reclaim_bytes_per_sec", BenchmarkExecutionMode::GuestAgentRequired, "trim-reclaim-rate"),
];

#[rustfmt::skip]
pub const MEMORY_CASES: &[BenchmarkCaseDefinition] = &[
    case("idle_footprint", "sandbox.mem.idle_host_footprint_bytes", BenchmarkExecutionMode::HostRunnable, "idle-host-footprint"),
    case("host_rss", "sandbox.mem.host_rss_bytes", BenchmarkExecutionMode::HostRunnable, "host-rss"),
    case("guest_available", "sandbox.mem.guest_available_bytes", BenchmarkExecutionMode::GuestAgentRequired, "guest-memavailable"),
    case("cgroup_current", "sandbox.mem.cgroup_current_bytes", BenchmarkExecutionMode::GuestAgentRequired, "cgroup-memory-current"),
    case("cgroup_peak", "sandbox.mem.cgroup_peak_bytes", BenchmarkExecutionMode::GuestAgentRequired, "cgroup-memory-peak"),
    case("oom", "sandbox.mem.cgroup_oom_count", BenchmarkExecutionMode::GuestAgentRequired, "cgroup-oom-count"),
    case("balloon_target", "sandbox.mem.balloon_target_bytes", BenchmarkExecutionMode::LiveVmRequired, "balloon-target"),
    case("retention", "sandbox.mem.retention_ratio", BenchmarkExecutionMode::LiveVmRequired, "memory-retention-ratio"),
    case("reclaim", "sandbox.mem.reclaim_effectiveness_ratio", BenchmarkExecutionMode::LiveVmRequired, "memory-reclaim-effectiveness"),
    case("recycle_required", "sandbox.mem.recycle_required", BenchmarkExecutionMode::LiveVmRequired, "recycle-after-spike"),
];

#[rustfmt::skip]
pub const CPU_CASES: &[BenchmarkCaseDefinition] = &[
    case("host_cpu", "sandbox.cpu.host_percent", BenchmarkExecutionMode::HostRunnable, "host-cpu-percent"),
    case("guest_cpu", "sandbox.cpu.guest_usage_usec", BenchmarkExecutionMode::GuestAgentRequired, "guest-cpu-time"),
    case("cgroup_cpu", "sandbox.cpu.cgroup_usage_usec", BenchmarkExecutionMode::GuestAgentRequired, "cgroup-cpu-time"),
    case("throttle_time", "sandbox.cpu.throttled_usec", BenchmarkExecutionMode::GuestAgentRequired, "cpu-throttled-time"),
    case("throttle_count", "sandbox.cpu.throttle_count", BenchmarkExecutionMode::GuestAgentRequired, "cpu-throttle-count"),
    case("idle_tax", "sandbox.cpu.idle_tax_percent", BenchmarkExecutionMode::HostRunnable, "idle-cpu-tax"),
];

#[rustfmt::skip]
pub const PRESSURE_CASES: &[BenchmarkCaseDefinition] = &[
    case("cpu_some", "sandbox.pressure.cpu_some_avg10", BenchmarkExecutionMode::GuestAgentRequired, "cpu-psi-some"),
    case("memory_some", "sandbox.pressure.memory_some_avg10", BenchmarkExecutionMode::GuestAgentRequired, "memory-psi-some"),
    case("io_full", "sandbox.pressure.io_full_avg10", BenchmarkExecutionMode::GuestAgentRequired, "io-psi-full"),
    case("events", "sandbox.pressure.events_per_task", BenchmarkExecutionMode::GuestAgentRequired, "pressure-events-per-task"),
];

#[rustfmt::skip]
pub const NETWORK_CASES: &[BenchmarkCaseDefinition] = &[
    case("dns", "sandbox.net.dns_ms", BenchmarkExecutionMode::GuestAgentRequired, "first-dns-lookup"),
    case("connect", "sandbox.net.connect_ms", BenchmarkExecutionMode::GuestAgentRequired, "first-tcp-connect"),
    case("tls", "sandbox.net.tls_handshake_ms", BenchmarkExecutionMode::GuestAgentRequired, "tls-handshake"),
    case("http_small", "sandbox.net.http_get_small_ms", BenchmarkExecutionMode::GuestAgentRequired, "small-http-get"),
    case("download", "sandbox.net.download_mib_s", BenchmarkExecutionMode::GuestAgentRequired, "download-throughput"),
    case("rtt", "sandbox.net.rtt_us", BenchmarkExecutionMode::GuestAgentRequired, "network-rtt"),
    case("vsock_rtt", "sandbox.net.vsock_rtt_us", BenchmarkExecutionMode::LiveVmRequired, "vsock-rtt"),
    case("vsock_throughput", "sandbox.net.vsock_throughput_mib_s", BenchmarkExecutionMode::LiveVmRequired, "vsock-throughput"),
    case("policy_denied", "sandbox.net.policy_denied_count", BenchmarkExecutionMode::GuestAgentRequired, "egress-policy-denials"),
    case("packet_loss", "sandbox.net.packet_loss_percent", BenchmarkExecutionMode::GuestAgentRequired, "packet-loss"),
];

#[rustfmt::skip]
pub const POD_CASES: &[BenchmarkCaseDefinition] = &[
    case("pod_vm_ready", "sandbox.pod.vm_ready_ms", BenchmarkExecutionMode::LiveVmRequired, "pod-vm-ready"),
    case("container_spawn", "sandbox.pod.container_spawn_ms", BenchmarkExecutionMode::LiveVmRequired, "container-spawn-inside-pod"),
    case("container_memory", "sandbox.pod.additional_container_memory_bytes", BenchmarkExecutionMode::HostRunnable, "marginal-container-memory"),
    case("localhost_rtt", "sandbox.pod.localhost_rtt_us", BenchmarkExecutionMode::GuestAgentRequired, "pod-localhost-rtt"),
    case("shared_volume", "sandbox.pod.shared_volume_latency_us", BenchmarkExecutionMode::GuestAgentRequired, "shared-volume-latency"),
    case("amortization", "sandbox.pod.amortization_savings_ratio", BenchmarkExecutionMode::HostRunnable, "pod-vs-separate-vm-savings"),
];

#[rustfmt::skip]
pub const CONTROL_CASES: &[BenchmarkCaseDefinition] = &[
    case("exec_latency", "sandbox.exec.latency_ms", BenchmarkExecutionMode::GuestAgentRequired, "exec-rpc-latency"),
    case("stdout_lag", "sandbox.exec.stdout_stream_lag_ms", BenchmarkExecutionMode::GuestAgentRequired, "stdout-stream-lag"),
    case("stderr_lag", "sandbox.exec.stderr_stream_lag_ms", BenchmarkExecutionMode::GuestAgentRequired, "stderr-stream-lag"),
    case("stdin", "sandbox.exec.stdin_write_latency_ms", BenchmarkExecutionMode::GuestAgentRequired, "stdin-write-latency"),
    case("signal", "sandbox.exec.signal_latency_ms", BenchmarkExecutionMode::GuestAgentRequired, "signal-delivery"),
    case("log_backpressure", "sandbox.exec.max_log_backpressure_ms", BenchmarkExecutionMode::GuestAgentRequired, "log-backpressure"),
];

#[rustfmt::skip]
pub const CLEANUP_CASES: &[BenchmarkCaseDefinition] = &[
    case("cleanup_total", "sandbox.cleanup.total_ms", BenchmarkExecutionMode::HostRunnable, "cleanup-total"),
    case("cleanup_fstrim", "sandbox.cleanup.fstrim_ms", BenchmarkExecutionMode::GuestAgentRequired, "fstrim-cleanup"),
    case("disk_reclaimed", "sandbox.cleanup.host_disk_reclaimed_bytes", BenchmarkExecutionMode::HostRunnable, "disk-reclaimed"),
    case("mem_reclaimed", "sandbox.cleanup.host_mem_reclaimed_bytes", BenchmarkExecutionMode::HostRunnable, "memory-reclaimed"),
    case("leftover", "cleanup.leftover_bytes", BenchmarkExecutionMode::HostRunnable, "leftover-bytes"),
    case("orphans", "sandbox.cleanup.orphan_count", BenchmarkExecutionMode::HostRunnable, "orphaned-resources"),
    case("leak_slope", "sandbox.cleanup.leaked_bytes_per_sandbox", BenchmarkExecutionMode::Manual, "long-run-leak-slope"),
];

#[rustfmt::skip]
pub const ISOLATION_CASES: &[BenchmarkCaseDefinition] = &[
    case("host_mounts", "sandbox.isolation.host_mount_count", BenchmarkExecutionMode::HostRunnable, "host-mount-count"),
    case("writable_mounts", "sandbox.isolation.host_mount_writable_count", BenchmarkExecutionMode::HostRunnable, "writable-host-mounts"),
    case("writable_bytes", "sandbox.isolation.host_writable_bytes_exposed", BenchmarkExecutionMode::HostRunnable, "writable-byte-exposure"),
    case("capabilities", "sandbox.isolation.capabilities_granted_count", BenchmarkExecutionMode::GuestAgentRequired, "capability-count"),
    case("network_profile", "sandbox.isolation.network_profile", BenchmarkExecutionMode::HostRunnable, "network-profile-risk"),
    case("risk_flags", "sandbox.isolation.risk_flags", BenchmarkExecutionMode::HostRunnable, "risk-flag-count"),
];

#[rustfmt::skip]
pub const CACHE_CASES: &[BenchmarkCaseDefinition] = &[
    case("hit_ratio", "sandbox.cache.hit_ratio", BenchmarkExecutionMode::HostRunnable, "cache-hit-ratio"),
    case("lookup", "sandbox.cache.lookup_ms", BenchmarkExecutionMode::HostRunnable, "cache-lookup"),
    case("restore", "sandbox.cache.restore_ms", BenchmarkExecutionMode::HostRunnable, "cache-restore"),
    case("save", "sandbox.cache.save_ms", BenchmarkExecutionMode::HostRunnable, "cache-save"),
    case("retained", "sandbox.cache.bytes_retained", BenchmarkExecutionMode::HostRunnable, "cache-retained-bytes"),
    case("evictions", "sandbox.cache.evictions", BenchmarkExecutionMode::HostRunnable, "cache-evictions"),
    case("corruption", "sandbox.cache.corruption_count", BenchmarkExecutionMode::HostRunnable, "cache-corruption-count"),
];

#[rustfmt::skip]
pub const DENSITY_CASES: &[BenchmarkCaseDefinition] = &[
    case("ready_p95", "sandbox.density.ready_p95_by_concurrency_ms", BenchmarkExecutionMode::LiveVmRequired, "ready-p95-by-concurrency"),
    case("task_wall_p95", "sandbox.density.task_wall_p95_by_concurrency_ms", BenchmarkExecutionMode::LiveVmRequired, "task-p95-by-concurrency"),
    case("idle_breakpoint", "sandbox.density.max_idle_hot_sandboxes_before_pressure", BenchmarkExecutionMode::LiveVmRequired, "idle-hot-sandbox-breakpoint"),
    case("active_breakpoint", "density.max_active_before_hot_to_first_stdout_p95_doubles", BenchmarkExecutionMode::LiveVmRequired, "hot-to-first-stdout-active-breakpoint"),
    case("retained_shell_breakpoint", "density.max_active_before_retained_shell_first_stdout_p95_doubles", BenchmarkExecutionMode::GuestAgentRequired, "retained-shell-first-stdout-active-breakpoint"),
    case("idle_per_gb", "sandbox.density.sandboxes_per_gb_idle", BenchmarkExecutionMode::HostRunnable, "idle-sandboxes-per-gb"),
    case("throughput", "sandbox.density.tasks_per_minute", BenchmarkExecutionMode::LiveVmRequired, "tasks-per-minute"),
    case("degradation", "sandbox.density.p95_degradation_per_doubling", BenchmarkExecutionMode::LiveVmRequired, "tail-degradation-slope"),
];

#[rustfmt::skip]
pub const POWER_CASES: &[BenchmarkCaseDefinition] = &[
    case("idle_cpu", "sandbox.power.idle_cpu_percent_with_hot_pool", BenchmarkExecutionMode::HostRunnable, "idle-hot-pool-cpu"),
    case("wakeups", "sandbox.power.wakeups_per_sec", BenchmarkExecutionMode::HostRunnable, "host-wakeup-rate"),
    case("battery", "sandbox.power.battery_drain_percent_per_hour", BenchmarkExecutionMode::Manual, "battery-drain-estimate"),
    case("thermal", "sandbox.power.thermal_pressure_state", BenchmarkExecutionMode::Manual, "thermal-pressure"),
];

#[rustfmt::skip]
pub const ABUSE_CASES: &[BenchmarkCaseDefinition] = &[
    case("pids_limit", "sandbox.pids.limit_hits", BenchmarkExecutionMode::GuestAgentRequired, "fork-bomb-pids-limit"),
    case("oom", "sandbox.mem.cgroup_oom_kill_count", BenchmarkExecutionMode::GuestAgentRequired, "memory-bomb-oom-kill"),
    case("disk_fill", "reliability.unknown_failure_rate", BenchmarkExecutionMode::GuestAgentRequired, "disk-fill-failure-classification"),
    case("stdout_flood", "sandbox.exec.max_log_backpressure_ms", BenchmarkExecutionMode::GuestAgentRequired, "stdout-flood-backpressure"),
    case("ignored_sigterm", "sandbox.exec.signal_latency_ms", BenchmarkExecutionMode::GuestAgentRequired, "ignored-sigterm-forced-kill"),
    case("cleanup", "sandbox.cleanup.orphan_count", BenchmarkExecutionMode::HostRunnable, "abuse-cleanup-orphans"),
];

#[rustfmt::skip]
pub const AGENT_REALISM_CASES: &[BenchmarkCaseDefinition] = &[
    case("repo_import", "sandbox.task.repo_import_ms", BenchmarkExecutionMode::ExternalToolRequired, "repo-import"),
    case("repo_index", "sandbox.task.repo_index_ms", BenchmarkExecutionMode::ExternalToolRequired, "repo-index"),
    case("first_tool", "sandbox.task.time_to_first_tool_ms", BenchmarkExecutionMode::GuestAgentRequired, "first-useful-tool"),
    case("first_test", "sandbox.task.time_to_first_test_ms", BenchmarkExecutionMode::ExternalToolRequired, "first-test-output"),
    case("task_wall", "sandbox.task.wall_ms", BenchmarkExecutionMode::ExternalToolRequired, "full-agent-task"),
    case("patch_apply", "sandbox.task.patch_apply_ms", BenchmarkExecutionMode::ExternalToolRequired, "patch-apply"),
    case("artifact_export", "sandbox.task.artifact_export_ms", BenchmarkExecutionMode::ExternalToolRequired, "artifact-export"),
];

#[rustfmt::skip]
pub const AGENT_COMPUTER_CASES: &[BenchmarkCaseDefinition] = &[
    case("product_ready", "product.agent_computer_ready_ms", BenchmarkExecutionMode::GuestAgentRequired, "external-request-to-browser-database-cli-ready"),
    case("product_resume", "product.agent_computer_resume_ms", BenchmarkExecutionMode::GuestAgentRequired, "pressure-suspended-agent-computer-to-product-ready"),
    case("cli_ready", "product.cli_ready_ms", BenchmarkExecutionMode::GuestAgentRequired, "external-request-to-first-useful-cli-stdout"),
    case("browser_ready", "product.browser_ready_ms", BenchmarkExecutionMode::GuestAgentRequired, "external-request-to-browser-control-ready"),
    case("database_ready", "product.database_ready_ms", BenchmarkExecutionMode::GuestAgentRequired, "external-request-to-database-ready"),
    case("density_agent_computer_ready", "density.max_agent_computers_before_ready_p95_doubles", BenchmarkExecutionMode::LiveVmRequired, "browser-database-cli-ready-p95-density-breakpoint"),
    case("cleanup_leftover", "cleanup.leftover_bytes", BenchmarkExecutionMode::HostRunnable, "run-scoped-leftover-bytes-after-destroy"),
    case("unknown_failure_rate", "reliability.unknown_failure_rate", BenchmarkExecutionMode::LiveVmRequired, "unknown-failure-rate-after-classification"),
];

#[rustfmt::skip]
pub const AUTOSCALE_CASES: &[BenchmarkCaseDefinition] = &[
    case("ready_queue_hit_rate", "autoscale.ready_queue_hit_rate_pct", BenchmarkExecutionMode::LiveVmRequired, "hot-or-resumed-ready-capacity-hit-rate"),
    case("product_ready", "product.agent_computer_ready_ms", BenchmarkExecutionMode::GuestAgentRequired, "external-request-to-browser-database-cli-ready"),
    case("product_resume", "product.agent_computer_resume_ms", BenchmarkExecutionMode::GuestAgentRequired, "pressure-suspended-agent-computer-to-product-ready"),
    case("safe_spare_limiting", "autoscale.safe_spare_limiting_utilization_pct", BenchmarkExecutionMode::LiveVmRequired, "limiting-resource-safe-spare-utilization"),
    case("pressure_to_safe_floor", "autoscale.pressure_to_safe_floor_ms", BenchmarkExecutionMode::LiveVmRequired, "pressure-detected-to-reserve-floors-satisfied"),
    case("pressure_clear_to_ready_target", "autoscale.pressure_clear_to_ready_target_ms", BenchmarkExecutionMode::LiveVmRequired, "pressure-clear-or-demand-rise-to-ready-target-restored"),
    case("density_agent_computer_ready", "density.max_agent_computers_before_ready_p95_doubles", BenchmarkExecutionMode::LiveVmRequired, "browser-database-cli-ready-p95-density-breakpoint"),
    case("density_prestarted_agent_slot_ready", "density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles", BenchmarkExecutionMode::LiveVmRequired, "prestarted-agent-slot-checkout-ready-p95-density-breakpoint"),
    case("density_prestarted_agent_slot_fifo_acceptance", "density.prestarted_agent_slot_fifo_acceptance_p95_ms", BenchmarkExecutionMode::LiveVmRequired, "prestarted-agent-slot-fifo-acceptance-p95-snappy-guard"),
    case("active_evictions_due_to_pool_pressure", "autoscale.active_evictions_due_to_pool_pressure", BenchmarkExecutionMode::LiveVmRequired, "active-session-evictions-caused-only-by-pool-comfort"),
    case("reserve_floor_violations", "autoscale.reserve_floor_violations", BenchmarkExecutionMode::LiveVmRequired, "controller-drove-resource-below-configured-reserve-floor"),
    case("cleanup_leftover", "cleanup.leftover_bytes", BenchmarkExecutionMode::HostRunnable, "run-scoped-leftover-bytes-after-destroy"),
    case("unknown_failure_rate", "reliability.unknown_failure_rate", BenchmarkExecutionMode::LiveVmRequired, "unknown-failure-rate-after-classification"),
];

pub const BENCHMARK_SUITES: &[BenchmarkSuiteDefinition] = &[
    BenchmarkSuiteDefinition {
        id: "agent-core",
        title: "Agent Core",
        purpose: "P0 dashboard for agent usefulness latency, resource floor, reclaim, and cleanup",
        cases: AGENT_CORE_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "startup",
        title: "Startup",
        purpose: "Cold/warm/hot sandbox readiness phase breakdown",
        cases: STARTUP_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "disk",
        title: "Disk",
        purpose: "Raw block, fsync, metadata, workload, and reclaim disk families",
        cases: DISK_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "memory",
        title: "Memory",
        purpose: "Host, guest, cgroup, balloon, and recycle behavior",
        cases: MEMORY_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "cpu",
        title: "CPU",
        purpose: "Host/guest/cgroup CPU usage and throttling",
        cases: CPU_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "pressure",
        title: "Pressure",
        purpose: "Linux PSI and task stall metrics",
        cases: PRESSURE_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "network",
        title: "Network",
        purpose: "Control-plane and data-plane network latency and policy",
        cases: NETWORK_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "pod",
        title: "Pod",
        purpose: "Pod amortization and marginal container costs",
        cases: POD_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "agent-control",
        title: "Agent Control Plane",
        purpose: "Exec/log/stdin/signal control-plane latency",
        cases: CONTROL_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "cleanup",
        title: "Cleanup",
        purpose: "Teardown, reclaim, leftovers, and orphan checks",
        cases: CLEANUP_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "isolation",
        title: "Isolation",
        purpose: "Machine-checkable exposure and risk summary metrics",
        cases: ISOLATION_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "cache",
        title: "Cache",
        purpose: "Cache lookup, restore, save, retained bytes, and poisoning indicators",
        cases: CACHE_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "density",
        title: "Density",
        purpose: "Startup storms, steady-state density, and noisy-neighbor curves",
        cases: DENSITY_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "power",
        title: "Power",
        purpose: "Apple Silicon laptop idle, wakeup, battery, and thermal costs",
        cases: POWER_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "abuse",
        title: "Abuse",
        purpose: "Fork, memory, disk, log, signal, and cleanup abuse matrix",
        cases: ABUSE_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "agent-realism",
        title: "Agent Realism",
        purpose: "Real coding-agent repo, test, patch, and artifact workflows",
        cases: AGENT_REALISM_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "agent-computer",
        title: "Agent Computer",
        purpose: "Default browser + database + CLI product-readiness path",
        cases: AGENT_COMPUTER_CASES,
    },
    BenchmarkSuiteDefinition {
        id: "autoscale",
        title: "Autoscale",
        purpose: "Ready queue, pressure shrink/refill, active protection, and reserve-floor behavior",
        cases: AUTOSCALE_CASES,
    },
];

#[must_use]
pub const fn benchmark_suites() -> &'static [BenchmarkSuiteDefinition] {
    BENCHMARK_SUITES
}

#[must_use]
pub fn benchmark_suite(id: &str) -> Option<&'static BenchmarkSuiteDefinition> {
    BENCHMARK_SUITES.iter().find(|suite| suite.id == id)
}

#[must_use]
pub fn benchmark_cases_for_metric(metric: &str) -> Vec<&'static BenchmarkCaseDefinition> {
    BENCHMARK_SUITES
        .iter()
        .flat_map(|suite| suite.cases.iter())
        .filter(|case| case.metric == metric)
        .collect()
}

#[must_use]
pub fn benchmark_suite_missing_metric_definitions() -> Vec<&'static str> {
    BENCHMARK_SUITES
        .iter()
        .flat_map(|suite| suite.cases.iter())
        .filter_map(|case| {
            benchmark_metric_definition(case.metric)
                .is_none()
                .then_some(case.metric)
        })
        .collect()
}

#[must_use]
pub fn agent_core_missing_scorecard_metrics() -> Vec<&'static str> {
    let Some(suite) = benchmark_suite("agent-core") else {
        return P0_SCORECARD_METRICS.to_vec();
    };
    P0_SCORECARD_METRICS
        .iter()
        .copied()
        .filter(|metric| !suite.cases.iter().any(|case| case.metric == *metric))
        .collect()
}

#[must_use]
pub fn autoscale_missing_scorecard_metrics() -> Vec<&'static str> {
    let Some(suite) = benchmark_suite("autoscale") else {
        return AUTOSCALE_EFFICIENCY_SCORECARD_METRICS.to_vec();
    };
    AUTOSCALE_EFFICIENCY_SCORECARD_METRICS
        .iter()
        .copied()
        .filter(|metric| !suite.cases.iter().any(|case| case.metric == *metric))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use firkin_evidence::{AGENT_COMPUTER_DRILLDOWN_METRICS, AGENT_COMPUTER_SCORECARD_METRICS};

    use super::*;

    #[test]
    fn suite_ids_are_unique() {
        let mut ids = BTreeSet::new();
        for suite in BENCHMARK_SUITES {
            assert!(ids.insert(suite.id), "duplicate suite {}", suite.id);
        }
        assert!(ids.contains("agent-core"));
        assert!(ids.contains("abuse"));
        assert!(ids.contains("agent-realism"));
        assert!(ids.contains("agent-computer"));
        assert!(ids.contains("autoscale"));
    }

    #[test]
    fn all_suite_case_metrics_exist_in_catalog() {
        assert_eq!(
            benchmark_suite_missing_metric_definitions(),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn agent_core_suite_covers_all_p0_scorecard_metrics() {
        assert_eq!(agent_core_missing_scorecard_metrics(), Vec::<&str>::new());
    }

    #[test]
    fn agent_core_suite_matches_p0_scorecard_metrics_exactly() {
        let agent_core = AGENT_CORE_CASES
            .iter()
            .map(|case| case.metric)
            .collect::<BTreeSet<_>>();
        let p0_scorecard = P0_SCORECARD_METRICS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        assert_eq!(agent_core, p0_scorecard);
        assert_eq!(AGENT_CORE_CASES.len(), P0_SCORECARD_METRICS.len());
    }

    #[test]
    fn agent_core_suite_matches_decision_grade_metric_contract() {
        let agent_core = AGENT_CORE_CASES
            .iter()
            .map(|case| case.metric)
            .collect::<BTreeSet<_>>();
        let decision_grade = firkin_evidence::decision_grade_metric_contract()
            .iter()
            .map(|metric| metric.metric())
            .collect::<BTreeSet<_>>();

        assert_eq!(agent_core, decision_grade);
    }

    #[test]
    fn autoscale_suite_covers_all_autoscale_scorecard_metrics() {
        assert_eq!(autoscale_missing_scorecard_metrics(), Vec::<&str>::new());
    }

    #[test]
    fn autoscale_suite_matches_autoscale_scorecard_metrics_exactly() {
        let autoscale = AUTOSCALE_CASES
            .iter()
            .map(|case| case.metric)
            .collect::<BTreeSet<_>>();
        let scorecard = AUTOSCALE_EFFICIENCY_SCORECARD_METRICS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        assert_eq!(autoscale, scorecard);
        assert_eq!(
            AUTOSCALE_CASES.len(),
            AUTOSCALE_EFFICIENCY_SCORECARD_METRICS.len()
        );
    }

    #[test]
    fn agent_computer_suite_is_product_path_not_full_autoscale_board() {
        let agent_computer = AGENT_COMPUTER_CASES
            .iter()
            .map(|case| case.metric)
            .collect::<BTreeSet<_>>();
        let scorecard = AGENT_COMPUTER_SCORECARD_METRICS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let drilldown = AGENT_COMPUTER_DRILLDOWN_METRICS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let expected = scorecard
            .union(&drilldown)
            .copied()
            .collect::<BTreeSet<_>>();

        assert_eq!(agent_computer, expected);
        assert_eq!(
            AGENT_COMPUTER_CASES.len(),
            AGENT_COMPUTER_SCORECARD_METRICS.len() + AGENT_COMPUTER_DRILLDOWN_METRICS.len()
        );
        assert!(!agent_computer.contains("autoscale.ready_queue_hit_rate_pct"));
        assert!(!agent_computer.contains("autoscale.reserve_floor_violations"));
    }

    #[test]
    fn agent_core_case_ids_are_unique() {
        let mut ids = BTreeSet::new();
        for case in AGENT_CORE_CASES {
            assert!(ids.insert(case.id), "duplicate agent-core case {}", case.id);
        }
        assert_eq!(ids.len(), AGENT_CORE_CASES.len());
    }
}
