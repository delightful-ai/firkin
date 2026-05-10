# Firkin pod support — design

Status: design draft, brainstormed and reviewed 2026-05-04. Implementation
plan to follow.

> Storage prerequisite correction, 2026-05-06:
> Runtime-added pod containers must not depend on
> `VZVirtualMachine.attachDevice:completionHandler:` for storage. The current
> implementation path is a preboot pod-store disk plus `VmRootfs::GuestPath`.
> See `docs/plans/2026-05-06-firkin-storage-pod-prerequisites.md`.

## Context

Firkin runs Linux containers inside Apple Virtualization.framework micro-VMs.
The production Cube/E2B mapping today is **one sandbox = one VM = one
container** (`docs/specs/rust_rewrite/07-production-substrate-goal.md:27-29`).
Multi-container per VM is exposed as an "advanced substrate mode" escape
hatch via `ContainerBuilder<OnVm>` / `ContainerBuilder<OnVmArc>`
(`crates/core/src/lib.rs:1409-1455`), with rootfses required to be
pre-declared at `VmConfig::block_device(...)` time (D-019, D-022, D-023).
Acceptance check `#14` in the substrate goal explicitly defers Cube-level
multi-container support until "lifecycle, snapshot, and isolation semantics
are designed for it."

Critical local prior art: **`Sources/Containerization/LinuxPod.swift`** (925
lines) is a Swift implementation of exactly this concept against the same
Apple-VZ substrate. It defines the lifecycle, namespace policy, pause-process
pattern, and per-container override semantics that this Rust design mirrors.
The two implementations should be observably equivalent so they can act as
evidence for each other.

This document specifies the Rust design.

## Goal

Add a `Pod` aggregate to `firkin-core` that turns the existing multi-container
escape hatch into a first-class, ergonomic primitive: one VM, N coordinated
containers, shared kernel + network namespace, opt-in shared PID namespace
via the pause-container pattern, shared volumes via virtiofs and tmpfs.

Lift D-019 in a separate workstream so containers can be added to / removed
from a running pod on macOS 26+. Defer the E2B control-plane surface
(option-3 territory in the brainstorm) and pod-aware substrate
`WarmPoolKey`/`ContinuationSnapshotPlan` to follow-on milestones, but record
pod membership in a forward-compatible shape so restore can land later
without a snapshot-format migration.

## Workloads

**Workload A — code interpreter (Jupyter-style cells).** *Not a pod.* Each
cell is `Container::exec(ExecConfig)` into a long-lived container that holds
the kernel/Python state. Scale-up is more sandboxes from the warm pool
(p95 ≤ 100ms checkout per `crates/cli/src/main.rs:314-319`). Listed here only
to document what pods are *not* for.

**Workload B — many parallel cargo builds with a shared cache.** *The pod
case.* One VM, N containers, each builds a different crate. They share an
`sccache` directory (or equivalently, a Bazel remote cache) over virtiofs.
Per `docs/specs/rust_rewrite/04-library-surface/01-container-surface.md:699`,
naive `cargo build --target-dir=<shared_virtiofs>` corrupts even on one
kernel — so the example here is *shared sccache + per-container target/`,
not a shared `target/`. The pod owns the sccache directory once; each
container gets its own ephemeral target directory.

**Workload C — agent + sidecar that talks on `localhost`.** *The pod case
that needs runtime add.* An Inspect-style agent decides mid-session to
launch a logging or monitoring sidecar that needs to reach the agent
container on `127.0.0.1:<port>`. Two separate VMs cannot do this without a
loopback bridge; same-VM pods get it for free because they share the VM's
network namespace. Requires `Pod::add_container` (PR 3).

## Why same-VM, not N coordinated VMs

The brainstorm considered a "pod = N coordinated VMs" coordinator and
rejected it. Reasons same-VM is correct:

1. **Shared VM network namespace.** Containers in one VM share the VM's
   network stack — `localhost` reachability between pod containers is a
   property of the kernel, not something we plumb. Two VMs require a
   vsock/loopback bridge.
2. **Optional shared kernel PID namespace via the pause-container pattern.**
   A second VM cannot share Linux kernel namespaces with the first;
   same-VM can. Default is per-container PID namespaces (matching K8s
   convention and `crates/core/src/lib.rs:3471-3475`); opt-in shared PID
   uses the same pause-container pattern Swift `LinuxPod` uses. **Only
   PID is shared** — IPC, mount, UTS, and cgroup namespaces remain fresh
   per container even when `share_process_namespace = true`, matching
   Swift `LinuxPod.swift:566-585` (each non-pause container declares
   fresh `.cgroup/.ipc/.mount/.uts` and conditionally joins the pause
   container's `.pid`).
3. **Per-VM cost amortization.** Per-sandbox host overhead is
   `per_sandbox_host_rss` p95 ≤ 64 MiB and `warm_snapshot_restore` p95 ≤
   1.5s (`crates/cli/src/main.rs:271-326`). N containers in one VM amortize
   the per-VM cost; N VMs do not.
4. **Atomic VM snapshot.** A VZ snapshot captures the VM's RAM and device
   state in one image. For workloads that want to checkpoint a coordinated
   group of containers (less common; not workload B or C, more
   workload-D-style "save the whole agent loop"), this is materially
   simpler than coordinating N independent snapshots. *This is a property,
   not a load-bearing claim* — we ship pods even if no current workload
   needs atomic-multi-container snapshot.

Trade-offs accepted:

- Pod membership is fixed at create time *unless* macOS 26+ (D-019 lift).
  Workloads that know N up front (build farms, batch jobs) are fine; runtime
  add is the workload-C path and lands in PR 3.
- Sharing a VM means containers cannot have independent kernel versions or
  kernel module sets. For our use cases this is irrelevant.
- New volumes cannot be added after pod boot (virtiofs runtime attach is
  unverified on Apple VZ). PR 3's `Pod::add_container` accepts a new rootfs
  block device but cannot introduce new pod volumes; the new container can
  only mount volumes already declared at `PodBuilder::spawn` time.

## Prior art

Sources used. None are added as crate dependencies; we borrow names and
patterns only.

| Layer | K8s/CRI prior art | Local Rust/Swift prior art | firkin |
|---|---|---|---|
| Container runtime | OCI (runc) | youki, `firkin-core::Container` | exists |
| Pod sandbox | CRI `PodSandbox` (containerd, CRI-O) | **Swift `LinuxPod`** (this repo); Kata shim, firecracker-containerd | `firkin-core::Pod` (this design) |
| Cluster control | K8s `Pod` resource | kube-rs `Api<Pod>` | `firkin-e2b::POST /pods` (deferred) |

### Swift `LinuxPod` (primary local prior art)

`Sources/Containerization/LinuxPod.swift`. The most important reference for
this design: same Apple-VZ substrate, same problem, already solved in Swift.
Specifically copied:

- **Lifecycle**: `initialized → created` (pod phase, line 126-130);
  containers move `registered → created → started → stopped → errored`
  (line 102-106). Containers are added via `addContainer` *before*
  `create()` is called (line 278-279); no runtime-add in Swift either. PR 1
  matches this exactly. PR 3 extends past Swift.
- **Pod-level configuration with per-container overrides** (line 39-65):
  `cpus`, `memoryInBytes`, `interfaces`, `bootLog` are pod-level only;
  `hostname`, `dns`, `hosts`, `sysctl` exist at both levels with
  container-level taking precedence. Per-container `cpus` and
  `memoryInBytes` may exceed pod totals (intentional oversubscription, line
  71-73).
- **`shareProcessNamespace: Bool = false`** (line 52). When `true`, a pause
  container is spawned running `/sbin/vminitd pause` with fresh
  cgroup/ipc/mount/pid/uts namespaces (line 357-405); other containers
  declare their *own* fresh cgroup/ipc/mount/uts namespaces and only
  *join* the pause container's PID namespace via `LinuxNamespace(type:
  .pid, path: "/proc/<pause_pid>/ns/pid")` (line 566-585). Only PID is
  shared, not IPC. PR 1 mirrors this pattern with the same
  `vminitd pause` invocation.
- **Per-container `useInit: Bool`** (line 88) for `/.cz-init`-wrapped entry
  points. Adopted as `PodContainerSpec.use_init: bool`.
- **Phase guards**: `validateForCreate()`, `createdState(operation:)` —
  state-machine guards that fail fast if an operation is invoked in the
  wrong phase. Mirrored in Rust with explicit `PodState` and method
  preconditions.

### CRI vocabulary

`pkg/apis/runtime/v1/api.proto`. Pod-sandbox verbs are `RunPodSandbox` /
`StopPodSandbox` / `RemovePodSandbox`; container-within-sandbox verbs are
`CreateContainer` / `StartContainer` / `StopContainer` / `RemoveContainer`.
`PodBuilder::spawn` corresponds to `RunPodSandbox` + N × (`CreateContainer`
+ `StartContainer`). `Pod::add_container` (PR 3) is one (`CreateContainer`
+ `StartContainer`). `Pod::stop` is N × `StopContainer` + `RemoveContainer`
+ `StopPodSandbox` + `RemovePodSandbox`. Doc-comments on each method cite
the CRI verb so a CRI-fluent reader recognizes the shape immediately.

### Kata sandbox FSM

`src/runtime/virtcontainers/sandbox.go`. Four states — `Ready`, `Running`,
`Paused`, `Stopped` — with `s.state.ValidTransition()` checks (~line 2014).
We adopt the same four states for `PodState`. CRI's two-state
`READY`/`NOTREADY` is too coarse; Kata's matches our needs (Apple VZ has
explicit pause/resume).

### Kata hotplug + firecracker-containerd attach (PR 2 / PR 3)

`Sandbox.HotplugAddDevice` (~line 2150): `(ctx, device, DeviceType) -> error`,
no rollback at the sandbox level — caller decides recovery. We are more
aggressive: PR 3 wraps attach-then-spawn in an RAII `BlockDeviceGuard` that
detaches on drop unless committed.

firecracker-containerd's `PatchAndMount` (`runtime/drive_handler.go`): three
phases — jail-expose → host API call → guest-side mount with a 100×10ms
retry loop for the timing race where the guest doesn't see the device
immediately. Apple VZ has the same race shape; PR 2 adopts the same retry
pattern in `attach_block_device`.

### `k8s-openapi` field-name vocabulary

`k8s_openapi::api::core::v1::{Container, Volume, VolumeMount, EnvVar,
ResourceRequirements}` on docs.rs. Used as a *naming* source so Rust devs
get instant LSP-completion recognition. Mirrored fields: `name`, `command`,
`args`, `env`, `working_dir`, `volume_mounts`, `resources`,
`security_context`. Trimmed: `mount_propagation`, `recursive_read_only`,
`sub_path_expr`, the 30-source `Volume::*` enum cardinality (we expose two:
`HostPath` virtiofs, `EmptyDir` tmpfs), and all K8s scheduler-control
fields (we are single-host).

We do **not** depend on `k8s-openapi`.

## Scope

In:

- `firkin-core::pod` module (PR 1): `Pod`, `PodBuilder`, `PodSpec`,
  `PodContainerSpec`, `PodVolume`, `PodVolumeSource`, `VolumeMount`,
  `EnvVar`, `ResourceRequirements`, `SecurityContext`, `PodId`, `PodState`,
  `PodError`. Pre-declared containers only. Built on top of existing
  `OnVm`/`OnVmArc` builders. Snapshot path returns a core-shaped result;
  the substrate manifest extension is produced by the composition layer
  (see "crate graph" below).
- D-019 lift in `firkin-vmm` (PR 2): `VirtualMachine<Running>::attach_block_device`
  and `detach_block_device` over `VZVirtualMachine.attachDevice:completionHandler:`.
  Capability advertised dynamically (not via the existing static
  `apple_local_runtime_capabilities()`); pre-26 macOS hard-fails before any
  VZ call.
- `Pod::add_container` / `Pod::remove_container` in `firkin-core::pod` (PR 3),
  using PR 2's primitives. Restriction: new containers can only mount
  *already-declared* pod volumes (no runtime virtiofs attach).
- Substrate manifest extension (`firkin-substrate`): optional
  `pod_membership: Option<RecordedPodMembership>` field on
  `SnapshotArtifactManifest`. Constructed by `firkin-runtime`
  (composition crate), not by `firkin-core`. Read by no consumer yet —
  written defensively so a future pod-aware restore has the data it needs.
- `firkin` facade re-exports `firkin_core::pod::*`.

Out (deferred; referenced where they constrain this design):

- E2B control-plane surface (`POST /pods`, `PodRecord`, etc.).
- Substrate `WarmPoolKey` / `ContinuationSnapshotPlan` extensions for pods.
- `Rootfs::OciBundle` on `OnVm` builders (D-023 lift). `Pod::add_container`
  takes pre-assembled rootfs paths; OCI assembly at attach time is its own
  follow-on.
- Virtiofs runtime attach (adding new pod volumes after spawn).
- `init_containers`, `restart_policy`, probes, lifecycle hooks. Defined in
  K8s, not in Swift `LinuxPod`, not in this milestone.
- Pod-level network policy. Inherits the existing E2B `network_policy`
  hard-fail posture.

## Workstream split and ship order

Three independent PRs. Each lands signed live VZ smoke. Each is useful on
its own.

**PR 1 — `firkin-core::pod` over pre-declared containers.**
Workstream 2 from the brainstorm. No D-019 lift, no runtime add. Covers
workload B end-to-end. Snapshot writes nothing into the substrate manifest
(see crate graph); composition-layer code in `firkin-runtime` is what
constructs `RecordedPodMembership` from the pod's state. Ship the substrate
manifest extension *with* PR 1 (separate commit if needed) so the data
shape is in place before PR 2/3 land.

**PR 2 — `firkin-vmm` runtime block-device attach.**
Workstream 1. The VZ delegate work: `VirtualMachine<Running>::attach_block_device`
/ `detach_block_device`, completion-handler dispatch-queue plumbing, the
guest-side retry loop, the macOS-26 dynamic capability check. No
`firkin-core` API surface change. Standalone PR so the VZ work is reviewable
in isolation.

**PR 3 — `Pod::add_container` / `Pod::remove_container`.**
Workstream 3. Bolts the runtime-add API onto `Pod` using PR 2's primitives.
Uses RAII `BlockDeviceGuard` for attach-then-spawn rollback. Refuses to
mount volumes not declared at PodBuilder time.

Sequencing rationale: PR 1 ships value without VZ delegate risk. PR 2's
risk is contained to vmm. PR 3 is small once 1 and 2 land.

## Prerequisite spike (before PR 1)

Before any pod API code, validate the substrate properties this design
relies on. Two live VZ smokes, both using the *existing* `OnVm` builder, no
pod types yet:

1. **Multi-container shared kernel + network.** Spawn 2 `OnVm` containers
   in 1 VM. From container A, run a long-lived listener on
   `127.0.0.1:<port>`. From container B, `curl` it. Verify the request
   succeeds. Proves the VM-level network namespace is in fact shared
   between OnVm containers (it should be by construction; this is a
   sanity check before we build a Pod aggregate around the assumption).

2. **Atomic snapshot of multi-container in-flight state.** Spawn 2 `OnVm`
   containers. In each, spawn a long-lived process that holds a unique
   marker string in its address space and exposes a vsock query that
   returns it. Snapshot the VM. Stop the VM. Restore from snapshot. Query
   both containers' processes. Verify each returns its expected marker.
   *This must use long-lived processes with externally-queryable in-memory
   state* — exec-and-exit doesn't preserve in-flight state and would
   trivially pass without proving anything.

If smoke #1 fails, the same-VM thesis is broken at the substrate level —
revisit the design from scratch (likely option 4, multi-VM coordinator).

If smoke #2 fails, the *atomic-snapshot property* is gone but workloads B
and C still want same-VM pods (B for shared cache + per-VM cost
amortization; C for shared `localhost`). Drop the atomic-snapshot framing
from PR 1's snapshot section, ship the rest.

Cost: ~1 day. Worth it.

## API surface

```rust
// crates/core/src/pod.rs (new module). PR 1 shape — no add_container/remove_container.

/// A logical group of containers sharing one micro-VM.
///
/// Equivalent to a Kubernetes pod sandbox / Swift `LinuxPod`. The VM is the
/// boundary; containers inside share the VM's network namespace by virtue of
/// running in one kernel. Other namespaces (PID, IPC, mount, UTS, cgroup) are
/// per-container by default. Opt-in `share_process_namespace` shares **PID
/// only** (via the pause-container pattern); IPC remains per-container.
pub struct Pod { /* internal: vm, container map, attached BlockDeviceIds, volumes, pause process */ }

/// Pod lifecycle. Mirrors Kata's sandbox FSM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PodState {
    Ready,       // PodBuilder::spawn returned but no containers started yet (transient)
    Running,     // ≥1 container started
    Paused,      // VM paused via VZ pauseWithCompletionHandler
    Stopped,     // terminal
}

pub struct PodBuilder {
    /* VmConfigBuilder + Vec<PodContainerSpec> + Vec<PodVolume> + PodSpec metadata */
}

impl PodBuilder {
    pub fn new(id: PodId, vm_config: VmConfigBuilder) -> Self;

    /// Set pod-wide config (hostname, dns, hosts, share_process_namespace).
    pub fn spec(self, spec: PodSpec) -> Self;

    /// Add a container spec. Resolves to a pre-declared block device at spawn.
    /// Mirrors Swift `LinuxPod.addContainer`.
    pub fn container(self, spec: PodContainerSpec) -> Self;

    /// Declare a pod-level volume. Containers reference by `volume.name`
    /// from their `volume_mounts`.
    pub fn volume(self, volume: PodVolume) -> Self;

    /// Sugar: mount an *already-declared* pod volume into every container at
    /// the same guest path with the same options. Common for "everyone sees
    /// the sccache directory" patterns. The volume must have been added via
    /// `.volume(...)` already; otherwise spawn fails with VolumeNotFound.
    pub fn mount_volume_in_all_containers(
        self,
        volume_name: impl Into<String>,
        mount: VolumeMountOptions,
    ) -> Self;

    /// CRI: RunPodSandbox + N×(CreateContainer + StartContainer). Spawns
    /// the pause container first if `share_process_namespace`.
    pub async fn spawn(self) -> Result<Pod, PodError>;
}

impl Pod {
    pub fn id(&self) -> &PodId;
    pub fn state(&self) -> PodState;
    pub fn container(&self, name: &ContainerId) -> Option<Arc<Container>>;
    pub fn containers(&self) -> Vec<(ContainerId, Arc<Container>)>;

    /// VZ pause for the entire VM.
    pub async fn pause(&mut self) -> Result<(), PodError>;
    pub async fn resume(&mut self) -> Result<(), PodError>;

    /// Atomic snapshot of the VM. Returns the local snapshot output path;
    /// substrate-layer manifest construction is the composition crate's
    /// responsibility (see "Crate graph" section).
    pub async fn snapshot(&self, target: PathBuf) -> Result<PodSnapshotOutput, PodError>;

    /// CRI: StopPodSandbox + RemovePodSandbox. Best-effort; consumes self.
    pub async fn stop(self) -> Result<PodStopReport, PodError>;
}

/// Pod-wide configuration. Field names mirror Swift `LinuxPod.Configuration`.
pub struct PodSpec {
    pub hostname: Option<Hostname>,             // pod-level default; per-container override wins
    pub dns: Option<DnsConfig>,
    pub hosts: Option<HostsConfig>,
    /// When true, spawns a pause container (`/sbin/vminitd pause`) and joins
    /// every other container's PID namespace to it. Default false. Mirrors
    /// Swift `LinuxPod.Configuration.shareProcessNamespace`.
    pub share_process_namespace: bool,
}

/// Per-container spec. Field names mirror `k8s_openapi::api::core::v1::Container`
/// where they apply, plus Swift `LinuxPod.ContainerConfiguration` where K8s
/// has no analogue. We do not depend on `k8s-openapi`.
pub struct PodContainerSpec {
    pub name: ContainerId,                       // k8s `name`
    pub rootfs: PodRootfsSource,                 // pre-boot rootfs (resolved to BlockDeviceId at spawn)
    pub rootfs_logical_id: String,               // matches SnapshotArtifactManifest::logical_id pattern
    pub command: Vec<String>,                    // k8s `command`
    pub args: Vec<String>,                       // k8s `args`
    pub env: Vec<EnvVar>,                        // k8s `env`
    pub working_dir: Option<PathBuf>,            // k8s `working_dir`
    pub volume_mounts: Vec<VolumeMount>,         // k8s `volume_mounts`; references PodVolume.name
    pub resources: Option<ResourceRequirements>, // k8s `resources` (subset; per-container override)
    pub security_context: Option<SecurityContext>,
    /// Per-container hostname override. Falls back to PodSpec::hostname.
    /// Mirrors Swift `LinuxPod.ContainerConfiguration.hostname`.
    pub hostname: Option<Hostname>,
    pub dns: Option<DnsConfig>,
    pub hosts: Option<HostsConfig>,
    /// Wrap the entry point in `/.cz-init` for signal forwarding and zombie
    /// reaping. Mirrors Swift `useInit`.
    pub use_init: bool,
    pub sysctl: Option<BTreeMap<String, String>>,
}

/// Pre-boot rootfs source for a pod container. Distinct from `VmRootfs` (the
/// runtime-resolved `BlockDeviceId` wrapper at `crates/core/src/lib.rs:893`)
/// because a `PodContainerSpec` is constructed before the VM exists.
pub enum PodRootfsSource {
    Ext4Image(PathBuf),
    RawBlock(PathBuf),
    // OciBundle deferred until D-023 lifts on the OnVm path.
}

pub struct PodVolume {
    pub name: String,
    pub source: PodVolumeSource,
}

pub enum PodVolumeSource {
    /// Equivalent to k8s `hostPath`. Backed by a virtiofs share at the VM level.
    HostPath { path: PathBuf, tag: VirtiofsTag },
    /// Equivalent to k8s `emptyDir`. Backed by guest-side tmpfs in v1;
    /// durable variant deferred.
    EmptyDir { size: Option<Size> },
}

pub struct VolumeMount {                         // mirrors k8s VolumeMount, trimmed
    pub name: String,                            // references PodVolume.name
    pub mount_path: PathBuf,
    pub read_only: bool,
    pub sub_path: Option<PathBuf>,
}

pub struct VolumeMountOptions {                  // for mount_volume_in_all_containers
    pub mount_path: PathBuf,
    pub read_only: bool,
    pub sub_path: Option<PathBuf>,
}

pub struct EnvVar {                              // mirrors k8s EnvVar
    pub name: String,
    pub value: String,                           // value_from variants deferred
}

pub struct ResourceRequirements {                // trimmed; per-container limits only
    pub cpus: Option<u32>,                       // mirrors Swift LinuxPod's per-container `cpus` (may exceed pod total)
    pub memory: Option<Size>,                    // mirrors Swift `memoryInBytes`
}

pub struct SecurityContext {                     // trimmed; uid/gid + capabilities
    pub run_as_user: Option<u32>,
    pub run_as_group: Option<u32>,
    pub capabilities: Option<LinuxCapabilities>,
}

pub struct PodSnapshotOutput {
    pub snapshot_path: PathBuf,
    pub size_bytes: u64,
    pub sha256_hex: String,                      // computed via SnapshotArtifactIntegrity-equivalent
    pub container_membership: Vec<PodSnapshotContainer>,
}

pub struct PodSnapshotContainer {
    pub name: ContainerId,
    pub rootfs_logical_id: String,
    pub rootfs_sha256_hex: String,
    pub rootfs_size_bytes: u64,
    pub volume_mounts: Vec<VolumeMount>,
}
```

`Pod` lives **alongside** `OnVm`/`OnVmArc`, not as a replacement. The
low-level escape hatch stays for power users; `Pod` is the surface 90% of
callers want.

### PR 3 additions (separate PR, separate doc-comments)

```rust
impl Pod {
    /// CRI: CreateContainer + StartContainer. Requires runtime block-device
    /// attach (macOS 26+); returns PodError::Unsupported otherwise.
    /// New container can only mount volumes already declared at PodBuilder
    /// time — no runtime virtiofs attach in this milestone.
    pub async fn add_container(&mut self, spec: PodContainerSpec)
        -> Result<Arc<Container>, PodError>;

    /// CRI: StopContainer + RemoveContainer. Detaches the underlying
    /// block device.
    pub async fn remove_container(&mut self, name: &ContainerId)
        -> Result<(), PodError>;
}
```

`&mut self` chosen over `&self` + interior mutability: pod ownership is
typically held by one task at a time (the orchestrator); requiring `&mut`
makes that explicit and avoids returning borrows into an internally-locked
map. Mirrors how `firkin-core::Container` already structures mutating
operations.

## Crate graph

The production goal explicitly forbids `firkin-core` depending on
`firkin-substrate` (`07-production-substrate-goal.md:19`): "`substrate` owns
portable policy/models, … `core` owns VM/container mechanics." The original
brainstorm proposed `Pod::snapshot -> SnapshotArtifactManifest`; that
violates the rule and would fail
`scripts/check-firkin-crate-graph.sh`.

Corrected layering:

- `firkin-core::Pod::snapshot` returns `PodSnapshotOutput` (a core-owned
  shape). Writes the snapshot file. Does **not** touch substrate types.
- `firkin-substrate::SnapshotArtifactManifest` gains an optional
  `pod_membership: Option<RecordedPodMembership>` field. The struct
  definition lives in substrate.
- `firkin-runtime` (the composition crate) is what calls `pod.snapshot(...)`,
  receives the `PodSnapshotOutput`, and constructs the
  `SnapshotArtifactManifest` with `pod_membership` populated. Runtime
  already does composition for `RuntimeSnapshotRestore` etc.
  (`crates/runtime/src/lib.rs`).
- `RecordedPodMembership` is structurally identical to
  `PodSnapshotOutput::container_membership` but lives in substrate. Runtime
  copies between them. Trivial; keeps the layering intact.

## Lifecycle and data flow

### `PodBuilder::spawn` (PR 1, no D-019 dependency)

1. Validate. Container-name uniqueness; every `VolumeMount.name` resolves to
   a `PodVolume.name`; no pod-volume sources duplicate the same virtiofs
   tag. Errors before any VZ call.
2. For each `PodContainerSpec`, call `vm_config.block_device(rootfs_path)`
   to mint a `BlockDeviceId`. All N rootfses pre-declared at boot — D-019
   satisfied. Wrap each as `VmRootfs(BlockDeviceId)` for the OnVm builder.
3. For each `PodVolume::HostPath { path, tag }`, add a virtiofs share to
   `VmConfig` with the tag.
4. Boot the VM (`VirtualMachine::start`).
5. **If `share_process_namespace`**: spawn the pause container.
   `ContainerBuilder<OnVm>` with `name = "pause-{pod_id}"`, command =
   `["/sbin/vminitd", "pause"]` (mirrors Swift `LinuxPod.swift:373`),
   namespaces = fresh cgroup/ipc/mount/pid/uts. Hold its handle in `Pod`'s
   internal state.
6. For each container spec:
   - Build `ContainerBuilder<OnVm>` with the spec's `BlockDeviceId`, env,
     working-dir, security context.
   - If `share_process_namespace`: set the container's PID namespace to
     `LinuxNamespace::join("/proc/{pause_pid}/ns/pid")` (mirrors Swift
     `LinuxPod.swift:585`).
   - For each `PodVolume::EmptyDir`, issue a tmpfs mount in the guest at
     the requested path.
   - Resolve `volume_mounts` to virtiofs paths inside the guest.
   - `spawn()`.
7. Return `Pod { vm, containers, attached_devices, volumes, pause_process,
   state: Running }`.

### `Pod::add_container` (PR 3, requires D-019 lift)

1. Capability check: dynamic `runtime_block_attach_supported()` probe (not
   the static `apple_local_runtime_capabilities()`; see Capability
   detection below). Hard-fail with `PodError::Unsupported` on pre-26 macOS
   before any VZ call.
2. Validate spec's `volume_mounts` reference only volumes already declared
   at PodBuilder time. New volume names → `PodError::VolumeNotDeclared`
   with the explicit name.
3. `vm.attach_block_device(spec.rootfs.path()).await?` → new `BlockDeviceId`.
   Wrap in a `BlockDeviceGuard` (RAII; detaches on drop unless `commit()`).
4. Build `ContainerBuilder<OnVm>` with the new id; set namespaces (joining
   pause if `share_process_namespace`); spawn.
5. On spawn failure: guard drops, detaches; failure recorded in
   `PodError::ContainerSpawnFailed { name, source, detach_succeeded }`.
6. On success: `guard.commit()`, register container, return `Arc<Container>`.

This is more aggressive than Kata or firecracker-containerd — both leak
device state to the caller on partial failure. RAII guards in Rust make the
clean version cheap.

### `Pod::snapshot`

1. Forward to `VirtualMachine::snapshot(target).await?` → one VZ image.
   Captures memory + device state of the VM atomically (one syscall, one
   file).
2. Compute SHA-256 + size of the snapshot file. Substrate provides
   `SnapshotArtifactIntegrity::from_file` for this exact pattern at
   `crates/substrate/src/lib.rs:360-374`, but `firkin-core` cannot call
   substrate. `firkin-core` already depends on `sha2`
   (`crates/core/Cargo.toml:31`, used at `crates/core/src/lib.rs:1945` for
   `file_mount_tag`), so the digest helper is a 5-line wrapper around
   `Sha256::digest` — no new dependency. Substrate's
   `SnapshotArtifactIntegrity::from_file` becomes one of several callers
   of the same canonical hash-and-size pattern.
3. For each container: snapshot rootfs file's SHA-256 + size + the spec's
   `rootfs_logical_id`.
4. Return `PodSnapshotOutput`. Caller (typically `firkin-runtime`)
   constructs `SnapshotArtifactManifest` with `pod_membership` populated.

### Restore (out of scope for PR 1; designed forward-compatible)

Future restore code (in `firkin-runtime`) reads `pod_membership` from the
substrate manifest, verifies each container's rootfs SHA-256 against the
recorded value, rehydrates a `Pod` with container handles bound to the
restored VM. PR 1 ships the write-side; the read-side lands when a
substrate consumer needs it.

## Concurrency contract (PR 2)

D-006 (`docs/specs/rust_rewrite/DECISIONS.md:75-95`) requires every VZ call
and delegate callback to run on a single serial dispatch queue per VM. D-019
explicitly cites "dispatch-queue-sync invariants" as the cost of runtime
attach. PR 2 must respect this:

- `attach_block_device` and `detach_block_device` schedule the VZ call on
  the VM's serial queue.
- The VZ completion handler runs on that same queue; it bridges back to
  Rust via the existing `VzSend<T>` pattern (D-006).
- The 100×10ms guest-side mount retry runs *off* the VZ queue (it's a vsock
  RPC to vminitd, not a VZ call); only the kickoff and result handling
  touch the queue.
- `attach_block_device` mutation of the `BlockDeviceId` registry is
  protected by `Mutex` outside the VZ queue; the queue is not blocked
  waiting for the registry lock.
- Failure paths (VZ rejected attach, guest mount failed) issue
  `detach_block_device` synchronously to avoid leaking state. The detach
  itself goes through the VZ queue.

PR 2's signed live VZ smoke includes a stress test that interleaves
attach/detach with active container exec calls to surface any queue
contention.

## Capability detection

`apple_local_runtime_capabilities()` at `crates/firkin/src/lib.rs:209-217`
is `const` and returns a static array. It cannot conditionally advertise
based on macOS version. PR 2 needs a runtime detection mechanism. Two
options; the brainstorm did not pick one:

**Option A** — gate inside `attach_block_device` itself. The static
capability set still lists `runtime-block-attach`; the actual call returns
`PodError::Unsupported` on pre-26 macOS. Simplest. Capability advertisement
is a slight lie (advertised on macOS < 26 even though it doesn't work).

**Option B** — add a `dynamic_capabilities()` function alongside the const
set that probes `@available(macOS 26, *)` at runtime and returns a non-const
`RuntimeCapabilities`. Cleanest. Adds a new function to the facade.

Recommendation: **B**. The capability advertisement contract should not
lie. Cost is one extra function in the facade.

## Error model

```rust
pub enum PodError {
    /// Validation failure caught before any VZ call.
    Validation(PodValidationError),
    /// VM failed to boot.
    VmStartFailed(vmm::Error),
    /// Pause container failed to spawn (only when share_process_namespace).
    PauseSpawnFailed(core::Error),
    /// One or more containers failed to spawn at PodBuilder::spawn.
    SpawnFailed { failures: Vec<(ContainerId, core::Error)> },
    /// Runtime add: spawn failed after attach succeeded. detach_succeeded
    /// indicates whether the cleanup detach succeeded.
    ContainerSpawnFailed {
        name: ContainerId,
        source: core::Error,
        detach_succeeded: bool,
    },
    /// Capability gate (e.g., runtime-block-attach on pre-26 macOS).
    Unsupported(UnsupportedRuntimeCapability),
    /// VZ runtime attach/detach failure.
    AttachFailed(vmm::Error),
    DetachFailed(vmm::Error),
    /// VZ pause/resume failure.
    PauseFailed(vmm::Error),
    /// Snapshot file write or hash failure.
    SnapshotFailed(core::Error),
    /// Stop failure detail.
    StopFailed { failures: Vec<(ContainerId, core::Error)> },
}

pub enum PodValidationError {
    DuplicateContainerName(ContainerId),
    DuplicateVolumeName(String),
    DuplicateVirtiofsTag(VirtiofsTag),
    /// VolumeMount.name not declared as a PodVolume.
    VolumeNotFound { container: ContainerId, volume_name: String },
    /// Add-container time: spec references a volume that wasn't declared
    /// at PodBuilder time. Kept distinct from VolumeNotFound because it's
    /// caught at a different lifecycle point.
    VolumeNotDeclared { volume_name: String },
}
```

Every variant carries enough structured detail to diagnose without log
spelunking. Validation errors fire before any VZ syscall.

## Testing

**Prerequisite spike (before PR 1):** see "Prerequisite spike" section
above — two live VZ smokes (shared net + atomic snapshot of long-lived
state). If snapshot atomicity fails, drop the atomic-snapshot framing but
ship the rest.

**PR 1 (`firkin-core::pod`):**
- Unit (no VZ): `PodBuilder` validation — duplicate container names,
  duplicate volume names, duplicate virtiofs tags, unknown
  `VolumeMount.name` references, `mount_volume_in_all_containers`
  references-undeclared-volume case.
- Live VZ smoke: workload B — 3 containers in one pod, each runs `cargo
  build` of a small crate against per-container `target/`, all share an
  `sccache` directory via virtiofs `HostPath`. Verify each build completes
  and `sccache --show-stats` shows shared cache hits across containers.
- Live VZ smoke: tmpfs `EmptyDir` mounted in 2 containers, write from one,
  read from the other.
- Live VZ smoke: `share_process_namespace = true` — 2 containers, each
  spawns a sleeping process, container A runs `ps -ef` and sees container
  B's process. Verifies pause container is PID 1 and others joined.
- Live VZ smoke: snapshot a 2-container pod with long-lived state, verify
  `PodSnapshotOutput` has correct sha256 + size + per-container membership.

**PR 2 (`firkin-vmm` D-019 lift):**
- Live VZ smoke (macOS 26+): boot one-rootfs VM, `attach_block_device` for
  a second device, exec a guest command that mounts and reads it,
  `detach_block_device`, verify VM healthy and no leaked devices in
  `/sys/block`.
- Capability test (any macOS): on pre-26, `attach_block_device` returns
  `UnsupportedRuntimeCapability("runtime-block-attach")` before any VZ
  syscall.
- Failure-injection test: induced VZ attach failure does not leave the VM
  in a broken state; subsequent operations succeed.
- Concurrency stress: interleave attach/detach with active container exec
  calls; verify no VZ queue contention or deadlock.

**PR 3 (`Pod::add_container`):**
- Live VZ smoke: spawn one-container pod, `add_container` with a
  pre-assembled rootfs, exec in the new container, `remove_container`,
  verify clean.
- Live VZ smoke: workload C — pod with agent container; `add_container` a
  sidecar; sidecar `curl`s agent on `127.0.0.1:<port>`; verify request
  succeeds.
- Volume validation: `add_container` with `VolumeMount` that references an
  undeclared volume returns `PodError::Validation(VolumeNotDeclared)`
  before any VZ call.
- RAII test: induce spawn failure after attach; verify `BlockDeviceGuard`
  detaches and `PodError::ContainerSpawnFailed { detach_succeeded: true }`
  is reported.

**Cross-cutting:**
- `scripts/check-firkin-crate-graph.sh` enforces that `firkin-core::pod`
  does not depend on `firkin-e2b` or `firkin-substrate`. The
  `SnapshotArtifactManifest` extension is constructed only in
  `firkin-runtime`.
- Snapshot-manifest forward-compatibility test: round-trip
  serialize/deserialize a `SnapshotArtifactManifest` with `pod_membership =
  None` (legacy shape) and `pod_membership = Some(...)` (new shape).
- Behavioral parity test (manual): pick a `PodSpec` shape, construct
  equivalent Swift `LinuxPod` and Rust `Pod`, observe both pods'
  `ps`/network/volume behavior matches.

## Decisions log

- **Same-VM, not multi-VM coordinator.** Justified by shared VM network
  namespace + per-VM cost amortization. Atomic snapshot is a property, not
  a load-bearing claim.
- **Mirror Swift `LinuxPod` semantics.** Same lifecycle phases, same
  `share_process_namespace` opt-in pattern, same per-container override
  fields, same pause-container approach (`/sbin/vminitd pause`).
- **Lift D-019 in PR 2; ship PR 1 without it.** PR 1 covers workload B
  standalone; isolating the VZ delegate work limits per-PR risk.
- **`PodContainerSpec.rootfs: PodRootfsSource`, not `VmRootfs`.** Spec is
  pre-boot; `VmRootfs` is runtime-resolved.
- **`Pod::add_container(&mut self) -> Arc<Container>`.** `&mut self` over
  interior mutability; `Arc<Container>` over `&Container` to avoid
  borrowing into internally-mutable state.
- **`mount_volume_in_all_containers` references already-declared volume.**
  Takes `VolumeMountOptions` so `read_only` and `sub_path` are
  expressible.
- **Crate graph: `Pod::snapshot` returns core-shaped `PodSnapshotOutput`;
  substrate manifest constructed in `firkin-runtime`.**
- **Manifest durable identity: `logical_id + sha256_hex + size_bytes`,
  no path.** Matches the existing pattern of
  `SnapshotArtifactManifest::logical_id` + `SnapshotArtifactIntegrity`.
- **Capability detection: dynamic probe (option B).** `apple_local_runtime_capabilities()`
  stays as the const baseline; new `dynamic_capabilities()` function
  layers macOS-version checks on top. Capability advertisement does not
  lie.
- **PR 3 cannot add new pod volumes at runtime.** Virtiofs runtime attach
  unverified on Apple VZ; runtime-added containers can only mount volumes
  already declared at PodBuilder time.
- **`EmptyDir` defaults to tmpfs in guest in v1.** Durable variant deferred.
- **K8s field-name vocabulary, no `k8s-openapi` dependency.** Familiarity
  win without dependency cost.
- **`init_containers`, `restart_policy`, probes, lifecycle hooks deferred.**
  K8s features without a current consumer in firkin's workloads (and not in
  Swift `LinuxPod` either).

## Open questions

1. **Where does the snapshot SHA-256 helper live?** `firkin-core` already
   depends on `sha2` (`crates/core/Cargo.toml:31`) and uses
   `Sha256::digest` for `file_mount_tag` at `crates/core/src/lib.rs:1945`,
   so adding the helper is free dependency-wise. The real question is API
   responsibility: (a) `Pod::snapshot` reads the file and computes the
   digest as part of the operation (one read pass, simpler caller), or
   (b) `Pod::snapshot` returns path + size and exposes a separate
   `pod_snapshot_integrity(path)` helper that the composition layer
   invokes (matches substrate's existing
   `SnapshotArtifactIntegrity::from_file` shape exactly). Recommendation:
   **(a)**, single-pass and simpler. Substrate's `from_file` becomes one
   of several callers of the same hash-and-size pattern.

2. **Does Apple VZ guarantee in-flight process state is preserved in the
   memory snapshot, or only "kernel-quiesced" state?** The atomic-snapshot
   spike will tell us empirically. If only quiesced: PR 1's snapshot
   section needs a brief note that callers should expect post-restore
   processes may be in pre-syscall state, not mid-syscall. No design change
   required, just documentation.

3. **Is there a Cube/E2B product route on the near horizon that would
   consume pod-aware snapshots?** If yes, the deferred substrate
   `WarmPoolKey` extension and the E2B `POST /pods` surface should join
   this milestone. If no, defer as planned.

4. **Should the pause container appear in `Pod::containers()`?** Two
   reasonable answers: (a) yes, it's a real container — caller can see it;
   (b) no, it's an implementation detail of `share_process_namespace =
   true`. Recommendation: **(b)**, hide it. Mirrors how K8s pause
   containers don't show up in `kubectl get pods` container listings.
   Expose via `Pod::pause_process_pid()` for debug/observability.

## References

- `Sources/Containerization/LinuxPod.swift` — local Swift implementation
  of the same concept; primary prior art.
- `crates/core/src/lib.rs:1409-1455` — `OnVm` / `OnVmArc` builder context.
- `crates/core/src/lib.rs:893` — `VmRootfs` (runtime-resolved handle).
- `crates/core/src/lib.rs:3471-3475` — default OCI namespace unsharing
  (PID/Mount/IPC/UTS).
- `crates/cli/src/main.rs:271-366` — committed SLO targets used in cost
  analysis.
- `crates/substrate/src/lib.rs:260-291` — `SnapshotArtifactManifest`.
- `crates/substrate/src/lib.rs:350-413` — `SnapshotArtifactIntegrity`
  (logical-id + sha256 pattern).
- `crates/firkin/src/lib.rs:167, 209` — capability advertisement
  (`const`/static; insufficient for runtime macOS-version gating).
- `docs/specs/rust_rewrite/04-library-surface/01-container-surface.md:699`
  — cargo `target/` corruption warning under shared virtiofs.
- `docs/specs/rust_rewrite/07-production-substrate-goal.md:16, 19, 27-32,
  148-150, 167-168` — multi-container-per-VM posture, crate-graph rule,
  manifest persistence gap, acceptance check 14.
- `docs/specs/rust_rewrite/08-production-substrate-current-audit.md:51` —
  "advanced-mode lifecycle/isolation design" gap row.
- `docs/specs/rust_rewrite/DECISIONS.md` D-006, D-019, D-022, D-023 —
  serial dispatch queue, block-device pre-declaration constraint and its
  supersedences.
- CRI proto: `kubernetes/cri-api/pkg/apis/runtime/v1/api.proto`.
- Kata sandbox FSM: `kata-containers/kata-containers/src/runtime/virtcontainers/sandbox.go`.
- firecracker-containerd attach: `firecracker-microvm/firecracker-containerd/runtime/drive_handler.go`.
- `k8s_openapi::api::core::v1::{Container, Volume, VolumeMount, PodSpec,
  EnvVar, ResourceRequirements}` on docs.rs (vocabulary source).
