# firkin-runtime

`firkin-runtime` is the production Apple/VZ composition crate.

It is allowed to know about:

- `firkin-core` VM and container mechanics.
- `firkin-template` checkout, setup, cache-warming, and freshness execution.
- `firkin-substrate` capacity, snapshot, warm-pool, benchmark, and hygiene policy.
- `firkin-types` shared value types.
- `firkin-e2b` Cube/E2B-compatible API contracts.

Lower-level crates must not depend back on this crate. VZ-backed template
snapshot sinks, snapshot-restore session creation, live capacity admission, and
`CubeAPI` runtime wiring belong here.

Runtime storage must not default to `TMPDIR`. `FirkinStorageConfig` is the
library surface for storage roots: durable state defaults to `~/.firkin/state`,
rebuildable caches default to `~/.firkin/cache`, and callers can pass explicit
roots or use `FIRKIN_STATE_DIR` / `FIRKIN_CACHE_DIR`. Continuation snapshots,
restore staging, single-node snapshots, and E2B local state derive from this
layout unless a more specific artifact root is configured.

The first restore seam is `RuntimeSnapshotRestore`: it admits active capacity,
calls a `SnapshotSessionLauncher` with the durable snapshot manifest, records a
`warm_snapshot_restore` latency sample, and releases capacity if restore fails.
`RuntimeCubeSandboxCreate` wraps that path for `CubeAPI`/E2B create: it restores
the prepared template snapshot as the primary create path and returns the
SDK-visible `RuntimeSandbox` shape while keeping the Firkin session and active
capacity reservation in the runtime report.
`FirkinRuntimeAdapter` implements `firkin_e2b::RuntimeAdapter` over that same
path, so `LocalRuntimeBackend` can create Cube/E2B sandboxes by restoring
prepared Firkin snapshot artifacts directly. It also hard-rejects direct
template preparation and restrictive network policy until those production
paths have real Firkin-owned enforcement. Port routing is session-owned:
`RuntimePortRouter` lets the restored Firkin session open the live byte stream,
and `firkin-core` containers implement it through VM vsock dialing.
Readiness is session-owned as well: `RuntimeReadinessProbe` checks that the
restored service surface is reachable before the adapter exposes the sandbox,
records `ready_probe` latency, and tears the session down if readiness fails.
Command execution is session-owned too: `RuntimeCommandRunner` lets the adapter
serve the local envd process start contract from the restored session and retain
`command_start` plus `first_stdout_byte` samples.
The first non-interactive command after a prepared-template restore also records
`sandbox.start.resume_snapshot_to_first_stdout_ms`, which composes restore start
through the first stdout byte while preserving the lower-level restore and
command spans.
Session stop is session-owned too: `RuntimeSessionStop` lets the adapter stop
the live runtime session before releasing active capacity, and `firkin-core`
containers implement it by signaling the init process.
The adapter retains lifecycle benchmark samples from create/follow-up restore
readiness, command start, first output, and stop, giving the Cube-facing path a
concrete source for measured `warm_snapshot_restore`, `ready_probe`,
`command_start`, `first_stdout_byte`, and `kill_delete` evidence.
`RuntimeSessionTerminate` asks a `SessionTerminator` to stop/delete an active
session, releases active capacity only after termination succeeds, and records
`kill_delete` latency.
`RuntimeDiskPressureGuard` is the host free-space floor admission seam; callers
provide a `DiskPressureProbe` for the operator-selected runtime root, and work is
blocked before snapshot or VM pressure can drive the host below that floor.
`HostDiskPressureProbe` is the default host implementation; it reads available
space with `df -Pk` to preserve this crate's no-unsafe boundary.
`RestartReconciliation` applies substrate restart decisions through a runtime
executor adapter, giving the controller one ordered path for recover, cleanup,
and quarantine work after restart.
`RuntimeStuckVmCleanup` applies heartbeat-timeout cleanup decisions through a
runtime cleaner adapter, keeping stuck-VM policy separate from live VZ
termination mechanics.

With the `snapshot` feature enabled, `CoreContainerSnapshotSink` adapts a live
`firkin-core` container into `firkin-template`'s async `TemplateSnapshotSink`.
`RuntimeContinuationSnapshotCapture` uses that same sink contract to save a
stopped, idle, or exited session as a continuation snapshot and records
`snapshot_save` latency for the capture path.
`RuntimeTemplateBuildSnapshot` is the runtime-owned template build seam: a live
adapter runs repo clone/setup/cache-warm commands inside the target runtime, a
snapshot sink persists the prepared VM/container state, and the report records
`cold_template_build` plus `snapshot_save` latency.
`CoreTemplateCommandRunner` runs that command path in a live `firkin-core`
container through exec calls, while `CubeAPI` create wiring remains a separate
adapter.
`RuntimeCubeSandboxFollowup` wraps continuation restore for follow-up prompts:
it restores the continuation snapshot, preserves the stopped/idle/exited
reason, and returns the SDK-visible `RuntimeSandbox` shape with the Firkin
session reservation in the report. `FirkinRuntimeAdapter::start_followup`
exposes that path to Cube-facing runtime wiring without pretending it is the
same operation as E2B pause/resume.

`RuntimeWarmPoolMaintain` is the first warm-pool runtime seam: it asks a
`WarmPoolSessionLauncher` to restore a snapshot for a repo/template/runtime key
and records the resulting `WarmPoolEntry` in the substrate ledger.
`RuntimeWarmPoolCheckout` promotes that warm entry to active use, returns an
active reservation, and records `warm_pool_checkout` latency.
`RuntimeWarmPoolService` owns the retained snapshot pool, replenishment
supervisor, and snapshot launcher together so a runtime process has one
testable object for refill ticks and warm checkouts. It can also spawn an
owned refill task with a shutdown handle for process-level service wiring.
