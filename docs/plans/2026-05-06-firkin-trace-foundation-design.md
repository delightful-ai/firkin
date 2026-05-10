# Firkin Trace Foundation Design

Status: design spec, 2026-05-06.

This spec sets up the instrumentation foundation Firkin needs to optimize toward
a single product metric — *time from "agent needs a computer" to "first useful
tool result"* — and to grow that measurement surface as new metric families come
online. It is the primitive layer beneath benchmark aggregation, SLO gating, and
evidence reporting.

This document is paired with `2026-05-06-firkin-workspace-crate-split-spec.md`
and `2026-05-07-firkin-trace-pre-split-surface-spec.md`. The trace primitive
crate and benchmark evidence/report surface land before the broad workspace
split; full lifecycle wiring lands after the split reshapes ownership seams.

## Goal in one sentence

> Make every interesting moment in a Firkin sandbox lifecycle a recorded span,
> attach polled gauges and host/guest checkpoint pairs to the same bus, and
> deliver typed samples to the existing benchmark/SLO/evidence machinery —
> with instrumentation overhead that never shows up in the workload it
> measures.

## What this spec is not

- Not an implementation plan. The migration order in §9.4 is the input to a
  separate writing-plans pass.
- Not a metric catalog. Phase names are scoped to the host-observable lifecycle
  decomposition; full metric expansion (disk personalities, memory four-plane,
  density, network policy, power) is named and deferred per §9.3.
- Not a compatibility shim strategy. Benchmark evidence is a hard-cut surface:
  regenerate artifacts when the summary schema changes. The curated public
  `firkin::*` re-exports stay intentional, but substrate/evidence do not keep
  duplicate primitive ownership.

## 1. Why this exists

Today the workspace has a real benchmark/SLO scaffold in `firkin-substrate`:
`BenchmarkSample`, `BenchmarkSummary`, `BenchmarkSloTarget`,
`BenchmarkEvidenceReport`, `BenchmarkOverheadEvidenceReport`,
`REQUIRED_LIFECYCLE_LATENCY_METRICS`, `REQUIRED_FIRKIN_OVERHEAD_METRICS`, and
default p95 SLO targets. `firkin-runtime` already times some lifecycle phases
ad-hoc with `Instant::now()` and pushes `BenchmarkSample`s through return
types. `firkin-cli` already gates on those targets.

Three structural gaps prevent the next step:

1. **No unified span/sample bus.** Phase timings are scattered `Instant::now()`
   calls inside individual call sites; there is no `SandboxId`/`TaskId`
   correlation, no per-phase span tree, no consistent way for a new layer to
   record without changing return-type signatures.
2. **Required-metric set is too narrow.** It captures op-level latency
   (`command_start`, `first_stdout_byte`) but not the boot decomposition
   (`image_resolve`, `rootfs_prepare`, `vm_start`, `vm_kernel_boot`,
   `guest_init`, `vsock_handshake`, `network_device_ready`, `mounts_ready`)
   that determines the headline number.
3. **No taggability.** `BenchmarkSample` carries metric/kind/unit/value but no
   way to slice cold/warm/hot, machine, chip, storage_backend, sync_mode, or
   pod_container_count without colliding on metric name.

This spec fixes those three.

## 2. Architecture

One bus, three drivers, one drain.

```text
        ┌─ spans (Recorder::span) ─────┐
        │                              │
        ├─ samplers (interval polls) ──┼──► sample bus (per-Recorder Vec)
        │                              │     ↓
        └─ checkpoints (paired pairs) ─┘     drain at op end
                                              ↓
                                  Vec<BenchmarkSample>
                                              ↓
                            BenchmarkEvidenceReport / SLO gate
```

Decisions baked in:

- **Single primitive sample type.** Everything ultimately becomes a
  `BenchmarkSample` carrying `metric`, `kind`, `unit`, `value`, and only the
  tags that vary for that individual sample. Shared host/config/task tags live
  on `RecordedTrace`, not on every sample. This keeps SLO aggregation compatible
  with the existing metric/kind/unit/value shape while making tag-aware trace
  export explicit.
- **Per-task ownership.** A `Recorder` is owned by a `Pod` (or task root),
  `Arc`-cloned to anything inside it. No global state. Tests inspect samples
  directly.
- **Close-and-drain at op boundary.** Every public op that already returns
  `Vec<BenchmarkSample>` keeps doing so. Internally it calls
  `recorder.close_and_drain()` so sampler tasks stop before the sample vector is
  drained. New phase samples land alongside existing samples automatically.
- **No-op when disabled.** `Recorder::Disabled` is an enum variant with no
  `Arc`, mutex, `Vec`, or sampler state. Disabled spans are stack-only sentinels.
  This is how we avoid breaking existing tests during the wiring phase.
- **Sampler lifecycle is tied to recorder closure.** `close_and_drain()` closes
  the recorder, aborts/joins sampler drivers, then drains. Dropping the last
  enabled `Recorder` also aborts drivers as a last-resort leak guard.
- **No global subscriber, no `tracing::Layer`.** Keeps cost predictable and
  per-op isolated; structured measurement has a strict typed shape that
  `tracing::Layer` would only obscure. `tracing` events can still be emitted
  inside spans for log purposes — orthogonal.

## 3. Perf and footprint budget

Hard targets for instrumentation overhead — not the metrics it records:

|                                          | Default profile | Detailed profile |
| ---------------------------------------- | --------------: | ---------------: |
| Wall-time per 30-phase lifecycle         |          <500µs |             <2ms |
| Allocations per lifecycle                |             <50 |             <200 |
| Memory held at op end                    |           <16KB |           <128KB |
| Steady-state CPU at idle (sampler tax)   |              0% |            <0.5% |

These are deliberately ~100× stricter than the headline "first exec p95 < 50ms"
target so instrumentation never appears in the workload it measures.

Design decisions that get us there:

1. **`metric: Cow<'static, str>` on the hot path.** Built-in phase names are
   compile-time constants in a single `phase` module. JSON still serializes the
   metric as a string, and `BenchmarkSample::new(impl Into<String>)` remains the
   ordinary dynamic constructor, but `BenchmarkSample::from_static` stores a
   borrowed metric name and does not allocate.
2. **Shared tags live on the Recorder, not on every sample.** Host-level tags
   (machine, chip, macos, storage_backend, sync_mode, library_version,
   kernel_version, sandbox_profile, pod_container_count, isolation_mode) are
   set once at Recorder construction as `Arc<Tags>`. They attach to the **drain
   envelope** (`RecordedTrace { tags, samples }`), not to each sample.
   Per-sample tags exist only for things that vary across spans within one
   sandbox (typically `phase_variant: cold|warm|hot`, `outcome`, or
   checkpoint side).
3. **RAII span guards.** `let span = self.recorder.span(phase::VM_START);` —
   guard is stack-only, no heap. Finish/drop computes elapsed and pushes. The
   design budgets two `Instant::now()` calls and one short bus push per span.
4. **`parking_lot::Mutex<Vec<Sample>>` for the bus, pre-sized at recorder
   construction.** The target is one short uncontended lock per span. Sampler
   drivers are default-off and must be single-flight, so a slow sampler cannot
   build an unbounded backlog. If real overhead tests show mutex contention on
   lifecycle spans, swap to `crossbeam::SegQueue` behind the same API.
5. **Samplers are off by default.** Span recording is always on, but it is the
   only default-profile hot-path cost and is enforced by the overhead test.
   Polled gauges are opt-in via `BenchProfile::Detailed` or per-sampler
   enable. Default-profile idle CPU tax is exactly 0%.
6. **Per-span allocation budget: zero for built-in span names with no dynamic
   sample tags.** A `Span` is a stack struct holding `&'static str` + `Instant`
   + `&Recorder`. `Drop` converts to a borrowed-metric `BenchmarkSample` and
   pushes into the pre-sized buffer. Dynamic metric names and dynamic tag values
   are allowed only off the default lifecycle hot path.
7. **Bounded sample buffer with class-aware backpressure.** Recorder owns a
   pre-sized `Vec<Sample>` with a hard cap (default 4096). On overflow, oldest
   sampler/gauge readings are dropped first; lifecycle spans are admitted up to
   a reserved floor. An `instrumentation.overflow` counter records drops by
   class. If the lifecycle reserve is exhausted, recording keeps the first and
   last lifecycle sample per phase and marks the trace invalid for SLO gating.
8. **Tag cardinality limits.** `Tags` enforces bounded key/value length and a
   bounded key count. High-cardinality values such as sandbox/task IDs are
   allowed only in the trace envelope, never in SLO grouping keys.

The budget is verified by an in-tree test (§9.5).

## 4. Crate placement

The workspace-crate-split spec proposes `firkin-evidence` as the new home for
`BenchmarkSample`/`BenchmarkSummary`/SLO targets/evidence artifacts. This spec
proposes a small adjustment: extract the *primitive* sample bus into a leaf
crate beneath `firkin-evidence`, because the primitive must be reachable from
`vmm`/`oci`/`vminitd-client`/`core` without those crates depending on
`evidence`.

### 4.1 New leaf: `firkin-trace`

Sentence:

> `firkin-trace` hides how lifecycle moments and gauge readings become typed
> measurement samples.

Owns:

- `BenchmarkSample`, `BenchmarkMetricKind`, `BenchmarkUnit` (moved from current
  `firkin-substrate`; under the split spec these were headed for
  `firkin-evidence`, this spec moves them one tier lower).
- `Recorder`, `Span` (RAII guard), `Tags`, `RecordedTrace` (drain envelope).
- `Sampler` trait + interval driver (tokio feature, default on).
- `Checkpoint` primitive for paired host/guest readings.
- `phase` module of `&'static str` phase-name constants.
- `BenchProfile` enum (`Off | Default | Detailed`).

### 4.2 Adjustment to the workspace-crate-split spec

`firkin-evidence` keeps everything aggregation-or-higher and gains a dep on
`firkin-trace`. Concretely, after the split:

- Move from current `firkin-substrate` into `firkin-evidence` (per split spec):
  `BenchmarkSummary`, `BenchmarkSummaryError`, `BenchmarkSloTarget`,
  `BenchmarkSloGateReport`, `RequiredFirkinOverheadMetric`,
  `RequiredLifecycleLatencyTarget`, `REQUIRED_LIFECYCLE_LATENCY_METRICS`,
  `REQUIRED_FIRKIN_OVERHEAD_METRICS`, `default_*_slo_targets`,
  `BenchmarkEvidenceReport`, `BenchmarkOverheadEvidenceReport`,
  `BenchmarkEvidenceArtifact`, `BenchmarkOverheadEvidenceArtifact`.
- Move from current `firkin-substrate` into `firkin-trace` (this spec):
  `BenchmarkSample`, `BenchmarkMetricKind`, `BenchmarkUnit`.
- Current callers import primitive sample types from `firkin-trace` directly,
  or through the top-level `firkin` facade. `firkin-evidence` consumes them but
  does not own or re-export them.

### 4.3 Resulting graph (relevant edges only, post-split)

```text
firkin-trace
  -> []

firkin-evidence
  -> firkin-trace
  -> firkin-types

firkin-benchmark
  -> firkin-evidence
  -> firkin-trace
  -> firkin-runtime / firkin-single-node / firkin-template where suites need them

firkin-vmm
  -> firkin-trace
  -> firkin-types

firkin-oci
  -> firkin-trace
  -> firkin-types

firkin-vminitd-client
  -> firkin-trace
  -> firkin-types

firkin-core
  -> firkin-trace
  -> firkin-vmm / firkin-oci / firkin-vminitd-client / firkin-types

firkin-runtime / firkin-template / firkin-single-node
  -> firkin-trace

firkin-vsock, firkin-ext4, firkin-vminitd-bytes
  -> no trace dependency until they own first-class measured operations
```

### 4.4 Forbidden edges (additive to the split spec)

- `firkin-trace` must not depend on any workspace crate.
- `firkin-trace` must not own domain types (no `Pod`, `Container`, `Sandbox`).
  IDs enter as tags via `Display`.
- `firkin-evidence` may depend on `firkin-trace`, never the reverse.

### 4.5 Why this scales when the workspace modularizes further

Anything new (a network-policy crate, a quota crate, a guest-metrics scraper
crate) takes the same shape: `firkin-types` + `firkin-trace`, records its own
spans/samplers, never imports `evidence`/`runtime`/`core`. `firkin-trace` is
intentionally narrow and stable — adding new phase-name constants is additive,
never breaking.

Two design decisions protect this:

1. **No domain types in `firkin-trace`.** Keeps the leaf truly leaf.
2. **Samplers are trait-based, not enum-based.** `Sampler` is one method
   (`async fn snapshot(&self) -> Vec<BenchmarkSample>`). Concrete impls live
   wherever their data lives:
   - `HostRssSampler` → `firkin-runtime` (uses macOS `task_info`)
   - `GuestMetricsSampler` → `firkin-vminitd-client` (calls vminitd RPC)
   - `CgroupMemorySampler` / `PsiSampler` → guest-side, scraped via
     `GuestMetricsSampler`
   - `PowerSampler` (deferred) → some future host crate

   `firkin-trace` does not grow when new metric sources are added — only the
   implementing crate does.

## 5. API surface

Sketches, not final code.

### 5.1 `BenchmarkSample` (moves to `firkin-trace`)

```rust
pub struct BenchmarkSample {
    metric: Cow<'static, str>,
    kind: BenchmarkMetricKind,
    unit: BenchmarkUnit,
    value: f64,
    sample_tags: SampleTags, // serde field name: "tags"; default empty; skipped when empty
}

pub struct SampleTag {
    key: &'static str,
    value: Cow<'static, str>,
}

impl BenchmarkSample {
    pub fn new(metric: impl Into<String>, kind, unit, value) -> Self;
    pub fn from_static(metric: &'static str, kind, unit, value) -> Self;
    pub fn with_static_tag(self, k: &'static str, v: &'static str) -> Self;
    pub fn with_dynamic_tag(self, k: &'static str, v: impl Into<String>) -> Self;
}
```

`metric` serializes as a string and `tags` defaults to an empty map for sample
construction and trace export. Evidence artifacts are a hard-cut schema: new
`BenchmarkSummary` artifacts include p50, p90, p95, p99, and max, so old
summary-only artifacts should be regenerated through the current benchmark CLI.
The default lifecycle path uses `from_static` and `with_static_tag` only;
`with_dynamic_tag` is for low-rate gauge/checkpoint samples and trace export,
not hot spans.

### 5.2 `Recorder`

```rust
pub enum Recorder {
    Disabled,
    Enabled(Arc<EnabledRecorder>),
}

struct EnabledRecorder {
    samples: parking_lot::Mutex<Vec<BenchmarkSample>>,
    shared_tags: Arc<Tags>,
    profile: BenchProfile,
    samplers: parking_lot::Mutex<Vec<SamplerHandle>>,
    sample_cap: usize,
    closed: AtomicBool,
    overflow: AtomicU64,
}

impl Recorder {
    pub fn disabled() -> Self;
    pub fn enabled(profile: BenchProfile, tags: Tags) -> Self;

    pub fn span(&self, metric: &'static str) -> Span<'_>;
    pub fn span_kind(&self, metric: &'static str, kind: BenchmarkMetricKind, unit: BenchmarkUnit) -> Span<'_>;
    pub fn sample(&self, sample: BenchmarkSample);
    pub fn checkpoint(&self, name: &'static str) -> Checkpoint<'_>;

    pub fn attach_sampler<S: Sampler>(&self, sampler: S, interval: Duration) -> Result<SamplerId, RecorderError>;
    pub fn drain(&self) -> RecordedTrace;
    pub async fn close_and_drain(&self) -> RecordedTrace;
}
```

### 5.3 `Span` (RAII)

```rust
pub struct Span<'r> {
    recorder: &'r Recorder,
    metric: &'static str,
    kind: BenchmarkMetricKind,    // default LifecycleLatency
    unit: BenchmarkUnit,          // default Milliseconds
    started: Instant,
    sample_tags: SmallVec<[SampleTag; 2]>,
    outcome: SpanOutcome,         // default Cancelled
    recorded: bool,
}

impl<'r> Span<'r> {
    pub fn tag_static(self, k: &'static str, v: &'static str) -> Self;
    pub fn tag_dynamic(self, k: &'static str, v: impl Into<String>) -> Self;
    pub fn cold(self) -> Self;   // tag_static("phase_variant", "cold")
    pub fn warm(self) -> Self;
    pub fn hot(self)  -> Self;
    pub fn finish_ok(self);
    pub fn finish_error(self, class: FailureClass);
    pub fn discard(self);        // explicit "do not record"
}

impl Drop for Span<'_> { /* compute elapsed, build sample, push under mutex */ }
```

Canonical use site:

```rust
let span = self.recorder.span(phase::VM_START).cold();
let result = self.start_vm().await;
match &result {
    Ok(_) => span.finish_ok(),
    Err(error) => span.finish_error(FailureClass::from(error)),
}
result?;
```

If a future is cancelled or a caller forgets to finish the span, `Drop` records
the sample with `outcome=cancelled`. If the thread is already panicking, `Drop`
does not allocate and must not panic; it either records a pre-tagged
`outcome=panic` sample or increments the overflow counter if the bus cannot
accept it. Success SLO gates consider only `outcome=ok` samples.

### 5.4 `Sampler`

```rust
#[async_trait]
pub trait Sampler: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    async fn snapshot(&self) -> Vec<BenchmarkSample>;
}

struct SamplerHandle { abort: Option<tokio::task::AbortHandle> }
impl Drop for SamplerHandle {
    fn drop(&mut self) { if let Some(h) = self.abort.take() { h.abort(); } }
}
```

`Recorder::attach_sampler` spawns a tokio task that drives
`tokio::time::interval`, calls `s.snapshot()`, pushes results to the bus, and
stores the handle. Disabled recorders ignore attach calls and return
`Ok(SamplerId::disabled())`.

The method must use `tokio::runtime::Handle::try_current()` and return
`RecorderError::NoRuntime` when no runtime exists. Sampler drivers are
single-flight: if one snapshot is still running at the next interval, the tick
is skipped and `instrumentation.sampler_skipped` is incremented. During
`close_and_drain()`, the recorder flips `closed`, aborts sampler tasks, waits
for the aborts to settle, then drains the bus. Any sample submitted after close
is dropped and counted as `instrumentation.closed_drop`.

### 5.5 `Checkpoint` (paired host/guest readings)

```rust
pub struct Checkpoint<'r> {
    recorder: &'r Recorder,
    name: &'static str,
    started: Instant,
}

impl<'r> Checkpoint<'r> {
    pub fn record_pair(self, host_value: f64, guest_value: f64, unit: BenchmarkUnit);
}
```

Emits two samples tagged `checkpoint=<name>` with `side=host|guest`, plus a
third elapsed-since-checkpoint-start sample (`fstrim_ms` etc.). Used for
reclaim-effectiveness, sparse-bloat-ratio, post-task-residual.

### 5.6 `RecordedTrace` (drain envelope)

```rust
pub struct RecordedTrace {
    pub shared_tags: Arc<Tags>,
    pub samples: Vec<BenchmarkSample>,
    pub overflowed: u64,
}

impl RecordedTrace {
    pub fn into_samples(self) -> Vec<BenchmarkSample>;     // old SLO path; shared tags are not flattened
    pub fn into_flat_samples(self) -> Vec<BenchmarkSample>; // trace export path; merges shared tags into sample tags
    pub fn into_envelope(self) -> /* serde-serializable trace */;
}
```

### 5.7 `phase` constants (initial set)

```rust
pub mod phase {
    pub const REQUEST_RECEIVED: &str          = "phase.request_received";
    pub const IMAGE_RESOLVE: &str             = "phase.image_resolve";
    pub const IMAGE_PULL: &str                = "phase.image_pull";
    pub const IMAGE_UNPACK: &str              = "phase.image_unpack";
    pub const ROOTFS_CLONE: &str              = "phase.rootfs_clone";
    pub const ROOTFS_PREPARE: &str            = "phase.rootfs_prepare";
    pub const OVERLAY_CREATE: &str            = "phase.overlay_create";
    pub const WORKSPACE_CREATE: &str          = "phase.workspace_create";
    pub const WORKSPACE_READY: &str           = "phase.workspace_ready";
    pub const VM_CONFIG_BUILD: &str           = "phase.vm_config_build";
    pub const VZ_VALIDATE: &str               = "phase.vz_validate";
    pub const VM_CREATE: &str                 = "phase.vm_create";
    pub const DISK_ATTACH: &str               = "phase.disk_attach";
    pub const VM_START: &str                  = "phase.vm_start";
    pub const VM_KERNEL_BOOT: &str            = "phase.vm_kernel_boot";
    pub const GUEST_INIT: &str                = "phase.guest_init";
    pub const GUEST_AGENT_LISTENING: &str     = "phase.guest_agent_listening";
    pub const VSOCK_HANDSHAKE: &str           = "phase.vsock_handshake";
    pub const AGENT_HANDSHAKE: &str           = "phase.agent_handshake";
    pub const NETWORK_DEVICE_READY: &str      = "phase.network_device_ready";
    pub const IP_ASSIGNED: &str               = "phase.ip_assigned";
    pub const DNS_READY: &str                 = "phase.dns_ready";
    pub const MOUNTS_READY: &str              = "phase.mounts_ready";
    pub const CGROUPS_READY: &str             = "phase.cgroups_ready";
    pub const FIRST_EXEC: &str                = "phase.first_exec";
    pub const FIRST_STDOUT: &str              = "phase.first_stdout";
    pub const AGENT_TASK_READY: &str          = "phase.agent_task_ready";
    pub const PROCESS_GRACEFUL_STOP: &str     = "phase.process_graceful_stop";
    pub const PROCESS_FORCED_KILL: &str       = "phase.process_forced_kill";
    pub const VM_STOP: &str                   = "phase.vm_stop";
    pub const DISK_DETACH: &str               = "phase.disk_detach";
    pub const FSTRIM: &str                    = "phase.fstrim";
    pub const TEARDOWN_TOTAL: &str            = "phase.teardown_total";
}
```

Existing op-level metric names (`command_start`, `first_stdout_byte`,
`warm_snapshot_restore`, etc.) coexist; the `phase.*` set is finer-grained.
SLO targets can be defined for either or both.

### 5.8 P0 scorecard names

The first dashboard is not a catalog of every possible container statistic. It
is the agent-usefulness spine plus the minimum resource/reclaim counters needed
to explain that spine.

The pre-split trace surface must make these names legal and taggable even when
only a subset is populated:

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

The headline derived value is:

```text
sandbox.task.agent_task_ready_ms =
  t_first_stdout_byte - t_request_received
```

The pre-split surface reserves the names. Post-split wiring populates the
host-observable startup/exec/cleanup subset first, with host RSS as the concrete
memory proof. Disk reclaim, DNS, pressure, pids, cache, and power remain
deferred metric families, but they must use the same `Recorder` bus, shared tag
envelope, and SLO/export path.

### 5.9 Errors

One new error type, mostly surfaced as instrumentation samples and counters in
`RecordedTrace`:

```rust
pub enum RecorderError {
    SampleCapExceeded { dropped: u64, class: SampleClass },
    NoRuntime,
    TagLimitExceeded { key: &'static str },
    Closed,
}
```

Span recording itself is best-effort and must not be able to fail the workload.
`parking_lot` avoids mutex poisoning; a panic in `Drop` is unacceptable.
Operations that allocate or spawn (`enabled`, `attach_sampler`, dynamic tags)
must either return an error where the API exposes `Result` or drop
instrumentation, but lifecycle work continues.

## 6. Phase observation points

Host-observable in plan #1 (no vminitd changes needed):

| Phase | Observation site |
| --- | --- |
| `phase.request_received` | entry of public ops in single-node orchestration (`create_sandbox`, `start_sandbox`, etc.) |
| `phase.image_resolve` / `image_pull` / `image_unpack` | `firkin-oci` pull paths |
| `phase.rootfs_clone` / `rootfs_prepare` / `overlay_create` / `workspace_create` / `workspace_ready` | single-node staging/workspace setup plus `firkin-core::pod::materialize_rootfs_in_pod_store`, `mount_pod_store` |
| `phase.vm_config_build` | runtime/core call sites that build `VmConfig`, not a recorder field inside `VmConfig` |
| `phase.vz_validate` / `vm_create` / `disk_attach` / `vm_start` | around VZ configuration validation, VM object construction, storage attachment lowering, and `VZVirtualMachine.start()` in `crates/vmm/src/vz.rs` and the apple-vz driver |
| `phase.vm_kernel_boot` | `vm_start` end → first successful vsock connect (collapsed with `guest_init` until vminitd reports its own init-started timestamp; deferred) |
| `phase.vsock_handshake` / `agent_handshake` | `firkin-core::connect_vminitd` and `firkin-vminitd-client::connect_with_dialer` wrapper/helper paths |
| `phase.network_device_ready` | host-side around VZ network device configuration |
| `phase.first_exec` / `phase.first_stdout` | refactor of existing `Instant::now()` measurements in current runtime exec path |
| `phase.agent_task_ready` | wrapper span at the public op boundary |
| `phase.vm_stop` / `disk_detach` / `teardown_total` | in delete paths |

Guest-emitted in plan #2 (require vminitd RPC):

- `phase.guest_init`, `phase.guest_agent_listening` (init-started timestamp from
  vminitd)
- `phase.ip_assigned`, `phase.dns_ready`
- `phase.mounts_ready`, `phase.cgroups_ready`
- `phase.fstrim`

The post-split trace wiring plan stubs the proto
(`service GuestMetrics { rpc Snapshot returns Empty; }`) so the surface exists;
Swift implementation is plan #2.

Clock rule: host spans use host `Instant` only. Guest-emitted checkpoints may
include guest monotonic nanoseconds and guest wall time, but host/guest
duration math is valid only after translating through a host-observed receive
time or an explicit clock-sync sample. The first wiring plan must not subtract
guest wall time from host `Instant`.

## 7. Three first-class drivers (one bus)

The Recorder is a span/event sink. The second driver class — polled gauges —
is needed for PSI (`/proc/pressure/{cpu,memory,io}`), cgroup `memory.current`/
`memory.peak`/`memory.events`, host RSS, `/proc/diskstats` deltas, `df`. The
third — host/guest checkpoint pairs — is needed for reclaim effectiveness,
`sparse_bloat_ratio`, `post_task_residual_bytes`, fstrim-discarded.

All three feed the same `BenchmarkSample` bus; they differ only in what drives
the emission:

1. **Spans.** `Recorder::span(metric).cold()` returns an RAII guard; Drop
   pushes a sample. Used for: lifecycle latency, control-plane RPC latency,
   exec latencies, workload wall-times.
2. **Samplers.** `Recorder::attach_sampler(s, Duration)` spawns a tokio task
   that drives `s.snapshot()` on an interval; results push to the bus. Used
   for: PSI, cgroup memory, host RSS, diskstats deltas, df, power.
3. **Checkpoints.** `Recorder::checkpoint(name).record_pair(host, guest, unit)`
   emits two paired samples plus an elapsed sample. Used for: reclaim
   effectiveness, bloat ratio, post-task residual.

Checkpoint samples must include a `checkpoint=<name>` tag and `side=host|guest`.
If the pair compares host and guest byte counters, the derived ratio is computed
by evidence/reporting code after drain. If the pair compares timestamps, the
trace must also carry the clock domain so evidence cannot accidentally compare
host monotonic time to guest wall time.

The pre-split surface ships the driver types. The post-split wiring plan ships
driver #1 broadly, #2 with one concrete impl (`HostRssSampler`) as proof, and
#3 as the type only. Later plans wire more samplers/checkpoints; none of them
require trace-API changes.

## 8. vminitd `GuestMetrics` RPC reservation

The post-split trace wiring plan reserves the RPC surface but leaves the Swift
implementation to plan #2.

```proto
service GuestMetrics {
  rpc Snapshot(SnapshotRequest) returns (SnapshotResponse);
}

message SnapshotRequest {}
message SnapshotResponse {
  // populated in plan #2; empty in plan #1
}
```

Plan #2 fills `SnapshotResponse` with PSI lines, cgroup memory.current/peak/
events, diskstats, df, /proc/meminfo, and the guest's view of fstrim discard
counts. Rust client lives in `firkin-vminitd-client`; a `GuestMetricsSampler`
in the same crate drives it from a `Recorder`.

## 9. Scope

### 9.1 Pre-split deliverable

- `crates/trace/` exists with all primitive measurement types from §5.
- `BenchmarkSample`/`BenchmarkMetricKind`/`BenchmarkUnit` move from current
  `firkin-substrate` into `firkin-trace`. Existing consumers hard-cut to direct
  `firkin-trace` imports or the curated top-level `firkin` facade re-exports;
  `firkin-substrate` and post-split `firkin-evidence` do not re-export them.
- Per-sample `tags` field exists on `BenchmarkSample`, default-empty,
  skip-serializing-empty. Shared tags stay on `RecordedTrace`.
- `BenchmarkSummary` stores and reports count, p50, p90, p95, p99, and max.
  Current SLO gates remain p95-based.
- `fk benchmark` is the first-class CLI group for targets, reports, and
  lifecycle/overhead SLO gates.
- P0 scorecard names are legal and reserved, even when only the existing
  lifecycle/overhead subset is populated.

See `2026-05-07-firkin-trace-pre-split-surface-spec.md` for the exact
pre-split scope and verification gates.

### 9.2 Post-split trace wiring deliverable

- `Recorder` carried by runtime operation context, `PodBuilder`/`Pod`,
  `ContainerBuilder`/`ContainerRuntime`, and `VirtualMachine`. `VmConfig`
  remains pure validated configuration. OCI pull and vminitd connect paths
  receive `&Recorder` through request/helper methods rather than storing a
  recorder on reusable clients or generated tonic clients. Default
  `Recorder::disabled()`.
- Host-observable phases wired (per §6 first table).
- Existing `command_start`/`first_stdout_byte` refactored to `Recorder`; current
  required metrics remain semantically identical and continue feeding the same
  SLO gates.
- `metrics.proto` stub (vminitd RPC surface reserved).
- `HostRssSampler` as proof of the Sampler shape (default-off).
- Overhead test enforcing §3 budget.
- `LIFECYCLE_PHASE_SLO_TARGETS` constant + opt-in CLI gate (additive; existing
  required-metric set unchanged).

### 9.3 Deferred plans (named, not orphaned)

- **Plan #2:** vminitd Swift `GuestMetrics::Snapshot` impl (PSI, cgroup
  memory.events, /proc/diskstats, df, /proc/meminfo). Guest-emitted phases
  (`mounts_ready`, `cgroups_ready`, etc.). Reclaim/checkpoint metrics
  (`fstrim`, `sparse_bloat_ratio`, `host_bytes_reclaimed`).
- **Plan #3:** disk personality matrix (fsync p99, metadata throughput, image
  materialization, bloat/reclaim, durability cost split).
- **Plan #4:** workload-realism benches (cargo build, npm install, sqlite txn)
  as a `firkin-bench-realism` crate.
- **Plan #5:** density/concurrency dashboards (`start_N_parallel_p95`,
  density breakpoint, contention breakpoint, per-sandbox fairness).
- **Plan #6:** host `PowerSampler` (powermetrics/IOReport) and thermal
  pressure.
- **Plan #7:** network policy denial counters and isolation-score metrics.

### 9.4 Migration order (input to writing-plans)

1. Create `crates/trace/`. No callers yet.
2. Move `BenchmarkSample`/`Kind`/`Unit` from substrate (or evidence post-split)
   to `firkin-trace`. Update construction sites to import from `firkin_trace`;
   update the top-level facade re-exports. CI green.
3. Add per-sample `tags` field to `BenchmarkSample`; shared tags stay on
   `RecordedTrace`.
4. Extend `BenchmarkSummary` and CLI reports to p50/p90/p95/p99/max. Regenerate
   benchmark evidence artifacts through the current CLI after the schema
   hard-cut.
5. Add recorder plumbing at ownership seams only:
   `RuntimeTraceContext`/op root → `PodBuilder`/`Pod` →
   `ContainerBuilder`/`ContainerRuntime` → `VirtualMachine`. Do not put the
   recorder in `VmConfig`. Add `pull_with_recorder` or an OCI pull request
   context rather than storing a recorder in `firkin_oci::Client`. Instrument
   vminitd through `connect_vminitd`/`connect_with_dialer` wrappers because
   `VminitdClient` is a generated tonic client/type alias. No span calls yet —
   just plumbing. CI green.
6. Wire host-observable spans, one per commit, in §6 table order. Refactor
   existing `command_start`/`first_stdout_byte` to `Recorder` spans; output
   remains semantically identical for the current required metrics.
7. Stub `metrics.proto` + `GuestMetricsClient` returning empty. Add
   `GuestMetricsSampler` shell in `firkin-vminitd-client`.
8. Add `HostRssSampler` in `firkin-runtime` as a concrete `Sampler` impl.
   Opt-in via `BenchProfile::Detailed`.
9. Add evidence integration: extend lifecycle SLO targets to optionally
   include `phase.*` targets (additive). `firkin-cli benchmark
   validate-lifecycle-slo` reads current lifecycle evidence artifacts.
10. Land overhead test.

Each step is a single commit, individually testable.

### 9.5 Testing

- **Unit (`crates/trace/`):** Span finish/drop emission; `outcome=ok` versus
  `outcome=error|cancelled|panic`; Recorder close-and-drain semantics;
  disabled no-op asserts zero allocations via counting allocator; sample-cap
  overflow behavior by sample class; tag key/value limit enforcement;
  `attach_sampler` no-runtime error; sampler abort-on-close using
  `tokio::time::pause()`; mid-snapshot close drops late samples and increments
  `instrumentation.closed_drop`.
- **Overhead test (`crates/firkin/tests/bench_instrumentation_overhead.rs`):**
  synthetic 30-phase + 10s sampler lifecycle; asserts §3 budget — wall-time
  via `Instant`, alloc count via `dhat`, retained memory via
  `Recorder::stats()`.
- **Integration (`crates/single-node/tests/lifecycle_phase_trace.rs` post-split,
  `crates/runtime/tests/lifecycle_phase_trace.rs` pre-split):** boots a real
  VZ sandbox under `BenchProfile::Detailed`; asserts every host-observable
  phase appears exactly once; `phase.agent_task_ready` ≥ sum of decomposed
  children (modulo overlap); success SLO inputs include only `outcome=ok`;
  `RecordedTrace::overflowed == 0`.
- **Evidence (`crates/evidence/tests/benchmarks.rs` post-split,
  `crates/substrate/tests/benchmarks.rs` pre-split):** summaries expose count,
  p50, p90, p95, p99, and max; current required lifecycle/overhead reports
  still gate on p95.
- **Live-runtime (`Justfile` `live-runtime-benchmark-evidence`):** unchanged
  output for the existing required-metrics set; new `phase.*` metrics appear
  in the artifact alongside.

## 10. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| Threading `Recorder` through ownership seams is more invasive than expected | Step 4 is a self-contained commit. Keep `VmConfig`, OCI clients, and generated tonic clients clean; if plumbing bloats, top-level structs (`Pod`, `LocalRuntimeBackend`) carry the recorder and lower layers receive `&Recorder` as function arg on the few hot-path methods. |
| Sample-cap drops bite a long-running pod | `instrumentation.overflow` is itself a sample; SLO gate can include it. Default cap (4096) is generous for any realistic op. |
| Error/cancelled spans contaminate success latency | `SpanOutcome` is mandatory, default is `cancelled`, and success SLO inputs filter to `outcome=ok`. Failure traces remain available for debugging and failure-rate metrics. |
| Per-span allocation budget broken by dynamic tags or SmallVec spills | Overhead test catches this immediately. Built-in lifecycle spans use static names and static tags only. Dynamic tags are allowed only in lower-rate samplers/checkpoints or trace export. |
| Sampler task races the final drain | `close_and_drain()` sets `closed`, aborts/joins sampler handles, then drains. Late submissions are counted and dropped. |
| `parking_lot` not yet a workspace dep | Add to `[workspace.dependencies]`. It is a near-universal Rust dep. |
| `smallvec` not yet a workspace dep | Either add it to `[workspace.dependencies]` with the trace crate, or replace it with a tiny fixed-cap inline tag container. Do not let dynamic tag storage become a default-span allocation. |
| Evidence or substrate starts re-exporting `BenchmarkSample` again | Keep the primitive ownership hard-cut: direct construction imports use `firkin_trace`; the top-level facade is the curated public convenience surface. |
| Workspace-crate-split spec drifts after `firkin-trace` lands | Keep the split spec authoritative: trace primitives stay in existing `firkin-trace`, evidence moves to `firkin-evidence`, and benchmark-suite execution moves to `firkin-benchmark`. |

## 11. Immediate follow-on artifacts

After this design is approved:

1. Add the dependency allowlist that encodes the revised workspace split graph.
2. Write the implementation plan that does the migration order in the workspace
   split spec with exact file moves, dep edges, and test updates.
