# Firkin Trace Pre-Split Surface Specification

**Goal:** land the first-class trace and benchmark surface that must exist before the workspace crate split, without pulling broad runtime/guest instrumentation into the pre-split churn.

**Architecture:** `firkin-trace` is the low-level measurement leaf. Existing substrate evidence consumes trace primitives and remains the temporary evidence owner until the crate split creates `firkin-evidence`. CLI benchmark commands operate on evidence artifacts and expose the current gates with p50, p90, p95, p99, and max summaries.

**Hard Cutover:** new benchmark evidence artifacts include p90, p99, and max in every `BenchmarkSummary`. Old summary-only artifacts without those fields are not a compatibility target; regenerate evidence through the current runtime/CLI flow.

---

## Scope

This is the complete pre-split surface:

1. `crates/trace/` exists as package `firkin-trace`.
2. `BenchmarkSample`, `BenchmarkMetricKind`, and `BenchmarkUnit` live in `firkin-trace`, not substrate/evidence.
3. `firkin-trace` owns only measurement primitives:
   - `BenchmarkSample`
   - `BenchmarkMetricKind`
   - `BenchmarkUnit`
   - `Recorder`
   - `Span`
   - `Sampler`
   - `Checkpoint`
   - `Tags`
   - `RecordedTrace`
   - overflow/closed-drop stats
   - stable phase constants
4. `firkin-trace` must not depend on any workspace crate.
5. `firkin-trace` must not own domain types such as `Pod`, `Container`, `Sandbox`, `VirtualMachine`, `VmConfig`, OCI clients, vminitd clients, runtime state, artifacts, SLO gates, or evidence I/O.
6. `firkin-substrate` remains the temporary evidence owner before the split. It owns:
   - `BenchmarkSummary`
   - required lifecycle/overhead metric lists
   - SLO target models
   - lifecycle/overhead evidence reports
   - evidence artifact read/write
7. `firkin-substrate` depends on `firkin-trace` for primitive samples and does not re-own those primitives.
8. The top-level `firkin` crate may re-export trace primitives as the curated public facade.
9. `firkin-cli` exposes benchmark commands at top level:
   - `fk benchmark targets`
   - `fk benchmark report lifecycle <artifact>`
   - `fk benchmark report overhead <artifact>`
   - `fk benchmark validate-lifecycle-slo <artifact>`
   - `fk benchmark validate-overhead-slo <artifact>`
10. Justfile live benchmark gates call the top-level `benchmark` command group.

## Summary Statistics

Every `BenchmarkSummary` stores and exposes:

```text
count
p50
p90
p95
p99
max
```

Percentiles use nearest-rank over sorted `f64` values. `max` is the last sorted value. SLO gates remain p95-based for the current required lifecycle and overhead targets; adding p90/p99/max does not change pass/fail semantics.

CLI report lines include every stored statistic:

```text
metric=<name> kind=<kind> unit=<unit> count=<n> p50=<v> p90=<v> p95=<v> p99=<v> max=<v>
```

## Pre-Split Metric Surface

The current required lifecycle evidence set remains:

```text
cold_template_build
warm_snapshot_restore
command_start
first_stdout_byte
ready_probe
snapshot_save
kill_delete
warm_pool_checkout
concurrent_create
```

The current required overhead evidence set remains:

```text
control_plane_cpu_idle
control_plane_rss_idle
per_sandbox_host_rss
disk_metadata_growth
idle_wakeup_rate
```

The P0 scorecard names are legal and reserved for trace samples, but they are not all required before the split:

```text
sandbox.task.agent_task_ready_ms
sandbox.start.total_ms
sandbox.start.image_resolve_ms
sandbox.start.rootfs_prepare_ms
sandbox.start.disk_attach_ms
sandbox.start.vm_boot_ms
sandbox.start.guest_agent_ms
sandbox.start.network_ready_ms
sandbox.start.mounts_ready_ms
sandbox.start.workspace_ready_ms
sandbox.start.first_exec_ms
sandbox.exec.first_stdout_ms
sandbox.mem.host_footprint_bytes
sandbox.mem.retention_ratio
sandbox.disk.host_allocated_bytes
sandbox.disk.trim_ms
sandbox.disk.trim_reclaimed_bytes
sandbox.net.dns_ms
sandbox.cleanup.total_ms
sandbox.cleanup.leaked_bytes
sandbox.failure.unknown_count
```

## Explicitly Deferred Until After The Split

These are not pre-split acceptance requirements:

- threading `Recorder` through every `Pod`, `ContainerRuntime`, `VirtualMachine`, OCI, vminitd, and single-node seam
- derived end-to-end `agent_task_ready_ms`
- guest metrics RPC implementation
- PSI/cgroup/diskstats/procfs sampling
- disk personality matrix
- network DNS/connect/RTT/policy counters
- memory reclaim/balloon/retention measurements
- density/concurrency dashboards
- pod amortization metrics
- cache metrics
- security/isolation score metrics
- power/thermal sampling
- workload-realism benchmark suites

Those families must use `firkin-trace` when they land.

## Success Criteria

1. `cargo test -p firkin-trace` passes.
2. `cargo test -p firkin-substrate benchmarks` passes.
3. `cargo test -p firkin-cli benchmark` passes.
4. `cargo check -p firkin-cli` passes.
5. `cargo fmt --check` passes.
6. `git diff --check` passes.
7. `fk benchmark targets` prints lifecycle and overhead targets from the current evidence surface.
8. `fk benchmark report` prints p50, p90, p95, p99, and max.

## Implementation Plan

1. Keep `firkin-trace` as a leaf package with no workspace crate dependencies.
2. Keep primitive sample construction imports pointed at `firkin_trace`.
3. Extend `BenchmarkSummary` with `p90`, `p99`, and `max`.
4. Add accessors for the new summary fields.
5. Update CLI benchmark report output to print all summary fields.
6. Update substrate tests to assert p50/p90/p95/p99/max nearest-rank behavior.
7. Update CLI tests to assert p90/p99/max report output for lifecycle and overhead artifacts.
8. Update trace/split docs so the pre-split versus post-split line is explicit.
