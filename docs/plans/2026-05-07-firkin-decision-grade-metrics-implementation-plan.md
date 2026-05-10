# Firkin Decision-Grade Metrics Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace smoke-test benchmark numbers with decision-grade Firkin metrics that have one meaning, one trace source, lifecycle/workload labels, sample-confidence gates, and fault-injection proof.

**Architecture:** `firkin-trace` owns the raw timestamped event trace and sample primitives. `firkin-evidence` owns the metric contract, event-to-metric derivation, confidence/variance policy, and coverage gates. `firkin-benchmark` owns suite execution, disk/density/readiness validation harnesses, and batch orchestration. `firkin-cli` exposes operator commands and proof artifacts. Runtime, single-node, vminitd-client, template, VMM, and benchmark live tests emit events at the phase they own; they do not define dashboard policy.

**Tech Stack:** Rust workspace crates `firkin-trace`, `firkin-evidence`, `firkin-benchmark`, `firkin-runtime`, `firkin-single-node`, `firkin-vminitd-client`, `firkin-cli`; signed-live Apple/VZ harness through `scripts/run-signed-live-runtime-test.sh`; JSON evidence artifacts under `target/firkin-live-evidence/`; markdown proof under `docs/artifacts/`.

---

## Objective Behavior Validation

The work is done when these commands prove the measurement layer is truthful enough to drive optimization:

```bash
cargo run -q -p firkin-cli -- benchmark metric-contract
cargo run -q -p firkin-cli -- benchmark run agent-core --mode signed-live --runs 100 \
  --out target/firkin-live-evidence/agent-core-decision-100.json
cargo run -q -p firkin-cli -- benchmark report decision \
  target/firkin-live-evidence/agent-core-decision-100.json
cargo run -q -p firkin-cli -- benchmark validate-metrics \
  --mode signed-live \
  --out target/firkin-live-evidence/metric-validation.json
cargo run -q -p firkin-cli -- benchmark stability \
  --suite agent-core \
  --mode signed-live \
  --runs-per-batch 100 \
  --batches 3 \
  --out target/firkin-live-evidence/agent-core-stability.json
```

Expected pass/fail signals:

- `metric-contract` prints every headline metric with exact start event, end event, included phases, excluded phases, lifecycle, workload, profile, owner, and percentile sample floors.
- `agent-core --runs 100` writes an artifact containing raw event traces for every run plus derived headline samples. No headline metric is manually timed outside the event trace.
- `report decision` prints p95 only as decision-grade when the metric has at least 100 samples. It omits or marks p99 experimental until at least 500 samples.
- `validate-metrics` injects delays/failures and proves the affected metric bucket moves or fails in the intended phase.
- `stability` reports batch variance, noise floor, and minimum detectable delta across three 100-run batches.

Residual risks:

- Signed-live Apple/VZ availability, macOS power/thermal state, and local disk pressure can still block live closure.
- Cold path and disk/density suites can use 30-run decision gates initially; fast-path p95 requires 100 runs.
- p99 remains experimental until 500-run artifacts exist.

## Current Ground Truth

Current code is useful but still smoke-grade for optimization:

- `crates/evidence/src/lifecycle.rs` requires old lifecycle names: `command_start`, `first_stdout_byte`, `ready_probe`, `warm_pool_checkout`, `sandbox.start.hot_pool_checkout_ms`, `concurrent_create`.
- `crates/evidence/src/catalog.rs` promotes `sandbox.start.hot_pool_checkout_ms`, `sandbox.disk.sparse_bloat_ratio`, and `sandbox.density.max_active_before_p95_doubles` as P0 names even though their current wording does not fully specify endpoints or workload.
- `crates/runtime/tests/live_snapshot_restore.rs` currently sets `sandbox.start.hot_pool_checkout_ms` equal to `agent_task_ready_ms`, so the name says checkout but the value includes SDK create and first stdout.
- `crates/runtime/src/session.rs` emits `command_start` and `first_stdout_byte` directly from a local `Instant`, independent of a canonical operation trace.
- `crates/runtime/src/adapter.rs` emits `ready_probe` around `session.probe_ready()`, but the report does not split local flag, guest agent ping, workspace probe, exec probe, and DNS probe.
- `crates/benchmark/src/disk.rs` has one sparse bloat ratio and trim throughput, not before-task/after-delete/after-trim/after-destroy stage semantics.
- `crates/benchmark/src/density.rs` computes a generic `max_active_before_p95_doubles` with no workload encoded in the metric name or required tag.
- `crates/evidence/src/benchmark.rs` stores p50/p90/p95/p99/max only. It does not store min, mean, MAD, coefficient of variation, or percentile availability.
- `crates/trace/src/lib.rs` has `Recorder`, `Span`, `Checkpoint`, `RecordedTrace`, and `BenchmarkSample`, but `RecordedTrace` currently serializes samples rather than a first-class raw event timeline.

This plan is a hard cutover. Do not add compatibility aliases for the old names.

## Canonical Headline Metrics

Implement this focused dashboard first:

| Metric | Meaning |
| --- | --- |
| `start.hot_to_first_stdout_ms` | Hot pooled sandbox lease acquired to first stdout byte from the first probe command. |
| `start.hot_to_ready_ms` | Hot pooled sandbox lease acquired to real readiness passing. |
| `start.resume_to_first_stdout_ms` | Snapshot restore start to first stdout byte from the first post-restore command. |
| `start.warm_to_first_stdout_ms` | Warm but not leased/prepped sandbox start to first stdout byte. |
| `start.agent_task_ready_ms` | External API request accepted to first useful stdout. |
| `pool.lease_ms` | Pool lease acquisition only. No readiness, workspace, exec, or stdout. |
| `exec.command_start_ms` | Exec request sent to process started. |
| `exec.first_stdout_byte_ms` | Exec request sent to first stdout byte. |
| `exec.batch_100_small_commands_ms` | Batch workload wall time for 100 tiny commands in one ready sandbox. |
| `density.max_active_before_hot_to_first_stdout_p95_doubles` | Largest active concurrency before `start.hot_to_first_stdout_ms` p95 exceeds 2x single-sandbox p95. |
| `disk.sparse_bloat_after_trim` | Host allocated bytes / guest used bytes after delete inside guest and fstrim. |
| `disk.host_bytes_reclaimed_after_trim` | Host allocated-byte delta caused by fstrim. |
| `cleanup.leftover_bytes` | Run-scoped Firkin-owned bytes left after sandbox or pod destroy. |
| `reliability.unknown_failure_rate` | Classified attempts that cannot be assigned to a specific failure class. |

Support these required drilldown metrics in the same trace/artifact, but keep them out of the focused dashboard until the headline set is stable:

```text
template.resolve_image_ms
template.pull_ms
template.unpack_ms
template.rootfs_bake_ms
template.snapshot_create_ms
ready.local_flag_check_ms
ready.guest_agent_ping_ms
ready.workspace_probe_ms
ready.exec_probe_ms
ready.network_dns_probe_ms
disk.sparse_bloat_after_task
disk.sparse_bloat_after_delete
disk.sparse_bloat_after_destroy
disk.trim_effectiveness_pct
reliability.boot_failure_rate
reliability.agent_failure_rate
reliability.dns_failure_rate
reliability.oom_kill_rate
```

## Canonical Event Trace

Add these event names as a closed set in `firkin-trace`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub enum SandboxEventName {
    RequestStart,
    TemplateLookupDone,
    PoolLeaseRequested,
    PoolLeaseAcquired,
    SnapshotRestoreStart,
    SnapshotRestoreDone,
    VzStartCalled,
    GuestAgentConnected,
    LocalReadyFlagChecked,
    GuestAgentPingPassed,
    NetworkReady,
    DnsReady,
    WorkspaceReady,
    CgroupsReady,
    ReadyProbePassed,
    ExecRequestSent,
    ProcessStarted,
    FirstStdoutByte,
    ProcessExited,
    CleanupStart,
    FstrimStart,
    FstrimDone,
    CleanupDone,
}
```

Every event carries:

```rust
pub struct SandboxTraceEvent {
    pub name: SandboxEventName,
    pub host_monotonic_ns: u128,
    pub lifecycle: LifecycleClass,
    pub workload: WorkloadClass,
    pub profile: RuntimeProfile,
    pub outcome: TraceOutcome,
    pub failure_class: Option<FailureClass>,
}
```

Closed labels:

```rust
pub enum LifecycleClass {
    ColdUnprepared,
    ColdPrepared,
    Warm,
    Hot,
    Resumed,
}

pub enum WorkloadClass {
    TinyExec,
    ShellExec,
    Batch100Execs,
    WorkspaceImportSmall,
    RepoGitStatus,
    CargoBuildSmall,
    NpmInstallSmall,
    DiskBloatReclaim,
    ConcurrentCreate,
    ReadinessProbe,
}
```

Rules:

- Host-observed durations subtract host monotonic event timestamps only.
- Guest-only timings stay guest-only unless paired with host receive/send metadata.
- Headline metrics are derived from events by `firkin-evidence`, not manually emitted as independent timers.
- Every derived metric records the event pair used.
- Every run persists raw events before summaries.

## Task 1: Metric Contract and Docs Table

**Files:**

- Create: `docs/specs/firkin-decision-grade-metric-contract.md`
- Create: `crates/evidence/src/metric_contract.rs`
- Modify: `crates/evidence/src/lib.rs`
- Modify: `crates/evidence/src/catalog.rs`
- Modify: `crates/benchmark/src/suite.rs`
- Modify: `crates/cli/src/main.rs`

**Steps:**

1. Write `docs/specs/firkin-decision-grade-metric-contract.md` with one row per canonical headline metric: metric, start event, end event, lifecycle, workload, included phases, excluded phases, owner, minimum sample count for p95, minimum sample count for p99.
2. Add `MetricContract`, `MetricEndpoint`, `PercentilePolicy`, and `DecisionMetricLevel` in `crates/evidence/src/metric_contract.rs`.
3. Move the focused dashboard list into `DECISION_GRADE_METRICS`.
4. Replace the current P0 list in `crates/evidence/src/catalog.rs` and `crates/benchmark/src/suite.rs` with the hard-cut names above.
5. Add `fk benchmark metric-contract` to print the same table from code.
6. Add tests proving the docs table and code table have the same metric names. Use a simple parser over markdown table rows; do not hand-maintain two independent lists silently.

**Verification:**

```bash
cargo test -p firkin-evidence metric_contract
cargo test -p firkin-benchmark suite
cargo test -p firkin-cli metric_contract
cargo run -q -p firkin-cli -- benchmark metric-contract
```

Expected:

- Output contains `metric=start.hot_to_first_stdout_ms start=PoolLeaseAcquired end=FirstStdoutByte lifecycle=hot workload=tiny_exec`.
- No output contains `sandbox.start.hot_pool_checkout_ms`, `command_start`, `first_stdout_byte`, `ready_probe`, `warm_pool_checkout`, or `sandbox.density.max_active_before_p95_doubles`.

## Task 2: Raw Event Trace in `firkin-trace`

**Files:**

- Create: `crates/trace/src/events.rs`
- Modify: `crates/trace/src/lib.rs`
- Modify: `crates/trace/tests/recorder.rs`

**Steps:**

1. Add the closed event, lifecycle, workload, profile, and outcome enums.
2. Add `SandboxTraceEvent`, `SandboxEventTrace`, and `EventTraceRecorder`.
3. Make the event recorder use one host monotonic origin per operation and serialize event offsets as nanoseconds from origin.
4. Add `Recorder::event_trace(...)` or a narrow event recorder constructor without moving benchmark policy into `firkin-trace`.
5. Preserve existing `BenchmarkSample` support, but make the new event trace the input to headline metric derivation.
6. Test ordering, missing endpoint handling, duplicate endpoint classification, overflow policy, and serde roundtrip.

**Verification:**

```bash
cargo test -p firkin-trace event_trace
cargo test -p firkin-trace recorder
```

Expected:

- A synthetic trace with `PoolLeaseAcquired` at 10ms and `FirstStdoutByte` at 83ms serializes raw events and can be read back with exact offsets.
- A trace with duplicate `FirstStdoutByte` keeps the first successful event and marks the duplicate as non-headline debug data.

## Task 3: Derive Metrics From Events Only

**Files:**

- Create: `crates/evidence/src/derive.rs`
- Modify: `crates/evidence/src/benchmark.rs`
- Modify: `crates/evidence/src/scorecard.rs`
- Modify: `crates/evidence/src/lifecycle.rs`
- Modify: `crates/evidence/src/lib.rs`

**Steps:**

1. Add `DerivedMetricSample` with fields: metric name, value, unit, lifecycle, workload, profile, start event, end event, trust label, and confidence label.
2. Implement `derive_metric_samples(trace, contract)` for all headline metrics.
3. Remove old `REQUIRED_LIFECYCLE_LATENCY_METRICS` names and replace them with the metric-contract list.
4. Update `BenchmarkEvidenceReport` to include raw event traces plus derived summaries. Summary-only artifacts are not decision-grade.
5. Add errors for missing start event, missing end event, wrong lifecycle, wrong workload, and mixed clocks.
6. Keep non-headline debug samples possible, but coverage and reports must use derived headline samples.

**Verification:**

```bash
cargo test -p firkin-evidence derive
cargo test -p firkin-evidence lifecycle
cargo test -p firkin-evidence scorecard
```

Expected:

- `start.hot_to_first_stdout_ms` derives from `PoolLeaseAcquired -> FirstStdoutByte`.
- `pool.lease_ms` derives from `PoolLeaseRequested -> PoolLeaseAcquired`.
- Missing `WorkspaceReady` blocks `start.hot_to_ready_ms` rather than producing a zero or local-flag value.

## Task 4: Percentile and Variance Honesty

**Files:**

- Modify: `crates/evidence/src/benchmark.rs`
- Modify: `crates/evidence/src/slo.rs`
- Modify: `crates/evidence/src/scorecard.rs`
- Modify: `crates/cli/src/main.rs`

**Steps:**

1. Extend `BenchmarkSummary` with `min`, `mean`, `median_absolute_deviation`, `coefficient_of_variation`, and `PercentileAvailability`.
2. Use these sample floors:

```text
n < 10: smoke_only
n >= 30: p50_p90_decision_grade
n >= 100: p95_decision_grade
n >= 500: p99_decision_grade
```

3. Make `benchmark report decision` hide or mark p95/p99 when below floor.
4. Make `benchmark compare --rank bottlenecks` rank by p95 only when p95 is decision-grade; otherwise it prints `next_action=collect_more_samples`.
5. Make `sprint-ready` fail if any focused dashboard p95 metric has fewer than 100 samples.
6. Keep max visible for smoke runs, but never use max to infer percentile behavior.

**Verification:**

```bash
cargo test -p firkin-evidence benchmark_summary_confidence
cargo test -p firkin-cli benchmark_report_decision
cargo test -p firkin-cli benchmark_compare_sample_floors
```

Expected:

- A 3-sample artifact prints `confidence=smoke_only unstable_percentile=true`.
- A 100-sample artifact prints `p95_status=decision_grade`.
- A 499-sample artifact prints `p99_status=experimental`.

## Task 5: Runtime Event Emission

**Files:**

- Modify: `crates/runtime/src/adapter.rs`
- Modify: `crates/runtime/src/session.rs`
- Modify: `crates/runtime/src/warm_pool.rs`
- Modify: `crates/runtime/src/restore.rs`
- Modify: `crates/runtime/src/template_build.rs`
- Modify: `crates/vminitd-client/src/process.rs`
- Modify: `crates/runtime/tests/e2b_adapter.rs`
- Modify: `crates/runtime/tests/warm_pool.rs`

**Steps:**

1. Emit `RequestStart` at the external create/followup/create-from-pool boundary.
2. Emit `PoolLeaseRequested` and `PoolLeaseAcquired` around pool checkout only.
3. Emit `SnapshotRestoreStart` and `SnapshotRestoreDone` around snapshot restore.
4. Emit `GuestAgentConnected`, `GuestAgentPingPassed`, `WorkspaceReady`, `CgroupsReady`, and `ReadyProbePassed` from actual probes.
5. Emit `ExecRequestSent`, `ProcessStarted`, `FirstStdoutByte`, and `ProcessExited` from the runtime/vminitd command path.
6. Delete direct manual headline sample creation for old metric names.
7. Add focused unit tests for each event pair at the owning seam.

**Verification:**

```bash
cargo test -p firkin-runtime event_trace
cargo test -p firkin-runtime runtime_adapter
cargo test -p firkin-vminitd-client process
```

Expected:

- Warm-pool checkout tests assert `pool.lease_ms` excludes readiness and exec.
- Command tests assert `exec.command_start_ms` and `exec.first_stdout_byte_ms` derive from one command event trace.
- No test fixture requires old names.

## Task 6: Honest Readiness Probes

**Files:**

- Modify: `crates/runtime/src/adapter.rs`
- Modify: `crates/runtime/src/session.rs`
- Modify: `crates/runtime/tests/e2b_adapter.rs`
- Modify: `crates/runtime/tests/live_snapshot_restore.rs`
- Modify: `crates/benchmark/src/p0_live.rs`

**Steps:**

1. Replace the headline `ready_probe` concept with drilldown metrics:

```text
ready.local_flag_check_ms
ready.guest_agent_ping_ms
ready.workspace_probe_ms
ready.exec_probe_ms
ready.network_dns_probe_ms
```

2. Define headline ready as:

```text
guest agent responds
&& workspace probe passes
&& cgroup/mount state is applied when required by profile
&& exec probe succeeds
&& DNS probe succeeds for networked profiles
```

3. Add explicit failure classes:

```text
local_not_marked_ready
guest_agent_unreachable
workspace_missing
exec_probe_failed
dns_probe_failed
timeout
```

4. Add live or fake-runtime tests that make each readiness sub-probe fail and prove the blocked phase is correct.

**Verification:**

```bash
cargo test -p firkin-runtime readiness_probe
cargo test -p firkin-benchmark readiness_validation
```

Expected:

- Guest agent up but workspace missing fails `workspace_missing`.
- Guest agent up but exec broken fails `exec_probe_failed`.
- DNS blocked fails `dns_probe_failed`, not `unknown`.

## Task 7: Disk Bloat Stages

**Files:**

- Modify: `crates/benchmark/src/disk.rs`
- Modify: `crates/runtime/tests/live_snapshot_restore.rs`
- Modify: `crates/evidence/src/catalog.rs`
- Modify: `crates/benchmark/src/suite.rs`

**Steps:**

1. Add `DiskReclaimStage` and `DiskReclaimMeasurement` types.
2. Record host allocated and guest used bytes at:

```text
before_task
after_task
after_delete_inside_guest
after_fstrim
after_destroy
```

3. Derive:

```text
disk.sparse_bloat_after_task
disk.sparse_bloat_after_delete
disk.sparse_bloat_after_trim
disk.sparse_bloat_after_destroy
disk.host_bytes_reclaimed_after_trim
disk.trim_effectiveness_pct
cleanup.leftover_bytes
```

4. Replace focused dashboard `disk.sparse_bloat_ratio` with `disk.sparse_bloat_after_trim`.
5. Update product-pod live disk harness to perform a deterministic write/delete/fstrim/destroy flow.

**Verification:**

```bash
cargo test -p firkin-benchmark disk_reclaim_stages
scripts/run-signed-live-runtime-test.sh live_runtime_benchmark_evidence_writes_required_lifecycle_artifact
```

Expected:

- The synthetic stage test proves bloat increases after task, remains after guest delete, and decreases after fstrim.
- Live artifact includes every disk stage with the same pod/sandbox run id.

## Task 8: Workload-Specific Density

**Files:**

- Modify: `crates/benchmark/src/density.rs`
- Modify: `crates/benchmark/src/suite.rs`
- Modify: `crates/evidence/src/catalog.rs`
- Modify: `crates/runtime/tests/live_snapshot_restore.rs`

**Steps:**

1. Replace `density.max_active_before_p95_doubles` with `density.max_active_before_hot_to_first_stdout_p95_doubles`.
2. Require `workload=TinyExec` and `lifecycle=Hot` in the density input points.
3. Add `DensityMetricTarget` so future metrics can add separate density rows without generic naming.
4. Update live density sweep to use `start.hot_to_first_stdout_ms` only.

**Verification:**

```bash
cargo test -p firkin-benchmark density
cargo test -p firkin-evidence metric_contract_density
```

Expected:

- Density helper rejects points without `lifecycle=Hot`.
- Density helper rejects points without `workload=TinyExec`.
- The emitted metric name says which p95 doubled.

## Task 9: Explicit Run Counts and Environment Fingerprint

**Files:**

- Modify: `crates/cli/src/main.rs`
- Modify: `crates/benchmark/src/artifact.rs`
- Modify: `crates/benchmark/src/suite.rs`
- Modify: `crates/evidence/src/benchmark.rs`

**Steps:**

1. Add `--runs <N>` to `benchmark run` and make signed-live decision runs use explicit run counts.
2. Stop treating `--duration` as the decision-grade repeat policy. It can stay only for smoke loops if the command output says `mode=smoke`.
3. Add required run context fields:

```text
git_or_jj_revision
build_profile
macos_version
machine_model
chip
memory_bytes
power_source
thermal_state
runtime_profile
guest_image_digest
rootfs_digest
storage_backend
cache_mode
sync_mode
network_profile
pool_key
cpu_limit
memory_limit
```

4. Make `benchmark run --runs 100` refuse to write a decision artifact if required context is missing.

**Verification:**

```bash
cargo test -p firkin-cli benchmark_run_explicit_runs
cargo test -p firkin-evidence run_context_required
cargo run -q -p firkin-cli -- benchmark run agent-core --mode signed-live --runs 1 \
  --out target/firkin-live-evidence/smoke-context.json
```

Expected:

- Artifact includes `sample_mode=smoke` for `--runs 1`.
- Artifact includes all required environment/config fields.
- Decision mode refuses missing fingerprints.

## Task 10: Fault-Injection Metric Validation

**Files:**

- Create: `crates/benchmark/src/metric_validation.rs`
- Modify: `crates/benchmark/src/lib.rs`
- Modify: `crates/cli/src/main.rs`
- Modify: `crates/runtime/tests/e2b_adapter.rs`
- Modify: `crates/runtime/tests/live_snapshot_restore.rs`

**Steps:**

1. Add validation scenarios:

```text
delay_guest_agent_100ms
delay_workspace_100ms
delay_first_stdout_100ms
write_delete_trim_1gb
guest_agent_crash
dns_blocked
workspace_missing
oom_kill
```

2. Each scenario declares expected affected metric, expected delta, and expected failure class.
3. Synthetic tests must run without Apple/VZ for metric derivation logic.
4. Signed-live validation runs only scenarios practical on the local Apple/VZ path; unavailable live scenarios are reported as blocked, not passed.
5. Add `benchmark validate-metrics` to write a JSON validation artifact plus a compact text summary.

**Verification:**

```bash
cargo test -p firkin-benchmark metric_validation
cargo run -q -p firkin-cli -- benchmark validate-metrics --mode host-only \
  --out target/firkin-live-evidence/metric-validation-host.json
```

Expected:

- A 100ms injected first-stdout delay increases `exec.first_stdout_byte_ms` by roughly 100ms and does not move `pool.lease_ms`.
- Guest agent crash classifies as `agent_failure`.
- Forced OOM classifies as `oom_kill`.
- DNS blocked classifies as `dns_probe_failed`.

## Task 11: Stability and Noise Floor

**Files:**

- Create: `crates/evidence/src/stability.rs`
- Create: `crates/benchmark/src/stability.rs`
- Modify: `crates/evidence/src/lib.rs`
- Modify: `crates/benchmark/src/lib.rs`
- Modify: `crates/cli/src/main.rs`

**Steps:**

1. Add batch comparison over three or more artifacts.
2. Compute p50 drift, p95 drift, max drift, failure drift, and minimum detectable delta.
3. Use this rule:

```text
p50 drift across batches < 10%
p95 drift across batches < 20%
failure classification stable
no unexplained multimodal distribution
```

4. Add `benchmark stability` to orchestrate repeated runs or read existing artifacts.
5. Make `sprint-ready` require either a fresh stability artifact or explicitly print `blocked_by_missing_stability=true`.

**Verification:**

```bash
cargo test -p firkin-evidence stability
cargo test -p firkin-benchmark stability
cargo test -p firkin-cli benchmark_stability
```

Expected:

- Identical synthetic batches report `minimum_detectable_delta_ms=0`.
- Batches with 25% p95 drift fail the stability gate.
- Failure-class drift is reported separately from latency drift.

## Task 12: Proof Artifact and Final Gates

**Files:**

- Create: `docs/artifacts/firkin-decision-grade-metrics-proof.md`
- Modify: `docs/artifacts/README.md`
- Modify: `docs/plans/archived/2026-05-07-firkin-trustworthy-performance-loop-spec.md`
- Modify: `docs/plans/archived/2026-05-07-firkin-trustworthy-benchmark-milestones.md`
- Modify: `docs/plans/archived/2026-05-07-firkin-p0-benchmark-iteration-plan.md`

**Steps:**

1. Update existing benchmark docs to point to this hard-cut metric contract.
2. Mark old P0 sprint artifacts as smoke evidence unless they are regenerated with raw event traces and sample floors.
3. Add `benchmark proof decision-grade` to render:

```text
metric contract command
100-run agent-core artifact
decision report excerpt
metric validation artifact
stability artifact
strict coverage result
residual risks
```

4. Run unit, workspace, graph, and live verification.

**Verification:**

```bash
cargo fmt --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
scripts/check-firkin-crate-graph.sh
git diff --check
```

Live closure:

```bash
cargo run -q -p firkin-cli -- benchmark doctor --mode signed-live
cargo run -q -p firkin-cli -- benchmark run agent-core --mode signed-live --runs 100 \
  --out target/firkin-live-evidence/agent-core-decision-100.json
cargo run -q -p firkin-cli -- benchmark validate-metrics --mode signed-live \
  --out target/firkin-live-evidence/metric-validation.json
cargo run -q -p firkin-cli -- benchmark stability --suite agent-core --mode signed-live \
  --runs-per-batch 100 --batches 3 \
  --out target/firkin-live-evidence/agent-core-stability.json
cargo run -q -p firkin-cli -- benchmark proof decision-grade \
  --agent-core target/firkin-live-evidence/agent-core-decision-100.json \
  --validation target/firkin-live-evidence/metric-validation.json \
  --stability target/firkin-live-evidence/agent-core-stability.json \
  --out docs/artifacts/firkin-decision-grade-metrics-proof.md
```

Expected:

- `docs/artifacts/firkin-decision-grade-metrics-proof.md` says `decision_grade_metrics=passed`.
- The proof includes raw event trace counts, derived metric counts, p95 sample confidence, fault-injection results, stability/noise floor, cleanup leftovers, and unknown failure rate.

## Implementation Order

1. Land metric contract and CLI printout.
2. Land event trace types and derivation on synthetic tests.
3. Hard-cut evidence/catalog/suite names.
4. Move runtime/startup/exec/pool emissions to events.
5. Split readiness probes.
6. Split disk stages and density workload metric.
7. Add run-count/context/sample-confidence gates.
8. Add fault-injection validation.
9. Add stability/noise-floor reports.
10. Regenerate proof artifact from live signed-run evidence.

Do not start optimization until Task 12 passes or the proof artifact clearly marks the remaining blocker and why it is not relevant to the proposed optimization.
