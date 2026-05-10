# Firkin Sandbox Library Surface Spec

Date: 2026-05-07

Status: proposed hard cutover

## Problem

Firkin has enough lower-level runtime machinery to boot, exec in, snapshot,
restore, warm, and route local Apple/VZ-backed sandboxes, but the usable Rust
library surface is not where a Rust orchestrator would look for it.

Today the closest usable surfaces are split across:

- `firkin-runtime`, which owns reusable runtime mechanics and an E2B-shaped
  adapter.
- `firkin-single-node`, which owns a concrete stateful local Apple/VZ backend.
- `firkin-e2b-contract`, which owns useful process/filesystem shapes, but names
  them after the E2B/envd compatibility layer.
- `firkin`, which is a broad facade that reexports too many unrelated crates.

That leaves external Rust users choosing between low-level VM/container
mechanics, a single-node service object, or E2B DTOs. None of those is the
right law surface for people writing their own agent orchestration in Rust.

The missing library is a neutral sandbox orchestration crate:

```text
firkin-sandbox
```

It must expose sandbox primitives for Rust orchestrators without becoming a
new product backend, a concrete Apple/VZ implementation crate, an E2B DTO
crate, a benchmark crate, or a generic shared dumping ground.

## Goal

Expose an idiomatic Rust API for orchestrators that need to:

- configure a host runtime
- prepare templates
- prepare arbitrary OCI images into Firkin-owned derived template artifacts
- maintain warm pools
- create, attach, inspect, stop, kill, and delete sandboxes
- execute foreground and detached processes
- stream process stdout, stderr, PTY, and stdin
- read, write, list, stat, move, and remove filesystem entries
- expose and connect ports
- capture, restore, export, and delete snapshots
- surface pause/resume only where capability-gated support exists
- observe lifecycle events, command events, logs, and lightweight metrics

The API should feel E2B-like in user shape, but it must be local/backend-explicit
where E2B hides managed-cloud decisions.

For the high-level sandbox surface, a managed template includes a guest data
plane. The default data plane for OCI templates should be envd-compatible
because that is the useful command/filesystem/PTY/files/port surface people
expect from an agent sandbox. The envd choice is a template/data-plane decision,
not an E2B API decision.

## Non-Goals

`firkin-sandbox` is not:

- an LLM agent framework
- an E2B compatibility crate
- an HTTP server crate
- an Apple/VZ implementation crate
- a single-node scheduler or state-store crate
- a VM/container mechanics crate
- an envd binary builder, packager, or vendoring crate
- a benchmark/evidence/reporting crate
- a CLI crate
- a generic `contracts`, `common`, `shared`, or `utils` crate

Do not use this work to keep old public paths alive. This is a hard cutover
surface.

## Current Topology Anchor

The workspace crate graph has already been split. The live split includes:

- `firkin-runtime`
- `firkin-single-node`
- `firkin-e2b-wire`
- `firkin-e2b-contract`
- `firkin-e2b-server`
- `firkin-benchmark`
- `firkin-evidence`
- `firkin-trace`

Current envd/template facts to account for:

- `firkin-e2b-contract::PreparedTemplate` currently has `has_envd: bool`. That
  is too weak for a real public template contract because it cannot describe
  version, architecture, startup mode, health proof, access-token init state, or
  provenance.
- `firkin` currently marks `e2b-envd-compatible-api` unsupported unless there
  is a guest envd or deliberate envd bridge.
- `firkin-single-node` currently has an Apple/VZ host-side envd adapter bridge.
  That is useful migration machinery, but it is not the same contract as a
  prepared template with guest envd installed.
- CubeSandbox's bring-your-own-image path treats envd on `:49983` plus
  `/health` as the template readiness contract. That is the closest prior art
  for the Firkin high-level sandbox data-plane ABI.

This spec does not replace the crate-split plan. It adds one missing public
law crate to the post-split graph.

Existing ownership remains:

- `firkin-runtime` hides reusable runtime workflows: snapshot restore,
  continuation, warm-pool mechanics, runtime preflight, disk guard,
  template-build support, and session traits.
- `firkin-single-node` hides how one local host runs, persists, schedules,
  and exposes Apple/VZ-backed sandboxes.
- `firkin-e2b-wire` hides E2B/Cube wire DTO shape.
- `firkin-e2b-contract` hides what a runtime must provide to satisfy the
  local E2B compatibility layer.
- `firkin-e2b-server` hides local E2B-compatible HTTP/data-plane servers.
- `firkin-core` hides VM-backed container and pod mechanics.
- `firkin-template` hides template build and freshness-sync execution.
- `firkin-admission` hides host capacity accounting.
- `firkin-artifacts` hides durable artifact manifests and integrity.
- `firkin-trace` hides low-level sample recording.
- `firkin-evidence` and `firkin-benchmark` stay outside runtime library APIs.

## New Crate

### `firkin-sandbox`

Sentence:

> `firkin-sandbox` hides the public Rust laws for creating, controlling,
> observing, warming, and restoring sandbox sessions.

It owns:

- public sandbox/domain identifiers
- public runtime, template, sandbox, process, filesystem, port, warm-pool,
  snapshot, event, log, metric, capability, and error records
- public data-plane intent and prepared data-plane records
- the capability traits external orchestrators can code against
- lightweight generic handle wrappers over those traits
- contract-test helpers for backend implementors
- conversion targets used by facade and compatibility adapters

It may depend on:

- `async-trait`
- `bytes`
- `futures-core` or `tokio-stream` only for stream trait types
- `serde` behind a feature for stable public records that callers persist
- `thiserror`
- `time`
- `tokio` only for `io` traits or stream channel types if needed
- `url` only if `TemplateSource::Git` or remote artifact sources need a parsed URL
- `uuid` only if the crate generates default IDs
- `firkin-types`
- `firkin-artifacts` only for neutral snapshot artifact integrity/manifests
- `firkin-trace` only for raw trace sample interop, not benchmark/evidence policy

It must not depend on:

- `firkin-core`
- `firkin-runtime`
- `firkin-single-node`
- `firkin-template`
- `firkin-admission`
- `firkin-hygiene`
- `firkin-evidence`
- `firkin-benchmark`
- `firkin-e2b-wire`
- `firkin-e2b-contract`
- `firkin-e2b-server`
- any envd implementation or generated RPC package
- `firkin-vmm`
- `firkin-vminitd-*`
- `firkin-oci`
- `firkin-ext4`
- `reqwest`
- `axum`
- `hyper`
- `tower`
- `clap`
- any Apple/VZ or Virtualization.framework binding

Why it is a crate:

1. External orchestrators need to import these laws without pulling Apple/VZ,
   E2B, benchmark, or VM mechanics.
2. Backend implementations need a target trait surface to implement.
3. E2B compatibility should adapt to the neutral surface instead of owning the
   useful process/filesystem/process-stream semantics.
4. The compiler must forbid `sandbox` law code from reaching down into
   concrete backend behavior.

## Revised Target Graph

The public-library graph becomes:

```text
firkin
  -> firkin-sandbox
  -> firkin-single-node          # optional backend reexport module only
  -> firkin-core                 # low-level explicit module only
  -> firkin-runtime              # lower mechanics explicit module only
  -> firkin-e2b-*                # compatibility explicit module only
  -> other leaf crates as explicit modules

firkin-sandbox
  -> firkin-types
  -> firkin-artifacts            # optional, neutral snapshot artifact refs
  -> firkin-trace                # optional, raw trace sample refs

firkin-single-node
  -> firkin-sandbox
  -> firkin-runtime
  -> firkin-admission
  -> firkin-artifacts
  -> firkin-hygiene
  -> firkin-trace
  -> firkin-template
  -> firkin-core
  -> firkin-vmm
  -> firkin-vminitd-*
  -> firkin-oci
  -> firkin-ext4
  -> firkin-e2b-server           # only for compatibility/domain-proxy wiring
  -> firkin-e2b-contract         # temporary during migration

firkin-runtime
  -> firkin-sandbox              # only for public law types it implements/adapts
  -> firkin-core
  -> firkin-template
  -> firkin-admission
  -> firkin-artifacts
  -> firkin-hygiene
  -> firkin-trace
  -> firkin-types

firkin-e2b-contract
  -> firkin-sandbox              # after migration, adapt neutral laws into E2B contracts
  -> firkin-e2b-wire
  -> firkin-types

firkin-e2b-server
  -> firkin-e2b-contract
  -> firkin-e2b-wire
  -> firkin-sandbox              # optional adapter construction, no concrete backend
  -> firkin-types
```

The exact `firkin-runtime -> firkin-sandbox` edge is allowed because the
sandbox crate is the law surface. The runtime crate implements or adapts lower
mechanics to those laws. `firkin-sandbox` must never depend back on runtime.

If `firkin-runtime` does not need `firkin-sandbox` during the first slice, keep
that edge out until the compiler proves it is needed.

## Public Facade Shape

The `firkin` crate should eventually expose:

```rust
pub mod sandbox {
    pub use firkin_sandbox::*;

    #[cfg(feature = "apple-vz")]
    pub mod apple_vz {
        pub use firkin_single_node::{AppleVzBackend, AppleVzConfig};
    }
}
```

The concrete backend convenience lives in the facade or concrete backend crate,
not in `firkin-sandbox`.

Preferred core shape:

```rust
use firkin::sandbox::{
    Capacity, Command, DataPlaneSpec, Runtime, SandboxSpec, TemplateSpec,
    WarmPoolSpec,
};
use firkin::sandbox::apple_vz::AppleVzBackend;

let backend = AppleVzBackend::builder()
    .root("/var/lib/firkin")
    .build()
    .await?;

let runtime = Runtime::builder()
    .backend(backend)
    .capacity(Capacity::new().max_sandboxes(8))
    .build()
    .await?;

let template = runtime.templates()
    .prepare(
        TemplateSpec::oci("docker.io/library/rust:latest")
            .data_plane(DataPlaneSpec::envd().inject())
            .setup("rustup component add clippy")
            .ready("cargo --version"),
    )
    .await?;

runtime.warm_pool()
    .prewarm(&template, WarmPoolSpec::depth(2))
    .await?;

let sandbox = runtime.sandboxes()
    .create(
        SandboxSpec::from_template(&template)
            .env("RUST_LOG", "info")
            .timeout(Duration::from_secs(1800)),
    )
    .await?;

let output = sandbox.exec(Command::shell("cargo test")).await?;
sandbox.fs().write("/work/task.md", b"...").await?;

let snapshot = sandbox.snapshot("after-tests").await?;
sandbox.stop().await?;

let resumed = runtime.sandboxes().restore(snapshot).await?;
```

Do not put `.apple_vz()` on `firkin_sandbox::RuntimeBuilder` unless that method
is defined in a separate extension trait owned by `firkin-single-node` or the
`firkin` facade. The neutral law crate must not know concrete backend names.

## Crate Module Tree

`crates/sandbox/src/lib.rs` should be a map only:

```text
sandbox/
  lib.rs
  ids.rs
  error.rs
  capability.rs
  runtime.rs
  template.rs
  data_plane.rs
  sandbox.rs
  process.rs
  filesystem.rs
  snapshot.rs
  warm_pool.rs
  ports.rs
  event.rs
  logs.rs
  metrics.rs
  backend.rs
  contract.rs
  prelude.rs
```

Forbidden module names:

- `common.rs`
- `shared.rs`
- `utils.rs`
- `helpers.rs`
- `models.rs`
- `service.rs`
- `manager.rs`
- `impl.rs`

Each module must own one concept and reject unrelated code.

## Module Contracts

### `ids.rs`

Owns stable public identifiers.

Exports:

- `RuntimeId`
- `SandboxId`
- `TemplateId`
- `TemplateBuildId`
- `SnapshotId`
- `ProcessId`
- `ProcessTag`
- `PortName`
- `WarmPoolKey`
- `BackendName`

Rules:

- IDs are opaque newtypes, not raw `String` aliases.
- Constructors validate non-empty values and obvious illegal separators where
  the ID is used in paths or URLs.
- Accessors expose `as_str()`.
- Serialization is allowed behind `serde`.
- No backend-specific ID formats are encoded here.

### `error.rs`

Owns structured public errors.

Exports:

- `Error`
- `Result<T>`
- `UnsupportedCapability`
- `InvalidSpec`
- `NotFound`
- `AlreadyExists`
- `CapacityRejected`
- `DeadlineExceeded`
- `BackendFailure`
- `IoFailure`
- `SnapshotIntegrityFailure`
- `ProcessFailure`
- `FilesystemFailure`
- `PortFailure`
- `TemplatePrepareFailure`

Rules:

- Display text is for humans, not downstream branching.
- Every public error variant must carry enough structured context for a caller
  to choose retry, fallback, cleanup, or fatal handling.
- Backend-specific failures are wrapped with `BackendName`, operation, and
  source string/object. They do not leak Apple/VZ or E2B error types into the
  public law crate.
- Avoid `String`-only catch-all variants except at foreign boundaries.
- `TemplatePrepareFailure` is the primary typed error family for arbitrary
  image sharp edges. Callers must be able to branch on image, entrypoint, arch,
  port, user, health, and writable-layer problems without parsing display text.

### `capability.rs`

Owns capability discovery and capability-gated refusals.

Exports:

- `Capabilities`
- `Capability`
- `CapabilityName`
- `CapabilityStatus`
- `CapabilityReason`
- `CapabilitySet`
- `CapabilityRequirement`

First-class capability names:

- `runtime.create`
- `runtime.attach`
- `runtime.list`
- `runtime.deadline`
- `sandbox.stop`
- `sandbox.kill`
- `sandbox.delete`
- `snapshot.capture`
- `snapshot.restore`
- `snapshot.delete`
- `snapshot.export`
- `snapshot.import`
- `pause.capture`
- `pause.resume`
- `process.run`
- `process.start`
- `process.stream`
- `process.stdin`
- `process.signal`
- `process.pty`
- `filesystem.read`
- `filesystem.write`
- `filesystem.copy_in`
- `filesystem.copy_out`
- `filesystem.list`
- `filesystem.watch`
- `ports.connect`
- `ports.expose`
- `ports.domain_proxy`
- `template.prepare`
- `template.ready`
- `template.freshness`
- `template.data_plane.none`
- `template.data_plane.envd.inject`
- `template.data_plane.envd.verify`
- `sandbox.data_plane.init`
- `warm_pool.prewarm`
- `warm_pool.checkout`
- `events.subscribe`
- `metrics.host`
- `metrics.guest`
- `network.policy`

Rules:

- Unsupported capabilities are normal data, not hidden panics.
- Unknown capability names are unsupported.
- `pause.resume` is not implied by `snapshot.restore`.
- `network.policy` is unsupported until the backend can enforce policy.
- `template.data_plane.envd.inject` is stronger than
  `template.data_plane.envd.verify`. A backend may verify images that already
  contain envd without being able to mutate arbitrary OCI images.
- `sandbox.data_plane.init` must be present for envd-backed managed sandboxes;
  without it, a sandbox cannot be declared ready.

### `runtime.rs`

Owns the host-level public runtime handle.

Exports:

- `Runtime<B = BoxBackend>`
- `RuntimeBuilder`
- `RuntimeConfig`
- `RuntimeInfo`
- `RuntimeState`
- `RuntimeRoot`
- `Capacity`
- `DeadlinePolicy`
- `HygienePolicy`
- `RuntimePreflight`
- `TemplateClient`
- `SandboxClient`
- `WarmPoolClient`
- `SnapshotClient`

Public methods:

```rust
impl RuntimeBuilder {
    pub fn backend<B>(self, backend: B) -> Self
    where
        B: SandboxBackend + 'static;

    pub fn capacity(self, capacity: Capacity) -> Self;
    pub fn deadline_policy(self, policy: DeadlinePolicy) -> Self;
    pub fn hygiene_policy(self, policy: HygienePolicy) -> Self;
    pub async fn build(self) -> Result<Runtime>;
}

impl Runtime {
    pub fn capabilities(&self) -> Capabilities;
    pub async fn preflight(&self) -> Result<RuntimePreflight>;
    pub async fn info(&self) -> Result<RuntimeInfo>;
    pub fn templates(&self) -> TemplateClient;
    pub fn sandboxes(&self) -> SandboxClient;
    pub fn warm_pool(&self) -> WarmPoolClient;
    pub fn snapshots(&self) -> SnapshotClient;
    pub async fn subscribe(&self, filter: EventFilter) -> Result<EventStream>;
}
```

Rules:

- `Runtime` is a handle over a backend, not a scheduler implementation.
- It may coordinate calls across clients, but it does not own concrete local
  state persistence.
- It does not know Apple/VZ, E2B, HTTP, or CLI details.
- No background task should start from the neutral crate without being owned by
  a backend implementation.

### `template.rs`

Owns public template preparation intent and prepared template references.

Exports:

- `TemplateSpec`
- `TemplateSource`
- `OciTemplateSource`
- `GitTemplateSource`
- `LocalTemplateSource`
- `TemplateCommand`
- `TemplateReadyProbe`
- `TemplateEnv`
- `TemplateEntrypointPolicy`
- `TemplateUserPolicy`
- `PreparedTemplate`
- `TemplateInfo`
- `TemplateState`
- `TemplateClient`
- `TemplatePrepareFailure`
- `TemplatePrepareIssue`

Public shape:

```rust
impl TemplateSpec {
    pub fn oci(reference: impl Into<String>) -> Self;
    pub fn git(url: impl Into<String>) -> Self;
    pub fn local(path: impl Into<PathBuf>) -> Self;
    pub fn id(self, id: TemplateId) -> Self;
    pub fn data_plane(self, data_plane: impl Into<DataPlaneSpec>) -> Self;
    pub fn env(self, key: impl Into<String>, value: impl Into<String>) -> Self;
    pub fn setup(self, command: impl Into<String>) -> Self;
    pub fn start(self, command: impl Into<String>) -> Self;
    pub fn ready(self, command: impl Into<String>) -> Self;
    pub fn timeout(self, timeout: Duration) -> Self;
}

impl TemplateClient {
    pub async fn prepare(&self, spec: TemplateSpec) -> Result<PreparedTemplate>;
    pub async fn get(&self, id: &TemplateId) -> Result<TemplateInfo>;
    pub async fn list(&self) -> Result<Vec<TemplateInfo>>;
    pub async fn delete(&self, id: &TemplateId) -> Result<()>;
}
```

Rules:

- `TemplateSpec` describes intent. It is not an OCI implementation, git clone
  engine, or template-build scheduler.
- Prepared templates are references to backend-owned artifacts, not direct
  `firkin-template` job records.
- `TemplateSpec::oci` defaults to the managed sandbox profile:
  `DataPlaneSpec::envd().inject()`. Callers that want a raw boot artifact must
  opt out with `DataPlaneSpec::none()` and should expect process/filesystem/PTY
  convenience operations to be unsupported.
- User OCI images are immutable source inputs. A backend may create a derived
  prepared artifact by adding an envd layer, wrapping startup, adding metadata,
  proving health, and snapshotting the result. The source image is not mutated.
- Template source-specific validation is limited to public spec validity.
  Pulling/building belongs in runtime/backend crates.
- Do not silently accept a template whose requested data plane cannot be
  installed or verified. Return `TemplatePrepareFailure` before a sandbox can
  be created.

### `data_plane.rs`

Owns the public guest data-plane ABI contract for managed templates.

This module is allowed to name envd because envd is a concrete guest
data-plane ABI. The process, filesystem, PTY, and port modules remain neutral
and must not expose E2B/envd DTO names.

Exports:

- `DataPlaneSpec`
- `DataPlaneKind`
- `DataPlaneProvisioning`
- `DataPlaneInfo`
- `EnvdDataPlaneSpec`
- `PreparedEnvdDataPlane`
- `EnvdSource`
- `EnvdStartup`
- `EnvdInitMode`
- `EnvdHealthProbe`
- `GuestArch`
- `ReservedPort`

Public shape:

```rust
impl DataPlaneSpec {
    pub fn none() -> Self;
    pub fn envd() -> EnvdDataPlaneSpec;
}

impl EnvdDataPlaneSpec {
    pub fn inject(self) -> DataPlaneSpec;
    pub fn already_present(self) -> DataPlaneSpec;
    pub fn version(self, version: impl Into<String>) -> Self;
    pub fn source(self, source: EnvdSource) -> Self;
    pub fn arch(self, arch: GuestArch) -> Self;
    pub fn port(self, port: u16) -> Self;
    pub fn startup(self, startup: EnvdStartup) -> Self;
    pub fn init_mode(self, mode: EnvdInitMode) -> Self;
    pub fn default_user(self, user: impl Into<String>) -> Self;
    pub fn health(self, probe: EnvdHealthProbe) -> Self;
}
```

Prepared template records should carry:

```rust
pub enum DataPlaneInfo {
    None,
    Envd(PreparedEnvdDataPlane),
}

pub struct PreparedEnvdDataPlane {
    pub version: String,
    pub commit: Option<String>,
    pub sha256: String,
    pub arch: GuestArch,
    pub port: ReservedPort,
    pub startup: EnvdStartup,
    pub init_mode: EnvdInitMode,
    pub default_user: Option<String>,
    pub health: EnvdHealthProbe,
    pub health_checked_at: OffsetDateTime,
}
```

Rules:

- `DataPlaneSpec::envd().inject()` means "make this image a managed Firkin
  sandbox template." The backend prepares a derived artifact if the source image
  does not already contain a compatible envd.
- `DataPlaneSpec::envd().already_present()` means "verify the guest already
  satisfies the envd ABI." Verification still includes arch, version/provenance
  if requested, reserved port, health, startup, default user, and runtime init.
- `DataPlaneSpec::none()` is valid for lower-level users, but process,
  filesystem, PTY, and port-scanner capabilities should be reported unsupported
  unless another backend-specific data plane provides them.
- Firkin-managed envd must not rely on Firecracker MMDS on Apple/VZ. The
  backend must use `-isnotfc` plus `/init`, or provide an explicit metadata
  capability that satisfies envd's init/auth requirements.
- Access tokens, sandbox ID, template ID, env vars, default workdir, CA bundle,
  volume mounts, and deadline/runtime metadata are per-sandbox runtime init
  data. They are not baked into the prepared template artifact.
- Port `49983` is reserved for envd by default. User applications that need
  `49983` must either move or use a different explicit envd port accepted by
  the backend.
- The neutral law crate stores envd version/provenance intent and prepared
  facts. It does not build Go binaries, mutate OCI layers, bind sockets, start
  envd, or issue envd HTTP/RPC calls.
- The first production packaging path should be a pinned multi-arch envd layer
  or base artifact controlled by Firkin, not an unpinned pull from a third-party
  image tag.

Arbitrary image contract:

1. The user may pass any supported OCI reference without modifying its
   Dockerfile.
2. `prepare` inspects the image, chooses a compatible guest architecture,
   injects or verifies the requested data plane, preserves the original
   entrypoint/CMD/user/workdir where possible, boots once, proves health, runs
   setup/ready commands, snapshots, and returns a prepared artifact reference.
3. The prepared artifact is Firkin-owned and may differ from the source image.
4. Unsupported image shapes fail during `prepare` with typed errors, not after
   sandbox creation.
5. Prepared artifacts are cacheable by source image digest, data-plane spec,
   envd provenance, setup/start/ready commands, backend ABI version, and guest
   architecture. Bring-your-own-image should be slow only on first prepare or
   cache miss; sandbox create/restore should use the prepared artifact and warm
   pool.

Typed prepare failures:

- `EnvdMissing`
- `EnvdWrongArch { expected, found }`
- `EnvdVersionMismatch { expected, found }`
- `EnvdIntegrityMismatch { expected_sha256, found_sha256 }`
- `EnvdHealthFailed { port, path, status }`
- `EnvdInitFailed`
- `EntrypointUnsupported`
- `EntrypointNotWrapped`
- `PortConflict { port }`
- `DefaultUserMissing { user }`
- `DefaultWorkdirMissing { path }`
- `ReadOnlyRootfs`
- `WritableLayerRequired`
- `UnsupportedImageConfig`
- `RegistryUnavailable`
- `SnapshotAfterPrepareFailed`

### `sandbox.rs`

Owns the live sandbox public handle and lifecycle specs.

Exports:

- `Sandbox`
- `SandboxSpec`
- `SandboxInfo`
- `SandboxState`
- `SandboxClient`
- `SandboxResources`
- `SandboxDeadline`
- `SandboxMetadata`
- `SandboxEnvironment`
- `StopMode`
- `KillSignal`
- `AttachOptions`
- `DeleteOptions`

Public shape:

```rust
impl SandboxSpec {
    pub fn from_template(template: &PreparedTemplate) -> Self;
    pub fn from_snapshot(snapshot: SnapshotRef) -> Self;
    pub fn resources(self, resources: SandboxResources) -> Self;
    pub fn env(self, key: impl Into<String>, value: impl Into<String>) -> Self;
    pub fn timeout(self, timeout: Duration) -> Self;
    pub fn metadata(self, key: impl Into<String>, value: impl Into<String>) -> Self;
}

impl SandboxClient {
    pub async fn create(&self, spec: SandboxSpec) -> Result<Sandbox>;
    pub async fn restore(&self, snapshot: SnapshotRef) -> Result<Sandbox>;
    pub async fn attach(&self, id: &SandboxId, options: AttachOptions) -> Result<Sandbox>;
    pub async fn inspect(&self, id: &SandboxId) -> Result<SandboxInfo>;
    pub async fn list(&self, filter: SandboxFilter) -> Result<Vec<SandboxInfo>>;
    pub async fn stop(&self, id: &SandboxId, mode: StopMode) -> Result<()>;
    pub async fn kill(&self, id: &SandboxId, signal: KillSignal) -> Result<()>;
    pub async fn delete(&self, id: &SandboxId, options: DeleteOptions) -> Result<()>;
    pub async fn update_deadline(&self, id: &SandboxId, deadline: SandboxDeadline) -> Result<()>;
}

impl Sandbox {
    pub fn id(&self) -> &SandboxId;
    pub async fn info(&self) -> Result<SandboxInfo>;
    pub async fn stop(&self) -> Result<()>;
    pub async fn kill(&self) -> Result<()>;
    pub async fn delete(self) -> Result<()>;
    pub async fn update_deadline(&self, deadline: SandboxDeadline) -> Result<()>;
    pub fn process(&self) -> ProcessClient;
    pub fn fs(&self) -> FilesystemClient;
    pub fn ports(&self) -> PortClient;
    pub fn logs(&self) -> LogClient;
    pub fn metrics(&self) -> MetricClient;
    pub async fn snapshot(&self, name: impl Into<String>) -> Result<SnapshotRef>;
    pub async fn pause(&self, options: PauseOptions) -> Result<PausedSandbox>;
}
```

Convenience:

```rust
impl Sandbox {
    pub async fn exec(&self, command: Command) -> Result<CommandOutput>;
}
```

Rules:

- `Sandbox` is a live handle. It is not a persisted state record.
- `SandboxInfo` is an inspect/list record. It is not a live control handle.
- `stop`, `kill`, and `delete` are separate concepts.
- `pause` must check capabilities and return `UnsupportedCapability` when the
  backend only supports snapshot/restore semantics.

### `process.rs`

Owns process execution and streaming laws.

Exports:

- `Command`
- `CommandMode`
- `CommandOutput`
- `CommandStatus`
- `CommandExit`
- `Process`
- `ProcessInfo`
- `ProcessSelector`
- `ProcessEvent`
- `ProcessEventStream`
- `ProcessInput`
- `Signal`
- `Pty`
- `PtySize`
- `ProcessClient`

Public shape:

```rust
impl Command {
    pub fn shell(command: impl Into<String>) -> Self;
    pub fn argv(program: impl Into<String>, args: impl IntoIterator<Item = impl Into<String>>) -> Self;
    pub fn cwd(self, cwd: impl Into<String>) -> Self;
    pub fn env(self, key: impl Into<String>, value: impl Into<String>) -> Self;
    pub fn user(self, user: impl Into<String>) -> Self;
    pub fn stdin(self, bytes: impl Into<Bytes>) -> Self;
    pub fn pty(self, size: PtySize) -> Self;
    pub fn tag(self, tag: ProcessTag) -> Self;
    pub fn timeout(self, timeout: Duration) -> Self;
}

impl ProcessClient {
    pub async fn run(&self, command: Command) -> Result<CommandOutput>;
    pub async fn start(&self, command: Command) -> Result<Process>;
    pub async fn start_stream(&self, command: Command) -> Result<ProcessEventStream>;
    pub async fn list(&self) -> Result<Vec<ProcessInfo>>;
    pub async fn connect(&self, selector: ProcessSelector) -> Result<Process>;
    pub async fn signal(&self, selector: ProcessSelector, signal: Signal) -> Result<()>;
    pub async fn send_input(&self, selector: ProcessSelector, input: ProcessInput) -> Result<()>;
    pub async fn close_stdin(&self, selector: ProcessSelector) -> Result<()>;
    pub async fn resize_pty(&self, selector: ProcessSelector, size: PtySize) -> Result<()>;
}

impl Process {
    pub fn id(&self) -> ProcessId;
    pub async fn info(&self) -> Result<ProcessInfo>;
    pub async fn next_event(&mut self) -> Option<Result<ProcessEvent>>;
    pub async fn send_input(&self, input: ProcessInput) -> Result<()>;
    pub async fn close_stdin(&self) -> Result<()>;
    pub async fn signal(&self, signal: Signal) -> Result<()>;
    pub async fn resize_pty(&self, size: PtySize) -> Result<()>;
    pub async fn wait(self) -> Result<CommandOutput>;
}
```

Rules:

- Shell and argv forms are distinct.
- `run` is finite output. `start` is detached/live. `start_stream` is a
  streaming shortcut.
- PTY is a capability, not the default command shape.
- Process IDs are sandbox-scoped.
- E2B/envd names do not appear in this module.

### `filesystem.rs`

Owns sandbox filesystem laws.

Exports:

- `SandboxPath`
- `FilesystemClient`
- `FileEntry`
- `FileType`
- `FileStat`
- `FilePermissions`
- `ReadOptions`
- `WriteOptions`
- `ListOptions`
- `CopyOptions`
- `WatchOptions`
- `FilesystemEvent`
- `FilesystemEventStream`

Public shape:

```rust
impl FilesystemClient {
    pub async fn read(&self, path: impl Into<SandboxPath>) -> Result<Bytes>;
    pub async fn write(&self, path: impl Into<SandboxPath>, data: impl Into<Bytes>) -> Result<FileEntry>;
    pub async fn append(&self, path: impl Into<SandboxPath>, data: impl Into<Bytes>) -> Result<FileEntry>;
    pub async fn copy_in(&self, host: impl AsRef<Path>, sandbox: impl Into<SandboxPath>, options: CopyOptions) -> Result<FileEntry>;
    pub async fn copy_out(&self, sandbox: impl Into<SandboxPath>, host: impl AsRef<Path>, options: CopyOptions) -> Result<()>;
    pub async fn list(&self, path: impl Into<SandboxPath>, options: ListOptions) -> Result<Vec<FileEntry>>;
    pub async fn mkdir(&self, path: impl Into<SandboxPath>) -> Result<FileEntry>;
    pub async fn mkdir_all(&self, path: impl Into<SandboxPath>) -> Result<FileEntry>;
    pub async fn rename(&self, from: impl Into<SandboxPath>, to: impl Into<SandboxPath>) -> Result<FileEntry>;
    pub async fn remove(&self, path: impl Into<SandboxPath>) -> Result<()>;
    pub async fn stat(&self, path: impl Into<SandboxPath>) -> Result<FileStat>;
    pub async fn watch(&self, path: impl Into<SandboxPath>, options: WatchOptions) -> Result<FilesystemEventStream>;
}
```

Rules:

- `SandboxPath` is not `PathBuf`; it is a guest path with sandbox semantics.
- `copy_in` and `copy_out` are high-level API laws. Their implementation may
  use vminitd, process commands, virtiofs, tar streams, or another backend.
- Watch support is optional and capability-gated.
- Do not implement filesystem operations with shell snippets in this crate.

### `snapshot.rs`

Owns durable sandbox continuation/template snapshot references and lifecycle.

Exports:

- `SnapshotRef`
- `SnapshotInfo`
- `SnapshotKind`
- `SnapshotOptions`
- `SnapshotExport`
- `SnapshotImport`
- `SnapshotIntegrity`
- `PausedSandbox`
- `PauseOptions`
- `ResumeOptions`
- `SnapshotClient`

Public shape:

```rust
impl SnapshotClient {
    pub async fn capture(&self, sandbox: &SandboxId, options: SnapshotOptions) -> Result<SnapshotRef>;
    pub async fn restore(&self, snapshot: SnapshotRef, options: RestoreOptions) -> Result<Sandbox>;
    pub async fn get(&self, id: &SnapshotId) -> Result<SnapshotInfo>;
    pub async fn list(&self, filter: SnapshotFilter) -> Result<Vec<SnapshotInfo>>;
    pub async fn delete(&self, id: &SnapshotId) -> Result<()>;
    pub async fn export(&self, id: &SnapshotId) -> Result<SnapshotExport>;
    pub async fn import(&self, import: SnapshotImport) -> Result<SnapshotRef>;
}
```

Rules:

- `SnapshotRef` is public and durable enough for callers to store.
- `SnapshotInfo` is inspect/list metadata.
- `SnapshotKind::Template` and `SnapshotKind::Continuation` are distinct.
- Pause/resume are capability-gated lifecycle operations, not aliases for all
  snapshot operations.
- Apple/VZ-specific machine identifier bytes and network MAC records stay in
  backend/runtime internals unless represented through neutral artifact
  metadata.

### `warm_pool.rs`

Owns public warm-pool intent, status, and leases.

Exports:

- `WarmPoolClient`
- `WarmPoolSpec`
- `WarmPoolStatus`
- `WarmPoolEntry`
- `WarmLease`
- `WarmLeasePolicy`
- `WarmEvictionPolicy`
- `WarmMaintainReport`

Public shape:

```rust
impl WarmPoolSpec {
    pub fn depth(depth: usize) -> Self;
    pub fn min_ready(self, min_ready: usize) -> Self;
    pub fn eviction(self, policy: WarmEvictionPolicy) -> Self;
}

impl WarmPoolClient {
    pub async fn prewarm(&self, template: &PreparedTemplate, spec: WarmPoolSpec) -> Result<WarmMaintainReport>;
    pub async fn maintain(&self, targets: Vec<WarmPoolTarget>) -> Result<WarmMaintainReport>;
    pub async fn status(&self) -> Result<WarmPoolStatus>;
    pub async fn checkout(&self, template: &PreparedTemplate, policy: WarmLeasePolicy) -> Result<WarmLease>;
    pub async fn evict(&self, key: WarmPoolKey, count: usize) -> Result<WarmMaintainReport>;
}
```

Rules:

- Warm-pool policy is caller-visible.
- Warm-pool implementation remains backend/runtime-owned.
- A lease is an explicit ownership token. Dropping it must have documented
  behavior, but do not hide expensive async cleanup in `Drop`.
- Capacity rejection is structured.

### `ports.rs`

Owns sandbox port exposure/routing laws.

Exports:

- `PortClient`
- `Port`
- `GuestPort`
- `HostPort`
- `PortProtocol`
- `PortBinding`
- `PortTarget`
- `PortExposure`
- `DomainProxy`
- `DomainProxySpec`

Public shape:

```rust
impl PortClient {
    pub async fn list(&self) -> Result<Vec<PortBinding>>;
    pub async fn connect(&self, port: GuestPort) -> Result<PortTarget>;
    pub async fn expose(&self, port: GuestPort, spec: PortExposure) -> Result<PortBinding>;
    pub async fn unexpose(&self, binding: PortBinding) -> Result<()>;
    pub async fn domain_proxy(&self, spec: DomainProxySpec) -> Result<DomainProxy>;
}
```

Rules:

- Port routing is neutral. Domain proxy hosting is backend/facade integration,
  not E2B-only.
- Network policy is separate and capability-gated.
- Do not expose raw `axum`, `hyper`, or `tower` types from this crate.

### `event.rs`

Owns event subscriptions.

Exports:

- `Event`
- `EventKind`
- `EventFilter`
- `EventStream`
- `LifecycleEvent`
- `ProcessEvent`
- `FilesystemEvent`
- `SnapshotEvent`
- `WarmPoolEvent`
- `PortEvent`

Rules:

- Events are operational facts, not benchmark evidence.
- Event streams are best-effort unless a backend documents stronger delivery.
- Event records must include `SandboxId` where applicable.
- Do not import `firkin-evidence`.

### `logs.rs`

Owns public log retrieval intent and records.

Exports:

- `LogClient`
- `LogEntry`
- `LogStream`
- `LogFilter`
- `LogSource`
- `LogLevel`

Rules:

- Logs are runtime/sandbox observations.
- Benchmark proof HTML, evidence reports, and SLO summaries stay outside this
  crate.
- Backend-specific boot logs may be mapped into `LogSource::Boot`, but raw VZ
  types do not cross the law boundary.

### `metrics.rs`

Owns lightweight runtime/sandbox metric records.

Exports:

- `MetricClient`
- `Metric`
- `MetricName`
- `MetricValue`
- `MetricUnit`
- `MetricScope`
- `MetricSnapshot`
- `MetricFilter`

Rules:

- Metrics are raw observations only.
- SLO gates, scorecards, benchmark suites, trust promotion, and evidence
  validation stay in `firkin-evidence` and `firkin-benchmark`.
- If `firkin-trace` interop is enabled, conversions must preserve raw sample
  units and labels.

### `backend.rs`

Owns the law traits backend implementations must satisfy.

Exports:

- `SandboxBackend`
- `TemplateControl`
- `SandboxControl`
- `ProcessControl`
- `FilesystemControl`
- `SnapshotControl`
- `WarmPoolControl`
- `PortControl`
- `EventControl`
- `LogControl`
- `MetricControl`
- `BackendInfo`
- `BoxBackend`

Trait shape:

```rust
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    async fn capabilities(&self) -> Result<Capabilities>;
    async fn preflight(&self) -> Result<RuntimePreflight>;
    async fn info(&self) -> Result<BackendInfo>;

    fn templates(&self) -> &dyn TemplateControl;
    fn sandboxes(&self) -> &dyn SandboxControl;
    fn snapshots(&self) -> &dyn SnapshotControl;
    fn warm_pool(&self) -> Option<&dyn WarmPoolControl>;
    fn events(&self) -> Option<&dyn EventControl>;
    fn logs(&self) -> Option<&dyn LogControl>;
    fn metrics(&self) -> Option<&dyn MetricControl>;
}

#[async_trait]
pub trait TemplateControl: Send + Sync {
    async fn prepare_template(&self, spec: TemplateSpec) -> Result<PreparedTemplate>;
    async fn get_template(&self, id: &TemplateId) -> Result<TemplateInfo>;
    async fn list_templates(&self) -> Result<Vec<TemplateInfo>>;
    async fn delete_template(&self, id: &TemplateId) -> Result<()>;
}

#[async_trait]
pub trait SandboxControl: Send + Sync {
    async fn create_sandbox(&self, spec: SandboxSpec) -> Result<SandboxHandle>;
    async fn restore_sandbox(&self, snapshot: SnapshotRef, options: RestoreOptions) -> Result<SandboxHandle>;
    async fn attach_sandbox(&self, id: &SandboxId, options: AttachOptions) -> Result<SandboxHandle>;
    async fn inspect_sandbox(&self, id: &SandboxId) -> Result<SandboxInfo>;
    async fn list_sandboxes(&self, filter: SandboxFilter) -> Result<Vec<SandboxInfo>>;
    async fn stop_sandbox(&self, id: &SandboxId, mode: StopMode) -> Result<()>;
    async fn kill_sandbox(&self, id: &SandboxId, signal: KillSignal) -> Result<()>;
    async fn delete_sandbox(&self, id: &SandboxId, options: DeleteOptions) -> Result<()>;
    async fn update_deadline(&self, id: &SandboxId, deadline: SandboxDeadline) -> Result<()>;
}
```

Live sandbox capabilities should be split, not bagged:

```rust
pub trait LiveSandbox: Send + Sync {
    fn id(&self) -> &SandboxId;
    fn process(&self) -> Option<&dyn ProcessControl>;
    fn filesystem(&self) -> Option<&dyn FilesystemControl>;
    fn ports(&self) -> Option<&dyn PortControl>;
    fn snapshots(&self) -> Option<&dyn SnapshotControl>;
}
```

Rules:

- Traits are split by capability so unsupported behavior is visible.
- Optional capability accessors must line up with `Capabilities`.
- Do not add methods because one backend has an easy shortcut.
- Do not leak concrete backend request/response records into trait signatures.
- Contract tests belong in this crate because this crate can condemn wrong
  implementations.

### `contract.rs`

Owns backend conformance tests and fakes.

Exports:

- `BackendContract`
- `BackendContractConfig`
- `ContractBackendFactory`
- `run_backend_contract`
- `run_process_contract`
- `run_filesystem_contract`
- `run_snapshot_contract`
- `run_warm_pool_contract`
- `FakeBackend`
- `RecordingBackend`

Contract tests must cover:

- capability reports match optional trait accessors
- unsupported operations return `UnsupportedCapability`
- create/inspect/list/stop/delete lifecycle
- deadline update classification
- foreground command success and nonzero exit
- detached process start/connect/signal/stream
- stdin and PTY behavior when supported
- filesystem read/write/list/stat/remove
- copy-in/copy-out where supported
- snapshot capture/restore/delete where supported
- pause/resume refusal when unsupported
- warm-pool prewarm/checkout/evict where supported
- port connect/expose refusal or success
- event stream type stability
- error classification without parsing display text

These tests should run with fake backends in `firkin-sandbox` and be reusable
from backend crates.

### `prelude.rs`

Owns the small default import set for application authors.

Allowed exports:

- `Runtime`
- `RuntimeBuilder`
- `Capacity`
- `DataPlaneSpec`
- `TemplateSpec`
- `PreparedTemplate`
- `Sandbox`
- `SandboxSpec`
- `SandboxInfo`
- `SandboxState`
- `SnapshotRef`
- `WarmPoolSpec`
- `Command`
- `CommandOutput`
- `SandboxPath`
- `Capabilities`
- `Capability`
- `Error`
- `Result`

Rules:

- No backend concrete types in prelude.
- No E2B types.
- No benchmark/evidence types.
- No lower VM/container types.

## Public Type Families

Keep these families separate:

- runtime policy: `RuntimeConfig`, `Capacity`, `DeadlinePolicy`,
  `HygienePolicy`
- template intent: `TemplateSpec`, `TemplateSource`, `TemplateCommand`,
  `PreparedTemplate`
- data-plane ABI: `DataPlaneSpec`, `EnvdDataPlaneSpec`, `DataPlaneInfo`,
  `PreparedEnvdDataPlane`
- sandbox lifecycle: `SandboxSpec`, `SandboxInfo`, `SandboxState`,
  `SandboxDeadline`
- process execution: `Command`, `Process`, `ProcessEvent`, `PtySize`,
  `Signal`
- filesystem access: `SandboxPath`, `FileEntry`, `FileStat`
- snapshot lifecycle: `SnapshotRef`, `SnapshotInfo`, `SnapshotKind`,
  `PausedSandbox`
- warm-pool policy: `WarmPoolSpec`, `WarmLease`, `WarmEvictionPolicy`
- ports: `PortBinding`, `PortTarget`, `DomainProxy`
- events/logs/metrics: `Event`, `LogEntry`, `Metric`
- backend laws: `SandboxBackend`, `*Control`

Do not collapse these into one `SandboxConfig`, `VmSpec`, `RuntimeSpec`, or
`SessionRecord`.

## Relationship to Existing Crates

### `firkin-runtime`

Move neutral public records out of runtime only when they are real sandbox laws.

Keep in runtime:

- `SnapshotSessionLauncher`
- `SnapshotRestoreRequest`
- `RuntimeCommandRunner`
- `RuntimeInteractiveProcessRunner`
- `RuntimeReadinessProbe`
- `RuntimeSessionStop`
- `RuntimePortRouter`
- runtime disk guard and preflight execution
- runtime warm-pool mechanics
- continuation snapshot mechanics
- template-build runtime support

Runtime may implement `firkin-sandbox` traits for its own generic adapters, but
runtime remains a mechanics crate.

### `firkin-template`

Own actual template preparation mechanics:

- OCI pull/import
- architecture selection and validation
- envd layer/base injection
- entrypoint wrapping or backend-supervised envd startup
- default user/workdir provisioning
- setup/start/ready command execution during preparation
- health checks
- snapshot production
- artifact metadata and integrity production

`firkin-template` can know about envd packaging, OCI manifests, layer mutation,
rootfs writes, and backend preparation workflows. `firkin-sandbox` only owns the
public intent and prepared records those workflows satisfy.

### `firkin-single-node`

Implement `firkin-sandbox` using the existing local backend:

- map `SingleNodeConfig` into backend builder config
- map `SingleNodeCreateRequest` from `SandboxSpec`
- map `RuntimeCreatedSandbox` into live `Sandbox`
- map `CommandRequest`/`CommandOutput` into `Command`/`CommandOutput`
- map `SnapshotRecord`/`RuntimeSnapshotRef` into `SnapshotInfo`/`SnapshotRef`
- expose domain proxy through `PortControl`, not through E2B names
- keep local JSON state records private to `firkin-single-node`

`SingleNodeBackend` should stop being the preferred public library API. It can
remain an implementation object behind `AppleVzBackend` or equivalent.

### `firkin-e2b-contract`

During migration, keep E2B traits where they are. The final direction is:

- E2B contract types adapt to/from `firkin-sandbox` records.
- `EnvdProcessAdapter` and `EnvdFilesystemAdapter` stop being the only
  accessible process/filesystem law surfaces.
- E2B `RuntimeAdapter` becomes a compatibility adapter over `firkin-sandbox`,
  not the main runtime API.
- E2B/envd wire DTOs remain edge types. `firkin-sandbox::data_plane` may name
  envd as a guest ABI, but it must not import E2B wire or contract crates.

Do not move E2B wire DTOs into `firkin-sandbox`.

### `firkin`

Shrink root exports.

The facade should be curated:

- `firkin::sandbox` for orchestrators
- `firkin::core` for VM/container mechanics
- `firkin::runtime` for lower orchestration mechanics
- `firkin::e2b` for compatibility
- `firkin::benchmark`, `firkin::evidence`, `firkin::trace` as explicit opt-in
  modules only

Avoid root-level wildcard reexports of E2B server/wire types, benchmark suites,
and evidence records.

## Capability Semantics

Capabilities are mandatory API shape because local backends will differ.

Rules:

1. Methods may exist even when unsupported.
2. Unsupported methods return `Error::UnsupportedCapability`.
3. Capability status must explain whether the missing support is permanent,
   build-feature-gated, host-preflight-gated, or runtime-state-gated.
4. Callers must be able to ask for capabilities before starting work.
5. Contract tests must assert that capability reports and method behavior agree.

`pause` and `resume` are the most important example. A backend may support:

- snapshot capture
- snapshot restore into a new sandbox
- memory-backed pause
- reconnect/resume to the same sandbox identity

Those are different capabilities.

## Error Semantics

Public errors should preserve enough structure to avoid display parsing:

```rust
pub enum Error {
    UnsupportedCapability(UnsupportedCapability),
    InvalidSpec(InvalidSpec),
    NotFound(NotFound),
    AlreadyExists(AlreadyExists),
    CapacityRejected(CapacityRejected),
    DeadlineExceeded(DeadlineExceeded),
    TemplatePrepareFailure(TemplatePrepareFailure),
    ProcessFailure(ProcessFailure),
    FilesystemFailure(FilesystemFailure),
    SnapshotIntegrityFailure(SnapshotIntegrityFailure),
    PortFailure(PortFailure),
    BackendFailure(BackendFailure),
}
```

Every error carries:

- operation name
- sandbox/template/snapshot/process ID where applicable
- backend name where the failure came from a backend
- retryability when known
- source error where available

Do not define `Error::Runtime(String)` in this crate.

Template preparation failures must be structured because they are how Firkin
makes arbitrary images usable without turning runtime into a pile of late
surprises:

```rust
pub enum TemplatePrepareFailure {
    EnvdMissing { reference: Option<String> },
    EnvdWrongArch { expected: GuestArch, found: GuestArch },
    EnvdVersionMismatch { expected: String, found: String },
    EnvdIntegrityMismatch { expected_sha256: String, found_sha256: String },
    EnvdHealthFailed { port: u16, path: String, status: Option<u16> },
    EnvdInitFailed { reason: String },
    EntrypointUnsupported { reason: String },
    EntrypointNotWrapped,
    PortConflict { port: u16 },
    DefaultUserMissing { user: String },
    DefaultWorkdirMissing { path: SandboxPath },
    ReadOnlyRootfs,
    WritableLayerRequired,
    UnsupportedImageConfig { reason: String },
    RegistryUnavailable { reference: String },
    SnapshotAfterPrepareFailed { reason: String },
}
```

These variants are public law, not exact backend implementation errors. Backend
crates may keep richer internal error types and map them at the boundary.

## Bug Surface Containment

This is the central constraint.

`firkin-sandbox` should mostly contain:

- small newtypes
- public specs
- public records
- trait definitions
- thin handle forwarding methods
- capability checks
- validation
- contract-test fakes

It should not contain:

- VM launch code
- process spawning code
- shell snippets
- filesystem traversal on the host
- disk pressure checks
- schedulers
- background supervisor loops
- HTTP server construction
- envd binary builds
- envd HTTP/RPC calls
- OCI layer mutation
- benchmark runners
- evidence promotion
- local JSON stores
- Apple/VZ feature probes
- OCI pulls
- git clones
- ext4 image generation

Implementation code with operational side effects belongs in concrete backend
or runtime crates.

Use these size heuristics:

- `lib.rs` should be readable in under one minute.
- Each module should explain its refusal at the top.
- If a module exceeds roughly 500 lines, split by named concept, not by
  `types`/`impls`.
- If a trait has more than 8-10 methods, split it into a capability trait.
- If a type needs backend-specific fields, keep those fields in the backend and
  expose a neutral `metadata` map or typed extension only after a second backend
  proves the need.
- If a method starts a task, opens a file, binds a socket, launches a VM, or
  shells out, it does not belong in this crate.

## Implementation Plan

### Slice 1: Spec and crate scaffold

1. Add `crates/sandbox`.
2. Add `firkin-sandbox` to workspace members.
3. Add module skeletons and empty public law types.
4. Add the crate to `scripts/check-firkin-crate-graph.sh` allowlist with only
   permitted dependencies.
5. Add compile tests that prove forbidden imports are absent.

Acceptance:

- `cargo check -p firkin-sandbox --all-targets`
- `scripts/check-firkin-crate-graph.sh`

### Slice 2: Public types and capability/error laws

1. Implement IDs, errors, capabilities, specs, and records.
2. Add unit tests for validation and error classification.
3. Add prelude.

Acceptance:

- `cargo test -p firkin-sandbox`
- docs compile for basic examples

### Slice 3: Template data-plane contract

1. Add `data_plane.rs`.
2. Add `DataPlaneSpec`, `EnvdDataPlaneSpec`, `DataPlaneInfo`, and
   `TemplatePrepareFailure`.
3. Make `TemplateSpec::oci` default to `DataPlaneSpec::envd().inject()`.
4. Add explicit `DataPlaneSpec::none()` and `already_present()` paths.
5. Add validation tests for reserved ports, arch requirements, init mode,
   version/provenance fields, and typed prepare failures.

Acceptance:

- `cargo test -p firkin-sandbox data_plane`
- docs compile for envd inject, envd verify, and no-data-plane examples

### Slice 4: Backend traits and fake contract backend

1. Add `SandboxBackend` and split control traits.
2. Add fake/recording backend.
3. Add contract tests over fake backend.
4. Ensure unsupported capability behavior is tested.

Acceptance:

- `cargo test -p firkin-sandbox backend_contract`

### Slice 5: Single-node backend adapter

1. Add `firkin-single-node -> firkin-sandbox`.
2. Introduce `AppleVzBackend` or equivalent public backend wrapper.
3. Map existing `SingleNodeBackend` operations into sandbox laws.
4. Keep `SingleNodeBackend` available only as implementation detail or
   explicit low-level module.

Acceptance:

- targeted single-node tests through `firkin-sandbox`
- existing single-node tests still pass

### Slice 6: Runtime/template/envd rebasing

1. Move template prepare orchestration behind `firkin-template`/runtime
   implementation code.
2. Replace `has_envd: bool` with `DataPlaneInfo::Envd` at the public boundary.
3. Add envd injection or verification for OCI templates.
4. Ensure Apple/VZ envd starts with non-Firecracker init semantics or an
   explicit metadata provider.
5. Prove arbitrary-image prepare errors fail before sandbox create.

Acceptance:

- envd-injected OCI template prepares and health-checks on Apple/VZ
- already-present envd template verifies without reinjection
- unsupported arbitrary image shapes return `TemplatePrepareFailure`

### Slice 7: Runtime/E2B process/filesystem rebasing

1. Move neutral process/filesystem concepts out from E2B naming into
   `firkin-sandbox`.
2. Adapt E2B envd process/filesystem traits to call sandbox controls.
3. Keep E2B wire and server shapes isolated.

Acceptance:

- E2B adapter tests pass through the new neutral process/filesystem surface
- no E2B DTOs imported by `firkin-sandbox`

### Slice 8: Facade hard cutover

1. Add `firkin::sandbox`.
2. Reexport `firkin_sandbox::*` there.
3. Add backend reexports under `firkin::sandbox::apple_vz` behind feature or
   explicit facade dependency.
4. Remove broad root-level E2B/benchmark/evidence reexports from the default
   public path.

Acceptance:

- public example compiles using `firkin::sandbox`
- old facade wildcard path is gone, not shimmed

### Slice 9: Live proof

1. Run a local Apple/VZ flow through `firkin::sandbox`:
   - prepare template
   - prewarm
   - create sandbox
   - exec command
   - filesystem write/read
   - snapshot
   - stop
   - restore
   - exec command again
2. Capture JSON/HTML evidence if this becomes part of a release milestone.

Acceptance:

- live flow passes on supported host
- unsupported features report structured capability errors

## Verification

For spec-only changes:

```bash
scripts/check-firkin-crate-graph.sh
git diff --check -- docs/plans/2026-05-07-firkin-sandbox-library-surface-spec.md
```

For implementation slices:

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo fmt --all --check
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo check --workspace --all-targets
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo clippy --workspace --all-targets -- -D warnings
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test --workspace --all-targets
scripts/check-firkin-crate-graph.sh
git diff --check
```

Use an external Cargo target dir on the odd `https:/github.com` checkout path
when compiling from that checkout family.

## Decisions Locked By This Revision

1. The high-level sandbox template surface has a guest data-plane contract.
2. envd is the default guest data plane for managed OCI sandbox templates.
3. envd is not E2B ownership. `firkin-sandbox` may name envd only as a
   data-plane ABI and must keep E2B DTOs out.
4. Arbitrary user images remain immutable source inputs. Firkin prepares a
   derived template artifact when envd injection, entrypoint wrapping, metadata,
   or snapshotting is required.
5. `has_envd: bool` is not an acceptable public prepared-template fact. Replace
   it at the public boundary with `DataPlaneInfo::Envd`.
6. `prepare` is the sharp-edge boundary. Unsupported image shape, wrong arch,
   bad entrypoint, missing user/workdir, port conflict, read-only rootfs, envd
   health failure, and registry/image failures return typed
   `TemplatePrepareFailure`.
7. Per-sandbox auth/init/env/default workdir/runtime metadata are applied after
   create/restore. They are not baked into the prepared template.
8. On Apple/VZ, Firkin-managed envd must not accidentally depend on Firecracker
   MMDS. Use non-Firecracker init semantics or make metadata support an explicit
   backend capability.

## Open Decisions

1. Whether `firkin-sandbox` should depend on `firkin-artifacts` directly for
   snapshot integrity, or define neutral `SnapshotIntegrity` records and convert
   in runtime/backend crates.
2. Whether stream types should use `tokio_stream::Stream`,
   `futures_core::Stream`, or a small crate-owned wrapper to minimize
   dependencies.
3. Whether backend traits should use `async-trait` and dyn dispatch from day
   one, or generic associated future style. The existing workspace commonly
   uses `async-trait`, so the pragmatic first cut is `async-trait`.
4. Whether `firkin-runtime` should depend on `firkin-sandbox` immediately or
   only after the first backend adapter proves the edge.
5. Whether `TemplateSource::Git` belongs in `firkin-sandbox` now or should wait
   until a second template backend proves it is a stable public law.
6. Exact envd provenance source for first release: vendored source build,
   pinned release artifact, or pinned OCI layer/base image. The public law
   requires version/commit/sha256 either way.
7. Exact injection mechanism for arbitrary OCI images: offline OCI layer/config
   mutation, boot-and-copy through a preparer VM, or both. The public behavior
   is already decided; the implementation route is not.
8. First-cut support tier for distroless/scratch images. The desired behavior is
   typed prepare failure or backend-supervised envd startup, never a late
   runtime surprise.
9. Whether Firkin should use envd's internal port scanner/fan-out for app ports
   or only use envd for process/filesystem/PTY while keeping all external port
   routing in Firkin's `PortControl`.
10. Whether the default envd port is always `49983` or backend-configurable in
    the first public release. If configurable, prepared-template metadata must
    record the actual reserved port.

## Final Rule

`firkin-sandbox` should make the obvious user move correct:

```rust
use firkin::sandbox::{Runtime, SandboxSpec, TemplateSpec, Command};
```

It should not make the obvious contributor move dangerous. If adding a feature
to `firkin-sandbox` requires knowing how Apple/VZ boots, how envd routes HTTP,
how E2B serializes DTOs, how benchmark evidence is promoted, or how local JSON
state is persisted, the feature belongs in another crate.
