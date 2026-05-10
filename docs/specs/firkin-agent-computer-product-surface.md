# Firkin Agent Computer Product Surface

Status: design spec, 2026-05-08.

This is the product surface that the snappy metrics work now has to serve:

```text
give an agent a real mini computer
make it cheap to keep ready
make it fast to lease, resume, and use
make it disappear quickly under host pressure
make every performance claim traceable to decision-grade evidence
```

The default product profile is `browser + database + CLI`. Shell-only sandbox
metrics remain necessary, but they are not sufficient to prove the product
path.

## Validation Target

The product surface is ready when these commands can write inspectable signed
live artifacts:

```bash
cargo build --release -p firkin-cli

FIRKIN_BASELINE_NAME=local-agent-computer-product-pod-narrow-start-gate-20260508 \
FIRKIN_BASELINE_DURATION=60s \
scripts/run-firkin-decision-baseline.sh agent-computer

target/release/fk benchmark report-agent-computer-scorecard \
  target/firkin-live-evidence/local-agent-computer-product-pod-narrow-start-gate-20260508.json

target/release/fk benchmark validate-agent-computer-scorecard \
  --require-promotable \
  --require-snappy \
  target/firkin-live-evidence/local-agent-computer-product-pod-narrow-start-gate-20260508.json

FIRKIN_LIVE_AUTOSCALE_REPEATS=3 \
FIRKIN_LIVE_AUTOSCALE_ARTIFACT=target/firkin-live-evidence/local-autoscale-scorecard-pressure-promotes-superfast.json \
scripts/run-signed-live-runtime-test.sh \
  live_runtime_autoscale_scorecard_writes_product_path_artifact

target/release/fk benchmark report-autoscale-scorecard \
  target/firkin-live-evidence/local-autoscale-scorecard-pressure-promotes-superfast.json

target/release/fk benchmark validate-autoscale-scorecard \
  --min-samples 3 \
  --require-promotable \
  --require-snappy \
  target/firkin-live-evidence/local-autoscale-scorecard-pressure-promotes-superfast.json
```

The baseline script's `agent-computer` mode now captures the autoscale proof
beside the product-path proof by default. The first two files to open are the
manifest and HTML proof:

```text
<baseline>.product-pod-artifacts.txt
<baseline>.product-pod-ready-deck-proof.html
```

The same run writes these product-path and density siblings:

```text
<baseline>.product-pod-readiness.json
<baseline>.product-pod-readiness-traces.txt
<baseline>.product-pod-ready-deck.json
<baseline>.product-pod-ready-deck-traces.txt
<baseline>.product-pod-ready-deck-density.json
<baseline>.product-pod-ready-deck-density-traces.txt
<baseline>.product-pod-prestarted-agent-slot-density.json
<baseline>.product-pod-prestarted-agent-slot-density-traces.txt
<baseline>.autoscale-scorecard.json
<baseline>.autoscale-scorecard.txt
<baseline>.autoscale-scorecard-validate.txt
<baseline>.autoscale-scorecard-promotable.txt
```

`<baseline>.autoscale-scorecard-promotable.txt` is allowed to record the known
promotion blockers or sample-floor failures during iteration; it is the handoff
file for the next metric cleanup slice, not an excuse to enforce blocked SLAs.

The live `agent-computer` scorecard has cut over to the product-pod ready-deck
path for product ready/resume: CLI runs through the product pod, browser
readiness comes from the real browser sidecar, and database readiness comes
from the real DB sidecar. The current scorecard smoke artifact proves the
promotable shape and validates with `promotion_blockers=0`, but its count `3`
sample set is not decision-grade:

```bash
target/release/fk benchmark validate-agent-computer-scorecard \
  --require-promotable \
  --require-snappy \
  target/firkin-live-evidence/local-agent-computer-product-pod-narrow-start-gate-20260508.json
```

That representative smoke artifact has `count=3`, reports
`product.agent_computer_ready_ms` max `55.78ms`,
`product.agent_computer_resume_ms` max `67.86ms`, and reports product-path
density breakpoint `4`. It proves the current shape, not decision-grade p95.

The standalone DB-sidecar and browser-sidecar proofs are now lower-level
component checks. They are useful when a product-pod ready-deck drilldown points
at one sidecar, but they do not replace the integrated browser + database + CLI
scorecard:

```bash
FIRKIN_LIVE_DB_SIDECAR_ARTIFACT=target/firkin-live-evidence/local-db-sidecar-readiness-smoke.json \
  scripts/run-signed-live-runtime-test.sh \
  live_runtime_db_sidecar_readiness_writes_exact_sample

FIRKIN_LIVE_BROWSER_SIDECAR_ARTIFACT=target/firkin-live-evidence/local-browser-sidecar-readiness-smoke.json \
  scripts/run-signed-live-runtime-test.sh \
  live_runtime_browser_sidecar_readiness_writes_exact_sample
```

Pass signal:

```text
product.agent_computer_ready_ms p95 <250ms
product.agent_computer_resume_ms p95 <75ms
exec.first_stdout_byte_ms p95 <25ms
start.warm_to_first_stdout_ms p95 <350ms
exec.batch_100_small_commands_ms p95 <500ms
density.max_active_before_hot_to_first_stdout_p95_doubles >=8
density.max_agent_computers_before_ready_p95_doubles >=4
density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles >=4
disk.sparse_bloat_after_trim <1.25x
cleanup.leftover_bytes = 0
reliability.unknown_failure_rate = 0
autoscale.ready_queue_hit_rate_pct >90%
autoscale.safe_spare_limiting_utilization_pct >70%
autoscale.pressure_to_safe_floor_ms <5s
autoscale.pressure_clear_to_ready_target_ms <10s
autoscale.reserve_floor_violations = 0
autoscale.active_evictions_due_to_pool_pressure = 0
```

The current prerequisite board is printed by:

```bash
fk benchmark autoscale-contract
```

Metrics with `status=needs_live_harness`, `status=unit_validated_only`, or
proxy status are not optimization-grade yet. Old proxy/SDK artifacts are
pressure telemetry only; they must not be read as product-ready evidence for
real CLI, browser, or DB boundaries. A structurally valid autoscale scorecard
is only measurement-enforceable when `validate-autoscale-scorecard` reports
`promotion_blockers=0`; it is only snappy-enforceable when the same command
reports `snappy_target_status=pass`. Use
`--require-promotable --require-snappy` for hard gates. Without those flags,
validation reports blockers and target misses but still succeeds so harness
development can inspect partial artifacts.

The baseline helper defaults the autoscale proof to `3` repeats and validates
with `--min-samples 3`, matching the `superfast_iteration` tier. Raise
`FIRKIN_BASELINE_AUTOSCALE_REPEATS` and
`FIRKIN_BASELINE_AUTOSCALE_MIN_SAMPLES` together when moving from smoke shape to
decision-grade pressure/autoscale evidence. The expensive decision-grade shape
is:

```bash
FIRKIN_BASELINE_NAME=local-agent-computer-decision-grade-$(date +%Y%m%d) \
FIRKIN_BASELINE_DURATION=35m \
FIRKIN_BASELINE_AUTOSCALE_REPEATS=100 \
FIRKIN_BASELINE_AUTOSCALE_MIN_SAMPLES=100 \
FIRKIN_BASELINE_SHELL_DENSITY_LEVELS=1,2,4,8 \
FIRKIN_BASELINE_PRODUCT_POD_READY_DECK_DENSITY_LEVELS=1,2,4,8 \
FIRKIN_BASELINE_PRODUCT_POD_PRESTARTED_AGENT_SLOT_DENSITY_LEVELS=1,2,4,8 \
scripts/run-firkin-decision-baseline.sh agent-computer
```

The same baseline helper keeps density tiers explicit. Shell density defaults to
`FIRKIN_BASELINE_SHELL_DENSITY_LEVELS=1,2`; product ready-deck and prestarted
slot density default to `1,2,4`. Set the relevant variable to `1,2,4,8` for a
snappy-density proof, and check the emitted `concurrency_levels` tag before
treating the breakpoint as a target-tier result.

The current autoscale smoke artifact
`target/firkin-live-evidence/local-autoscale-scorecard-pressure-promotes-superfast.json`
structurally validates and passes `--require-promotable` with
`promotion_blockers=0`. Product ready/resume, ready-hit-rate, safe-spare
resource accounting, pressure shrink/refill, both density rows,
cleanup/unknown-failure, and active-protection/floor-safety rows are signed-live
or signed-live scoped product-path samples in that artifact. This is
`superfast_iteration` evidence with `count=3`, so it proves promotable metric
shape and trace boundaries, not decision-grade p95.

Safe-spare is scoped to the current ready-queue harness: host CPU/memory/disk
capacity is probed live, runtime active allocation is zero for the idle ready
slot, the reserve floor is runtime config, and the ready queue budget is one
browser + database + CLI product pod. Pressure shrink/refill is scoped to a
host disk-pressure scenario: an elevated runtime disk floor triggers
`PressureDetected`, the normal reserve-floor probe gates `SafeFloorRestored`,
and a ready-queue hit probe gates `ReadyTargetRestored`.

The two product density metrics are intentionally separate and must not be
substituted for each other:

| Metric | Proves | Does not prove | Target | Current smoke status |
| --- | --- | --- | ---: | --- |
| `density.max_agent_computers_before_ready_p95_doubles` | full browser + database + CLI product-pod agent add/start contention | checkout of already-running slots | `>=4` | breakpoint `4`, count `3`; decision-grade sample count still open |
| `density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles` | checkout acceptance of slots already kept on deck | full product-pod add/start capacity | `>=4` | breakpoint `2` for levels `1,2,4`; full output max remained about `20ms` |

The full product-pod path uses prepared guest-path pod spawn so container rootfs
preparation does not hold the pod lock, then gates only the fragile
`startProcess` RPC with one per-pod permit to avoid the repeatable vminitd
concurrent-start failure observed during the representative baseline. Promoting
breakpoint `4` to decision-grade sample counts is the next density lane.

The prestarted-slot metric must carry
`measurement_boundary=prestarted_slot_checkout`,
`slot_surface=prestarted_agent_slot`, `excludes_container_add=true`,
`ready_signal=request_fifo_acceptance`, and `output_wait_preserved=true`. Slot
dispatch now uses FIFO request acceptance and detached emptyDir writes; the
remaining target miss is concurrent dispatch overhead against a sub-millisecond
baseline.

## Topology Anchor

The live crate-topology anchor is
[`docs/specs/rust_rewrite/10-post-split-crate-topology.md`](rust_rewrite/10-post-split-crate-topology.md).
The split plan
[`docs/plans/2026-05-06-firkin-workspace-crate-split-spec.md`](../plans/2026-05-06-firkin-workspace-crate-split-spec.md)
is historical implementation context.

Keep these boundaries:

- `firkin-trace`: raw events, spans, samples, recorder mechanics.
- `firkin-evidence`: metric catalog, summaries, validation, SLO gates.
- `firkin-benchmark`: suite definitions, live runners, artifact production.
- `firkin-sandbox`: neutral sandbox/computer primitives.
- `firkin-agent-computer`: product semantics for the default agent computer.
- `firkin-admission`: pure admission, ready-queue, pressure, and autoscale
  policy.
- `firkin-runtime` and `firkin-single-node`: runtime orchestration and concrete
  Apple/VZ single-node backend.

Do not put product semantics into `firkin-sandbox`. Do not put runtime or VM
knowledge into `firkin-evidence`.

## Rust Product API

The product API should be native, not E2B-shaped:

```rust
let runtime = firkin::sandbox::Runtime::build(AppleVzBackend::from_config(config)).await?;

let product = firkin::agent_computer::AgentComputerRuntime::builder(runtime)
    .profile(AgentComputerProfile::browser_db_cli())
    .autoscale(AutoscalePolicy::snappy_default())
    .build()?;

let computer = product
    .computers()
    .checkout(AgentComputerSpec::interactive(template_id))
    .await?;

computer.exec(Command::shell("git status")).await?;
computer.browser().endpoint().await?;
computer.database().health().await?;
computer.suspend().await?;
```

`firkin-agent-computer` should own these product concepts:

- `AgentComputerRuntime`
- `AgentComputerSpec`
- `AgentComputerProfile`
- `AgentComputerHandle`
- `BrowserEndpoint`
- `DatabaseEndpoint`
- `CliProbe`
- `ProductReadiness`
- `ReadyQueue`
- `AutoscalePolicy`

## CLI Surface

First native CLI surface:

```text
fk agent-computer prepare
fk agent-computer prewarm
fk agent-computer create
fk agent-computer exec
fk agent-computer files
fk agent-computer browser
fk agent-computer database
fk agent-computer snapshot
fk agent-computer suspend
fk agent-computer resume
fk agent-computer destroy
fk ready-queue status
fk autoscale status
fk autoscale policy
fk benchmark autoscale-contract
```

The E2B-compatible server remains a compatibility surface. It should not be the
canonical product API.

## Server Surface

Native server routes:

```text
POST   /v1/agent-computers
GET    /v1/agent-computers/{id}
DELETE /v1/agent-computers/{id}
POST   /v1/agent-computers/{id}/exec
GET    /v1/agent-computers/{id}/browser
GET    /v1/agent-computers/{id}/database
POST   /v1/agent-computers/{id}/suspend
POST   /v1/agent-computers/{id}/resume
GET    /v1/ready-queue
GET    /v1/autoscale/status
PUT    /v1/autoscale/policy
GET    /v1/events
```

Keep wire DTOs separate from domain types. Request/response structs should live
in the server/wire crate that owns the HTTP contract.

## Lifecycle

Use closed states:

```text
TemplatePreparing -> TemplateReady
TemplateReady -> WarmSandbox -> ReadyAgentComputer
ReadyAgentComputer -> Leased -> Active
Active -> IdleReady
IdleReady -> SidecarsStopped -> Suspended -> ReadyAgentComputer
IdleReady -> Evicting -> Destroyed
any -> Failed(classified)
```

Product ready means all of this is true:

```text
CLI first useful stdout succeeds
browser control endpoint is ready
database healthcheck is ready
workspace is mounted
network policy is applied
cgroups and mounts are applied
```

Hot/resumed ready capacity feeds the ready queue. Warm and cold paths are ready
queue misses.

## Admission And Autoscale

`firkin-admission` should keep pure policy types:

- `ReserveFloorPolicy`
- `DemandSnapshot`
- `SupplySnapshot`
- `PressureSnapshot`
- `ReadyQueueTarget`
- `AutoscalePlan`
- `AutoscaleAction`
- `ActiveBackpressurePlan`

Policy knobs:

```text
max_agent_computers
max_cli_sandboxes
min_ready_agent_computers
max_ready_agent_computers
reserved_free_disk = max(20GiB, 10%)
reserved_free_memory = max(8GiB, 15%)
safe_spare_target_pct
hysteresis_hold_ms
refill_cooldown_ms
queue_max_pending
queue_max_wait_ms
idle_ready_ttl
suspend_after_idle
destroy_after_suspended
```

Pressure action order:

```text
stop refill
stop idle browser/db sidecars
evict warm entries
fstrim idle disks
suspend idle agent computers
destroy cold prepared state
refuse new work
```

Active work must not be evicted for pool comfort.

## Evidence Surface

Raw product/autoscale trace events should include:

```text
AgentComputerRequestStart
AgentComputerSandboxCreated
AgentComputerProbeStart
ReadyQueueHit
ReadyQueueMiss
CliFirstUsefulStdout
BrowserReady
DatabaseReady
AgentComputerReady
AgentComputerSuspended
AgentComputerResumed
PressureDetected
SafeFloorRestored
ReadyTargetRestored
AutoscaleDecisionMade
AutoscaleActionStarted
AutoscaleActionDone
```

`AgentComputerRequestStart -> AgentComputerReady` remains the headline product
latency. `AgentComputerRequestStart -> AgentComputerSandboxCreated` and
`AgentComputerProbeStart -> AgentComputerReady` are drilldowns that separate
control-plane create/followup overhead from product readiness probe overhead.

Artifacts should include:

- raw event traces
- product probe outcomes
- policy snapshot
- machine fingerprint
- pressure samples
- demand samples
- controller decisions
- actuator before/after samples
- metric summaries
- classified failures

`firkin-evidence` derives and validates `product.*`, `autoscale.*`, `demand.*`,
`capacity.*`, and `pressure.*`. `firkin-benchmark` runs `agent-computer` and
`autoscale` suites and writes JSON plus optional HTML proof.

## Crate Ownership

Proposed new crate graph:

```text
firkin-agent-computer
  -> firkin-sandbox
  -> firkin-admission
  -> firkin-trace
  -> firkin-types

firkin-agent-server
  -> firkin-agent-computer
  -> firkin-sandbox
  -> firkin-types

firkin-benchmark
  -> firkin-agent-computer
  -> firkin-single-node
  -> firkin-evidence
```

Suggested `firkin-agent-computer` modules:

```text
ids.rs
error.rs
profile.rs
browser.rs
database.rs
cli.rs
readiness.rs
ready_queue.rs
lifecycle.rs
policy.rs
event.rs
runtime.rs
contract.rs
```

Do not add modules named `common`, `shared`, `utils`, `helpers`, or `models`.

## Error Design

Typed errors should include:

- `AgentComputerReadinessFailure`
- `SidecarFailure`
- `AutoscaleAdmissionFailure`
- `PressureRefusal`
- `CapacityRejected`
- `UnsupportedCapability`

Every failure should carry:

- operation
- affected resource or state
- failure class
- retry class
- source error when there is one

`unknown` is allowed only as an explicit classified failure bucket, and any
unknown rate above zero blocks readiness claims.

## Milestones

1. Finish metrics-first: strict decision-grade shell/core metrics, raw event
   traces, sample floors, and no legacy aliases.
2. Promote the product-pod ready-deck scorecard from smoke to decision-grade
   sample counts for browser/db/CLI readiness and `product.agent_computer_*`
   derivation.
3. Land `firkin-agent-computer` API with fake-backend contract tests.
4. Wire Apple/VZ single-node product readiness using existing sandbox process,
   filesystem, and port surfaces.
5. Add `fk agent-computer` CLI and native server route skeleton.
6. Add pure autoscale planner in `firkin-admission` with host-only scenario
   tests.
7. Wire pressure sampler and ready-queue actuators in the product layer.
8. Add signed-live agent-computer and autoscale scenario artifacts.
9. Optimize to snappy only after product/autoscale evidence is decision-grade.

The signed-live agent-computer scorecard now uses the product-pod ready-deck
path for product ready/resume and drilldown samples. The smoke artifact
`target/firkin-live-evidence/local-agent-computer-product-pod-narrow-start-gate-20260508.json`
validates with `promotion_blockers=0` under
`validate-agent-computer-scorecard --require-promotable`, and carries
`cli_boundary=real_cli`, `browser_boundary=real_browser_sidecar`, and
`database_boundary=real_db_sidecar`. Its representative superfast-iteration
run reports product ready max `55.78ms`, explicit resume max `67.86ms`, and
product density breakpoint max `4` at count `3`. This is the promotable shape
and first density target in smoke evidence, not a decision-grade sample-count
claim.

Legacy proxy artifacts remain useful pressure telemetry only. They tag DB
readiness as `measurement_boundary=sqlite_proxy_not_db_sidecar`,
`cli_boundary=code_interpreter_exec`,
`browser_boundary=code_interpreter_health`, and
`database_boundary=sqlite_proxy_not_db_sidecar`; they must not be treated as
final database-sidecar readiness or promotable product ready/resume evidence.
