# Firkin Trustworthy Performance Loop Spec

Date: 2026-05-07

Status: proposed implementation spec

Scope: pre-split and post-split Firkin Rust workspace

Decision-grade follow-up: the next implementation pass is specified in
`docs/plans/2026-05-07-firkin-decision-grade-metrics-implementation-plan.md`.
It supersedes the smoke-era P0 names in this document for implementation:
headline optimization metrics must be derived from one raw event trace, carry
explicit lifecycle/workload labels, and satisfy sample-count, fault-injection,
disk-stage, and stability gates before being used for optimization decisions.

## 1. Purpose

Firkin needs benchmarking that is useful enough to drive real optimization, not
just broad enough to look comprehensive. The target operating mode is:

1. Run a short live benchmark loop.
2. Get trustworthy p50, p90, p95, p99, max, and failure data.
3. See the slowest lifecycle phases mapped back to owning crates and code paths.
4. Change one thing.
5. Re-run the same loop and accept or reject the change from evidence.

The north-star metric is:

```text
agent_task_ready_ms =
  request_received_to_first_stdout_byte_ms
```

That number must be decomposed into phases that identify what to optimize:

```text
image_resolve_ms
image_pull_ms
image_unpack_ms
rootfs_prepare_ms
disk_attach_ms
vm_config_build_ms
vz_validate_ms
vm_start_call_ms
vm_boot_ms
guest_init_ms
guest_agent_handshake_ms
network_ready_ms
dns_ready_ms
workspace_ready_ms
cgroups_ready_ms
first_exec_ms
first_stdout_ms
cleanup_ms
```

The goal is not to make every possible metric exact immediately. The goal is to
ensure every number that appears in an optimization report has an explicit trust
level, a measurement source, an overhead budget, and enough samples to support
the percentile being shown.

## 2. Non-Goals

This spec does not replace `firkin-trace`. The trace crate remains the low-level
span, checkpoint, sample, and drain substrate.

This spec does not make OpenTelemetry or `tracing::Layer` the internal lifecycle
bus. Those tools can be useful export adapters, but the optimization loop needs a
typed, low-cardinality, allocation-controlled data model with lifecycle-specific
trust metadata. Generic telemetry should be downstream of the Firkin evidence
artifact, not the source of truth.

This spec does not promise to land the full disk, memory, network, density,
security, and abuse matrix in one patch. It defines the shape required for those
metrics to become first-class without weakening trust in the initial P0 set.

## 3. Topology

Keep the crate graph acyclic and preserve the existing split direction:

```text
cli/facade/product servers
  -> firkin-benchmark / firkin-single-node
  -> firkin-runtime
  -> firkin-core
  -> firkin-vmm / firkin-vminitd-client / firkin-oci / firkin-ext4 / firkin-vsock
  -> firkin-trace / firkin-types
```

Responsibilities:

| Crate | Responsibility |
| --- | --- |
| `firkin-trace` | Typed low-overhead spans, checkpoints, samples, sampler snapshots, recorder drain. No benchmark policy and no VM running. |
| `firkin-evidence` | Evidence artifact schema, trust labels, scorecard validation, SLO and coverage gates, summary statistics, compare math. No VM running. |
| `firkin-benchmark` | Benchmark plans, live runners, host/guest harness orchestration, baseline storage, comparison reports, optimization loop commands. |
| `firkin-single-node` | Apple/VZ live sandbox execution path used by signed-live runners. |
| `firkin-runtime` | Lifecycle orchestration spans for sandbox/task operations. |
| `firkin-core` | Container/pod/task domain spans that are not Apple/VZ-specific. |
| `firkin-vmm` | VM creation, VZ validation, start/stop, memory/balloon host-side checkpoints. |
| `firkin-vminitd-client` | Vsock connect, guest-agent handshake, exec, stdout/stderr, shutdown/control-plane checkpoints. |
| `firkin-oci` | Image resolution, layer fetch/unpack, rootfs materialization checkpoints. |

Do not introduce a new crate for the optimization loop yet. Add modules inside
`firkin-benchmark` first:

```text
crates/benchmark/src/
  baseline.rs
  compare.rs
  doctor.rs
  loop_plan.rs
  live.rs
  p0.rs
  report.rs
  trust.rs
```

Only split out new crates when there are multiple independent consumers:

| Future crate | Split trigger |
| --- | --- |
| `firkin-host-metrics` | Host process, footprint, APFS allocation, power, and thermal collectors are used by benchmark, runtime, and cleanup tooling. |
| `firkin-guest-metrics` | `/proc`, PSI, cgroup, diskstats, network, and pid collectors become shared between vminitd, benchmark runners, and product telemetry. |
| `firkin-profiler` | Flamegraph/profile capture becomes a product feature rather than a benchmark-only helper. |

## 4. Trust Model

Every metric in an artifact must have a trust classification:

| Trust level | Meaning | May satisfy P0? |
| --- | --- | --- |
| `signed_live_exact` | Measured on the live Apple/VZ path with signed entitlements and exact user-visible endpoints. | Yes |
| `host_exact` | Host source of truth for host-only facts, such as host file allocated bytes. | Yes, for host metrics |
| `guest_exact` | Guest source of truth for guest-only facts, such as `/proc/meminfo` or cgroup counters. | Yes, for guest metrics |
| `paired_host_guest` | Guest measurement paired with host monotonic receive timestamps and clock metadata. | Yes, if skew rules are satisfied |
| `calibrated_proxy` | Proxy metric with a documented correlation study and accepted error bound. | Only where the scorecard explicitly allows proxy |
| `schema_only` | Type/schema/smoke coverage only. | No |
| `untrusted` | Source, endpoint, sample count, or clock semantics are insufficient. | No |

Optimization reports must hide or clearly quarantine `schema_only` and
`untrusted` metrics. They can appear in coverage reports as missing work, but
they must not appear as actionable performance signals.

## 5. Evidence Artifact Contract

Each benchmark run produces one durable evidence artifact with these sections:

```text
schema_version
run_id
runner_version
command
git_or_jj_revision
build_profile
feature_flags
started_at_wall_time
host_fingerprint
guest_fingerprint
config_fingerprint
cache_state
suite
mode
sample_requirements
raw_samples
span_timeline
sampler_snapshots
phase_summaries
scorecard
trust_report
overhead_report
failure_report
cleanup_report
diagnostics
```

Host fingerprint:

```text
machine_model
chip
memory_bytes
macos_version
kernel_version
power_source
thermal_state
host_pressure_state
cpu_count
```

Guest fingerprint:

```text
guest_image
image_digest
rootfs_digest
kernel_version
architecture
vminitd_version
guest_agent_version
cgroup_version
psi_available
```

Config fingerprint:

```text
cpus
memory_max_bytes
storage_backend
cache_mode
sync_mode
filesystem
image_type
network_profile
sandbox_profile
isolation_mode
pod_container_count
workspace_mode
runtime_storage_root
runtime_cache_root
```

Run mode:

```text
host_only
signed_live_apple_vz
guest_agent_live
external_tool
manual_calibration
```

The artifact must include raw samples, not only summaries. Summary data alone is
not enough for later comparison, outlier inspection, or p99 confidence checks.

## 6. Percentile Honesty

Percentiles must be sample-count aware.

| Statistic | Minimum samples for optimization report | Below minimum behavior |
| --- | ---: | --- |
| p50 | 5 | Show as smoke only |
| p90 | 20 | Show as smoke only |
| p95 | 40 | Show as smoke only |
| p99 | 100 | Show as smoke only |
| max | 1 | Always show, but never infer percentile behavior |

Short 30s and 60s loops may not have enough samples for p99 on slow suites. In
that case they should say so. A small-n p99 is worse than useless because it
invites optimization against noise.

Every summary should include:

```text
count
min
p50
p90
p95
p99
max
mean
median_absolute_deviation
coefficient_of_variation
confidence_label
```

Confidence labels:

```text
exact_enough
low_sample_count
high_variance
environment_unstable
overhead_too_high
collector_overflow
clock_untrusted
```

## 7. Clock Semantics

Use host monotonic time for lifecycle durations when the host observes both
endpoints. Examples:

```text
request_received -> first_stdout_byte
vm_start_call -> vminitd_vsock_connected
exec_request_sent -> first_stdout_byte_received
cleanup_begin -> cleanup_end
```

Guest checkpoints must never be subtracted directly from host wall time. Guest
events are represented as:

```text
guest_event_name
guest_monotonic_ns
guest_wall_time_ns
host_receive_monotonic_ns
host_send_monotonic_ns, when request/response based
clock_source
```

Allowed derivations:

1. Host-observed durations.
2. Guest-only durations from the same guest monotonic clock.
3. Paired host/guest intervals with explicit skew/error bounds.

Disallowed derivations:

1. Host wall time minus guest wall time.
2. Cross-VM guest monotonic comparisons.
3. Phase summaries that mix host and guest clocks without trust metadata.

## 8. Span and Phase Model

Every lifecycle span must include:

```text
span_id
parent_span_id
operation_id
sandbox_id
task_id
phase
component
owner_crate
trust_domain
start_monotonic_ns
end_monotonic_ns
outcome
failure_class
```

Use a small fixed phase vocabulary:

```text
request_received
config_build
image_resolve
image_pull
image_unpack
rootfs_prepare
disk_create
disk_attach
vm_config_build
vz_validate
vm_create
vm_start_call
kernel_boot
guest_init
vsock_connect
agent_handshake
network_create
network_attach
ip_assign
dns_ready
workspace_create
workspace_mount
cgroup_apply
first_exec_start
first_stdout_byte
first_exec_exit
task_run
shutdown_graceful
shutdown_forced
disk_detach
fstrim
cleanup_delete
cleanup_verify
```

Owner crates should be enumerable rather than free-form strings:

```text
firkin-admission
firkin-artifacts
firkin-benchmark
firkin-core
firkin-e2b-contract
firkin-e2b-server
firkin-evidence
firkin-ext4
firkin-facade
firkin-hygiene
firkin-oci
firkin-runtime
firkin-single-node
firkin-template
firkin-trace
firkin-vminitd-client
firkin-vmm
firkin-vsock
guest
external
```

This owner map is what makes benchmark output actionable. A report should be
able to say:

```text
Largest p95 contributor:
  phase: rootfs_prepare
  owner: firkin-template/firkin-oci
  p95: 412ms
  regression: +38ms
  trust: signed_live_exact
```

## 9. Recorder and Overhead Requirements

Default lifecycle tracing must be cheap enough to leave enabled for all live
benchmark runs.

Budget for a 30-phase default lifecycle:

```text
wall overhead: <500us
allocations: <50
additional retained memory: <16KB
```

The optimization loop must measure this budget, not assume it.

Required overhead gates:

1. Host-only microbenchmark for recorder span/checkpoint cost.
2. Signed-live A/B run with tracing disabled/enabled.
3. Sampler-on/sampler-off comparison for each sampler class.
4. Artifact serialization cost measured outside the lifecycle critical path.

If overhead exceeds budget, the affected benchmark report must say
`overhead_too_high` and block optimization conclusions for close deltas.

Default trace profile:

```text
spans: enabled
checkpoints: enabled
samplers: disabled unless the suite requires them
per-sample string tags: disabled
shared envelope tags: enabled
raw sample cap: enabled
overflow marker: enabled
```

## 10. Cardinality and Backpressure

High-cardinality data belongs in artifact envelopes and raw event fields, not in
grouping tags.

Allowed grouping tags:

```text
machine_model
chip
macos_version
guest_image_family
storage_backend
cache_mode
sync_mode
network_profile
sandbox_profile
isolation_mode
pod_container_count
suite
mode
warmth
```

Disallowed grouping tags:

```text
sandbox_id
task_id
container_id
absolute_path
full_image_digest
full_command_line
random_tmp_name
stdout_text
stderr_text
```

The recorder must have explicit caps:

```text
max_samples_per_operation
max_sampler_snapshots_per_operation
max_string_bytes_per_operation
max_tag_key_bytes
max_tag_value_bytes
max_failure_detail_bytes
```

Overflow behavior:

1. Lifecycle spans and outcome events are protected.
2. Sampler snapshots are dropped first.
3. Repeated equivalent counter samples are coalesced.
4. Overflow is recorded as a sample.
5. Any metric depending on dropped samples becomes `collector_overflow`.

## 11. P0 Scorecard Metrics

The first optimization dashboard should expose these metrics only when their
trust requirements are met:

| Metric | Required source | Required trust |
| --- | --- | --- |
| `agent_task_ready_ms` | Live runtime benchmark evidence from host sandbox create request through first useful stdout | `signed_live_exact` |
| `sandbox.start.warm_ready_ms` | Live runtime benchmark evidence for warm local image/rootfs create through first useful stdout | `signed_live_exact` |
| `sandbox.start.cold_ready_ms` | Live runtime benchmark evidence for cold local rootfs/snapshot materialization through first useful stdout | `signed_live_exact` |
| `sandbox.start.hot_pool_checkout_ms` | Live runtime benchmark evidence for prewarmed sandbox checkout through first useful stdout | `signed_live_exact` |
| `sandbox.exec.first_latency_ms` | Live runtime benchmark evidence for command request to process start | `signed_live_exact` |
| `sandbox.exec.first_stdout_ms` | Live runtime benchmark evidence for command request to first stdout byte | `signed_live_exact` |
| `sandbox.disk.metadata_create_stat_unlink_ms` | Signed-live guest harness create/stat/unlink bundle inside the VM workspace | `signed_live_exact` |
| `sandbox.disk.fsync_p99_us` | Signed-live guest harness fsync p99 inside the VM workspace | `signed_live_exact` |
| `sandbox.mem.idle_host_footprint_bytes` | Signed-live overhead artifact from exclusive VM task-set attribution | `signed_live_exact` when artifact sample is present |
| `sandbox.mem.post_task_residual_bytes` | Signed-live overhead artifact after task cleanup from exclusive VM task-set attribution | `signed_live_exact` when artifact sample is present |
| `sandbox.mem.reclaim_effectiveness_ratio` | Signed-live overhead artifact pairing VM task-set attribution with guest reclaim evidence | `signed_live_exact` when artifact sample is present |
| `sandbox.pressure.io_full_avg10` | Signed-live lifecycle artifact containing guest `/proc/pressure/io` evidence | `signed_live_exact` when artifact sample is present |
| `sandbox.disk.sparse_bloat_ratio` | Signed-live product-pod harness pairing host allocated bytes with guest used bytes | `signed_live_exact` |
| `sandbox.disk.trim_reclaim_bytes_per_sec` | Signed-live product-pod harness timing vminitd fstrim with host allocated-byte delta | `signed_live_exact` |
| `sandbox.density.max_active_before_p95_doubles` | Signed-live lifecycle concurrency sweep against exact ready metrics | `signed_live_exact` |
| `sandbox.reliability.boot_failure_rate` | Signed-live classified runtime attempts for snapshot/create failures | `signed_live_exact` |
| `sandbox.reliability.unknown_failure_rate` | Signed-live classified runtime attempts for post-create command/delete failures | `signed_live_exact` |
| `sandbox.cleanup.leftover_bytes` | Signed-live run-scoped cleanup scan after sandbox teardown | `signed_live_exact` |

This is a hard-cut contract with 18 P0 rows. `sandbox.cleanup.orphan_count`
remains a Core cleanup metric until a real artifact source and strict coverage
promotion path exist for it.

Metrics outside this table can exist in the catalog, but the optimization loop
should not treat them as P0 blockers until they have the same level of source and
trust discipline.

## 12. Live Harness Requirements

The signed-live Apple/VZ harness is the canonical path for startup, readiness,
exec, cleanup, and density metrics.

Required live scenarios:

```text
agent_core_smoke
agent_core_30s
agent_core_60s
startup_cold_local
startup_warm
startup_hot_pool
first_exec_true
first_exec_shell
exec_100_small_commands
cleanup_to_zero
density_1_2_4_8
```

Required subsystem scenarios:

```text
disk_metadata_create_stat_unlink
disk_fsync_p99
disk_sparse_bloat_and_trim
memory_idle_footprint
memory_allocate_free_reclaim
network_dns_first_use
network_guest_to_host_tcp
vsock_ping_and_exec_rpc
pod_marginal_container_start
```

`benchmark doctor --mode signed-live` checks exactly these prerequisites today:

```text
state_root writable
cache_root writable
benchmark_root writable
benchmark_root free bytes at or above --min-free-bytes
Apple Virtualization host preflight succeeds on arm64
current executable signing and Virtualization entitlement are reported
scripts/run-signed-live-runtime-test.sh exists
signing/vz.entitlements exists
guest PSI source config and rebuilt bin/vmlinux are ready for /proc/pressure/io
embedded vminitd bytes are available
```

If a preflight fails, the benchmark command must fail before collecting partial
numbers or writing benchmark artifacts. The doctor prints the executable signing
state; it does not currently fail solely because the signing fields are false.

## 13. Storage and Cleanup

Firkin runtime state and cache roots must be explicit and configurable. Defaults:

```text
state root: ~/.firkin/state
cache root: ~/.firkin/cache
benchmark artifacts: ~/.firkin/benchmarks
temporary staging: ~/.firkin/state/tmp
```

Library consumers must be able to override these paths through configuration.
The CLI should expose them through config and flags.

The benchmark harness must record:

```text
runtime_state_root
runtime_cache_root
tmpdir
bytes_before
bytes_after_task
bytes_after_cleanup
bytes_after_fstrim
bytes_after_delete
leftover_paths
```

Cleanup is not best-effort for benchmark runs. A benchmark run either verifies
cleanup-to-zero for Firkin-owned artifacts or emits a failing cleanup report.

The CLI needs:

```text
fk clear --state
fk clear --cache
fk clear --benchmarks
fk clear --all
fk clear --dry-run
fk clear --older-than <duration>
```

`clear` must only delete Firkin-owned roots or explicitly registered Firkin-owned
artifacts. It must not scan and delete arbitrary user home content.

## 14. CLI Surface

First-class benchmark commands:

```text
fk benchmark doctor
fk benchmark run agent-core --mode signed-live --duration 30s --out current.json
fk benchmark run agent-core --mode signed-live --duration 60s --out current.json
fk benchmark run p0 --mode signed-live --out p0.json
fk benchmark run overnight --mode signed-live --out overnight.json
fk benchmark compare baseline.json current.json
fk benchmark coverage --strict --artifact current.json --artifact overhead.json --artifact scorecard.json
fk benchmark report current.json
fk benchmark baseline save current.json --name local-m4-max-agent-core
fk benchmark baseline list
```

`coverage --strict` is artifact-aware across lifecycle, overhead, and
scorecard artifacts. It may count a source metric from any supplied artifact,
but it only passes when the catalog marks that P0 metric exact and the artifact
contains real samples. Proxy, schema-only, and missing metrics remain blocked.

`sprint-ready` must pass the current lifecycle artifact, the overhead artifact,
and any scorecard artifact into the same strict coverage gate. Its proof status
is `passed` only for full exact P0 coverage; partial, blocked, and smoke-only
states must remain visible in output and proof pages.

Later, when the evidence quality is high enough:

```text
fk benchmark loop --suite agent-core --baseline baseline.json --budget 2h
```

The compare report should lead with:

```text
top bottlenecks
top regressions
top improvements
trust failures
sample-size failures
environment instability
cleanup leaks
recommended next measurement
```

Do not print a wall of every metric by default. The default report exists to
make the next optimization move obvious.

## 15. Optimization Loop Tiers

### 15.1 30-Second Hot Loop

Use while editing code in a focused area.

Metrics:

```text
agent_task_ready_ms
warm_ready_ms
hot_checkout_ms
first_stdout_ms
first_exec_latency_ms
cleanup_leftover_bytes
trace_overhead_us
```

Expected behavior:

1. Fast enough to run after every focused code change.
2. Honest enough for p50 and p90 on fast operations.
3. Allows p95/p99 only when sample counts meet thresholds.
4. Reports largest phase contributor and owner crate.

### 15.2 60-Second Control Loop

Use before keeping or reverting a performance change.

Metrics:

```text
all 30s metrics
phase p95 where sample counts permit
failure classification
host footprint before/after
small concurrency smoke
network/DNS first-use smoke
```

Expected behavior:

1. Confirms the change is not just noise.
2. Detects obvious regressions in adjacent lifecycle phases.
3. Records environment stability.

### 15.3 5-10 Minute Subsystem Loop

Use when working on disk, memory, network, pool, or cleanup internals.

Metrics:

```text
disk metadata p50/p90/p95/p99
fsync/fdatasync p50/p90/p95/p99
sparse bloat and trim reclaim
idle footprint
post-task residual memory
guest PSI
DNS/connect latency
density 1/2/4/8
```

### 15.4 Overnight Truth Loop

Use before claiming broad performance improvement.

Metrics:

```text
p99 and max for P0 lifecycle metrics
cold/warm/hot matrix
density 1/2/4/8/16/32 where machine capacity allows
memory reclaim honesty
disk reclaim honesty
network first-use and steady-state
cleanup leak slope
failure rate and unknown failure rate
```

This is the loop that makes next-day optimization work real. It should produce a
baseline that short loops can compare against.

## 16. Metric-Specific Harness Notes

### 16.1 Agent Task Ready

Endpoint:

```text
host API call accepted -> first stdout byte observed by host
```

The readiness definition is:

```text
guest agent connected
workspace mounted/materialized
network policy applied
cgroups/resource limits applied
first exec succeeds
first stdout byte observed
```

Do not report VM-start-only latency as readiness.

### 16.2 Disk Metadata

Use a guest workload that creates, stats, reads, renames, and unlinks many small
files on the same workspace volume that agent tasks use.

Minimum first runner:

```text
create_10k_small_files_ms
stat_10k_files_ms
read_10k_small_files_ms
rename_10k_files_ms
unlink_10k_files_ms
```

Optional external tools such as `fio` are useful, but the first P0 metadata
runner should not depend on external installation in the guest.

### 16.3 Fsync and Durability

Run enough fsync/fdatasync operations to support p99. The result must be tagged
with:

```text
storage_backend
cache_mode
sync_mode
filesystem
image_type
```

If the guest or host cannot prove the storage mode, the metric is untrusted.

### 16.4 Host Footprint

Host footprint must be attributed to the sandbox VM process or VM object, not to
the CLI process. RSS is useful but not enough; record the best available macOS
footprint signal and label fallbacks explicitly.

Acceptable fields:

```text
host_vm_rss_bytes
host_vm_phys_footprint_bytes
host_private_dirty_bytes
host_compressed_bytes_estimate
host_swap_bytes_estimate
```

If only a proxy is available, mark it `calibrated_proxy` and keep it out of P0
until calibrated.

### 16.5 Memory Reclaim

The harness should run:

```text
measure host footprint before
allocate and touch guest memory
free guest memory
compact/drop caches when available
balloon or reclaim if configured
measure host footprint after 1s / 5s / 30s
decide recycle_required
```

Derived:

```text
memory_retention_ratio =
  host_footprint_after_guest_free / host_peak_footprint

memory_reclaim_effectiveness_pct =
  host_bytes_reclaimed / guest_reclaimable_bytes_estimate
```

### 16.6 Disk Reclaim

The harness should run:

```text
measure host allocated bytes before
write/delete data in guest
measure guest df and inode state
run fstrim when available
measure host allocated bytes after fstrim
destroy sandbox
measure host allocated bytes after delete
```

Derived:

```text
sparse_bloat_ratio =
  host_sparse_file_allocated_bytes / guest_df_used_bytes

disk_reclaim_effectiveness_pct =
  host_bytes_reclaimed_after_fstrim / deleted_inside_guest_bytes_estimate
```

### 16.7 Network and DNS

Separate setup from first-use:

```text
network_create_ms
network_attach_ms
ip_assign_ms
dns_ready_ms
first_dns_lookup_ms
first_tcp_connect_ms
http_get_small_ms
```

Control-plane vsock metrics must be separate:

```text
vsock_connect_ms
vsock_rpc_ping_us
exec_rpc_latency_ms
stdout_stream_lag_ms
signal_delivery_ms
```

### 16.8 Cleanup and Leak Rate

Cleanup verification scans only Firkin-owned roots and registered artifacts.

Required counters:

```text
orphan_vm_count
orphan_disk_count
orphan_network_count
orphan_mount_count
orphan_process_count
cleanup_leftover_bytes
cleanup_leftover_paths
```

For repeated loops:

```text
leaked_bytes_per_sandbox
leaked_paths_per_sandbox
cleanup_p50/p90/p95/p99/max
```

## 17. Correlating Numbers to Code

A performance loop is only useful if it names the code path to inspect next.

Every span should carry:

```text
owner_crate
owner_module
operation
phase
```

Where possible, span constructors should be local to the owning crate/module so
the compiler keeps ownership obvious. Avoid a central file that contains every
span name in the product.

The compare report should aggregate by:

```text
phase
owner_crate
owner_module
operation
```

Example output:

```text
Top bottleneck:
  metric: sandbox.task.agent_task_ready_ms
  phase: workspace_mount
  owner: firkin-core::pod
  p95: 184ms
  p99: smoke-only, count=43 < 100
  confidence: exact_enough for p95
```

For CPU-heavy or unknown regressions, the benchmark command should support a
profile mode:

```text
fk benchmark run agent-core --profile cpu
fk benchmark run agent-core --profile allocations
fk benchmark run agent-core --profile syscalls
```

Useful crates and tools:

| Need | Candidate |
| --- | --- |
| Low-overhead internal spans | Existing `firkin-trace` |
| Histograms and quantiles | `hdrhistogram` |
| Fast deterministic hash maps | `indexmap` or `hashbrown`, only where needed |
| Stable JSON artifact format | Existing `serde`/`serde_json` |
| Compact binary sidecar if artifacts get large | `postcard` or `rmp-serde` |
| CPU profiles on macOS | `samply`, Instruments, or `sample` wrapper |
| Allocation profiling | `dhat` in targeted dev mode, not default live loop |
| Host process metrics | macOS `proc_pidinfo`, `task_info`, `fs_usage`/`du` wrappers as needed |
| Guest Linux counters | vminitd/guest helper reading `/proc`, cgroup v2, PSI, diskstats |
| Statistical comparison | Initially internal simple tests; consider `statrs` later |

Do not add a dependency unless it removes real code or improves measurement
trust. The first priority is exact endpoints and artifacts, not clever stats.

## 18. Baseline and Comparison Rules

Baselines are keyed by:

```text
machine_model
chip
memory_bytes
macos_version
guest_image
rootfs_digest
storage_backend
cache_mode
sync_mode
network_profile
suite
mode
```

A comparison is valid only when the incompatible keys match or the report
explicitly marks the comparison as cross-environment.

Regression policy:

```text
large regression:
  p95 or p99 worsens by >= 10% and >= 10ms

small regression:
  p95 or p99 worsens by >= 5% and >= 5ms

large improvement:
  p95 or p99 improves by >= 10% and >= 10ms

noise:
  below threshold, low sample count, or high variance
```

For very small metrics, use absolute thresholds:

```text
first_exec_ms: 2ms threshold
vsock_rpc_ping_us: 100us threshold
trace_overhead_us: 100us threshold
```

Never claim a win when trust labels or sample counts do not support it.

## 19. Two-Hour Performance Sprint Readiness

Before starting a focused optimization sprint, we need:

1. A fresh overnight or representative P0 baseline.
2. A 30s hot loop that runs cleanly on the same machine.
3. A 60s control loop that catches adjacent regressions.
4. `benchmark compare` ranking bottlenecks by phase and owner.
5. Cleanup-to-zero verification passing.
6. Trace overhead passing.
7. A disk-space guard that fails before filling the machine.
8. A clear baseline save/restore path.

During the sprint:

1. Pick the largest trusted bottleneck.
2. Inspect the owner crate/module named by the report.
3. Make one focused change.
4. Run the 30s loop.
5. If promising, run the 60s loop.
6. Keep the change only if the trusted metric improves and P0 guardrails do not
   regress.
7. Every 30 minutes, run the representative suite.
8. End with the relevant subsystem suite plus workspace checks.

The loop should tell the agent what to do next within 30-60 seconds. If it only
produces a JSON blob and leaves diagnosis manual, it is not finished.

## 20. Implementation Order

1. Add the trust model and artifact schema in `firkin-evidence`.
2. Add `fk benchmark doctor` preflight.
3. Make signed-live `agent_task_ready_ms` exact from request to first stdout.
4. Add raw sample preservation and percentile honesty gates.
5. Add benchmark baseline save/list/compare.
6. Add phase/owner aggregation in compare reports.
7. Add trace overhead A/B gates.
8. Add cleanup-to-zero verification and `fk clear`.
9. Add disk metadata and fsync runners.
10. Add host footprint and memory residual runners.
11. Add disk reclaim runner.
12. Add network/DNS first-use runner.
13. Add density 1/2/4/8 runner.
14. Add overnight truth loop.
15. Only then automate broad refactors against this schema.

## 21. Refactor Automation Boundary

Programmatic refactor tooling should be used after this spec is accepted, not to
decide the spec.

Safe automation targets:

```text
add Recorder fields to lifecycle structs
add owner/phase metadata to span constructors
move metric schema types to the selected crate
replace ad-hoc Instant::now samples with Recorder spans
thread runtime state/cache roots through configs
rename scorecard fields to stable metric names
```

Unsafe automation targets without manual review:

```text
changing crate dependency direction
moving benchmark runner code into lower-level runtime crates
turning proxy metrics into exact metrics
adding high-cardinality tags
changing cleanup roots or deletion behavior
changing clock math
```

Automation must be followed by:

```text
cargo fmt --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
scripts/check-firkin-crate-graph.sh
git diff --check
```

## 22. Acceptance Criteria

This work is complete when:

1. `fk benchmark doctor` catches missing signing, missing VZ support, missing
   images, bad storage roots, and low disk before live runs.
2. `fk benchmark run agent-core --mode signed-live --duration 30s` produces an
   evidence artifact with raw samples, phase spans, trust labels, and overhead
   data.
3. `fk benchmark run agent-core --mode signed-live --duration 60s` produces a
   compare-ready artifact.
4. `fk benchmark compare baseline.json current.json` identifies bottlenecks,
   regressions, improvements, trust failures, and sample-size failures.
5. P0 metrics do not appear as optimization signals unless their trust
   requirements are satisfied.
6. `agent_task_ready_ms` is measured from host request acceptance to host
   observed first stdout byte.
7. VM-start-only latency is never reported as sandbox readiness.
8. p90, p95, p99, and max are all present when supported by sample counts.
9. Small-n percentiles are labeled smoke-only.
10. Cleanup-to-zero is verified for Firkin-owned state and cache roots.
11. Trace overhead is measured and blocks close-call conclusions when too high.
12. The report points to owning phases and crates.
13. The benchmark state/cache roots default to `~/.firkin/state` and
    `~/.firkin/cache`, with library configuration overrides.
14. `fk clear` can safely dry-run and clear Firkin-owned state/cache/benchmark
    artifacts.

## 23. The Bar

The bar is not "the code compiles" and not "the metric exists in a catalog."

The bar is:

```text
The benchmark loop can run on the live Apple/VZ path, produce trusted numbers,
rank the next bottleneck, survive repeated runs without leaking state, and tell a
performance engineer or agent which crate and phase deserve the next 30 minutes.
```

Anything short of that is useful scaffolding, but it is not the trustworthy
performance loop.
