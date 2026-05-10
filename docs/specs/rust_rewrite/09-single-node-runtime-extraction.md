# Single-node runtime extraction

Status: architecture spec, 2026-05-05.

This spec defines how to move the Firkin-backed single-node backend work out of
CubeAPI and into Firkin without turning Firkin into CubeAPI. It is intentionally
specific about what moves, what stays, what is merged with existing Firkin
runtime code, and what must not be imported.

The target is a production-grade Apple/VZ single-node runtime substrate for a
powerful Mac. CubeAPI remains the E2B/Cube-compatible HTTP product. Firkin owns
the VM/container runtime mechanics, snapshot lifecycle, template snapshot
execution, warm-pool/capacity policy, and local protocol sidecars needed to make
SDK clients work.

## Decision

Add a `single_node` module tree inside `firkin-runtime`.

Do not add a new `firkin-api` crate for this extraction. Do not move CubeAPI
HTTP routes into Firkin. Do not make `firkin-e2b` depend on Apple/VZ, snapshots,
template execution, capacity policy, durable runtime state, or Cube product
models.

The crate-level split is:

```text
firkin-e2b
  Protocol and control-plane compatibility contracts.
  Owns E2B/envd/domain-proxy wire types, RuntimeAdapter, LocalRuntimeBackend,
  and HTTP-shaped protocol sidecars.

firkin-runtime
  Production runtime composition.
  Owns Apple/VZ-backed lifecycle orchestration, snapshot restore/save, warm
  pools, capacity admission, disk pressure, runtime state, reconciliation,
  and concrete implementations of firkin-e2b contracts.

CubeAPI
  Product HTTP server.
  Owns routes, auth, request/response DTOs, API errors, OpenAPI shape,
  template build-status routes, and the SandboxBackend trait used by the
  Cube service layer.
```

The dependency direction stays:

```text
CubeAPI
  -> firkin facade / firkin-runtime
    -> firkin-core + firkin-template + firkin-substrate + firkin-e2b
```

No lower crate depends back on `firkin-runtime`, and `firkin-e2b` stays a
protocol crate rather than becoming the Apple/VZ runtime implementation.

## Why not a new API crate

A new API crate would only be justified if Firkin itself were shipping a
standalone HTTP API server. That is not this extraction.

The current need is not "put HTTP in Firkin." The need is "make the Cube-proven
single-node Apple/VZ backend a real Firkin runtime library surface." That
belongs in `firkin-runtime`, because it is the crate already allowed to compose
`core`, `template`, `substrate`, and `e2b`.

If a later product needs a standalone Firkin daemon, add a higher-level crate
such as `firkin-server` above `firkin-runtime`. That crate may own HTTP routes,
auth, config-file parsing, service lifecycle, and operator endpoints. It should
not be introduced now as a vague middle layer.

## HTTP and protocol semantics

Firkin runtime must not grow CubeAPI HTTP semantics.

It may own HTTP-shaped protocol sidecars when those are part of the runtime data
plane:

- envd process/filesystem protocol handling.
- domain proxy host routing.
- code-interpreter/MCP port routing when implemented as protocol transport.
- local proxy adapters that implement `firkin-e2b` traits.

It must not own product HTTP semantics:

- route paths such as `POST /sandboxes`.
- status-code selection.
- Cube/E2B request and response DTO layout.
- API auth and access-token policy.
- OpenAPI generation.
- template build route status shape.
- SaaS tenancy, quota, organization, billing, or cluster scheduling.

In code, Firkin should expose Rust APIs and trait implementations:

```rust
SingleNodeBackend::create(...)
SingleNodeBackend::delete(...)
SingleNodeBackend::snapshot(...)
SingleNodeBackend::run_command(...)
SingleNodeBackend::build_template_snapshot(...)
SingleNodeBackend::domain_proxy(...)
```

Cube maps those calls to HTTP behavior through its existing service layer.

## Module layout

Create this module tree under `crates/runtime/src/single_node/`:

```text
single_node/
  mod.rs
  config.rs
  error.rs
  model.rs
  orchestration.rs
  backend.rs
  state.rs
  proxy.rs
  template.rs
```

`mod.rs` exports a small, deliberate public surface. Everything else is
`pub(crate)` unless a downstream consumer needs it.

### `config.rs`

Owns single-node runtime configuration:

- `SingleNodeConfig`
- capacity limits
- default resource budget
- runtime roots
- snapshot/log/active-marker paths
- free-disk floors
- active queue policy
- warm-pool targets
- domain/envd metadata

It should use existing Firkin types where possible:

- `ResourceBudget`
- `CapacityLedger`
- `ActiveQueuePolicy`
- `Size`
- `RuntimePreflight`

It should not expose Cube config structs.

### `error.rs`

Owns a Firkin runtime error enum for single-node operations.

The error model must be runtime-oriented, not HTTP-oriented. It may include
variants such as:

- invalid request
- unsupported capability
- capacity rejected
- disk pressure
- snapshot not found
- sandbox not found
- template build failed
- runtime launch failed
- runtime command failed
- state persistence failed
- protocol/proxy failure

Cube maps these to `AppError` and HTTP status codes in its adapter.

### `model.rs`

Owns neutral runtime models. These are not Cube HTTP DTOs.

Initial model set:

- `SingleNodeCreateRequest`
- `SingleNodeRuntimeMode`
- `SandboxResources`
- `SandboxSession`
- `SandboxSessionState`
- `SnapshotRecord`
- `SnapshotKind`
- `TemplateMetadata`
- `CommandRequest`
- `CommandOutput`
- `LogEvent`
- `PortRoute`
- `RuntimeIdentity`

The default runtime mode is one product sandbox to one Firkin VM-backed
container. Multi-container-per-VM remains substrate/Pod work and must not become
the default Cube mapping through this extraction.

`TemplateMetadata` should preserve Cube-proven fields:

- environment variables captured by template build steps
- optional start command
- optional ready command

But it should not preserve Cube route/build-status record fields.

### `orchestration.rs`

Owns workflows that are lower than product "sandbox service" semantics:

- capacity admission and release
- disk preflight before disk-consuming work
- restore from prepared template snapshot
- restore from continuation snapshot
- save continuation snapshot
- build template snapshot
- warm-pool checkout and replenishment
- active-priority warm-session eviction
- runtime readiness probe
- stop/delete lifecycle
- restart marker publish/refresh/remove
- state reconciliation inputs and decisions

This is the main merge point for "best of both worlds":

- keep Firkin's existing `CapacityLedger`, `RuntimeDiskPressureGuard`,
  `HostDiskPressureProbe`, warm-pool, snapshot integrity, benchmark, and
  marker/reconciliation machinery;
- import Cube's proven product-backend behavior only where it fills a gap.

Orchestration must be directly testable without CubeAPI routes.

### `backend.rs`

Owns the high-level Rust facade:

- `SingleNodeBackend`
- creation from local Apple/VZ runtime configuration
- state-backed constructors
- thin lifecycle methods
- domain proxy construction
- template snapshot build entrypoint

`SingleNodeBackend` should coordinate state and call orchestration. It should
not become a 5,000-line service that mixes API mapping, runtime launch,
template execution, logs, proxy routing, and tests in one file.

### `state.rs`

Owns durable single-node runtime state:

- active session records
- snapshot records
- continuation snapshot records
- template snapshot metadata records
- log event persistence
- atomic JSON writes for the initial backend
- state reconciliation after restart
- orphan managed snapshot artifact cleanup

The initial persistence format may stay simple JSON because this is a
single-node backend, but the record types must be Firkin runtime records, not
Cube route DTOs.

### `proxy.rs`

Owns runtime port routing:

- port registry
- sandbox/port to runtime target mapping
- domain proxy adapter
- envd/code-interpreter/MCP routing registrations

It may use `firkin-e2b::PortTarget`, `LocalRuntimeBackend`, and
`DomainProxyHttpServer`. It must not import Cube route or model types.

### `template.rs`

Owns runtime template snapshot execution:

- apply SDK/classic template build steps
- support `ENV`, `WORKDIR`, `RUN`, `USER`, and `COPY`
- create a temporary build sandbox/session
- run commands inside the session
- save template snapshot
- attach `TemplateMetadata`
- replace existing template snapshot when requested
- always clean up the build sandbox/session

Cube keeps the route-facing template build status service. Firkin owns the
runtime execution that turns build steps into a durable snapshot.

## What moves from CubeAPI

The current CubeAPI Firkin implementation should be extracted in two passes:
first to a Cube-local temporary module, then into Firkin.

Move or adapt these groups from `CubeAPI/src/services/sandboxes.rs`:

| Cube-side item | Firkin destination | Treatment |
| --- | --- | --- |
| `LocalNodeSchedulerConfig` | `single_node::config` | Rename to `SingleNodeSchedulerConfig` or fold into `SingleNodeConfig`. |
| `SandboxResources` | `single_node::model` | Keep, but consider lowering to/from `ResourceBudget`. |
| `FirkinAppleVzCreateRequest` | `single_node::model` | Rename to `SingleNodeCreateRequest`. Remove Cube-only network DTO fields. |
| `FirkinAppleVzSandboxRuntimeMode` | `single_node::model` | Rename to `SingleNodeRuntimeMode`; keep explicit one-VM-backed-container default. |
| `RuntimeCreatedSandbox` | `single_node::model` | Rename to `SandboxSession` or `CreatedSession`. |
| `RuntimeSnapshotRef` | `single_node::model` | Merge with existing Firkin snapshot manifest/integrity concepts. |
| `SandboxCommandRequest` | `single_node::model` | Rename to `CommandRequest`. |
| `SandboxCommandOutput` | `single_node::model` | Rename to `CommandOutput`. |
| `FirkinAppleVzTemplateMetadata` | `single_node::model` | Rename to `TemplateMetadata`. |
| `FirkinAppleVzPortRegistry` | `single_node::proxy` | Keep behavior; use Firkin protocol types. |
| `FirkinAppleVzStateStore` | `single_node::state` | Keep persistence behavior; replace `AppError` and Cube models. |
| `FirkinAppleVzLogStore` | `single_node::state` | Keep bounded log behavior; replace Cube log entries with `LogEvent`. |
| `FirkinAppleVzProxyAdapter` | `single_node::proxy` | Keep as domain proxy adapter; rename. |
| `FirkinAppleVzRuntimeDriver` | `single_node::orchestration` or `driver` if needed | Prefer existing Firkin launcher/session traits where possible. |
| `FirkinAppleVzLocalRuntimeDriver` | `single_node::orchestration` | Merge into existing `CoreSnapshotSessionLauncher` and `FirkinRuntimeAdapter` paths rather than duplicate. |
| `FirkinAppleVzEnvdAdapter` and retained process records | existing runtime envd code / `single_node::proxy` | Merge with existing `FirkinRuntimeAdapter` envd process/filesystem implementation. |
| `LocalNodeScheduler` | existing Firkin capacity machinery | Do not copy as final architecture unless a gap remains after `CapacityLedger` integration. |
| `FirkinAppleVzSandboxBackend` | `single_node::backend` | Rename to `SingleNodeBackend`; make it a thin facade over orchestration. |
| `ActiveFirkinSandbox` | `single_node::state` | Rename to `ActiveSessionRecord`; keep restart-safe fields. |
| `FirkinAppleVzSnapshotRecord` | `single_node::state` | Rename to `SnapshotRecord`; merge with manifest/integrity sidecars. |

Move or adapt these groups from `CubeAPI/src/services/templates.rs`:

| Cube-side item | Firkin destination | Treatment |
| --- | --- | --- |
| template build step execution | `single_node::template` | Move runtime execution only. |
| `run_sdk_template_build` | `single_node::template` | Rename to `build_template_snapshot`; return runtime report. |
| `run_sdk_template_build_steps` | `single_node::template` | Keep semantics; replace build-record mutation with event/report output. |
| `run_sdk_template_copy_command` | `single_node::template` | Keep COPY behavior; ensure paths are runtime-neutral. |
| `run_sdk_template_shell_command` | `single_node::template` | Keep command execution behavior; use `CommandRequest`. |
| build log append helpers | CubeAPI and Firkin split | Firkin emits runtime events; Cube maps to route logs/status. |

## What stays in CubeAPI

These should not move into Firkin:

- `SandboxBackend`.
- `SandboxService`.
- `CubeLinuxSandboxBackend`.
- CubeMaster/CubeLinux integration.
- route handlers.
- `AppError` and HTTP status mapping.
- API request/response DTOs such as `NewSandbox`, `Sandbox`, `SandboxDetail`,
  `SnapshotInfo`, `ListSnapshotsQuery`, `SandboxLogs`, and metrics responses.
- API config names such as `CUBE_API_BACKEND`.
- API auth, rate limiting, OpenAPI, and HTTP server lifecycle.
- template build route records, route status strings, and upload endpoints.
- volume routes and future SaaS product semantics.

Cube should end with a thin adapter roughly shaped like:

```rust
pub struct FirkinSingleNodeCubeAdapter {
    backend: firkin_runtime::single_node::SingleNodeBackend,
}

#[async_trait]
impl SandboxBackend for FirkinSingleNodeCubeAdapter {
    async fn create_sandbox(&self, body: NewSandbox) -> AppResult<Sandbox> {
        let request = map_cube_create(body)?;
        let session = self.backend.create(request).await.map_err(map_firkin_error)?;
        Ok(map_session(session))
    }
}
```

That adapter is the only place where Cube product DTOs meet Firkin runtime DTOs.

## What merges with existing Firkin runtime

Prefer existing Firkin runtime primitives over Cube duplicates.

Keep and extend these Firkin-side implementations:

- `FirkinRuntimeAdapter`
- `RuntimeCubeSandboxCreate`
- `RuntimeCubeSandboxFollowup`
- `RuntimeTemplateBuildSnapshot`
- `RuntimeContinuationSnapshotCapture`
- `RuntimeContinuationSnapshotRestore`
- `RuntimeSnapshotWarmPool`
- `RuntimeWarmPoolService`
- `RuntimePreflight`
- `RuntimeDiskPressureGuard`
- `HostDiskPressureProbe`
- `CapacityLedger`
- `ActiveCapacityAdmissionPlan`
- `ActiveBackpressurePlan`
- `RuntimeHostScanner`
- `RuntimeRestartRecovery`
- `RuntimeFilesystemReconciler`
- `RuntimeHostProcessStuckVmCleaner`
- benchmark and soak evidence writers
- snapshot manifest and integrity sidecars

Cube code should fill these gaps:

- durable single-node active/snapshot/log state facade appropriate for CubeAPI
  style lifecycle calls;
- template metadata carrying env/start/ready commands;
- direct product-friendly `create/delete/snapshot/run_command` Rust facade;
- domain proxy adapter and port registry behavior tied to sandbox IDs;
- Cube-proven template build step semantics, especially `COPY` archive handling;
- restart reconciliation of Cube-style active records against runtime existence;
- timeout refresh/connect behavior where it is not already represented in
  Firkin runtime.

Do not keep duplicate versions of:

- disk probing;
- capacity scheduling;
- warm-pool accounting;
- snapshot integrity verification;
- envd process/filesystem protocol adapters;
- benchmark sample schemas.

If Cube has a duplicate and Firkin has a substrate/runtime version, the Firkin
version wins unless there is a concrete behavior gap.

## Orchestration versus backend

Separate runtime orchestration from high-level sandbox facade behavior.

`single_node::orchestration` owns "how runtime work happens":

- reserve capacity;
- check disk pressure;
- acquire or restore a runtime session;
- start readiness/start commands;
- save snapshots;
- update warm pool;
- release resources;
- write runtime evidence.

`single_node::backend` owns "how callers ask for work":

- create a session;
- get/delete a session;
- run a command;
- create/list/delete snapshots;
- build template snapshots;
- construct a domain proxy.

This avoids reproducing Cube's current large service-file shape in Firkin. It
also lets tests target orchestration directly without booting an HTTP server or
building Cube route DTOs.

## Naming rules

Use product-neutral Firkin runtime names inside Firkin:

| Current Cube name | Firkin name |
| --- | --- |
| `FirkinAppleVzSandboxBackend` | `SingleNodeBackend` |
| `FirkinAppleVzRuntimeDriver` | `RuntimeDriver` only if a driver trait remains needed |
| `FirkinAppleVzLocalRuntimeDriver` | `AppleVzRuntimeDriver` only if not absorbed by existing launchers |
| `FirkinAppleVzCreateRequest` | `SingleNodeCreateRequest` |
| `FirkinAppleVzSandboxRuntimeMode` | `SingleNodeRuntimeMode` |
| `FirkinAppleVzTemplateMetadata` | `TemplateMetadata` |
| `FirkinAppleVzStateStore` | `StateStore` |
| `FirkinAppleVzLogStore` | `LogStore` |
| `FirkinAppleVzPortRegistry` | `PortRegistry` |
| `FirkinAppleVzProxyAdapter` | `DomainProxyAdapter` |
| `ActiveFirkinSandbox` | `ActiveSessionRecord` |
| `FirkinAppleVzSnapshotRecord` | `SnapshotRecord` |

Keep `Cube`, `E2B`, and route vocabulary out of public Firkin runtime names
unless the type directly implements an E2B protocol contract from `firkin-e2b`.

## `rs-hack` migration workflow

Use `rs-hack` for the whole mechanical refactor, not just for a few searches.
The tool is AST-aware and stateful: it supports dry-run diffs, applied run
history, and `revert` by run id. Every write command should use
`--local-state` so the operation log lands in the repo-local `.hack/rs`
directory rather than global user state.

`rs-hack` is strongest at AST transformations:

- discovery across definitions and expression sites;
- function and trait-method renames;
- enum variant renames across definitions, patterns, and constructors;
- struct field add/update/remove across definitions and literals;
- enum-variant struct literal field updates through `Enum::Variant` targets;
- use-statement additions;
- derive additions;
- impl-method additions;
- call-argument add/update/remove across function or method calls;
- match-arm add/update/remove and missing-variant audits;
- generic AST-node transform, comment, remove, or replace;
- batch JSON/YAML operation files.

`rs-hack` does not replace the compiler and does not decide architecture. It
also does not physically move an item into a different file as a semantic Rust
module extraction operation. File creation, module declaration, and copy/import
cleanup still happen through normal editor work. Once code is in the right
file, `rs-hack` should own the repetitive Rust AST edits.

### Refactor loop

Each extraction step should use this loop:

1. Discover with `summary`, `find`, `impls`, `match-audit`, `neighbors`, and
   `doc-coverage`.
2. Generate a small command or batch spec for one mechanical change family.
3. Dry-run with `--format diff` or `--format summary`.
4. Apply with `--apply --local-state`.
5. Record the run id from `rs-hack history --local-state`.
6. Run focused `cargo check` or tests.
7. Revert the `rs-hack` run if the compiler exposes a bad assumption.
8. Commit the clean mechanical step.

Use `--limit N` when validating a new operation shape. Apply to one or two
matches first, compile if useful, then re-run without the limit.

### Phase 0: inventory the moving surface

Run these before moving anything:

```bash
RSH=/Users/darin/vendor/github.com/1e1f/rs-hack/target/debug/rs-hack
CUBE=/Users/darin/vendor/github.com/TencentCloud/CubeSandbox/CubeAPI/src

$RSH summary --path "$CUBE/services/sandboxes.rs"
$RSH summary --path "$CUBE/services/templates.rs"

$RSH impls --paths "$CUBE/**/*.rs" --trait SandboxBackend
$RSH impls --paths "$CUBE/**/*.rs" --trait FirkinAppleVzRuntimeDriver

$RSH find --paths "$CUBE/**/*.rs" --name FirkinAppleVz --format locations
$RSH find --paths "$CUBE/**/*.rs" --field-name sandbox_id --format locations
$RSH find --paths "$CUBE/**/*.rs" --node-type trait-impl \
  --name SandboxBackend --format snippets
```

Then use `find --format json` for machine-readable symbol maps that can feed
batch-spec generation:

```bash
$RSH find --paths "$CUBE/**/*.rs" --name FirkinAppleVz \
  --format json > /tmp/firkin-cube-symbols.json
```

### Phase 1: create a Cube-local extraction module

Use normal file edits to create the temporary module tree:

```text
CubeAPI/src/services/firkin_single_node/
  mod.rs
  config.rs
  error.rs
  model.rs
  state.rs
  proxy.rs
  orchestration.rs
  backend.rs
  template.rs
```

Move blocks from `sandboxes.rs` and `templates.rs` into those files manually or
with editor support. After each block move, use `rs-hack` to repair the AST
surface instead of regex editing.

Examples:

```bash
# Add module declarations without touching unrelated code.
$RSH add --local-state \
  --paths "$CUBE/services/firkin_single_node/mod.rs" \
  --use "crate::services::firkin_single_node::model::SingleNodeCreateRequest" \
  --format diff

# Add missing derives after a move.
$RSH add --local-state \
  --paths "$CUBE/services/firkin_single_node/**/*.rs" \
  --name TemplateMetadata \
  --derive "Clone,Debug,Default,Eq,PartialEq,serde::Deserialize,serde::Serialize" \
  --format diff
```

### Phase 2: product-neutral renames in Cube

Perform renames while the code still compiles inside Cube. This gives
`rs-hack` the largest intact reference graph before the Firkin copy.

Use individual commands for high-risk names so each run is revertible. The
`rename` command is first-class for function and enum-variant renames:

```bash
$RSH rename --local-state --paths "$CUBE/**/*.rs" \
  --name firkin_apple_vz_runtime_backend_name \
  --to single_node_runtime_backend_name \
  --kind function --format diff
```

`rs-hack` does not currently have a first-class "rename struct/type item and all
constructor paths" operation. For type names, move/rename the declaration with
the editor, then use targeted `find` plus `transform` for AST sites that can be
replaced safely. Start with type references:

```bash
$RSH find --paths "$CUBE/**/*.rs" --node-type type-ref \
  --name FirkinAppleVzSandboxBackend --format locations

$RSH transform --local-state --paths "$CUBE/**/*.rs" \
  --node-type type-ref \
  --name FirkinAppleVzSandboxBackend \
  --action replace \
  --with SingleNodeBackend \
  --format diff
```

For constructor paths and struct literal paths, first inventory the exact shape.
Do not replace a whole `struct-literal` node with a bare type name; that would
delete its fields. Use this discovery to decide whether a normal editor edit,
an identifier transform, or a small custom batch is the right move:

```bash
$RSH find --paths "$CUBE/**/*.rs" --node-type struct-literal \
  --name FirkinAppleVzCreateRequest --format locations

$RSH find --paths "$CUBE/**/*.rs" --node-type identifier \
  --name FirkinAppleVzCreateRequest --format locations
```

Prefer a YAML batch once the safe operation shape is proven:

```yaml
base_path: /Users/darin/vendor/github.com/TencentCloud/CubeSandbox/CubeAPI/src
operations:
  - type: Transform
    node_type: type-ref
    name_filter: FirkinAppleVzTemplateMetadata
    action:
      Replace:
        with: TemplateMetadata
  - type: Transform
    node_type: type-ref
    name_filter: FirkinAppleVzSnapshotRecord
    action:
      Replace:
        with: SnapshotRecord
  - type: RenameFunction
    old_name: firkin_apple_vz_runtime_backend_name
    new_name: single_node_runtime_backend_name
```

Apply batch specs only after a dry run:

```bash
$RSH batch --local-state --spec /tmp/firkin-single-node-renames.yaml --format diff
$RSH batch --local-state --spec /tmp/firkin-single-node-renames.yaml --apply
```

### Phase 3: structural API cleanup

Use field and literal operations to change DTO shapes before copying into
Firkin.

Examples:

```bash
# Add a new runtime-neutral field to definitions and all literals.
$RSH add --local-state --paths "$CUBE/services/firkin_single_node/**/*.rs" \
  --name SingleNodeCreateRequest \
  --field-name runtime_identity \
  --field-type "Option<RuntimeIdentity>" \
  --field-value "None" \
  --kind struct \
  --format diff

# Remove Cube-only fields from request literals but leave temporary definition
# fields until the adapter mapping compiles.
$RSH remove --local-state --paths "$CUBE/services/firkin_single_node/**/*.rs" \
  --name SingleNodeCreateRequest \
  --field-name network \
  --literal-only \
  --format diff

# Add Default rest to large literals while the neutral model settles.
$RSH add --local-state --paths "$CUBE/services/firkin_single_node/**/*.rs" \
  --name SingleNodeCreateRequest \
  --default-rest \
  --format diff
```

Use call-argument operations for constructor and method signature churn:

```bash
$RSH add --local-state --paths "$CUBE/services/firkin_single_node/**/*.rs" \
  --call with_driver_scheduler_ports_logs_and_state \
  --arg "SingleNodeConfig::default()" \
  --arg-position first \
  --call-type function \
  --format diff

$RSH update --local-state --paths "$CUBE/services/firkin_single_node/**/*.rs" \
  --call run_command \
  --arg-index 1 \
  --arg "CommandRequest::from(request)" \
  --call-type method \
  --content-filter "SandboxCommandRequest" \
  --format diff
```

### Phase 4: exhaustiveness and trait checks

Use `rs-hack` discovery commands as gates after model changes:

```bash
$RSH match-audit --paths "$CUBE/services/firkin_single_node/**/*.rs" \
  --enum SingleNodeRuntimeMode

$RSH impls --paths "$CUBE/services/firkin_single_node/**/*.rs" \
  --trait RuntimeDriver

$RSH doc-coverage --paths "$CUBE/services/firkin_single_node/**/*.rs" \
  --fields
```

When a new enum variant is deliberately added, use `add --auto-detect` to add
placeholder match arms before replacing bodies:

```bash
$RSH add --local-state --paths "$CUBE/services/firkin_single_node/**/*.rs" \
  --auto-detect \
  --enum-name SingleNodeRuntimeMode \
  --body "return Err(Error::UnsupportedRuntimeMode)" \
  --format diff
```

### Phase 5: copy to Firkin and purify imports

After the Cube-local module compiles, copy the module tree to:

```text
crates/runtime/src/single_node/
```

Then run the same `rs-hack` patterns in Firkin:

```bash
FIRKIN=/Users/darin/vendor/github.com/apple/containerization/crates/runtime/src

$RSH find --paths "$FIRKIN/single_node/**/*.rs" \
  --name AppError --format locations

$RSH transform --local-state --paths "$FIRKIN/single_node/**/*.rs" \
  --node-type type-ref \
  --name AppError \
  --action replace \
  --with Error \
  --format diff

$RSH add --local-state --paths "$FIRKIN/single_node/**/*.rs" \
  --use "crate::RuntimeDiskPressureGuard" \
  --format diff
```

Use `find --field-name`, `transform --node-type type-ref`, and call-argument
updates to replace Cube imports with Firkin-native equivalents. The compiler
then drives the non-mechanical cleanup.

### Phase 6: Cube adapter cutover

Once Firkin exposes `SingleNodeBackend`, use `rs-hack` to collapse Cube call
sites onto the adapter:

```bash
$RSH transform --local-state --paths "$CUBE/**/*.rs" \
  --node-type type-ref \
  --name SingleNodeBackend \
  --action replace \
  --with "firkin::runtime::single_node::SingleNodeBackend" \
  --format diff
```

Use `find --name`, `find --field-name`, and `impls` to prove no direct
Cube-owned Apple/VZ implementation remains outside the adapter.

### Revert discipline

Every applied operation should be revertible:

```bash
$RSH history --local-state --limit 10
$RSH revert --local-state <run-id>
```

If a command edits too broadly, revert the run immediately and re-run with
`--node-type`, `--content-filter`, `--exclude`, or `--limit`.

## Goal completion standard

This document is the completion standard for the active refactor goal.

The goal is not done, and should not be described as done, until every
acceptance item below is met. Partial success can be reported only as partial
success. A compiling intermediate, a Cube-local module split, a copied Firkin
module tree, or a passing unit-test subset is not enough.

The completion claim requires:

- a hard cutover from Cube-owned Apple/VZ backend mechanics to
  `firkin-runtime::single_node`;
- no compatibility shim that keeps the old Cube-local Firkin runtime
  implementation alive;
- every acceptance criterion in this document satisfied;
- `scripts/check-firkin-crate-graph.sh` passing after the final crate graph
  settles;
- focused Firkin and Cube checks for the moved surface;
- signed/live VZ smoke coverage for the non-deferred single-node path, except
  for explicitly deferred v2 scope listed under Non-goals;
- a clean `jj st` in both repos after the final commits.

The 24-hour soak remains part of the broader production substrate readiness
tracked in `08-production-substrate-current-audit.md`. It is not a blocker for
landing this extraction unless the implementation changes the soak path. If it
does, the smoke and validation commands for that path must be refreshed before
this goal is called done.

## Acceptance criteria

The extraction is complete when:

1. `firkin-runtime::single_node` exposes a stable Rust API for create, delete,
   snapshot, command execution, template snapshot build, and domain proxy
   construction.
2. CubeAPI's Firkin backend implementation is a thin adapter over
   `firkin-runtime::single_node`.
3. CubeAPI no longer owns Apple/VZ runtime mechanics, durable Firkin state,
   envd runtime adapters, warm-pool behavior, or template snapshot execution.
4. `firkin-e2b` still has no dependency on `firkin-core`, `firkin-runtime`,
   `firkin-template`, `firkin-substrate`, `firkin-vmm`, or other Apple/VZ
   runtime crates.
5. `scripts/check-firkin-crate-graph.sh` passes.
6. Focused `firkin-runtime` tests cover orchestration without CubeAPI.
7. CubeAPI tests cover only DTO mapping and adapter behavior for the Firkin
   backend.
8. Existing signed live VZ smokes still pass for create, command, filesystem,
   snapshot, follow-up, warm pool, and domain proxy flows.
9. Network policy remains honest: unrestricted networking may work, restrictive
   policy hard-fails until real enforcement exists.
10. The default mapping remains one Cube sandbox to one Firkin VM-backed
    container.

## Non-goals

This extraction does not:

- implement product pods;
- change the default one-sandbox-to-one-VM-backed-container mapping;
- implement full Jupyter kernel parity;
- implement guest MCP service semantics beyond the current deferred scope;
- add a Firkin SaaS or cluster scheduler;
- turn Firkin into CubeAPI;
- add a standalone Firkin HTTP API server;
- add backward-compatibility shims for old Cube-local Firkin implementation
  paths after the hard cutover;
- fake restrictive network policy enforcement.

## Risks

The main risk is copying too much Cube product shape into Firkin. The mitigation
is to force every moved public type through this question:

> Is this a runtime decision product, or is it an API representation?

Runtime decision products move to Firkin. API representations stay in Cube.

The second risk is duplicating runtime primitives that Firkin already has. The
mitigation is to treat Cube code as behavior evidence and test input, not as the
canonical implementation when Firkin already owns an equivalent primitive.

The third risk is making `single_node` too generic too early. The target is a
single powerful Mac running Apple/VZ. Keep cluster placement, multi-node
scheduling, product tenancy, and SaaS policy out of the module.

## Implementation notes

Keep commits small:

1. Cube temporary module split.
2. Cube product-neutral renames.
3. Firkin `single_node` skeleton.
4. Firkin model/error/state/proxy import.
5. Firkin orchestration merge with existing runtime primitives.
6. Firkin backend facade.
7. Cube adapter cutover.
8. Delete old Cube-local implementation.
9. Tests and signed smoke refresh.

Use a hard cutover. Once Cube calls Firkin's `SingleNodeBackend`, remove the old
Cube-local Firkin runtime implementation rather than maintaining both paths.
