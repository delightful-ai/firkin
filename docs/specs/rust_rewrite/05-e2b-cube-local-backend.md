# E2B/Cube-compatible local backend

> Status: working design note, 2026-05-03.
>
> Goal: make the E2B SDK and CubeSandbox examples work against an optional
> local backend, while keeping Linux/Cube as the current native-supported path
> and adding Apple Silicon as another host-runtime target.

This is a wire-compatibility project first. The client should be the existing
E2B SDK or the CubeSandbox examples; the new code should sit behind the
E2B-compatible control plane and data-plane contracts they already speak.

The local backend should not treat Apple Silicon as the whole architecture, and
it should not start by inventing a Rust E2B client. The native-feeling path is:

1. Keep the CubeAPI/E2B HTTP surface as the compatibility boundary.
2. Keep Linux/Cube as a first-class local runtime because that is the current
   supported runtime shape.
3. Add host-runtime adapters behind the same control/data-plane server.
4. Route the E2B SDK's envd and exposed-port traffic to the right sandbox,
   regardless of whether the sandbox is backed by Linux/KVM/Cube or Apple/VZ.

## Source map

Local sources used for this note:

- Apple Rust rewrite surface: `docs/specs/rust_rewrite/04-library-surface/`.
- Apple spike evidence: `docs/specs/rust_rewrite/spike-logs/s1-*` through
  `s9-*`.
- CubeSandbox checkout:
  `/Users/darin/vendor/github.com/https://github.com/TencentCloud/CubeSandbox`.
- E2B SDK checkout: `/Users/darin/vendor/github.com/e2b-dev/E2B`.

Important concrete files:

- Cube routes: `CubeAPI/src/routes.rs`.
- Cube request/response models: `CubeAPI/src/models/mod.rs`.
- Cube current sandbox service: `CubeAPI/src/services/sandboxes.rs`.
- Cube current template service: `CubeAPI/src/services/templates.rs`.
- E2B Python host naming: `packages/python-sdk/e2b/connection_config.py`.
- E2B Python sandbox data-plane setup:
  `packages/python-sdk/e2b/sandbox/main.py`.
- E2B Python envd helpers: `packages/python-sdk/e2b/envd/`.
- E2B JS host naming: `packages/js-sdk/src/connectionConfig.ts`.
- E2B envd specs: `spec/envd/envd.yaml`,
  `spec/envd/process/process.proto`, and
  `spec/envd/filesystem/filesystem.proto`.

## Compatibility boundary

The bridge must satisfy two separate contracts.

### 1. Control plane

The E2B/Cube control plane is the HTTP API that creates, lists, connects,
pauses, resumes, deletes, and inspects sandboxes.

CubeAPI already exposes the useful route shell:

- `GET /health`
- `GET /sandboxes`
- `POST /sandboxes`
- `GET /v2/sandboxes`
- `GET /sandboxes/:sandboxID`
- `DELETE /sandboxes/:sandboxID`
- `GET /sandboxes/:sandboxID/logs`
- `GET /v2/sandboxes/:sandboxID/logs`
- `POST /sandboxes/:sandboxID/timeout`
- `POST /sandboxes/:sandboxID/refreshes`
- `POST /sandboxes/:sandboxID/pause`
- `POST /sandboxes/:sandboxID/resume`
- `POST /sandboxes/:sandboxID/connect`
- `POST /sandboxes/:sandboxID/snapshots`
- template routes under `/templates`

The official SDK route set is a little wider than CubeAPI's current root
router. Full compatibility also needs:

- `GET /sandboxes/:sandboxID/metrics`
- `GET /sandboxes/metrics`
- `GET /snapshots`
- template build/tag/file-upload routes used by the SDK template builders
- volume routes used by the SDK volume clients

The create request must preserve the E2B field names. CubeAPI already models
the important ones:

- `templateID`
- `timeout`
- `autoPause`
- `autoResume`
- `secure`
- `allow_internet_access`
- `network.allowPublicTraffic`
- `network.allowOut`
- `network.denyOut`
- `network.maskRequestHost`
- `metadata`
- `envVars`
- `mcp`
- `volumeMounts`

The local vendored E2B Rust SDK currently models this as
`SandboxCreateOpts`: `allow_internet_access` is a top-level optional boolean
that defaults to `true`, while the nested `NetworkConfig` carries camelCase
`allowOut`, `denyOut`, `allowPublicTraffic`, and `maskRequestHost`. The Rust
rewrite substrate mirrors that request shape in `firkin_types::SandboxNetworkPolicy`;
runtime adapters must still lower it to real enforcement.

The create/connect response must return at least:

- `templateID`
- `sandboxID`
- `clientID`
- `envdVersion`
- optional `envdAccessToken`
- optional `trafficAccessToken`
- `domain`

That `domain` is not decorative. The E2B SDK uses it to construct sandbox
hosts.

`SandboxDetail` responses also need to return the SDK-visible runtime fields
that Cube currently only partially models:

- `allowInternetAccess`
- `network`
- `lifecycle`
- `volumeMounts`
- accurate `diskSizeMB`
- `endAt` that tracks timeout/refresh/connect behavior

### 2. Data plane

The E2B SDK talks to envd and exposed sandbox ports after the control plane
returns a sandbox.

Python SDK host construction:

- envd port: `49983`
- MCP port: `50005`
- debug mode: `localhost:{port}`
- normal mode: `{port}-{sandboxID}.{sandboxDomain}`

The local backend therefore needs a local CubeProxy-equivalent that can accept
hosts such as:

- `49983-<sandboxID>.<domain>` for envd commands/files.
- `49999-<sandboxID>.<domain>` for the code interpreter service used by the
  CubeSandbox quickstart template.
- `50005-<sandboxID>.<domain>` for MCP.
- any template-exposed port returned through `sandbox.get_host(port)`.

For single-sandbox smoke tests, E2B debug mode can use `localhost:{port}`. For
real compatibility, the backend needs the host-based proxy so multiple
sandboxes and arbitrary exposed ports work without patching SDK callers.

The envd data plane is not vminitd. vminitd is the Apple/containerization guest
agent on vsock `1024`; envd is the E2B SDK protocol surface on port `49983`.
The bridge must either run envd inside the sandbox or provide an envd bridge
that translates E2B calls to runtime operations.

Minimum envd surface:

- HTTP/OpenAPI endpoints: `/health`, `/files`, `/metrics`, `/init`, `/envs`.
- process Connect RPC JSON service `process.Process`:
  - `List`
  - `Connect`
  - `Start`
  - `Update`
  - `StreamInput`
  - `SendInput`
  - `SendSignal`
  - `CloseStdin`
- filesystem Connect RPC JSON service `filesystem.Filesystem`:
  - `Stat`
  - `MakeDir`
  - `Move`
  - `ListDir`
  - `Remove`
  - `WatchDir`
  - `CreateWatcher`
  - `GetWatcherEvents`
  - `RemoveWatcher`
- HTTP `/files` read/write/upload/download behavior, including optional signed
  URL support when `envdAccessToken` is returned.
- headers:
  - `X-Access-Token` for envd auth when enabled
  - `E2b-Sandbox-Id`
  - `E2b-Sandbox-Port`

## Backend shape

Add a backend seam inside CubeAPI instead of baking one host runtime into the
current CubeMaster service.

```rust
#[async_trait]
pub trait SandboxBackend: Clone + Send + Sync + 'static {
    async fn list(&self, filters: ListFilters) -> Result<Vec<ListedSandbox>>;
    async fn create(&self, req: NewSandbox) -> Result<Sandbox>;
    async fn get(&self, sandbox_id: &str) -> Result<SandboxDetail>;
    async fn delete(&self, sandbox_id: &str) -> Result<()>;
    async fn pause(&self, sandbox_id: &str) -> Result<()>;
    async fn resume(&self, sandbox_id: &str, timeout: i32) -> Result<Sandbox>;
    async fn connect(&self, sandbox_id: &str, timeout: i32) -> Result<Sandbox>;
    async fn set_timeout(&self, sandbox_id: &str, timeout: i32) -> Result<()>;
    async fn refresh(&self, sandbox_id: &str, duration: i32) -> Result<()>;
    async fn logs(&self, sandbox_id: &str, cursor: LogCursor) -> Result<SandboxLogs>;
    async fn snapshot(&self, sandbox_id: &str, name: Option<String>) -> Result<SnapshotInfo>;
    async fn list_snapshots(&self, query: SnapshotQuery) -> Result<Vec<SnapshotInfo>>;
    async fn delete_snapshot(&self, snapshot_id: &str) -> Result<()>;
    async fn metrics(&self, sandbox_id: &str, range: MetricsRange) -> Result<Vec<SandboxMetric>>;
    async fn metrics_many(&self, sandbox_ids: &[String]) -> Result<SandboxesWithMetrics>;
    async fn update_network(&self, sandbox_id: &str, network: SandboxNetworkConfig) -> Result<()>;
    async fn templates(&self) -> Result<Box<dyn TemplateBackend>>;
    async fn volumes(&self) -> Result<Box<dyn VolumeBackend>>;
}
```

Implementations:

- `CubeLinuxBackend`: current CubeMaster/CubeLinux behavior, moved out of the
  current service modules without changing the public routes.
- `LocalRuntimeBackend<R>`: local compatibility backend parameterized by a host
  runtime adapter. It owns the sandbox registry, template registry, proxy
  routes, timeout task, snapshot store, and E2B policy semantics.

Runtime adapters:

- `CubeLinuxRuntime`: Linux/KVM/Cube-backed runtime. This is the current
  supported native runtime family.
- `AppleVzRuntime`: Apple Silicon/macOS runtime backed by
  Virtualization.framework through the Rust rewrite/containerization substrate.

Configuration should select exactly one backend:

- `CUBE_BACKEND=cubemaster`
- `CUBE_BACKEND=local`

When `local` is selected, configuration should select exactly one runtime:

- `CUBE_LOCAL_RUNTIME=cube-linux`
- `CUBE_LOCAL_RUNTIME=apple-vz`

No hybrid fallback path. If a runtime is selected and a required capability is
missing, preflight should fail loudly.

Concrete CubeAPI layout:

```text
CubeAPI/src/backend/mod.rs
CubeAPI/src/backend/cube_linux.rs
CubeAPI/src/backend/local_runtime.rs
CubeAPI/src/runtime/mod.rs
CubeAPI/src/runtime/cube_linux.rs
CubeAPI/src/runtime/apple_vz.rs
CubeAPI/src/proxy/mod.rs
```

`backend/` owns E2B/Cube JSON semantics. `runtime/` owns host mechanics and
must not depend on E2B request/response models.

## Local components

### `LocalRuntimeBackend<R>`

Responsibilities:

- Keep `sandboxID -> SandboxRecord` state.
- Translate `NewSandbox` into a runtime launch request.
- Return the E2B-compatible `Sandbox` response with the local proxy domain.
- Drive lifecycle transitions: running, paused, deleted.
- Own timeout and refresh timers.
- Capture logs and metrics.
- Coordinate snapshots.

`SandboxRecord` should include:

- sandbox id, template id, client id, domain
- metadata, env vars, volume mounts
- requested network policy
- CPU, memory, disk sizing
- exposed ports
- VM/container handles
- envd/proxy target
- lifecycle timestamps and timeout deadline
- snapshot path, if paused/snapshotted

### `RuntimeAdapter`

The runtime adapter is the only layer that should know whether a sandbox is
Linux/KVM/Cube-backed or Apple/VZ-backed.

```rust
#[async_trait]
pub trait RuntimeAdapter: Clone + Send + Sync + 'static {
    async fn preflight(&self) -> Result<RuntimeCapabilities>;
    async fn prepare_template(&self, template: TemplateSpec) -> Result<PreparedTemplate>;
    async fn start(&self, req: StartSandbox) -> Result<RuntimeSandbox>;
    async fn stop(&self, sandbox_id: &str) -> Result<()>;
    async fn pause(&self, sandbox_id: &str) -> Result<PausedSandbox>;
    async fn resume(&self, paused: PausedSandbox) -> Result<RuntimeSandbox>;
    async fn snapshot(&self, sandbox_id: &str, name: Option<String>) -> Result<SnapshotRef>;
    async fn metrics(&self, sandbox_id: &str) -> Result<RuntimeMetrics>;
    async fn logs(&self, sandbox_id: &str, cursor: LogCursor) -> Result<RuntimeLogs>;
    async fn apply_network(&self, sandbox_id: &str, policy: NetworkPolicy) -> Result<()>;
    async fn port_target(&self, sandbox_id: &str, port: u16) -> Result<PortTarget>;
}
```

`LocalRuntimeBackend<R>` owns E2B/Cube semantics. `RuntimeAdapter` owns host
mechanics.

### Template registry

Cube's quickstart creates templates from OCI images with writable layer size,
exposed ports, and a probe port. The local backend needs the same minimum
template facts:

- `templateID`
- OCI image reference or local rootfs artifact
- platform policy: `linux/arm64` by default, `linux/amd64` only when Rosetta is
  enabled and preflight passed
- writable layer size
- exposed ports
- readiness probe
- default command/env/cwd/user from the OCI config
- whether envd/code-interpreter/MCP services are present

For first compatibility, use templates that already contain envd and the code
interpreter service. That avoids reimplementing the E2B envd protocol on day
one and lets the official SDK exercise its normal commands, files, and
`run_code` paths.

Full compatibility needs template operations to become real, not static config:

- create template from image
- rebuild template
- get build status
- stream real build logs
- delete template or snapshot-backed template id
- preserve tags/aliases if the SDK path uses them
- provide volume mounts declared by `volumeMounts`

### Proxy

The proxy is required for full SDK compatibility.

Inputs:

- host header of the form `{port}-{sandboxID}.{domain}`
- optional auth tokens from the E2B response
- sandbox registry entry

The host-header grammar is now represented by `firkin_types::PortSandboxHost`.
It parses and validates `{port}-{sandboxID}.{domain}` for the configured local
domain and deliberately rejects debug `localhost:{port}` shortcuts. The
remaining proxy work should consume that type rather than reparsing host strings
inside route handlers.

Outputs:

- host-to-guest TCP forwarding for template-exposed ports
- host-to-envd forwarding on port `49983`
- MCP forwarding on port `50005`

The proxy can forward through whichever route the selected runtime exposes:

- Linux/KVM/Cube network target
- vmnet IP if an Apple/VZ sandbox has a reachable guest/container address
- vsock-to-guest bridge if envd is reachable through vsock
- host-side envd shim if the project later chooses to implement envd natively

For a local developer setup, prefer `*.localhost` or an explicit local domain
with documented resolver/cert setup. Debug mode can be kept for single-sandbox
smoke, but full compatibility should not depend on debug mode.

The proxy needs HTTP, WebSocket, and Connect/gRPC-web behavior. It cannot be an
envd-only shortcut because `sandbox.get_host(port)` exposes arbitrary template
ports to agent frameworks and the code-interpreter path uses `49999`.

### Runtime substrates

#### Linux/Cube runtime

CubeSandbox is already a Linux/KVM-oriented runtime. For this design, that is
not a legacy path; it is the current native local runtime family. The backend
contract should keep it cleanly selectable rather than replacing it with
Apple-specific code.

The local compatibility work still matters on Linux because CubeAPI currently
has route coverage that is ahead of some backend behavior. Logs and snapshots,
for example, have placeholder/fallback behavior when CubeMaster endpoints are
missing. Full compatibility requires those to become real runtime operations,
not successful placeholder responses.

#### Apple/VZ runtime

Map current Apple Containerization features to E2B/Cube needs:

| Need | Apple surface | State |
|---|---|---|
| Boot VM | `ContainerManager`, `VZVirtualMachineManager` | implementable now |
| Guest agent RPC | `Vminitd` over vsock | implementable now |
| Process exec/stdio | `LinuxContainer.exec`, `LinuxProcess` | implementable now |
| Copy files/directories | `LinuxContainer.copyIn`, `copyOut` | implementable now |
| OCI pull/rootfs | `ImageStore.get(reference:pull:)`, EXT4 unpacker | implementable now |
| Metrics | `LinuxContainer.statistics` | implementable now for basic E2B metrics |
| Network attach | `VmnetNetwork` | implementable on macOS 26+ |
| amd64 on Apple Silicon | Rosetta option on VZ manager | implementable with preflight |
| E2B envd API | no native surface | not implementable without guest envd or bridge |
| Snapshot-backed pause/resume | VM pause/resume only | not E2B-compatible snapshot lifecycle |
| E2B network policy | interface configuration only | not implementable without guest/host firewall work |

The Apple/VZ runtime can be implemented as a local sidecar in this repo rather
than embedding Apple-specific logic inside CubeAPI. The sidecar would expose
runtime primitives over a Unix domain socket or localhost-only RPC interface,
while CubeAPI's `AppleVzRuntime` client implements the runtime adapter.

Concrete Apple-side layout now in this package:

```text
Package.swift
Sources/AppleLocalRuntime/main.swift
Sources/AppleLocalRuntimeCore/RuntimeTypes.swift
Sources/AppleLocalRuntimeCore/AppleVzRuntime.swift
Tests/AppleLocalRuntimeCoreTests/CapabilityTests.swift
```

The sidecar reuses the existing Swift surfaces first:

- `Sources/Containerization/ContainerManager.swift`
- `Sources/Containerization/LinuxContainer.swift`
- `Sources/Containerization/Vminitd.swift`
- `Sources/Containerization/SandboxContext/SandboxContext.proto`

The first Apple runtime does not include envd, domain proxy, snapshot pause, or
network policy. Those are listed as unsupported runtime capabilities until
there is a real Apple Containerization-backed implementation.

## Required feature set for full compatibility

### Must implement

1. Control-plane backend seam in CubeAPI.
2. Local backend registry, template registry, proxy, lifecycle timers, and
   runtime adapter contract.
3. Runtime-specific preflight:
   - Linux/Cube: KVM/RustVMM/Cube dependencies and network/runtime services.
   - Apple/VZ: macOS, architecture, Virtualization.framework entitlement,
     vmnet availability, Rosetta when requested, and rootfs writer support.
4. Template registry that can launch the Cube/E2B code-interpreter style image
   with envd on `49983` and service probe/exposed port `49999`.
5. Local proxy for `{port}-{sandboxID}.{domain}` hostnames.
6. Commands/files/run-code data-plane compatibility through guest envd or a
   native envd shim.
7. Lifecycle routes: create, get, list, delete, connect, pause, resume,
   timeout, refresh.
8. E2B metrics routes: `GET /sandboxes/:sandboxID/metrics` and
   `GET /sandboxes/metrics`.
9. E2B snapshot list route: `GET /snapshots`.
10. Real logs and metrics. Placeholders are not compatibility.
11. Real snapshots for `pause`/`resume`/`connect` behavior. In-memory VM pause
   is useful for smoke but is not E2B pause compatibility; do not implement it
   as the Apple backend's answer to E2B pause.
12. Network policy mapping for `allow_internet_access`, `allowPublicTraffic`,
   `allowOut`, `denyOut`, and `maskRequestHost`.
13. Volume/host mount handling for `metadata.host-mount` and `volumeMounts`.
14. Per-template exposed port routing, including browser/CDP style templates
    and MCP on `50005`.
15. Compatibility tests that run the official E2B SDK examples unchanged except
    for `E2B_API_URL`, `E2B_API_KEY`, and template id/domain environment.

### Hard blockers or not-yet-proven pieces

These cannot be hidden behind successful route handlers:

- Apple/VZ: the Rust rewrite public crates are landed for the local
  runtime/library surface, but that does not by itself provide the E2B/Cube
  control plane, envd data plane, domain proxy, or policy layer.
- Apple/VZ: the VZ save/restore primitive is proved by the Rust S10 replay,
  but full E2B pause/resume compatibility still needs the outer sandbox
  registry, snapshot retention, and connect/resume API semantics.
- Apple/VZ: large real templates need production-grade rootfs assembly.
  Multi-block-group images are implemented, but deeper extent trees, htree
  directories, metadata checksums, and journal support are still listed as gaps.
- Cube's egress policy model maps naturally to CubeVS on Linux. Apple/VZ
  needs an explicit implementation, likely guest firewall/nftables rules or
  host PF anchors per sandbox.
- Hostname/TLS setup must be solved for non-debug SDK traffic. Returning a
  domain that the SDK cannot resolve or trust is not compatibility.
- Current CubeAPI gaps need hard behavior, not soft success:
  - logs cannot return synthetic placeholder lines
  - snapshots cannot return synthetic ids
  - template build logs cannot be synthesized from status only
  - metrics routes cannot be absent
  - `/snapshots` cannot be absent
  - `nextToken` cannot be accepted and ignored if pagination is claimed
  - `maskRequestHost`, `envVars`, `secure`, `mcp`, and `volumeMounts` cannot be
    silently ignored if full SDK compatibility is claimed

## Minimal implementation sequence

### Milestone 0: contract doc and tests

- Check in this document.
- Add compatibility test definitions before backend code:
  - Python SDK create/get/list/delete.
  - Python SDK `commands.run("echo ok")`.
  - Python SDK file write/read.
  - Python SDK file stat/list/mkdir/move/remove/watch, or explicit unsupported
    failures before claiming filesystem compatibility.
  - Python SDK `sandbox.get_host(49999)` reaches the code interpreter probe.
  - pause/connect round trip.
  - metrics route if metrics are advertised.
  - snapshot create/list/delete if snapshots are advertised.
  - no-internet and allowlist/denylist behavior.

### Milestone 1: single-sandbox local smoke per runtime

Use the fastest route to prove the stack:

- one sandbox in the selected runtime
- one template with guest envd already installed
- E2B debug mode
- envd on localhost port `49983`
- code interpreter on localhost port `49999`
- create/get/delete, commands, files, and `run_code`

This is not full compatibility, but it proves the data-plane decision.

### Milestone 2: real proxy and multi-sandbox

- Add the host-based proxy.
- Return `domain` that the SDK can resolve locally.
- Run two sandboxes concurrently.
- Route `49983-<sandboxA>...` and `49983-<sandboxB>...` independently.
- Route arbitrary exposed template ports.

### Milestone 3: lifecycle parity

- Implement timeout and refresh as real local timers.
- Implement logs from captured boot/envd/process output.
- Implement metrics from VM/container statistics.
- Implement pause/resume/connect against a real snapshot path.
- Return correct `endAt`, `allowInternetAccess`, `network`, and `lifecycle`
  fields from `SandboxDetail`.

### Milestone 4: policy and storage parity

- Implement network allow/deny/mask policy.
- Implement host mounts and volume mounts.
- Implement template creation/build status enough for Cube's
  `create-from-image` flow.
- Add Rosetta-backed amd64 template support behind explicit preflight.

## What to avoid

- Do not make "route exists" count as compatibility when the implementation
  returns placeholders.
- Do not implement Apple backend operations that current Apple Containerization
  APIs cannot actually perform. Expose those as unsupported capabilities with a
  concrete reason.
- Do not make E2B debug mode the only supported path; it breaks multi-sandbox
  host routing.
- Do not claim snapshot/pause parity from the VM save/restore primitive alone;
  E2B parity also needs the outer lifecycle, retention, and connect semantics.
- Do not translate CubeSandbox's KVM host model directly; use the E2B/Cube
  wire contract and selected runtime primitives.
- Do not pretend vminitd is envd. Either run envd or bridge envd protocol
  calls deliberately.
- Do not hide missing network policy behind `allow_internet_access=true`; agent
  frameworks rely on the allow/deny surface for containment.

## Open design decisions

1. Whether first-class envd compatibility is guest-envd-only, host-native shim,
   or guest first with later host-native replacement.
2. Whether the local backend lives in CubeAPI directly or as a sibling crate
   that CubeAPI links behind the backend trait.
3. Local domain choice: `*.localhost`, `cube.localhost`, or a managed
   resolver/cert setup.
4. Snapshot format and retention policy for paused sandboxes.
5. Exact policy engine for egress allow/deny/mask per runtime.
