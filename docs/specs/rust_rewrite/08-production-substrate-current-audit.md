# Production Substrate Current Audit

Last updated: 2026-05-05.

This audit maps the production Firkin/CubeAPI goal to current repo evidence. It
is not a completion certificate. The goal remains incomplete until the 24-hour
soak artifact validates and the explicit v2/deferred scope decisions are
accepted for the current single-node production target.

## Success criteria

- Template builds clone a repo, run setup/cache-warm commands, and save a
  durable base snapshot.
- Snapshot restore is the default create path and records latency.
- Warm pools maintain pre-restored entries, check them out quickly, and avoid
  starving active sessions.
- Freshness sync permits reads while branch sync runs and blocks writes until
  safe.
- Continuation snapshots support follow-up prompts after stop, idle, or exit.
- Lifecycle latency evidence covers cold build, restore, command start, first
  stdout byte, ready probe, snapshot save, kill/delete, warm-pool checkout, and
  concurrent create.
- Overhead evidence separates Firkin tax from VM/container workload resources.
- Production hygiene covers preflight, restart reconciliation, artifact GC,
  integrity, log rotation, stuck-VM cleanup, disk pressure, and 24-hour soak.
- Default semantics are one Cube sandbox to one Firkin VM-backed container. One
  VM with multiple Firkin containers is core-smoke-proven, and the advanced
  `firkin-core::Pod` substrate now uses a preboot pod-store disk plus
  `VmRootfs::GuestPath`. Product Cube pods are still not implemented.
- Restrictive network policy hard-fails until real enforcement exists.

## Evidence checklist

| Requirement | Current evidence | Status | Missing before production |
| --- | --- | --- | --- |
| Template build snapshot | `TemplateBuildJob`, `TemplateBuildExecutor`, `RuntimeTemplateBuildSnapshot`, `CoreTemplateCommandRunner`, `TemplateSnapshotSink`, `CoreContainerSnapshotSink`, manifest/integrity sidecar writes after snapshot save, shared `RuntimeAdapter::build_template` and `FirkinRuntimeAdapter` hard-fail adapter-level template build calls instead of returning fake empty artifacts, signed live VZ proof that a host-served Git repo is cloned, checked out, setup/cache-warmed, snapshotted, restored, and verified, and CubeAPI unit plus signed live proof that Firkin classic create/rebuild/status/log/list/detail/delete routes launch and expose snapshot-template builds; acceptance `template_build_snapshot=signed_live_vz_template_snapshot_proven`. | Complete | None known for current single-node scope. |
| Snapshot restore create | `RuntimeSnapshotRestore`, `RuntimeCubeSandboxCreate`, async `SnapshotSessionLauncher`, reusable `CoreSnapshotSessionLauncher`, private restored rootfs staging, `LocalRuntimeBackend<FirkinRuntimeAdapter<_>>` create-path test coverage, signed live core snapshot restore smoke, signed live envd HTTP process smoke over a restored `FirkinRuntimeAdapter`, signed vendored-SDK create/run/kill proof through the domain proxy, and signed live concurrent two-sandbox restore proof through retained stdin; acceptance `snapshot_restore_default_create=core_live_restore_proven`. | Partial | Broader SDK route matrix beyond process create/run/kill. |
| Warm pool lifecycle | `WarmPoolLedger`, `WarmPoolReplenishmentPlan`, `ActiveCapacityAdmissionPlan`, `RuntimeWarmPoolMaintain`, `RuntimeWarmPoolCheckout`, same-template warm depth, `RuntimeSnapshotWarmPool::replenish_with_elapsed`, `RuntimeWarmPoolSupervisor`, spawnable `RuntimeWarmPoolService`, `FirkinRuntimeAdapter::prewarm_template`, `FirkinRuntimeAdapter::maintain_warm_templates`, `FirkinWarmTemplateMaintainer`, backend-derived latest ready template targets, adapter `start()` warm checkout, adapter active-priority cold create/follow-up eviction of retained warm sessions, a clean prewarm policy test proving prewarm retains a restored session without readiness-probe or command side effects until checkout, signed live VZ warm-pool checkout proof that a retained restored session runs a command after checkout, and signed live SDK/domain-proxy proof that product create consumes a prewarmed template and runs a command; acceptance `warm_pool_lifecycle=signed_live_product_route_proven`. | Partial | Richer production eviction tuning and 24-hour soak evidence. |
| Freshness sync | `FreshnessSyncGate`, host-side `FreshnessSyncExecutor`, and `FirkinRuntimeAdapter` metadata-driven freshness gates install after create, allow filesystem reads while syncing, block filesystem writes, spawn runtime-session `git fetch`, `git checkout`, and `git reset` inside the restored checkout when `firkin.sync.checkout` is present, and unlock writes after guest sync succeeds; acceptance `freshness_sync_readonly_then_writable=signed_live_product_route_proven`. Core snapshot restore now re-applies Firkin-managed guest network configuration after `boot_or_restore` for restore configs with explicit network interfaces, and vminitd default-route setup uses replace/create netlink flags so restored guests with an existing default route are reconfigured idempotently. Signed live VZ/vmnet freshness proofs restore a snapshot built from a public Git repo, fast-forward the restored checkout to the current branch head, prove writes unblock after guest sync, and pass through the Cube/E2B `POST /sandboxes` product route. | Partial | Representative latency and soak evidence. |
| Continuation snapshots | `ContinuationSnapshotPlan`, `RuntimeContinuationSnapshotCapture`, `RuntimeContinuationSnapshotRestore`, `RuntimeCubeSandboxFollowup`, `FirkinRuntimeAdapter::snapshot`, `FirkinRuntimeAdapter::start_followup`, `FirkinRuntimeAdapter::delete_snapshot`, E2B/Cube `POST /sandboxes/{id}/snapshots` route wiring that captures the active runtime session continuation artifact, E2B/Cube `POST /sandboxes/followups` product route wiring, manifest/integrity sidecar writes after continuation snapshot save, continuation artifact plus sidecar deletion, signed live VZ proof that a marker-bearing session is captured, restored, and verified from the continuation snapshot, signed live VZ proof that the create-snapshot product route captures restorable guest state, and signed live VZ proof that the follow-up product route restores the continuation snapshot and runs a command in the follow-up sandbox; acceptance `continuation_snapshot_resume=signed_live_product_route_proven`. | Partial | Soak coverage. |
| E2B port routing | `RuntimePortRouter`, `FirkinRuntimeAdapter::port_target`, `FirkinRuntimeAdapter::connect_port_target`, runtime-owned envd and code-interpreter listeners, and `firkin_core::Container<Streams>` vsock routing implementation; non-live domain-proxy coverage proves envd routing, code-interpreter `/execute` bash protocol routing, Python `context_id` namespace persistence for pickleable values, and MCP CONNECT tunneling on `50005`; signed live VZ coverage proves SDK-visible code-interpreter probe, `/execute` routing on `49999` through the product domain proxy, concurrent two-active-sandbox `/execute` isolation, and Python context state surviving repeated execute requests; acceptance `reserved_port_routing=code_interpreter_python_context_smoke_proven`. | Partial | Full Jupyter kernel parity and guest MCP service semantics are deferred to v2. |
| E2B process operations | `RuntimeCommandRunner` backs finite `FirkinRuntimeAdapter::start_process`, records command latency, scopes retained process metadata and handles by sandbox, preserves other active sandboxes' process records when one sandbox stops, serves finite output through process list/connect, passes gRPC-Web `EnvdProcessHttpServer` process-list and process-start requests, exposes split `firkin_core::Pty` input/output/control handles, and retains interactive stdin, signal, PTY input, PTY resize, PTY output buffering, and connect state. Signed live runtime snapshot smokes pass for restored command execution, retained stdin, retained PTY, envd HTTP process start over a restored `FirkinRuntimeAdapter`, vendored-SDK command run, vendored-SDK concurrent finite commands in two sandboxes, vendored-SDK retained stdin, vendored-SDK retained PTY input/resize/connect/signal, signed live concurrent two-sandbox retained stdin through `LocalRuntimeBackend` plus domain proxy, and non-live two-active-sandbox process plus retained-process connect routing; acceptance `envd_process_records=signed_live_sdk_domain_proxy_process_proven`. | Partial | Broader live concurrent process soak beyond focused finite command and retained stdin proofs. |
| E2B filesystem operations | `FirkinRuntimeAdapter` implements envd read/write/stat/list/mkdir/move/remove/watch through the active restored session command runner, passes `EnvdProcessHttpServer` file read/write requests, proves gRPC-Web text watch streams emit start and filesystem event frames before watch end, passes signed vendored-SDK write/read/stat/list/remove/missing-exists and concurrent two-sandbox filesystem smokes through `LocalRuntimeBackend` plus domain proxy, and passes non-live two-active-sandbox filesystem read/write/stat/list/remove routing; acceptance `envd_filesystem_operations=signed_live_sdk_domain_proxy_filesystem_proven`. | Partial | Broader live SDK watch soak coverage. |
| E2B stop lifecycle | `RuntimeSessionStop`, `FirkinRuntimeAdapter::stop`, and signed vendored-SDK `kill()` proof through the domain proxy stop the restored VZ session, release active capacity, and record `kill_delete`; acceptance `stop_lifecycle=signed_live_sdk_kill_delete_proven`. | Partial | Broader delete-path soak and failure recovery coverage. |
| Lifecycle latency evidence | `BenchmarkSample`, `BenchmarkSummary`, `BenchmarkEvidenceReport`, `BenchmarkEvidenceArtifact`, shared default lifecycle latency targets, `BenchmarkSloTarget`, `BenchmarkSloGateReport`, `RuntimeBenchmarkEvidenceWriter`, `fk substrate validate-lifecycle-slo`, `just live-runtime-benchmark-slo-gate`, `just live-runtime-benchmark-representative`, adapter-retained create/follow-up/readiness/command/stop samples, and signed live VZ representative evidence for cold template build, warm restore, command start, first stdout byte, ready probe, snapshot save, kill/delete, warm-pool checkout, and concurrent create; acceptance `latency_benchmarks=representative_slo_gate_proven`. Latest representative p95s are cold template build 163ms, warm snapshot restore 173ms, command start 36ms, first stdout byte 43ms, ready probe sub-ms, snapshot save 100ms, kill/delete 2ms, warm-pool checkout sub-ms, and concurrent create 2527ms. | Partial | 24-hour soak evidence under sustained product-route load. |
| Overhead evidence | Metric schema, CLI target manifest, `BenchmarkOverheadEvidenceReport`, `BenchmarkSloTarget`, `BenchmarkSloGateReport`, `RuntimeOverheadEvidenceWriter`, `BenchmarkOverheadEvidenceArtifact`, `fk substrate validate-overhead-slo`, `just live-runtime-overhead-slo-gate`, and `just live-runtime-overhead-representative`; acceptance `overhead_benchmarks=representative_slo_gate_proven`. Latest representative p95s are control-plane idle CPU 0%, incremental control-plane RSS 33.99 MiB, per-sandbox host RSS 14.04 MiB, disk metadata growth 4096 bytes, and idle wakeup rate 0 Hz. | Partial | 24-hour soak evidence under sustained product-route load. |
| Runtime preflight | `fk debug preflight` reports VMM capability/signing data, `RuntimePreflight` validates required runtime roots plus the configured host free-space floor before runtime work probes or starts, `fk e2b host` creates the sandbox/log roots, runs that 10 GiB disk preflight before serving the control plane or domain proxy, and spawns the control-plane lifecycle scheduler so expired sandboxes are cleaned up while the host runs; `FirkinRuntimeAdapter` can be configured to run the same preflight before `start()` or `prewarm_template()` restore work; and `FirkinRuntimeAdapter::with_managed_runtime_roots` wires snapshot/log preflight plus active-VM markers as one production composition helper; acceptance `runtime_preflight=product_and_adapter_preflight_wired`. | Partial | Broader daemon/operator launch integration if production uses a wrapper beyond `fk e2b host` or bypasses the managed roots helper. |
| Capacity and disk pressure | `CapacityLedger`, `ActiveCapacityAdmissionPlan`, `ActiveBackpressurePlan`, bounded active restore queueing, `RuntimeDiskPressureGuard`, `HostDiskPressureProbe`, active-priority adapter warm eviction for create and follow-up restore, 10 GiB active-restore and snapshot-save hard floor, and 20 GiB warm-pool refill floor; acceptance `capacity_scheduler_pressure=runtime_active_queue_backpressure_wired`. | Partial | 24-hour soak evidence under sustained pressure and delete/cleanup churn. |
| Restart reconciliation | `HostRuntimeScan`, `RuntimeHostScanner`, `ReconciliationPlan`, `RestartReconciliation`, `RuntimeRestartRecovery`, `RuntimeFilesystemReconciler` filesystem cleanup/quarantine, `FirkinRuntimeAdapter::with_managed_runtime_roots` runtime-owned active marker directory publish/heartbeat refresh/runtime PID and executable publish/remove, `fk substrate host-scan` JSON restart/stuck-VM decision output including runtime PIDs, `fk substrate reconcile-once` one-shot runtime recovery that treats absent marker roots as empty and uses an executable-checked host-process terminator for stuck active-VM cleanup, `fk substrate reconcile-launchd-{plist,install,bootstrap,status}` StartInterval LaunchAgent operator path, and signed live VZ proof that a running sandbox is visible to host scan with runtime PID, restart recover decision, stuck-VM preserve decision, and marker removal after stop; acceptance `restart_reconciliation=signed_live_vz_marker_host_scan_proven`. | Partial | Unmanaged external VZ process enumeration is out of scope; managed Firkin sessions use marker ownership. |
| Snapshot artifact GC | `ArtifactGcPlan`, `RuntimeSnapshotArtifactGc`, and `RuntimeHygieneMaintenance` delete unreferenced direct snapshot files and directories while preserving referenced artifacts, manifest sidecars, and recent unreferenced artifacts under age-based retention through a periodic runtime owner. `SnapshotArtifactManifest` persists JSON sidecars and discovers direct `*.manifest.json` sidecars in sorted order, runtime artifact GC can consume those sidecars, the maintenance owner can reread sidecars each tick when configured with a manifest directory, `fk substrate hygiene-once` runs a schedulable one-shot sidecar-backed GC pass, `fk substrate hygiene-daemon` runs the periodic owner until interrupted, `fk substrate hygiene-launchd-plist` renders a launchd service plist for that daemon, `fk substrate hygiene-launchd-install` writes it atomically to an operator-chosen path, `fk substrate hygiene-launchd-bootstrap` runs `launchctl bootstrap` plus `kickstart`, `fk substrate hygiene-launchd-status` runs `launchctl print`, and signed live VZ hygiene pressure proof preserves a manifest-referenced live snapshot artifact while reclaiming a stale snapshot directory; acceptance `snapshot_artifact_gc=signed_live_hygiene_pressure_proven`. | Partial | Longer service pressure soak. |
| Snapshot integrity | `SnapshotArtifactIntegrity` plus `RuntimeSnapshotRestore::execute_with_integrity_disk_probe_elapsed` verify artifact size and SHA-256 before disk probe, capacity admission, or snapshot launch. `PreparedTemplate` and `SnapshotRef` carry artifact integrity through Cube/E2B product create, warm prewarm, follow-up restore paths, and local state JSON persistence. `SnapshotArtifactManifest` persists and discovers JSON sidecars for durable artifact metadata; `SnapshotArtifactIntegrity` persists JSON sidecars; template build and continuation capture write both sidecars after snapshot save; restore can consume integrity sidecars before externally materialized snapshots launch; prepared template create and follow-up restore can fill missing embedded integrity from the artifact sidecar; `fk substrate snapshot-sidecars` writes import sidecars for existing artifacts; signed live VZ proof rejects a mutated snapshot before restore; acceptance `snapshot_artifact_integrity=signed_live_integrity_reject_proven`. | Complete | None known for current single-node scope. |
| Log rotation | `LogRotationPlan`, `RuntimeLogRotation`, and `RuntimeHygieneMaintenance` rotate oversized active logs while shifting and bounding retained `.N` generations through a periodic runtime owner, with optional gzip compression producing bounded `.N.gz` generations. `fk substrate hygiene-once` exposes that rotation path as a schedulable operator hook, `fk substrate hygiene-daemon` runs the same periodic owner until interrupted, `fk substrate hygiene-launchd-plist` renders a launchd service plist for that daemon, `fk substrate hygiene-launchd-install` writes it atomically to an operator-chosen path, `fk substrate hygiene-launchd-bootstrap` runs `launchctl bootstrap` plus `kickstart`, `fk substrate hygiene-launchd-status` runs `launchctl print`, and signed live VZ hygiene pressure proof rotates an oversized runtime log in the same maintenance tick as snapshot GC; acceptance `log_rotation=signed_live_hygiene_pressure_proven`. | Partial | Longer service log pressure soak. |
| Stuck VM cleanup | `HostRuntimeScan`, `RuntimeHostScanner`, `StuckVmCleanupPlan`, `RuntimeStuckVmCleanup`, `RuntimeFilesystemReconciler` active-VM marker cleanup/quarantine, `RuntimeHostProcessStuckVmCleaner`, `CommandHostProcessTerminator`, `FirkinRuntimeAdapter::with_managed_runtime_roots` runtime-owned active heartbeat/runtime-PID/runtime-executable marker publish/refresh/remove, `fk substrate stuck-vm-plan`, `fk substrate host-scan`, `fk substrate reconcile-once` missing-root no-op behavior, `fk substrate reconcile-launchd-{plist,install,bootstrap,status}` operator-visible decision/action output and scheduling, signed live VZ proof that a fresh running sandbox marker is preserved rather than cleaned up, and signed live proof that a stale marked host process is terminated before active marker cleanup; acceptance `stuck_vm_cleanup=signed_live_host_process_cleanup_proven`. | Partial | Unmanaged external VZ process enumeration is out of scope; managed Firkin sessions use marker ownership. |
| Multi-container VM semantics | Acceptance `multi_container_vm_substrate=core_live_smoke_proven`; signed core builder live smoke ran two containers on one existing VM; CubeAPI Firkin create plus snapshot-restore tests assert `SingleVmBackedContainer` runtime mode for the default one Cube sandbox to one Firkin VM-backed container mapping; `VmRootfs::GuestPath`, `PodStoreSpec`, `PodBuilder`, `Pod`, `PodContainerSpec`, `EmptyDirVolume`, and OCI rootfs materialization now provide the core pod substrate; signed live smokes prove pod-store rootfs materialization, shared pod `emptyDir`, same-VM loopback, and add/remove without VM reboot. | Partial | Product pods are not exposed yet: no Cube `POST /pods` surface, pod-aware manifest/restore consumer, production route matrix, or soak evidence has landed. |
| Network policy honesty | E2B/host adapter hard-fail tests plus Firkin `LocalRuntimeBackend<FirkinRuntimeAdapter<_>>` rollback coverage; acceptance `network_policy_hard_fail=final_adapter_path_proven`. | Partial | Real enforcement remains absent by design. |
| 24-hour soak | `SoakScenario`, `SoakEvidenceReport`, `SoakEvidenceArtifact`, `SoakCleanupEvidence`, `RuntimeProductSoakRunner`, `just live-runtime-soak-smoke`, `just live-runtime-soak-24h`, and `fk substrate validate-soak` define the required Inspect-like evidence path. The runner drives E2B/Cube product create, command, file write, snapshot save, follow-up restore/prompt, and cleanup; the production validator requires 24 hours, a readable benchmark artifact, zero step failures, and cleanup evidence proving no orphaned VMs, snapshots, logs, or capacity reservations; the signed live VZ 1-second smoke wrote a seven-step zero-failure artifact; acceptance `single_node_24h_soak=runner_smoke_proven`. | Weak | Actual validated 24-hour run artifact exercising create, command, file, snapshot, restore, follow-up, and cleanup. |
| Crate graph | `scripts/check-firkin-crate-graph.sh`; recent commits keep production wiring in `firkin-runtime`. | Partial | Keep enforced in CI and rerun after live adapter work. |

## Current blockers

The biggest blockers are no longer the substrate models or short live smokes.
They are long-run proof and explicitly deferred product depth:

1. Validated 24-hour single-node soak artifact for create, command, file,
   snapshot, follow-up restore, and cleanup.
2. Full Jupyter kernel parity and guest MCP service semantics, both deferred to
   v2 for the current goal.
3. Managed restart ownership depends on Firkin-owned markers. Arbitrary external
   VZ processes without Firkin markers are outside the current single-node
   production scope.
4. Advanced multi-container-per-VM product pods. The core `firkin-core::Pod`
   substrate has landed over a preboot pod-store disk plus guest-path rootfses,
   but the pod-aware manifest/restore consumer, Cube `POST /pods` surface,
   production route matrix, and soak evidence have not landed. The current
   production mapping remains one Cube sandbox to one Firkin VM-backed
   container.

## Next implementation order

Use this order unless live testing proves a lower step blocks an earlier one.
Each step should land with focused tests, an acceptance-checklist status update,
and a small commit.

1. **VZ snapshot restore launcher**
   - Landed: `CoreSnapshotSessionLauncher` reads persisted snapshot restore
     state and calls `firkin-core` VZ restore from the async runtime restore
     path.
   - Landed: `RuntimeCubeSandboxCreate` uses prepared template snapshot
     artifacts as the default CubeAPI/E2B create path with active capacity
     admission and `warm_snapshot_restore` latency.
   - Landed: signed live envd HTTP process-start proof over a restored
     `FirkinRuntimeAdapter`.
   - Landed: signed vendored-SDK command run through
     `firkin_e2b::LocalRuntimeBackend`, the domain proxy, and a restored
     `FirkinRuntimeAdapter`.
   - Landed: signed vendored-SDK filesystem write/read/stat/list/remove and
     missing-file `exists()` through the same path.
   - Landed: signed live two-sandbox filesystem write/read/stat/list/remove
     through the same path.
   - Landed: signed vendored-SDK retained stdin through the same path.
   - Landed: signed vendored-SDK retained PTY input/resize/connect/signal
     through the same path.
   - Landed: signed live two-sandbox retained stdin through the same path.
   - Landed: non-live two-active-sandbox process routing through the domain
     proxy.
   - Landed: non-live two-active-sandbox retained-process connect routing with
     same-pid process handles scoped by sandbox through the domain proxy.
   - Landed: signed live two-sandbox finite command routing through the domain
     proxy.
   - Landed: non-live two-active-sandbox filesystem read/write/stat/list/remove
     routing through the domain proxy.
   - Landed: signed live VZ domain-proxy code-interpreter probe proof on
     `49999`; code-interpreter `/execute` bash protocol runs through the
     runtime envd adapter and product domain proxy with single-sandbox and
     two-active-sandbox signed live VZ proofs; Python `context_id` persists
     pickleable namespace state across repeated execute requests with
     host-backed and signed live VZ proofs; non-live MCP CONNECT tunnel proof
     on `50005` routes through the Firkin port router.
   - Remaining gate: full Jupyter kernel parity and broader live concurrent
     process soak. Guest MCP service semantics are deferred to v2.
   - Do not add Cube/E2B API behavior here; keep launcher mechanics in
     `firkin-runtime`.
2. **Template build to live VZ snapshot**
   - Landed: `RuntimeTemplateBuildSnapshot` defines the runtime command path
     plus snapshot save orchestration and records `cold_template_build` and
     `snapshot_save` latency.
   - Landed: `CoreTemplateCommandRunner` runs repo clone, checkout,
     setup/cache-warm commands inside a live `firkin-core` container.
   - Landed: signed live VZ proof runs a local fixture repo through clone,
     checkout, setup/cache warm, snapshot save, and restore.
   - Landed: CubeAPI unit proof and signed live product-route proof wire Firkin
     classic template create/rebuild, status, logs, list, detail, and delete
     onto snapshot-template builds.
3. **Continuation restore/resume**
   - Landed: `RuntimeContinuationSnapshotRestore` restores the continuation
     manifest through the same async snapshot launcher while preserving
     stopped/idle/exited reason reporting.
   - Landed: `RuntimeCubeSandboxFollowup` wraps continuation restore for
     CubeAPI/E2B follow-up prompts without conflating continuation and base
     template manifests.
   - Landed: signed live VZ proof captures a marker-bearing session, restores
     the continuation snapshot, and verifies guest state in the restored
     session.
   - Landed: signed live VZ proof that the E2B/Cube
     `POST /sandboxes/{id}/snapshots` product route captures the active
     `FirkinRuntimeAdapter` session continuation artifact, deletes the source
     sandbox, restores it through `POST /sandboxes/followups`, and verifies
     guest state in the follow-up sandbox.
   - Landed: local E2B/Cube `POST /sandboxes/followups` product route looks up
     the recorded snapshot location, calls `FirkinRuntimeAdapter::start_followup`,
     and registers the new sandbox under the continuation snapshot id.
   - Landed: signed live VZ product-route proof captures a marker-bearing
     continuation snapshot, restores it through `POST /sandboxes/followups`,
     and verifies command execution in the follow-up sandbox.
   - Remaining gate: 24-hour soak coverage.
4. **Runtime host scanner**
   - Landed: `HostRuntimeScan` carries active VM records, snapshot artifacts,
     logs, stale processes, and heartbeat ages into `RestartReconciliation` and
     `RuntimeStuckVmCleanup`.
   - Landed: `RuntimeHostScanner` reads configured filesystem marker roots into
     `HostRuntimeScan`; absent marker roots are treated as empty so a scheduled
     reconcile job can run before the runtime has created any markers.
   - Landed: `RuntimeFilesystemReconciler` applies filesystem marker cleanup
     and quarantine for restart and stuck-VM plans.
   - Landed: `fk substrate reconcile-once` runs one filesystem-backed
     reconciliation pass and emits JSON action counts.
   - Landed: `fk substrate reconcile-launchd-plist` renders a launchd
     `StartInterval` LaunchAgent for the one-shot reconciliation path.
   - Landed: `fk substrate reconcile-launchd-install` writes that plist
     atomically to an operator-chosen path.
   - Landed: `fk substrate reconcile-launchd-bootstrap` and
     `reconcile-launchd-status` run `launchctl bootstrap`/`kickstart` and
     `launchctl print` for the reconciliation job.
   - Landed: `fk substrate host-scan` emits the reconciliation and stuck-VM
     cleanup decisions as JSON so daemon wrappers can consume the scan without
     scraping operator text.
   - Landed: `FirkinRuntimeAdapter::with_managed_runtime_roots` wires snapshot
     and log preflight together with the active marker root for production
     composition.
   - Landed: managed runtime roots make the adapter write an active marker
     directory with `heartbeat`, `runtime.pid`, and `runtime.executable` after a
     sandbox becomes active, refresh the heartbeat while the sandbox runs, and
     remove the marker on stop.
   - Landed: `RuntimeHostProcessStuckVmCleaner` reads `runtime.pid` and
     `runtime.executable`, terminates the executable-matched marked host
     process through `CommandHostProcessTerminator`, and then removes the
     active-VM marker for stuck cleanup decisions.
   - Landed: `RuntimeRestartRecovery` owns the one-shot recovery path by
     scanning marker roots, applying restart reconciliation, and running
     executable-checked stuck-VM cleanup.
   - Scope boundary: unmanaged external VZ processes without Firkin markers are
     not discovered or cleaned by this single-node runtime.
5. **Scheduler integration**
   - Landed: `ActiveCapacityAdmissionPlan` gives active restores priority over
     optional warm-pool capacity, and `FirkinRuntimeAdapter` evicts and stops
     retained warm sessions before cold create or follow-up restore reservation
     when that admits the request.
   - Landed: `fk e2b host` spawns the existing control-plane lifecycle
     scheduler on a configurable interval so sandbox timeout expiration runs
     during the long-lived host process.
   - Landed: persistent lifecycle scheduler expiration ticks save the state JSON
     after successful timeout mutations, so a host restart does not resurrect
     scheduler-deleted sandboxes.
   - Landed: `ActiveBackpressurePlan` and the adapter bounded active queue
     admit work immediately when it fits, queue only work that can fit after an
     active release, and reject impossible or over-queued work.
   - Remaining gate: sustained 24-hour pressure evidence across restore, warm
     refill, snapshot save, delete, and cleanup churn.
6. **Benchmark runner**
   - Landed: `RuntimeBenchmarkEvidenceWriter` validates runtime samples and
     emits `BenchmarkEvidenceArtifact` JSON. Shared default lifecycle latency
     targets feed both `fk substrate latency-targets` and the SLO gate.
     `BenchmarkSloGateReport` rejects missing targets, wrong metric shape, too
     few samples, and p95 regressions.
   - Landed: `just live-runtime-benchmark-representative` wrote a three-sample
     signed live VZ lifecycle artifact and passed the shared SLO gate for all
     nine required lifecycle metrics.
   - Landed: `just live-runtime-overhead-representative` wrote a three-sample
     signed live VZ overhead artifact and passed the shared SLO gate for all
     five required overhead metrics.
   - Remaining gate: 24-hour soak evidence under sustained product-route load.
7. **Soak runner**
   - Landed: `RuntimeProductSoakRunner` drives the product route loop through
     create, command, file write, snapshot save, follow-up restore/prompt, and
     cleanup, and writes a `SoakEvidenceReport` with benchmark artifact and
     cleanup evidence fields.
   - Landed: `just live-runtime-soak-smoke` ran a signed live VZ 1-second
     seven-step zero-failure smoke and wrote
     `target/firkin-live-evidence/live-soak-evidence-smoke.json`.
   - Remaining: execute `just live-runtime-soak-24h` on one Mac and persist
     logs, metrics, snapshot artifacts, cleanup reports, and the resulting
     validated `SoakEvidenceReport`.
   - Gate: `fk substrate validate-soak` passes for the 24-hour artifact, the
     artifact references the benchmark evidence artifact, and it proves no
     orphaned VMs, snapshots, logs, or capacity reservations remain.

## Low-disk operating rule

Do not start a broad workspace build while this machine has less than 20 GiB
free. Use focused package checks and `CARGO_TARGET_DIR=/tmp/firkin-target` when
space is available, then remove that target directory after verification. The
production runtime itself should enforce a 10 GiB host free-space floor through
`RuntimeDiskPressureGuard`.

## Disk note

The local development volume was last checked above the cleanup threshold after
manual target/toolchain/APFS snapshot cleanup. Continue checking free space
before broad builds; run cleanup before it drops below 20 GiB.
