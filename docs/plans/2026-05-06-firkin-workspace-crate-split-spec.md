# Firkin Workspace Crate Split Spec

Status: design spec, 2026-05-06. Updated 2026-05-07 after the pre-split
`firkin-trace` and first-class benchmark CLI surface landed.

This document is the repo-grounded answer to a simple question:

> If Firkin is going to become a real container library instead of a pile of
> successful spikes, what should the workspace graph be before scope blows up?

The short answer is:

- the current graph is directionally right at the bottom
- it is too coarse in the middle
- `firkin-substrate`, `firkin-e2b`, and the current `runtime + single_node`
  story are the main topology problems
- `firkin-core` is oversized, but it is mostly a module-boundary problem before
  it is a crate-boundary problem

This spec intentionally distinguishes:

- `keep as crate, split internally`
- `split into more crates`
- `move code across existing crates`
- `do not split yet`

It does not propose backward-compatibility shims.

## Why this spec exists

The existing rewrite docs and current crate roots diverged.

## 2026-05-07 crate drift update

The intended factorization changed in two concrete ways after the trace and
benchmark work landed:

1. `firkin-trace` is no longer part of the future substrate split. It already
   exists as the measurement leaf and owns `BenchmarkSample`,
   `BenchmarkMetricKind`, `BenchmarkUnit`, `Recorder`, spans, samplers,
   checkpoints, tags, drain envelopes, and recorder stats.
2. Benchmark execution is now a first-class product surface, not just an
   evidence artifact parser. That means the final graph needs a high-level
   `firkin-benchmark` crate in addition to `firkin-trace` and
   `firkin-evidence`.

The corrected observability split is:

- `firkin-trace`: low-level measurement primitives and recorder mechanics.
- `firkin-evidence`: summaries, SLO gates, evidence artifacts, and soak proof
  schemas.
- `firkin-benchmark`: named benchmark suites, benchmark run orchestration,
  benchmark artifact production, and CLI-facing benchmark workflows.

Runtime, template, single-node, VMM, OCI, and vminitd code may emit trace
samples when they own a measured lifecycle phase. They must not own SLO gates,
evidence schemas, benchmark-suite policy, or benchmark artifact validation.

This keeps the topology honest: recording facts, validating claims, and running
experiments are three different decisions.

The old intended story was roughly:

- `types` leaf
- `ext4`, `vsock`, `oci` as portable substrate
- `vmm` and `vminitd-client` as VM/guest adapters
- `core` as library surface
- `substrate` + `template` + `runtime` as production substrate
- `e2b` as compatibility boundary

That is still the right rough layering, but the live code shows three real
problems:

1. `firkin-substrate` is not one concept.
2. `firkin-e2b` is not one concept.
3. `firkin-runtime` currently contains both reusable runtime orchestration and a
   concrete single-node backend subtree that already wants its own story.
4. The benchmark runner surface would become another runtime/CLI junk drawer
   unless it gets its own high-level crate.

The line-count picture is a warning, not a proof, but it supports the topology
read:

| Crate | Approx source LOC | Read |
| --- | ---: | --- |
| `firkin-e2b` | 14.3k | clearly overstuffed |
| `firkin-runtime` | 13.4k | clearly overstuffed |
| `firkin-core` | 9.6k | large, but conceptually central |
| `firkin-substrate` | 3.6k | too many unrelated models in one leaf |
| `firkin-trace` | 1.1k | now a real leaf; keep narrow |
| `firkin-oci` | 2.1k | probably okay as one crate |
| `firkin-vminitd-client` | 1.4k | probably okay as one crate |
| `firkin-vmm` | 3.1k | okay as one crate |
| `firkin-ext4` | 3.2k | okay as one crate |

## Design rules

These are the rules this split follows.

1. A crate must hide a decision that can be described in one sentence.
2. Public traits live with the law-owning consumer, not in homeless
   abstraction crates.
3. `firkin-types` stays a small leaf of validated atoms and small wire-neutral
   value types.
4. Product HTTP compatibility and VM/runtime mechanics must not collapse into
   one crate.
5. Control-plane persistence and scheduling must not be smuggled into a
   “runtime helper module.”
6. `lib.rs` files stay maps, not implementation junk drawers.
7. Trace primitives, evidence validation, and benchmark execution stay separate.
8. The final graph contains no `firkin-substrate` compatibility shim.

## System slices

This is the whole-system slice list, phrased as “this hides the decision of …”.

| Slice | Hidden decision |
| --- | --- |
| validated atoms | how IDs, sizes, ports, hostnames, and tiny shared values are represented |
| vsock transport | how async byte streams are exposed over vsock fds |
| virtualization substrate | how Virtualization.framework is configured and driven |
| guest agent protocol | how host code speaks to `vminitd` and which guest ops exist |
| rootfs/image assembly | how OCI content becomes a rootfs or materialized image |
| ext4 writer | how Linux filesystems are synthesized on the host |
| container API | how a caller describes and drives VM-backed containers |
| pod API | how multi-container same-VM sandboxes are represented and executed |
| admission | how active and warm capacity are budgeted and rejected |
| snapshot artifacts | how template and continuation artifacts are described and verified |
| hygiene | how orphaned logs, snapshots, markers, and stuck VMs are found and cleaned |
| trace | how sandbox lifecycle moments become typed, low-overhead measurement samples |
| evidence | how benchmark, overhead, and soak proof is represented and validated |
| benchmark execution | how named suites exercise runtime paths and produce evidence artifacts |
| template execution | how repos are cloned, prepared, freshness-synced, and snapshotted |
| runtime orchestration | how core/template/admission/artifact/hygiene pieces are composed into live operations |
| single-node backend | how one local host persists state, schedules work, and exposes a local backend |
| E2B wire contract | how E2B/Cube request and response DTOs are shaped |
| E2B compatibility contracts | which runtime/data-plane capabilities the compatibility layer expects |
| E2B local servers | how the local control plane, domain proxy, and envd-compatible servers run |
| facade | what stable top-level Rust API Firkin exports |
| CLI | how developers exercise the library from a binary |

## Target graph

The target graph is:

```text
firkin-cli
  -> firkin
  -> firkin-benchmark          # benchmark run/report/validate workflows
  -> firkin-single-node          # dev/operator entrypoints when needed

firkin
  -> firkin-core
  -> firkin-runtime
  -> firkin-single-node          # optional re-export surface
  -> firkin-benchmark            # optional public benchmark suite surface
  -> firkin-e2b-contract
  -> firkin-e2b-wire
  -> firkin-ext4
  -> firkin-oci
  -> firkin-vminitd-client
  -> firkin-vminitd-bytes
  -> firkin-vmm
  -> firkin-vsock
  -> firkin-trace
  -> firkin-types

firkin-single-node
  -> firkin-runtime
  -> firkin-admission
  -> firkin-artifacts
  -> firkin-hygiene
  -> firkin-trace
  -> firkin-template
  -> firkin-e2b-contract
  -> firkin-e2b-server
  -> firkin-types

firkin-runtime
  -> firkin-core
  -> firkin-template
  -> firkin-admission
  -> firkin-artifacts
  -> firkin-hygiene
  -> firkin-trace
  -> firkin-e2b-contract
  -> firkin-types

firkin-template
  -> firkin-artifacts
  -> firkin-trace
  -> firkin-types

firkin-benchmark
  -> firkin-single-node
  -> firkin-runtime
  -> firkin-template
  -> firkin-evidence
  -> firkin-trace
  -> firkin-types

firkin-e2b-server
  -> firkin-e2b-wire
  -> firkin-e2b-contract
  -> firkin-types

firkin-e2b-contract
  -> firkin-e2b-wire
  -> firkin-types

firkin-e2b-wire
  -> firkin-types

firkin-core
  -> firkin-ext4
  -> firkin-oci
  -> firkin-vminitd-bytes
  -> firkin-vminitd-client
  -> firkin-vmm
  -> firkin-vsock
  -> firkin-trace
  -> firkin-types

firkin-oci
  -> firkin-ext4
  -> firkin-trace
  -> firkin-types

firkin-vminitd-client
  -> firkin-oci
  -> firkin-vsock
  -> firkin-trace
  -> firkin-types

firkin-vmm
  -> firkin-vsock
  -> firkin-trace
  -> firkin-types

firkin-ext4
  -> firkin-types

firkin-vsock
  -> firkin-types

firkin-admission
  -> firkin-types

firkin-artifacts
  -> firkin-types

firkin-hygiene
  -> firkin-artifacts
  -> firkin-types

firkin-evidence
  -> firkin-trace
  -> firkin-types

firkin-trace
  -> []

firkin-vminitd-bytes
  -> []

firkin-types
  -> []
```

This is more crates than today, but each added crate corresponds to a real
knowledge boundary that already exists in the live code.

Trace edges from low-level crates (`core`, `oci`, `vminitd-client`, and `vmm`)
are final post-wiring edges, not split blockers. During the split itself, add a
`firkin-trace` dependency to one of those crates only when that crate actually
emits samples or owns spans. The invariant is that all sample construction uses
`firkin-trace`; it is not that every crate imports trace on day one.

`firkin-benchmark` is intentionally high in the graph. It may import runtime and
single-node crates to run suites. No runtime/library crate may import it.

The final target graph has no `firkin-substrate` crate.

### Graph readiness gates

The split plan is ready to drive a whole-workspace refactor only if these remain
true at the end of the split:

1. `cargo metadata` has no package named `firkin-substrate`.
2. `firkin-trace` has no workspace crate dependencies.
3. `BenchmarkSample`, `BenchmarkMetricKind`, and `BenchmarkUnit` are defined
   only in `firkin-trace`.
4. `BenchmarkSummary`, SLO targets, evidence artifacts, and soak evidence are
   defined only in `firkin-evidence`.
5. Benchmark suite runners, suite matrices, evidence generation, and
   CLI-facing benchmark workflows are defined only in `firkin-benchmark`.
6. Runtime/template/single-node crates may emit samples, but they do not own
   evidence schemas or benchmark-suite policy.
7. No lower crate imports `firkin-benchmark` or `firkin-single-node`.
8. A dependency allowlist in CI rejects any edge not present in the target graph
   above.

## Crate-by-crate decisions

### Keep: `firkin-types`

Keep as a small leaf crate.

It should continue to own validated atoms and tiny cross-cutting values only:

- IDs
- `Size`
- hostnames
- vsock ports
- portable network-policy request shapes
- other small validated primitives

It must not become the home for:

- VM policy bundles
- runtime records
- template metadata
- E2B DTOs
- snapshot manifests
- admission policy

Rule: if a type needs to know about lifecycle, persistence, template builds,
snapshots, or HTTP compatibility, it does not belong here.

### Keep: `firkin-vsock`

Keep as a leaf transport crate.

This crate already has a clean story: async wrappers over vsock streams and
listeners. It should stay ignorant of:

- vminitd protobuf
- Virtualization.framework
- container semantics
- E2B semantics

### Keep: `firkin-vminitd-bytes`

Keep exactly as a tiny artifact-owner leaf.

Do not merge it upward.

### Keep: `firkin-vmm`

Keep as one crate, but split internally harder.

This crate has a coherent external sentence:

> `firkin-vmm` owns Virtualization.framework-backed VM configuration and live VM lifecycle.

Internal module extraction is still warranted:

- boot/kernel config
- storage attachment lowering
- network attachment lowering
- vsock listener/dialer plumbing
- snapshot plumbing
- VZ backend glue

But these are module cuts inside one concept, not evidence for more crates.

Exact internal tree:

```text
vmm/
  error.rs
  preflight.rs
  network.rs
  storage.rs
  kernel.rs
  config.rs
  vm.rs
  snapshot.rs
  disk_image.rs
  vz.rs
```

Move:

- `Error` into `error.rs`
- `HostArch`, `Preflight`, signing helpers into `preflight.rs`
- `Network`, `NetworkInterface` into `network.rs`
- `VirtiofsShare`, `BlockDevice` into `storage.rs`
- `KernelImage`, `BootLog` into `kernel.rs`
- `VmConfig`, `VmConfigBuilder` into `config.rs`
- `NotBooted`, `Running`, `VirtualMachine` into `vm.rs`
- snapshot-only helpers into `snapshot.rs`

### Keep: `firkin-vminitd-client`

Keep as one crate, split internally only.

Its story is still coherent:

> `firkin-vminitd-client` owns the typed host-side API for the guest agent protocol.

Recommended internal modules:

- `pb`
- `connect`
- `network`
- `process`
- `copy`
- `proxy`
- `stats`
- `guest_ops`

Do not let it start owning runtime policy or control-plane compatibility.

Exact internal tree:

```text
vminitd-client/
  error.rs
  connect.rs
  proxy.rs
  copy.rs
  stats.rs
  rosetta.rs
  bundle.rs
  network.rs
  process.rs
  guest_ops.rs
  pb.rs
```

Move:

- `VminitdError`, `RpcFamily` into `error.rs`
- `connect_with_dialer` into `connect.rs`
- `SocketProxy*` into `proxy.rs`
- `Copy*` into `copy.rs`
- stats types into `stats.rs`
- `RosettaSetup`, `RosettaRequests` into `rosetta.rs`
- `ContainerBundle` into `bundle.rs`
- `LinuxNamespace`, `NetworkConfig`, `NetworkRequests` into `network.rs`
- `ProcessStdio`, `ProcessCreate` into `process.rs`

### Keep: `firkin-ext4`

Keep as one crate.

It has a real single sentence today and should remain a low-level filesystem
writer with no runtime or product knowledge.

### Keep: `firkin-oci`, but split internally

Do not split into more crates yet.

The crate is doing three adjacent things:

- OCI reference and descriptor modeling
- registry/cache/pull orchestration
- OCI runtime-spec mirror types

That is not ideal, but it is still one adjacent concept cluster. The safer move
is internal modules, not more crates:

- `reference`
- `descriptor`
- `registry`
- `bundle`
- `runtime_spec`
- `image_config`

Do not let `firkin-oci` grow template, runtime, or guest-agent behavior.

Exact internal tree:

```text
oci/
  error.rs
  auth.rs
  client.rs
  cache.rs
  reference.rs
  descriptor.rs
  bundle.rs
  image_config.rs
  runtime_spec.rs
```

Move:

- `Auth` and auth lowering into `auth.rs`
- `Client` and `ClientBuilder` into `client.rs`
- cache helpers into `cache.rs`
- `Reference` into `reference.rs`
- `Digest`, `MediaType`, `ManifestPlatform`, `Descriptor` into `descriptor.rs`
- `Layer`, `ImageBundle`, and bundle metadata into `bundle.rs`

### Keep: `firkin-core`, but split internally now

`firkin-core` is too large, but I do not think the first move is another crate
split.

Its public sentence is still defensible:

> `firkin-core` owns the user-facing VM-backed container and pod execution surface.

The problem is that the current crate root is carrying too many internal
clusters:

- public API types and validation
- container builder typestates
- stdio and PTY plumbing
- rootfs and artifact staging
- vminitd request lowering
- runtime session execution
- snapshot lifecycle
- pod store and pod execution

That wants module extraction, not immediate new crates. Recommended internal
tree:

```text
core/
  api/
    ids.rs
    resources.rs
    mounts.rs
    exec.rs
    stdio.rs
    users.rs
    seccomp.rs
  builder/
    container.rs
    vm.rs
    typestate.rs
  rootfs/
    assembly.rs
    staging.rs
    bundle.rs
  runtime/
    session.rs
    process.rs
    copy.rs
    sockets.rs
    network.rs
  snapshot/
    save.rs
    restore.rs
    state.rs
  pod/
    mod.rs
    store.rs
    rootfs.rs
    volumes.rs
    exec.rs
```

Crucial refusal:

- `firkin-core` must not depend on `firkin-runtime`
- `firkin-core` must not depend on E2B compatibility crates
- `firkin-core` must not own production admission or hygiene models

#### Exact `firkin-core` module map

The right move is not just “split `lib.rs` up a bit.” The right move is to make
`lib.rs` almost entirely declarations and curated re-exports, then assign every
major type cluster to a named module that owns one fact.

Target `lib.rs` shape:

```text
mod error;
mod ids;
mod io;
mod process;
mod rootfs;
mod builder;
mod runtime;
mod vm_attach;
mod snapshot;
mod pod;

pub use error::{Error, Result};
pub use ids::{GuestPath, IntoContainerId, IntoProcessId};
pub use io::{
    ChildStderr, ChildStdin, ChildStdout, DnsConfig, FileMount, HostsConfig,
    HostsEntry, Pty, PtyConfig, PtyControl, PtyInput, PtyOutput, SocketDirection,
    Stdio, Streams, UnixSocketConfig,
};
pub use process::{
    Capability, ExecConfig, ExecConfigBuilder, ExitStatus, KilledReason,
    LinuxCapabilities, LinuxRlimit, Output, Process, ProcessKillHandle,
    RlimitKind, Signal, User,
};
pub use rootfs::{Rootfs, VmRootfs};
pub use builder::{ContainerBuilder, CoreContainerFactory};
pub use runtime::Container;
pub use snapshot::{
    ContainerRestoreTimings, ContainerSnapshotState, RestoredRootfsStage,
    RestoredRootfsStageMethod, TimedContainerRestore,
};
pub use pod::*;
```

Exact ownership:

##### `error.rs`

Owns:

- `Error`
- `Result<T>`

This is the only module that should define the core error surface.

##### `ids.rs`

Owns:

- `IntoContainerId`
- `IntoProcessId`
- `GuestPath`
- guest-path normalization helpers

This is the “validated guest identity/path vocabulary” module.

##### `io.rs`

Owns:

- `Streams`
- `PtyConfig`
- `DnsConfig`
- `HostsConfig`
- `HostsEntry`
- `UnixSocketConfig`
- `SocketDirection`
- `FileMount`
- `Pty`
- `PtyInput`
- `PtyOutput`
- `PtyControl`
- `Stdio`
- `ChildStdin`
- `ChildStdout`
- `ChildStderr`

Move the following private helpers here too:

- `PreparedFileMount`
- file-mount hashing/tagging helpers
- stdio port helpers
- socket-relay planning helpers

Rule: if it is about moving bytes, PTYs, sockets, or file-sidecar mounts, it
belongs here.

##### `process.rs`

Owns:

- `Signal`
- `KilledReason`
- `ExitStatus`
- `Output`
- `User`
- `LinuxCapabilities`
- `Capability`
- `InvalidCapability`
- `LinuxRlimit`
- `RlimitKind`
- `InvalidRlimit`
- `ExecConfig`
- `ExecConfigBuilder`
- `Process<S>`
- `ProcessKillHandle`

This is the “process contract” module. It should know nothing about implicit VM
boot or template builds.

##### `rootfs.rs`

Owns:

- `Rootfs`
- `VmRootfs`
- `StagedRootfs`
- `RootfsChoice`
- `CopyInPayload`
- rootfs assembly/staging helpers

This is where “how a rootfs choice becomes guest-visible storage” lives.

##### `builder.rs`

Owns:

- `VmContext`
- `BuilderState`
- `ContainerStdio`
- `ImplicitVm`
- `OnVm<'vm>`
- `OnVmArc`
- `Init`
- `Ready`
- `ReadyPty`
- `MissingCommand`
- `CommandSet`
- `CommandSetPty`
- `ContainerBuilder`
- `PreparedImplicitVm`
- `ImplicitStartContext`
- `StagedInitBlock`
- `NestedVirtualization`
- `UseInit`
- `SeccompConfig`

This is the public construction grammar. It may prepare a launch plan, but it
should not contain the long runtime execution code paths.

##### `runtime.rs`

Owns:

- `ContainerRuntime`
- `ProcessRuntime`
- `ProcessStdioPlan`
- `RuntimeStaging`
- `Container<S>`
- live container/process exec/copy/wait/kill implementations

This is the “do the thing” module for live session mechanics.

##### `vm_attach.rs`

Owns:

- `CoreContainerFactory`
- attached-to-running-VM builder constructors
- `OnVm` / `OnVmArc` specific glue

Keep “boot an implicit VM” and “attach to an existing VM” separate.

##### `snapshot.rs`

Owns:

- `ContainerSnapshotState`
- `ContainerRestoreTimings`
- `RestoredRootfsStageMethod`
- `RestoredRootfsStage`
- `TimedContainerRestore`
- snapshot save helpers
- snapshot restore helpers

Anything behind `#[cfg(feature = "snapshot")]` that is not a generic rootfs
staging primitive belongs here.

##### `pod/`

`pod.rs` should become a directory:

```text
pod/
  mod.rs
  id.rs
  spec.rs
  store.rs
  layout.rs
  rootfs.rs
  materialize.rs
  exec.rs
  tar_import.rs
```

Exact pod ownership:

- `id.rs`
  - `PodId`
  - `IntoPodId`
- `spec.rs`
  - `GuestFilesystem`
  - `PodValidationError`
  - `EmptyDirMedium`
  - `EmptyDirVolume`
  - `PodRootfsSource`
  - `PodVolumeMount`
  - `PodContainerSpec`
- `store.rs`
  - `PodStoreSpec`
  - `MountedPodStore`
  - guest pod-store mount/trim helpers
- `layout.rs`
  - `PodTemplateKey`
  - `PodTemplate`
  - `PodContainerLayout`
  - pod path constructors
- `rootfs.rs`
  - `PreparedPodRootfs`
  - `MaterializedRootfs`
  - template/overlay rootfs preparation
- `materialize.rs`
  - public materialization helpers
  - OCI layer import/copy-in flows
- `exec.rs`
  - `PodBuilder`
  - `Pod`
  - add/remove container flows
  - per-container mount composition
- `tar_import.rs`
  - tar/zstd metadata helpers
  - `MaterializationEntryMetadata`

That is the correct split because it separates:

- pod description
- pod storage substrate
- pod rootfs substrate
- pod execution lifecycle

### Split: `firkin-substrate`

This is the clearest crate split in the workspace.

The current crate mixes at least four decisions:

- admission and warm-pool budgeting
- snapshot/template artifact modeling
- hygiene/reconciliation/GC/stuck-VM cleanup
- benchmark/overhead/soak evidence

That is not one crate. The target replacement is:

#### `firkin-admission`

Owns:

- `ResourceBudget`
- `CapacityLedger`
- `WarmPoolKey`
- `WarmPoolEntry`
- `WarmPoolLedger`
- `ActiveCapacityAdmissionPlan`
- `ActiveQueuePolicy`
- `ActiveBackpressurePlan`
- `WarmPoolReplenishmentPlan`

Sentence:

> `firkin-admission` hides how active and warm runtime work competes for host capacity.

#### `firkin-artifacts`

Owns:

- `SnapshotArtifactKind`
- `SnapshotArtifactManifest`
- `SnapshotArtifactIntegrity`
- recorded pod membership metadata
- continuation snapshot plans

Sentence:

> `firkin-artifacts` hides how durable runtime artifacts are named, described, and verified.

#### `firkin-hygiene`

Owns:

- `ReconciliationPlan`
- `RestartStateRecord`
- `RestartResourceKind`
- `HostRuntimeScan`
- `ArtifactGcPlan`
- `LogRotationPlan`
- `StuckVmCleanupPlan`

Sentence:

> `firkin-hygiene` hides how runtime leftovers are discovered, classified, and cleaned up.

#### `firkin-trace`

Owns:

- `BenchmarkSample`
- `BenchmarkMetricKind`
- `BenchmarkUnit`
- `Recorder`
- span, sampler, checkpoint, and shared tag primitives

Sentence:

> `firkin-trace` hides how lifecycle moments and gauge readings become typed measurement samples.

It must not own aggregation, SLO gates, evidence artifact I/O, or domain types
such as `Pod`, `Container`, or `Sandbox`.

#### `firkin-evidence`

Owns:

- `BenchmarkSummary`
- lifecycle/overhead SLO target models
- evidence artifacts
- soak scenario/report models

Sentence:

> `firkin-evidence` hides how runtime claims are measured and validated.

It depends on `firkin-trace` for primitive sample types. It does not re-export
`BenchmarkSample`, `BenchmarkMetricKind`, or `BenchmarkUnit`; callers that
construct samples import them from `firkin-trace` or the top-level `firkin`
facade.

`BenchmarkSummary` stores count, p50, p90, p95, p99, and max. SLO gates stay
p95-based for the current lifecycle/overhead targets; reports expose the broader
tail shape so benchmark output is useful before new metric families land.

#### `firkin-benchmark`

Owns:

- named benchmark suites and suite matrices
- benchmark run configuration
- benchmark runner orchestration over runtime/single-node/template surfaces
- evidence artifact production from recorded samples
- CLI-facing benchmark report/validate/run workflows
- product soak runner configuration and execution

Sentence:

> `firkin-benchmark` hides how named benchmark suites exercise Firkin and turn
> traces into validated evidence artifacts.

It depends on `firkin-evidence` for summaries/SLOs/artifact schemas and on
`firkin-trace` for primitive samples. It may depend on high-level runtime and
single-node crates because it is an experiment runner. It must not be imported
by runtime, template, core, substrate replacement crates, E2B crates, VMM, OCI,
or vminitd-client.

Do not put benchmark-suite execution in `firkin-cli`. The CLI is only the
operator interface. Do not put suite policy in `firkin-runtime`; runtime emits
facts but does not decide which experiments prove the product.

#### Move out of current `substrate`

Move these elsewhere:

- `TemplateBuildJob` into `firkin-template`
- `FreshnessSyncGate` into `firkin-template`

Reason: both are template lifecycle concepts, not generic substrate facts.

#### Exact breakup of current `firkin-substrate`

This split should follow the actual clusters already present in
`crates/substrate/src/lib.rs`.

##### Move to `firkin-admission`

- `ResourceBudget`
- `CapacityError`
- `CapacityLedger`
- `WarmPoolKey`
- `WarmPoolEntry`
- `WarmPoolLedger`
- `ActiveCapacityAdmissionPlan`
- `ActiveQueuePolicy`
- `ActiveBackpressureDecision`
- `BackpressureRejection`
- `ActiveBackpressurePlan`
- `WarmPoolReplenishmentTarget`
- `WarmPoolReplenishmentSkipReason`
- `WarmPoolReplenishmentSkip`
- `WarmPoolReplenishmentPlan`

Module tree:

```text
admission/
  budget.rs
  capacity.rs
  warm_pool.rs
  active.rs
  replenish.rs
```

##### Move to `firkin-artifacts`

- `SnapshotArtifactKind`
- `RecordedPodMembership`
- `RecordedPodContainer`
- `RecordedPodVolumeMount`
- `SnapshotArtifactManifest`
- `SnapshotArtifactManifestError`
- `SnapshotArtifactIntegrity`
- `SnapshotArtifactIntegrityError`
- `ContinuationSnapshotReason`
- `ContinuationSnapshotPlan`

Module tree:

```text
artifacts/
  manifest.rs
  integrity.rs
  pod_membership.rs
  continuation.rs
```

##### Move to `firkin-evidence`

- `RequiredLifecycleLatencyTarget`
- `default_lifecycle_latency_slo_targets`
- `RequiredFirkinOverheadMetric`
- `default_firkin_overhead_slo_targets`
- `BenchmarkSummaryError`
- `BenchmarkSummary`
- `BenchmarkSloTarget`
- `BenchmarkSloGateError`
- `BenchmarkSloGateReport`
- `BenchmarkEvidenceError`
- `BenchmarkEvidenceReport`
- `BenchmarkOverheadEvidenceReport`
- `BenchmarkEvidenceArtifact`
- `BenchmarkOverheadEvidenceArtifact`
- `SoakStep`
- `SoakScenario`
- `SoakStepEvidence`
- `SoakEvidenceReport`
- `SoakCleanupEvidence`
- `SoakEvidenceError`
- `SoakEvidenceGateReport`
- `SoakEvidenceArtifact`

Module tree:

```text
evidence/
  benchmark.rs
  slo.rs
  lifecycle.rs
  overhead.rs
  soak.rs
```

##### Move to `firkin-hygiene`

- `RestartResourceKind`
- `RestartStateRecord`
- `ReconciliationDecision`
- `ReconciliationPlanEntry`
- `ReconciliationPlan`
- `HostRuntimeScan`
- `StuckVmObservation`
- `StuckVmCleanupDecision`
- `StuckVmCleanupPlanEntry`
- `StuckVmCleanupPlan`
- `ArtifactGcError`
- `ArtifactGcPlan`
- `ArtifactGcReport`
- `LogRotationError`
- `LogRotationPlan`
- `LogRotationReport`

Module tree:

```text
hygiene/
  restart.rs
  reconciliation.rs
  host_scan.rs
  stuck_vm.rs
  artifact_gc.rs
  log_rotation.rs
```

### Keep and sharpen: `firkin-template`

`firkin-template` should remain a crate, but it should own more of what it
already semantically owns.

It should own:

- template repo checkout description
- setup and cache-warm command descriptions
- freshness sync gates
- host-side freshness executor
- snapshot sink trait for template materialization

It should depend on:

- `firkin-artifacts`
- `firkin-trace`
- `firkin-types`

It should not depend on `firkin-evidence`. Template code may emit
`BenchmarkSample`s for checkout/setup/freshness phases, but it does not
summarize evidence or enforce SLOs.

It must not depend on:

- `firkin-core`
- `firkin-runtime`
- `firkin-e2b-*`

Sentence:

> `firkin-template` hides how a repo becomes a prepared, freshness-aware template snapshot candidate.

#### Exact internal cleanup for `firkin-template`

Move in from old `substrate`:

- `TemplateBuildJob`
- `FreshnessSyncPhase`
- `FreshnessSyncGate`

Then split `crates/template/src/lib.rs` into:

```text
template/
  model.rs
  snapshot.rs
  build.rs
  freshness.rs
  command.rs
```

Ownership:

- `model.rs`
  - `TemplateBuildJob`
  - template repo/setup/cache-warm typed models
- `snapshot.rs`
  - `TemplateSnapshotSink`
  - `SnapshotSinkError`
- `build.rs`
  - `TemplateBuildExecutor`
  - `PreparedTemplateCheckout`
  - `TemplateBuildReport`
- `freshness.rs`
  - `FreshnessSyncPhase`
  - `FreshnessSyncGate`
  - `FreshnessSyncExecutor`
  - `FreshnessSyncReport`
- `command.rs`
  - host git/shell helper functions

Rule: this crate owns host checkout/setup/freshness behavior, not live core VM
execution.

### Split in two levels: `firkin-runtime` and `firkin-single-node`

The current runtime story is wrong in one specific way: the crate contains both
reusable runtime orchestration and a concrete local backend subtree.

The existing `single_node` module is already a clue that the crate boundary is
late.

#### `firkin-runtime`

Keep this crate, but narrow its charter.

It should own reusable runtime orchestration that composes:

- `firkin-core`
- `firkin-template`
- `firkin-admission`
- `firkin-artifacts`
- `firkin-hygiene`
- `firkin-trace`
- `firkin-e2b-contract` where compatibility traits are needed

Examples that belong here:

- snapshot restore orchestration
- continuation snapshot capture/restore
- warm-pool maintenance and checkout
- runtime lifecycle trace sample emission
- runtime preflight
- runtime-owned hygiene execution wrappers
- session termination traits
- host-process termination traits

Sentence:

> `firkin-runtime` hides how low-level Firkin capabilities are composed into reusable runtime workflows.

Crucial refusal:

- no control-plane state registry
- no local backend JSON store
- no local scheduler service object
- no HTTP server construction
- no “product-neutral backend” wrapper
- no evidence artifact writer or benchmark-suite runner

#### Exact internal cleanup for `firkin-runtime`

After `single_node` moves out, `crates/runtime/src/lib.rs` should be broken
into:

```text
runtime/
  preflight.rs
  disk.rs
  measurement.rs
  hygiene.rs
  warm_pool.rs
  session.rs
  interactive.rs
  restore.rs
  continuation.rs
  template_build.rs
  adapter.rs
```

Ownership:

- `preflight.rs`
  - `RuntimePreflight`
  - `RuntimePreflightReport`
  - `RuntimePreflightError`
- `disk.rs`
  - `DiskPressureProbe`
  - `HostDiskPressureProbe`
  - `HostDiskPressureProbeError`
  - `DiskPressureError`
  - `DiskPressureReport`
  - `RuntimeDiskPressureGuard`
- `measurement.rs`
  - runtime lifecycle sample helpers
  - runtime-owned metric names that are not global trace phase constants
  - no SLO gates, evidence artifacts, or suite policy
- `hygiene.rs`
  - `RuntimeHostScanner`
  - `RuntimeRestartRecovery`
  - `RuntimeFilesystemReconciler`
  - `RuntimeHostProcessStuckVmCleaner`
  - `CommandHostProcessTerminator`
  - `RuntimeSnapshotArtifactGc`
  - `RuntimeLogRotation`
  - `RuntimeHygieneMaintenance`
  - `RestartReconciliation`
  - `RuntimeStuckVmCleanup`
- `warm_pool.rs`
  - all warm-pool launcher/checkout traits
  - `RuntimeWarmPoolMaintain`
  - `RuntimeWarmPoolCheckout`
  - `RuntimeSnapshotWarmPool`
  - `RuntimeWarmPoolSupervisor`
  - `RuntimeWarmPoolService`
- `session.rs`
  - `RuntimePortRouter`
  - `RuntimeSessionStop`
  - `RuntimeReadinessProbe`
  - `RuntimeCommandRunner`
  - core-backed impls for live `firkin_core::Container<Streams>`
- `interactive.rs`
  - `RuntimeInteractiveProcess`
  - `RuntimeInteractiveProcessRunner`
  - `CoreInteractiveProcess`
  - `CoreInteractivePtyProcess`
- `restore.rs`
  - `PersistedContainerSnapshotState`
  - `SnapshotRestoreRequest`
  - `SnapshotSessionLauncher`
  - `CoreSnapshotSessionLauncher`
  - `ActiveSessionReservation`
  - `SnapshotRestoreReport`
  - `RuntimeSnapshotRestore`
  - `RuntimeCubeSandboxCreate*`
- `continuation.rs`
  - `RuntimeContinuationSnapshotSource`
  - `CoreContainerSnapshotSink`
  - `ContinuationSnapshot*`
  - `RuntimeCubeSandboxFollowup*`
- `template_build.rs`
  - `TemplateBuildRuntimeRequest`
  - `TemplateCommandRunner`
  - `CoreTemplateCommandRunner`
  - `RuntimeTemplateBuildSnapshot`
  - `RuntimeTemplateBuildReport`
- `adapter.rs`
  - `FirkinRuntimeAdapter`
  - `FirkinWarmTemplateMaintainer`
  - adapter-only envd/code-interpreter glue

The key rule is that `firkin-runtime` remains reusable orchestration plus the
compatibility bridge adapter, but not a persisted single-node backend.

#### Exact extraction for `firkin-benchmark`

Create `crates/benchmark/` with:

```text
benchmark/
  suite.rs
  targets.rs
  report.rs
  runner.rs
  lifecycle.rs
  overhead.rs
  soak.rs
  artifact.rs
```

Move here:

- `RuntimeBenchmarkEvidenceWriter`
- `RuntimeOverheadEvidenceWriter`
- `RuntimeBenchmarkEvidenceError`
- `RuntimeProductSoakConfig`
- `RuntimeProductSoakRunner`
- CLI benchmark report/validate/run orchestration that is more than argument
  parsing

Own here:

- `BenchmarkSuite`
- `BenchmarkRunConfig`
- `BenchmarkRunReport`
- `BenchmarkTargetManifest`
- lifecycle/overhead run matrix definitions
- future `AgentCore`, `DiskMatrix`, `MemoryReclaim`, and `Density` suite
  runners

Rule: `firkin-benchmark` is the only crate allowed to decide what benchmark
suite is enough to prove a product claim. `firkin-evidence` validates the
artifact shape; `firkin-trace` records raw samples; lower runtime crates just
emit samples.

#### `firkin-single-node`

Create a new crate by extracting the current `runtime/src/single_node/` module
tree and the concrete stateful host-backend concerns that belong with it.

It should own:

- `SingleNodeConfig`
- `SingleNodeBackend`
- durable state store
- local scheduler object
- active/snapshot records
- single-node disk guard wiring
- domain proxy construction for one host
- concrete Apple/VZ runtime driver

Sentence:

> `firkin-single-node` hides how one local host runs, persists, and schedules Firkin-backed sandboxes.

This crate may depend on:

- `firkin-runtime`
- `firkin-admission`
- `firkin-artifacts`
- `firkin-hygiene`
- `firkin-e2b-contract`
- `firkin-e2b-server`

This crate must not be imported by `firkin-core`, `firkin-template`, or the new
split substrate crates.

Why a new crate instead of keeping the module:

1. It has its own distinct audience.
2. It has its own persistence model.
3. It has its own concrete backend semantics.
4. It is exactly the kind of thing that silently drags product concerns
   downward if left inside a general runtime crate.

#### Exact extraction of current `runtime/src/single_node`

Create `crates/single-node/` with:

```text
single-node/
  config.rs
  error.rs
  model.rs
  scheduler.rs
  disk.rs
  state.rs
  proxy.rs
  backend.rs
  driver.rs
  apple_vz.rs
```

Move here:

- `SingleNodeScheduler`
- `SingleNodeConfig`
- `SandboxResources`
- `SingleNodeCreateRequest`
- `SandboxSession`
- `SnapshotRecord`
- `TemplateMetadata`
- `CommandRequest`
- `CommandOutput`
- `LogEvent`
- `RuntimeIdentity`
- `StateStore`
- `FileStateStore`
- `LogStore`
- `PortRegistry`
- `DomainProxyAdapter`
- `SingleNodeBackend`
- `RuntimeDriver`
- `AppleVzLocalRuntimeDriver`

Prescriptive calls:

- delete the empty `orchestration.rs` placeholder instead of preserving fake
  structure
- keep generic runtime traits in `firkin-runtime`, not here

### Split: `firkin-e2b`

`firkin-e2b` is currently the largest boundary smell in the workspace.

It presently mixes:

- wire DTOs
- compatibility traits
- local control-plane backend logic
- domain proxy server
- envd-compatible process/filesystem server
- local registries and JSON state
- pod product types

That should become three crates.

#### `firkin-e2b-wire`

Owns only request/response and compatibility DTOs:

- sandbox, snapshot, template, volume, metrics DTOs
- control-plane method/request/response enums
- pod control-plane DTOs

Sentence:

> `firkin-e2b-wire` hides the exact E2B/Cube-compatible wire shape.

This crate may depend only on `firkin-types`.

Exact module tree:

```text
e2b-wire/
  sandbox.rs
  snapshot.rs
  template.rs
  volume.rs
  metrics.rs
  logs.rs
  pods.rs
  control_plane.rs
```

Move here:

- sandbox/snapshot/template/volume/metric/log DTOs
- control-plane method/request/response types
- pod control-plane DTOs now in `pods.rs`

#### `firkin-e2b-contract`

Owns compatibility-side traits and neutral runtime-facing records:

- `RuntimeAdapter`
- `EnvdProcessAdapter`
- `EnvdFilesystemAdapter`
- capability sets
- runtime-facing start/followup/template request records
- `PortTarget`
- adapter error types

Sentence:

> `firkin-e2b-contract` hides what a runtime must provide to satisfy the local E2B compatibility layer.

This crate depends on:

- `firkin-e2b-wire`
- `firkin-types`

It must not own HTTP servers or state persistence.

Exact module tree:

```text
e2b-contract/
  capability.rs
  runtime.rs
  envd_process.rs
  envd_filesystem.rs
  port.rs
  template.rs
```

Move here:

- `RuntimeCapabilitySet`
- `PreparedTemplate`
- `PreparedTemplateArtifactIntegrity`
- `StartSandboxRequest`
- `FollowupSnapshot`
- `RuntimeTemplateBuild`
- `RuntimeSandbox`
- `PausedSandbox`
- `SandboxExpirationAction`
- `SandboxExpiration`
- `SnapshotRef`
- `PortTarget`
- `PortProxyIo`
- `RuntimeAdapter`
- `EnvdProcess*` adapter traits and runtime-facing records
- `EnvdFilesystem*` adapter traits and runtime-facing records

#### `firkin-e2b-server`

Owns local server/process compatibility machinery:

- `LocalRuntimeBackend`
- `ControlPlaneHttpServer`
- `DomainProxyHttpServer`
- `EnvdProcessHttpServer`
- lifecycle scheduler for the local control plane
- local registries and persistence for compatibility state

Sentence:

> `firkin-e2b-server` hides how the local E2B-compatible control plane and data-plane servers run.

This crate depends on:

- `firkin-e2b-wire`
- `firkin-e2b-contract`
- `firkin-types`

It must not depend on `firkin-core` or `firkin-vmm` directly. Concrete runtime
implementations stay on the other side of `RuntimeAdapter`.

Exact module tree:

```text
e2b-server/
  control_plane.rs
  domain_proxy.rs
  envd_http.rs
  state.rs
  lifecycle.rs
  registry/
    sandbox.rs
    template.rs
    volume.rs
    pod.rs
  routes/
    sandbox.rs
    template.rs
    volume.rs
    pod.rs
```

Move here:

- `ControlPlaneHttpServer`
- `DomainProxyHttpServer`
- `DomainProxyTlsIdentity`
- `DomainProxyTlsError`
- `EnvdProcessHttpServer`
- `HostEnvdAdapter`
- `LifecycleClock`
- `SystemLifecycleClock`
- `LifecycleScheduler`
- `LocalRuntimeBackend`
- `LocalRuntimeState`
- `LocalRuntimeStateStoreError`
- `SandboxRuntimeConfig`
- `SandboxRecord`
- `SnapshotRecord`
- `LocalSandboxRegistry`
- `BackendError`
- `VolumeRecord`
- `VolumeContentEntry`
- `LocalVolumeRegistry`
- `TemplateRecord`
- `LocalTemplateRegistry`
- `SandboxRoutes`
- `PodRoutes`
- `LocalPodRegistry`

Rule: this crate owns compatibility servers and local control-plane persistence,
even when those are only used in local-dev or proof paths today.

### Keep: `firkin`

Keep as the narrow facade crate.

It should re-export curated entry points, capability surfaces, and the stable
top-level library story. It must not become a convenience pile for all internal
types.

### Keep: `firkin-cli`

Keep as a thin dev/operator binary.

It may depend on higher-level crates directly when needed, but it should not be
the reason crate boundaries stay broad.

## Forbidden edges

These are the important graph refusals.

- `firkin-types` must not depend on any workspace crate.
- `firkin-trace` must not depend on any workspace crate.
- `firkin-vsock` must not depend on `vmm`, `core`, `runtime`, or E2B crates.
- `firkin-vmm` must not depend on `core`, `runtime`, `single-node`, `template`,
  or any E2B crate.
- `firkin-vminitd-client` must not depend on `core`, `runtime`, `template`, or
  E2B crates.
- `firkin-oci` must not depend on `core`, `runtime`, `template`, or E2B crates.
- `firkin-core` must not depend on `runtime`, `single-node`, `template`,
  `admission`, `artifacts`, `hygiene`, `evidence`, `benchmark`, or E2B crates.
- `firkin-template` must not depend on `core`, `runtime`, `single-node`, `vmm`,
  `evidence`, `benchmark`, or E2B crates.
- `firkin-admission`, `firkin-artifacts`, `firkin-hygiene`, and
  `firkin-evidence` must not depend on `core`, `runtime`, `single-node`, or
  E2B crates.
- `firkin-runtime` must not own HTTP wire DTOs or persisted local control-plane
  state, and must not depend on `firkin-evidence` or `firkin-benchmark`.
- `firkin-single-node` must not be imported by lower runtime/library crates.
- `firkin-benchmark` must not be imported by lower runtime/library crates.
- `firkin-e2b-wire` must not depend on runtime/mechanics crates.
- `firkin-e2b-contract` must not depend on runtime/mechanics crates.
- `firkin-e2b-server` must not depend directly on `core`, `vmm`, or
  `single-node` concrete implementation crates.

## Type and trait placement rules

These rules matter more than crate names.

### Type rules

- VM launch policy, pod/container resource policy, and runtime lifecycle policy
  are different type families. Do not collapse them into one omnivorous
  `VmSpec`.
- Domain/runtime records are not E2B wire DTOs.
- Artifact manifests are not template jobs.
- Local persisted state records are not runtime orchestration requests.
- Trace samples are not evidence summaries.
- Benchmark suites are not runtime APIs.

### Trait rules

- `TemplateSnapshotSink` belongs with `firkin-template`, because template
  execution is the consumer of snapshot materialization.
- runtime session launcher/terminator traits belong with `firkin-runtime`,
  because that crate owns the orchestration laws.
- E2B compatibility traits belong with `firkin-e2b-contract`, not in `runtime`
  and not in `single-node`.
- Benchmark runner traits belong with `firkin-benchmark`, because that crate
  owns suite execution laws.

Do not add a generic “ports” or “contracts” crate.

## Test layout

The split should also sharpen test ownership.

### `firkin-admission`

- law tests for budget arithmetic
- admission/rejection examples
- warm-pool promotion and eviction regressions

### `firkin-artifacts`

- manifest roundtrip tests
- integrity hash/size laws
- continuation/base artifact classification regressions

### `firkin-hygiene`

- plan-construction tests
- filesystem executor scenario tests
- restart and stuck-VM regressions

### `firkin-evidence`

- summary/SLO law tests
- required-metric coverage tests
- regression fixtures for invalid evidence shapes
- p50/p90/p95/p99/max summary reporting tests
- artifact roundtrip tests for lifecycle, overhead, and soak evidence

### `firkin-trace`

- recorder disabled-path allocation tests
- span outcome/drop semantics
- sampler close-and-drain and no-runtime behavior
- tag cardinality and overflow policy tests

### `firkin-benchmark`

- suite manifest tests for P0 target coverage
- benchmark runner tests with fake runtime/single-node adapters
- evidence generation tests from recorded samples
- CLI workflow tests for report/validate/run command behavior

### `firkin-template`

- host checkout/setup/freshness scenarios
- snapshot sink failure handling

### `firkin-runtime`

- orchestration scenario tests with mock launchers/sinks/terminators
- active/warm/snapshot interaction regressions

### `firkin-single-node`

- persistence and restart state tests
- scheduler/state/backend integration tests
- Apple/VZ-backed live smokes where necessary

### `firkin-e2b-*`

- wire roundtrips in `wire`
- adapter law tests in `contract`
- control-plane/domain/envd server scenarios in `server`

## Migration order

This should land in this order.

0. Treat the current `firkin-trace` extraction and p50/p90/p95/p99/max evidence
   schema as already landed. Do not move benchmark primitives again.
1. Add the target dependency allowlist in CI before large moves. It may start
   in advisory mode for the current graph, but the target allowlist in this doc
   is the contract for the final split.
2. Split `firkin-substrate` into `firkin-admission`, `firkin-artifacts`,
   `firkin-hygiene`, and `firkin-evidence`; delete `firkin-substrate` from the
   workspace after consumers move. `firkin-evidence` consumes
   `BenchmarkSample`/`BenchmarkMetricKind`/`BenchmarkUnit` from `firkin-trace`
   without re-exporting them.
3. Move `TemplateBuildJob` and `FreshnessSyncGate` into `firkin-template`.
4. Create `firkin-benchmark`; move benchmark evidence writer wrappers, product
   soak runner, suite policy, and CLI benchmark workflow logic there.
5. Update `firkin-template`, `firkin-runtime`, `firkin-single-node`, `firkin`,
   and `firkin-cli` to consume the split crates.
6. Extract `firkin-e2b-wire`.
7. Extract `firkin-e2b-contract`.
8. Extract `firkin-e2b-server`.
9. Extract `firkin-single-node` from `runtime/src/single_node/`.
10. Shrink `firkin-runtime` back to reusable orchestration and trace sample
    emission only.
11. Do the large internal module split in `firkin-core`.
12. Do internal module cleanup in `oci`, `vminitd-client`, and `vmm`.

## Immediate follow-on docs

After this spec, the next two useful artifacts are:

1. a dependency allowlist file for CI
2. an implementation plan that does the split in migration order with exact
   file moves and test updates

Without those, this stays a good opinion. With them, it becomes executable.
