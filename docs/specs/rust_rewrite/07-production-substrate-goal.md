# Production Apple/VZ substrate goal

Status: goal document. This is the next target after proving that CubeAPI can
use Firkin as a real Apple/VZ runtime library.

## Prerequisite mapping

| Need | Firkin/Cube prerequisite |
|---|---|
| Image builds | Firkin needs a template/build pipeline that can clone a repo, install dependencies, run setup and cache-warming commands, then snapshot the ready state. |
| Warm starts | Snapshot restore should be the normal session-create path. Firkin also needs a warm pool keyed by repo, template, and runtime profile. |
| Freshness | Cube/Firkin needs a sync phase: restore the latest prepared snapshot, pull or rebase to the requested branch, allow read-only research early, and block writes until sync completes. |
| Follow-ups | Finished or idle sessions need durable continuation snapshots, separate from base template snapshots. |
| Latency and overhead | Every lifecycle path needs measured SLOs: cold template build, warm restore, command start, first stdout byte, ready probe, kill/delete, snapshot save, and warm-pool checkout. Firkin's own CPU, memory, disk, and wakeup overhead must be tracked separately from the real resources consumed by VMs, containers, rootfses, and snapshots. |
| Single-node capacity | The local scheduler must manage CPU, RAM, disk, and snapshot pressure while keeping pools warm without starving active sandboxes. |
| Semantics | The production Cube mapping is one Cube sandbox to one Firkin VM-backed container. One Firkin VM can host multiple Firkin containers; the advanced Pod path is designed in `docs/plans/2026-05-04-firkin-pod-support-design.md`, but it remains outside the default Cube mapping until the Pod API, Cube lifecycle, snapshot, and isolation semantics are implemented and live-proven. |
| Production hygiene | Firkin needs preflight, restart reconciliation, garbage collection, log rotation, stuck-VM cleanup, artifact integrity checks, pressure handling, and soak tests. |
| Policy | Unrestricted networking is supported. Restrictive network policy must hard-fail until Firkin has real guest firewall rules, host PF anchors, or another enforceable per-sandbox policy mechanism. |
| Crate graph | Low-level crates stay clean: `substrate` owns portable policy/models, `template` owns host build/freshness execution, `core` owns VM/container mechanics, and a production runtime composition crate wires them together for CubeAPI. |

## Goal

Build Firkin into a production-grade Apple/VZ substrate for CubeAPI, capable of
powering a Ramp Inspect-style background coding agent on one powerful Mac.

Firkin must provide fast, durable, VM-backed sandbox execution where CubeAPI
owns E2B/Cube product semantics and Firkin owns VM/container mechanics. The
default production mapping is one Cube sandbox to one Firkin VM-backed
container. Firkin must also explicitly document and test that one Firkin VM can
host multiple Firkin containers. Multi-container-per-VM stays an advanced
substrate mode: the Pod design exists in
`docs/plans/2026-05-04-firkin-pod-support-design.md`, but the default Cube
mapping remains one sandbox to one VM-backed container until the Pod API, Cube
lifecycle, snapshot, and isolation semantics are implemented and live-proven.

Snapshot restore is the primary create path. Firkin should support repo/template
build jobs that clone code, install dependencies, run setup and cache-warming
commands, and save durable base snapshots. Cube can then restore from those
snapshots, sync latest branch changes, allow read-only agent research before
sync completes, and block writes until sync is safe. Firkin should also support
warm pools for hot repos/templates and continuation snapshots for follow-up
prompts after a session exits.

The substrate must have concrete latency targets and benchmark evidence for
cold template build, warm snapshot restore, command start, first stdout byte,
snapshot save, kill/delete, and concurrent sandbox creation. It must also prove
that Firkin's own impact is barely perceptible: VM and container workloads may
consume real VM/container resources, but the extra host CPU, memory, disk, and
wakeup tax from Firkin/Cube control mechanics should stay small and visible in
metrics. Production hygiene includes preflight, capacity scheduling, state
reconciliation after restart, snapshot/artifact GC, log rotation, stuck VM
cleanup, pressure handling, and 24-hour soak tests.

Network policy must be honest: unrestricted networking works; restrictive
policy is rejected until real enforcement exists. Production readiness means
measured fast paths, durable snapshots, restart-safe state, explicit
VM/container semantics, and enough local acceptance coverage that a one-person
company could run an Inspect-like agent 24/7 on a beefy Mac box.

The crate graph is part of readiness. VZ-backed template snapshot saving,
snapshot-restore create, capacity admission, and Cube/E2B API semantics should
meet in a production runtime composition crate rather than by adding `core` or
E2B dependencies to `firkin-template` or `firkin-substrate`.

## Acceptance outline

The goal is not complete until each item has concrete artifact and command
evidence. `fk substrate acceptance-checklist` prints the stable check IDs and
current evidence state for this list.

1. A template build command produces a durable snapshot from a real repo after
   dependency install, setup, and cache-warming commands. `SnapshotArtifactManifest`
   is the initial substrate model for distinguishing base template snapshots
   from continuation snapshots. `TemplateBuildJob` models repo checkout,
   setup commands, cache-warming commands, and snapshot output; build execution
   now executes clone, checkout, setup, and cache-warming through
   `firkin_template::TemplateBuildExecutor`. `TemplateSnapshotSink` is the
   async runtime seam for durable snapshot creation; `firkin_runtime`
   provides `CoreContainerSnapshotSink` for `firkin-core` containers. Runtime
   template builds write manifest and integrity sidecars after snapshot save. A
   signed live VZ smoke now proves clone, checkout, setup, cache warm, snapshot
   save, restore, and restored checkout verification from a host-served Git
   repo. The shared `RuntimeAdapter::build_template` default and
   `FirkinRuntimeAdapter` hard-fail direct adapter-level template build calls
   instead of returning fake empty artifacts. CubeAPI has unit-proven and
   signed-live-proven Firkin classic create/rebuild/status/log/list/detail/delete
   wiring onto the snapshot-template path.
2. A session create restores from that snapshot by default and records latency.
   `firkin_runtime::RuntimeSnapshotRestore` is the initial runtime composition
   seam: it admits active capacity, calls a snapshot launcher with the manifest,
   records `warm_snapshot_restore`, and releases capacity when launch fails.
   Direct VZ-backed launcher integration is still missing.
3. Warm pools can maintain and expire pre-restored sandboxes for hot templates.
   `WarmPoolLedger` is the initial substrate model for repo/template/runtime
   profile keys, checkout promotion, and expiration;
   `firkin_runtime::RuntimeWarmPoolMaintain` is the initial runtime seam for
   restoring and recording warm entries. `RuntimeWarmPoolCheckout` promotes a
   warm entry to active use and records `warm_pool_checkout` latency.
   `RuntimeSnapshotWarmPool` retains restored sessions keyed by repo/template/
   runtime profile, checks them out directly, records checkout latency, and has
   signed live VZ proof that a checked-out warm session can run a command.
   `WarmPoolReplenishmentPlan` defines which missing warm entries to refill
   without consuming capacity needed by active sessions, and
   `RuntimeSnapshotWarmPool::replenish_with_elapsed` executes one refill pass
   through a snapshot launcher. `RuntimeWarmPoolSupervisor` provides the
   bounded, testable cadence loop for repeated refill passes.
   `RuntimeWarmPoolService` owns the retained pool, supervisor, and launcher
   together, proves refill skips while active capacity is exhausted, and can
   spawn an owned refill task with clean shutdown. `ActiveCapacityAdmissionPlan`
   makes active restore requests reclaim deterministic warm-pool entries before
   rejecting for capacity, and `FirkinRuntimeAdapter` now evicts and stops
   retained warm sessions before launching cold create or follow-up restores
   when that is enough to fit the request. The adapter prewarm path retains a
   clean restored session without readiness-probe or command side effects until
   checkout. Richer production eviction tuning is still missing.
4. A session sync phase permits read-only operations before sync completion and
   blocks writes until sync completes.
   `FreshnessSyncGate` is the initial substrate model for that read/write gate;
   `firkin_template::FreshnessSyncExecutor` performs git fetch, checkout, and
   reset for a host checkout and returns the ready write-unlocked gate.
   `FirkinRuntimeAdapter::run_freshness_sync` performs the same git
   fetch/checkout/reset sequence inside the restored runtime session when
   create metadata includes the checkout path, and the adapter spawns that
   runtime sync after restore so reads can proceed while writes remain gated.
5. A stopped or idle session can be resumed from a continuation snapshot.
   `ContinuationSnapshotPlan` is the initial substrate model for idle, stopped,
   and exited follow-up snapshots. `RuntimeContinuationSnapshotCapture` saves
   the continuation artifact through the async snapshot sink, writes manifest
   and integrity sidecars, and records `snapshot_save` latency. A signed live VZ
   smoke now proves continuation capture, restore, and guest-state verification.
   The local E2B/Cube
   product route `POST /sandboxes/{id}/snapshots` captures the active
   runtime session continuation artifact, `POST /sandboxes/followups`
   restores recorded continuation snapshots through
   `FirkinRuntimeAdapter::start_followup`, and signed live VZ smokes prove
   the create-snapshot route captures restorable guest state and the follow-up
   route restores a marker-bearing continuation snapshot and runs a command in
   the follow-up sandbox. `FirkinRuntimeAdapter::delete_snapshot` removes the
   continuation artifact plus manifest and integrity sidecars.
6. Benchmarks report cold build, restore, command start, first stdout byte,
   ready probe, snapshot save, kill/delete, and concurrent create latency.
   `BenchmarkSample`, `BenchmarkSummary`, and `BenchmarkEvidenceReport` define
   the metric sample schema, p50/p95 summaries, and required lifecycle metric
   coverage validator. `BenchmarkEvidenceArtifact` persists the validated report
   as JSON for later audit, and `RuntimeBenchmarkEvidenceWriter` gives runtime
   collected samples one validated artifact write path.
   `RuntimeWarmPoolCheckout` records the first runtime `warm_pool_checkout`
   sample. Template and continuation snapshot capture record `snapshot_save`.
   `RuntimeSessionTerminate` records `kill_delete`.
   `just live-runtime-benchmark-representative` runs the signed live VZ
   lifecycle artifact with three representative samples and the shared p95 SLO
   gate passes all nine required lifecycle metrics.
7. Benchmarks report Firkin/Cube overhead separately from VM/container workload
   resources, including idle CPU, resident memory, per-sandbox host-side memory,
   disk metadata growth, and idle wakeup rate.
   `BenchmarkOverheadEvidenceReport` validates required Firkin overhead metrics
   and units separately from VM/container workload resource metrics.
   `RuntimeOverheadEvidenceWriter` gives runtime overhead samples one validated
   artifact write path. `fk substrate validate-overhead-slo` and
   `just live-runtime-overhead-slo-gate` provide the operator gate for signed
   live overhead evidence. `just live-runtime-overhead-representative` runs the
   signed live VZ overhead artifact with three representative samples and the
   shared p95 SLO gate passes all five required overhead metrics.
8. Runtime preflight validates the host and runtime roots before runtime work
   starts. `fk debug preflight` reports Virtualization.framework capability and
   signing state, while `RuntimePreflight` checks required snapshot/log roots
   and the host free-space floor before disk-consuming work starts. `fk e2b
   host` creates the sandbox/log roots and runs that 10 GiB preflight before
   serving the product control plane or domain proxy. Configured
   `FirkinRuntimeAdapter` instances run the same preflight before `start()` and
   `prewarm_template()` restore work.
   `FirkinRuntimeAdapter::with_managed_runtime_roots` wires snapshot/log
   preflight and the active-VM marker root together for production composition;
   broader daemon/operator wrappers still need to use that helper or call the
   same startup gate if they bypass `fk e2b host`.
9. Restart reconciliation recovers or cleans up active VMs, snapshots, logs, and
   state records without orphaning resources.
   `ReconciliationPlan` is the initial substrate model for recover, cleanup,
   and quarantine decisions. `HostRuntimeScan` carries discovered active VMs,
   snapshot artifacts, logs, stale processes, and VM heartbeat observations into
   reconciliation and stuck-VM cleanup plans. `RestartReconciliation` applies
   those decisions through a runtime executor adapter. `RuntimeHostScanner`
   reads filesystem marker roots into that scan shape.
   `RuntimeFilesystemReconciler` applies filesystem marker cleanup and
   quarantine actions for restart and stuck-VM plans. `fk substrate
   reconcile-once` runs one filesystem-backed reconciliation pass and prints
   JSON action counts; `fk substrate reconcile-launchd-plist` renders an
   interval-launched LaunchAgent for that one-shot path, and
   `fk substrate reconcile-launchd-install`, `reconcile-launchd-bootstrap`,
   and `reconcile-launchd-status` provide the install/run/status operator path.
   `FirkinRuntimeAdapter` can publish filesystem active-VM marker directories
   with heartbeat timestamps plus the owning runtime PID and executable for
   started sessions, refresh the heartbeat while sessions run, and remove the
   marker directory on stop so the scanner sees runtime-owned adapter state,
   not just hand-authored marker fixtures.
   `FirkinRuntimeAdapter::with_managed_runtime_roots` makes that marker root
   part of the production runtime root configuration. `fk substrate
   reconcile-once` uses `RuntimeRestartRecovery` to scan marker roots, run
   restart reconciliation, and invoke the executable-checked
   host-process-backed stuck-VM cleaner for stuck active-VM cleanup. Arbitrary
   external VZ processes without Firkin ownership markers are outside the
   managed single-node scope.
   `fk substrate host-scan` reads marker roots and prints JSON restart and
   stuck-VM decisions, including owning runtime PIDs for stuck active VMs, for
   operator inspection and service consumption.
10. Snapshot artifact GC removes unreferenced snapshot files and directories
   without deleting referenced artifacts or manifest sidecars. `ArtifactGcPlan`
   is the filesystem executor and `RuntimeSnapshotArtifactGc` is the runtime
   wrapper; age-based retention keeps recent unreferenced artifacts out
   of GC while manifests catch up. `RuntimeHygieneMaintenance` gives GC a
   periodic runtime owner. `SnapshotArtifactManifest` persists JSON sidecars and
   discovers direct `*.manifest.json` sidecars in sorted order, and runtime GC
   can consume those sidecars while preserving the sidecar files themselves.
   The maintenance owner can reread sidecars each tick when configured with a
   manifest directory. `fk substrate hygiene-once` runs a schedulable one-shot
   pass over snapshot/log roots with optional manifest sidecar discovery and
   gzip log rotation. `fk substrate hygiene-daemon` runs the same periodic
   owner until interrupted for a long-lived single-node backend, and
   `fk substrate hygiene-launchd-plist` renders a launchd service plist for it.
   `fk substrate hygiene-launchd-install` writes that plist atomically to an
   operator-chosen path. `fk substrate hygiene-launchd-bootstrap` runs
   `launchctl bootstrap` and `kickstart` for that plist, and
   `fk substrate hygiene-launchd-status` runs `launchctl print`.
11. Snapshot artifact integrity verifies snapshot size and SHA-256 before use.
   `SnapshotArtifactIntegrity` is the initial verifier, and
   `SnapshotArtifactManifest` persists durable JSON sidecars for artifact
   metadata. `SnapshotArtifactIntegrity` also persists integrity JSON sidecars
   that restore can consume before externally materialized snapshots launch.
   Template build and continuation capture write both sidecars after snapshot
   save, and prepared-template create plus follow-up restore can fill missing
   embedded integrity from the artifact sidecar. `just live-runtime-integrity`
   captures a live VZ snapshot, mutates the artifact, and proves the runtime
   adapter rejects it before restore. `fk substrate snapshot-sidecars` writes
   manifest and integrity sidecars for an existing externally materialized
   artifact, giving operators the same durable import convention used by
   runtime-created template and continuation snapshots.
12. Log rotation rotates oversized log files from the log root. `LogRotationPlan`
   is the conservative filesystem executor and `RuntimeLogRotation` is the
   runtime wrapper. Bounded generation retention prevents unbounded `.1`, `.2`,
   etc. growth, and optional gzip compression writes bounded `.N.gz` rotated
   generations. `RuntimeHygieneMaintenance` gives log rotation a periodic
   runtime owner. `fk substrate hygiene-once` exposes it as an operator
   one-shot hook, `fk substrate hygiene-daemon` exposes the long-lived
   periodic entrypoint, and `fk substrate hygiene-launchd-plist` renders a
   launchd service plist for the daemon. `fk substrate hygiene-launchd-install`
   writes that plist atomically to an operator-chosen path, and
   `fk substrate hygiene-launchd-bootstrap` starts it through launchctl.
   `fk substrate hygiene-launchd-status` prints launchd state.
13. Stuck VM cleanup identifies VM records whose heartbeat age exceeds the
    configured timeout. `StuckVmCleanupPlan` is the initial deterministic
    preserve, cleanup, and quarantine planner. `RuntimeStuckVmCleanup` applies
    the plan through a runtime cleaner adapter.
    `RuntimeHostProcessStuckVmCleaner` reads `runtime.pid` and
    `runtime.executable`, terminates the executable-matched marked host process
    through `CommandHostProcessTerminator`, then removes the active-VM marker.
    `fk substrate stuck-vm-plan` prints operator-visible decisions from
    heartbeat observations, and `fk substrate host-scan` prints those decisions
    from filesystem marker roots. Live VZ enumeration is still missing.
14. Capacity scheduling accounts for CPU, RAM, disk, snapshot artifact pressure,
    active sandboxes, and warm-pool reservations.
   `firkin_substrate::CapacityLedger` is the initial substrate admission model
   for this requirement, and `ActiveCapacityAdmissionPlan` explicitly gives
   active work priority over optional warm-pool inventory.
   `firkin_runtime::RuntimeSnapshotRestore` wires active capacity admission into
   the snapshot-restore path, and `FirkinRuntimeAdapter` applies the
   active-priority warm eviction policy before cold create and follow-up restore
   reservation. `RuntimeDiskPressureGuard` adds an explicit host free-space
   floor admission seam for runtime roots and snapshot storage, and
   `HostDiskPressureProbe` reads real host free space through `df -Pk`. Runtime
   template snapshot saves and continuation snapshot captures now check the same
   10 GiB floor before running build commands or saving snapshot artifacts.
   `ActiveBackpressurePlan` defines the bounded queue policy above immediate
   admission: active work evicts optional warm entries first, queues only work
   that can fit after active releases, and rejects impossible or over-queued
   requests. `FirkinRuntimeAdapter` wires that policy into cold create and
   follow-up restore paths with a bounded active queue that wakes queued
   snapshot restores when active capacity is released.
15. Multi-container-per-VM remains documented and tested as substrate capability,
   not the default Cube sandbox mapping. The intended advanced path is the Pod
   design in `docs/plans/2026-05-04-firkin-pod-support-design.md`; it is not
   readiness evidence for Cube product pods until the `firkin-core::Pod` API,
   pod-aware manifests/restore, and Cube product lifecycle routes land with
   signed live coverage.
16. Restrictive network policy requests fail before boot until a real policy
    enforcement backend exists.
17. A 24-hour single-node soak runs an Inspect-like loop with create, command,
    file, snapshot, restore, follow-up, and cleanup behavior.
    `SoakScenario` defines the required loop and 24-hour evidence threshold;
    `RuntimeProductSoakRunner` drives the loop through the E2B/Cube product
    routes and a signed live 1-second smoke writes a seven-step zero-failure
    artifact. The production validator requires 24 hours, a readable benchmark
    artifact, zero step failures, and cleanup evidence proving no orphaned
    VMs, snapshots, logs, or capacity reservations. A real validated 24-hour
    run is still missing.
18. The production crate graph is enforced in CI. `firkin-substrate`,
    `firkin-template`, `firkin-core`, and `firkin-e2b` keep their current
    ownership boundaries; production wiring belongs in a top-level composition
    crate.
