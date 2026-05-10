# Firkin Dummy-Fast SLAs

Status: target SLA board for the snappy-first optimization program.

This document is the target surface for Firkin performance work after the
decision-grade metric contract has made the numbers trustworthy enough to guide
optimization. Keep one tight public scorecard. Everything else is supporting
telemetry used to explain why a scorecard metric moved.

The metric meanings, start events, end events, lifecycle labels, workload
labels, and sample-confidence floors live in
`docs/specs/firkin-decision-grade-metric-contract.md`. This document describes
where those metrics need to land.

## Operator Quick Read

For the representative local product-path baseline:

```bash
FIRKIN_BASELINE_NAME=local-agent-computer-snappy-smoke-$(date +%Y%m%d) \
FIRKIN_BASELINE_DURATION=60s \
scripts/run-firkin-decision-baseline.sh agent-computer
```

Open these first:

```text
target/firkin-live-evidence/<baseline>.agent-computer-scorecard.txt
target/firkin-live-evidence/<baseline>.decision.txt
target/firkin-live-evidence/<baseline>.product-pod-artifacts.txt
target/firkin-live-evidence/<baseline>.product-pod-ready-deck-proof.html
target/firkin-live-evidence/<baseline>.autoscale-scorecard.txt
target/firkin-live-evidence/<baseline>.autoscale-scorecard-promotable.txt
```

The pass/fail rows to inspect are product ready/resume, shell density, full
agent-computer density, prestarted-slot density, autoscale ready-hit/safe-spare
and pressure recovery, disk bloat after trim, cleanup leftovers, unknown
failures, and every sample tier. A count `3` result is a useful
`superfast_iteration` signal, not an SLA claim.

## Product North Star

Firkin's product is not a fast process runner. The product is a fleet of real
agent computers:

```text
agent computer =
  VM isolation envelope
  + browser control surface
  + database service
  + CLI / shell
  + workspace
  + network policy
  + cleanup and reclaim path
```

The optimization question is:

```text
how many useful little computers can this host keep ready,
how quickly can one become usable,
how cheaply can it idle,
and how cleanly can it disappear under pressure?
```

The target behavior is a pressure-adaptive ready queue. Keep useful agent
computers on deck when the host has room. When disk, memory, CPU, thermal, or
power pressure appears, they get out of the way in priority order. When pressure
clears or demand returns, they come back without making the user wait on cold
setup. The system should feel light at idle and already-there on demand.

Default product loop:

```text
browser + database + CLI
```

So the product-ready metric is broader than first stdout:

```text
product.agent_computer_ready_ms =
  external request
  -> CLI first useful stdout
  AND browser control endpoint ready
  AND database healthcheck ready
  AND workspace mounted
  AND network policy applied
```

Shell-only startup remains critical because agents run many commands, but it is
not sufficient for product readiness. The public scorecard keeps
`start.hot_to_first_stdout_ms`, `start.resume_to_first_stdout_ms`, and
`exec.first_stdout_byte_ms`, and now also includes
`product.agent_computer_ready_ms` from the real product-pod ready-deck path:
CLI through the product pod, real browser sidecar, real database sidecar,
workspace mount, and network policy readiness.

## Measurement Rules

All targets below are p95 unless a row says otherwise.

Fast-path p95 claims require at least `n >= 100`. Fast-path p99 is tracked in
reports, but it is not enforced until a metric has `n >= 500-1000`. Smaller
sample tiers are useful for iteration, but they cannot justify SLA claims:

| Sample tier | Minimum samples | Use |
| --- | ---: | --- |
| `smoke_only` | 1 | prove the harness reached the path |
| `superfast_iteration` | 3 | fastest local check after a small edit |
| `fast_iteration` | 5 | quick local comparison before longer loops |
| `p50_p90_decision_grade` | 30 | p50/p90 development decisions |
| `p95_decision_grade` | 100 for fast paths, 30 for slower/resource paths | p95 optimization decisions |
| `p99_decision_grade` | 500-1000 | p99 claims and release-quality tail comparisons |

The first optimization objective is snappiness. Optimize the user-visible
product path first, then density and resource adaptation, then slower guardrail
paths.

## Public Scorecard

This is the focused board. It is intentionally small enough to keep on an
internal dashboard.

| Area | Metric | Dummy-fast target | Stretch / absurd target | Current directional read |
| --- | --- | ---: | ---: | ---: |
| hot start | `start.hot_to_first_stdout_ms` | `<75ms` | `<50ms` | APFS shared-volume baseline `target/firkin-live-evidence/local-agent-core-direct-exec-diagnostic-final-1-2-4-8-20260508.json` reports p95 `97.57ms`, count `15`; storage-isolated ramdisk rerun `target/firkin-live-evidence/ramdisk/local-agent-core-direct-exec-ramdisk-state-1-2-4-8-20260508.json` reports p50 `42.03ms`, p95 `50.73ms`, count `15`, so hot start is snappy in fast-iteration evidence when state, benchmark, and artifact IO are isolated |
| snapshot resume | `start.resume_to_first_stdout_ms` | `<35ms` | `<25ms` | final baseline has canonical smoke sample `237.76ms`, count `1`, and legacy `sandbox.start.resume_snapshot_to_first_stdout_ms` p95 `265.47ms`, count `18`; this needs a clean resume-only rerun because older smoke read was around `42ms` |
| warm start | `start.warm_to_first_stdout_ms` | `<350ms` | `<200ms` | local baseline p50 `248.11ms`, p95 unstable at `938.01ms`, count `6`; old warm-ready read was around `694ms` |
| agent computer ready | `product.agent_computer_ready_ms` | `<250ms` | `<150ms` | narrow-start-gate product-pod scorecard `target/firkin-live-evidence/local-agent-computer-product-pod-narrow-start-gate-20260508.json` reports max `55.78ms`, `promotion_blockers=0`, count `3`, confidence `superfast_iteration`; decision-grade sample count still open |
| agent computer resume | `product.agent_computer_resume_ms` | `<75ms` | `<35ms` | narrow-start-gate product-pod scorecard reports explicit resume max `67.86ms`, `promotion_blockers=0`, count `3`, confidence `superfast_iteration`; older ready-deck p95-grade artifact `target/firkin-live-evidence/local-product-pod-ready-deck-sh-c-repeats100-phases.json` reported p95 `74.05ms`, count `100` |
| cold prepared | `start.cold_prepared_to_first_stdout_ms` | `<1s` | `<500ms` | not cleanly isolated yet |
| cold unprepared | `start.cold_unprepared_to_ready_ms` | `<10s` guardrail only | `<5s` | old smoke read was around `16.6s` |
| command start | `exec.command_start_ms` | `<15-20ms` | `<8-10ms` | latest first-stdout-split ramdisk rerun `target/firkin-live-evidence/ramdisk/local-agent-core-envd-first-stdout-split-20260508.json` reports aggregate p95 `47.67ms`, sandbox-first-command p95 `47.67ms`, direct envd health RTT `0.47ms`, proxied envd health RTT `0.36ms`, raw direct envd first stdout `27.44ms`, raw domain-proxy envd first stdout `28.93ms`, and SDK c1 command `43.47ms`; non-interactive streams no longer allocate an unnecessary stdin pipe, but aggregate command start still misses the `<15-20ms` SLA |
| first stdout | `exec.first_stdout_byte_ms` | `<25ms` | `<15ms` | first-stdout-split ramdisk rerun reports aggregate p95 `49.35ms`, sandbox-first-stdout p95 `49.88ms`, direct/proxied envd health RTT under `0.5ms`, raw direct/proxied envd first stdout about `27-29ms`, and c8 SDK command `67.28ms`; envd transport is isolated as sub-millisecond, raw envd first stdout is a separate ~28ms factor, and aggregate/density first-command pressure still misses |
| batch commands | `exec.batch_100_small_commands_ms` | `<500ms` | `<250ms` | latest null-stdin smoke reports `112.46ms`, count `1`, while the prior streaming smoke reported `9.62ms`, count `1`; both are smoke-only, so retained-shell p50 `11.08ms`, p95 `114.61ms`, count `100` remains the decision-grade retained-loop baseline |
| density | `density.max_active_before_hot_to_first_stdout_p95_doubles` | `>=8` | `>=16` | streaming + null-stdin ramdisk rerun reports breakpoint `8`, c8 SDK command `48.65ms`, hot p95 `44.27ms`, and aggregate first stdout p95 `41.67ms`; shared storage pressure is confirmed as a density variable, while product-path create+command density still needs decision-grade samples |
| product density | `density.max_agent_computers_before_ready_p95_doubles` | `>=4` | `>=8` | narrow-start-gate product-pod scorecard reports breakpoint max `4`, count `3`, confidence `superfast_iteration`; focused proof `target/firkin-live-evidence/local-product-pod-ready-deck-density-narrow-start-gate.json` also reports `4`, so the dummy-fast threshold is met in smoke evidence but decision-grade density remains open |
| prestarted slot density | `density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles` | `>=4` | `>=8` | refreshed smoke artifact `target/firkin-live-evidence/local-product-pod-prestarted-agent-slot-density-after-fast-spawn.json` reports breakpoint `2`, `7` traces, checkout baseline p95 `0.96ms`, threshold `1.92ms`, max checkout acceptance `2.30ms`, and full output max `19.05ms` for levels `1,2,4`; excludes container add/start; slot dispatch now uses FIFO request acceptance and detached emptyDir writes |
| disk bloat | `disk.sparse_bloat_after_trim` | `<1.25x` | `<1.10x` | latest lifecycle smoke `1.1758x`, matching raw pod-store smoke after smaller ext4 inode tables; previous staged raw read was `2.35x`, ASIF smoke was `43.35x` |
| trim reclaim | `disk.trim_reclaim_effectiveness_pct` | `>85%` | `>95%` | planned metric; current artifact has `disk.host_bytes_reclaimed_after_trim` |
| cleanup | `cleanup.leftover_bytes` | `0` | `0` | local baseline `0` |
| reliability | `reliability.unknown_failure_rate` | `0%` | `0%` | local baseline `0%` |

Use the active contract metric names in artifacts. Prose may say "tiny
commands" or "hot p95 doubles", but the artifact names are
`exec.batch_100_small_commands_ms` and
`density.max_active_before_hot_to_first_stdout_p95_doubles`. Do not add alias
metrics for convenience.

## Headline SLA

The headline SLA for snappy agent work is:

```text
hot or resumed sandbox -> first useful stdout:
  p50 < 25ms
  p95 < 75ms
  p99 < 150ms
```

Split the headline SLA into the lifecycle paths that explain it:

```text
start.resume_to_first_stdout_ms:
  p95 < 35ms
  p99 < 75ms

start.hot_to_first_stdout_ms:
  p95 < 75ms
  p99 < 150ms

start.warm_to_first_stdout_ms:
  p95 < 350ms
  p99 < 750ms
```

The product path should also keep an external request metric visible:

```text
start.agent_task_ready_ms:
  p95 < 150ms
  p99 < 300ms

product.agent_computer_ready_ms:
  p95 < 250ms
  p99 < 500ms

product.agent_computer_resume_ms:
  p95 < 75ms
  p99 < 150ms
```

`start.agent_task_ready_ms` is the product UX metric: external API request to
first useful stdout. `product.agent_computer_ready_ms` is the broader product
UX metric: external request to browser + database + CLI ready. Phase metrics
explain it, but this is the path users feel.

For agent-computer trace reports, keep the headline metric intact and split
diagnostics underneath it:

```text
create_ms = AgentComputerRequestStart or AgentComputerResumed
         -> AgentComputerSandboxCreated

probe_ms  = AgentComputerProbeStart
         -> AgentComputerReady
```

This prevents a slow product-ready number from being misattributed to browser,
database, or CLI probes when the actual cost is sandbox create/followup.

## Hot Start Phase Budgets

For `start.hot_to_first_stdout_ms <75ms p95`, the phase budget is:

```text
pool lease:                <1ms
health / guest ping:       <5ms
workspace ready check:    <10ms
ready probe / exec probe:  <5ms
exec dispatch:         <10-15ms
process start:         <10-15ms
first stdout wait:     <10-20ms
--------------------------------
total:                    <75ms p95
```

If one phase spends around `40ms`, that phase becomes the optimization target.
The scorecard should not hide the phase breakdown. It should let an operator
answer whether the regression is pool lease, health, workspace readiness, exec
dispatch, process start, or stdout streaming.

## Snapshot Resume Phase Budgets

For `start.resume_to_first_stdout_ms <35ms p95`, the budget is:

```text
snapshot restore / reattach:  <10-15ms
guest agent revalidation:      <3-5ms
workspace / ready probe:       <3-5ms
exec -> first stdout:        <10-15ms
-------------------------------------
total:                        <35ms p95
```

This is the demo path to make obviously fast. If resume is the common path, it
should feel instant: restore, validate that the guest is actually usable, run
the first command, and emit useful stdout.

## Warm Start Phase Budgets

For `start.warm_to_first_stdout_ms <350ms p95`, the budget is:

```text
template lookup / disk attach:  <25-50ms
VZ start -> guest agent:       <200-250ms
network / workspace ready:      <25-50ms
exec -> first stdout:           <25-50ms
----------------------------------------
total:                         <350ms p95
```

Stretch target:

```text
VZ start -> guest agent:       <125-150ms
start.warm_to_first_stdout_ms:     <200ms p95
```

Warm is not the first path users should hit if hot or resume is available, but
it needs to be fast enough that a pool miss does not feel like falling off a
cliff.

## Command Loop Targets

Agents run many tiny commands. The direct exec loop can make the product feel
snappy even when startup is not perfect.

```text
exec.command_start_ms:
  p50 < 5ms
  p95 < 15-20ms
  p99 < 40ms

exec.first_stdout_byte_ms:
  p50 < 10ms
  p95 < 25ms
  p99 < 50ms

debug.exec.direct_first_stdout_byte_ms:
  smoke target < 25ms
  use only to isolate raw exec from SDK or shell overhead

debug.exec.shell_first_stdout_byte_ms:
  smoke target < 25ms when the shell is executed directly in a quiet sandbox
  use only to distinguish shell startup from SDK/envd/concurrency overhead

exec.shell_first_stdout_ms:
  p95 < 60ms

exec.batch_100_small_commands_ms:
  p95 < 500ms
  stretch < 250ms
```

Do not let direct exec, shell exec, and SDK command transport collapse into one
unreadable conclusion. The public scorecard keeps `exec.command_start_ms` and
`exec.first_stdout_byte_ms` for the representative command mix. The diagnostic
metrics `debug.exec.direct_command_start_ms` and
`debug.exec.direct_first_stdout_byte_ms` exist only to answer whether raw VZ
process start is fast enough. The shell diagnostics
`debug.exec.shell_command_start_ms` and
`debug.exec.shell_first_stdout_byte_ms` answer whether `/bin/sh` or
`/bin/bash -l -c` startup is actually expensive. The sandbox-first diagnostics
`debug.exec.sandbox_first_command_start_ms` and
`debug.exec.sandbox_first_stdout_byte_ms` answer whether the aggregate miss is
really the first command in each freshly checked-out sandbox. If sandbox-first
matches aggregate while quiet direct shell passes, optimize first-command
runtime state or concurrent command scheduling before revisiting SDK/envd
transport.

The direct-fresh diagnostic
`target/firkin-live-evidence/ramdisk/local-agent-core-direct-first-ramdisk-1-2-4-8-20260508.json`
adds one more split: direct fresh `/bin/bash -l -c` first stdout was `9.99ms`,
while the SDK-served first command in the same run was `32.62ms` and density
sandbox-first p95 was `75.15ms`. That narrows the remaining command loop work to
the sandbox-scoped/envd-served path under active control-plane/proxy load versus
the direct unsandboxed adapter path.

The raw-envd-proxy-first diagnostic
`target/firkin-live-evidence/ramdisk/local-agent-core-raw-envd-proxy-first-20260508.json`
creates a separate SDK sandbox and sends a raw gRPC-web
`/process.Process/Start` request as that sandbox's first command. The raw
envd-proxy first command was `26.15ms`, SDK c1 command was `25.53ms`, and
direct adapter first stdout was `8.19ms`. That separates SDK client overhead
from Firkin's envd-compatible proxy surface: the SDK wrapper is not the main
overhead in this run. The next command-loop target is the Firkin
domain-proxy/envd HTTP layer and concurrent first-command behavior.

The direct-vs-proxy diagnostic
`target/firkin-live-evidence/ramdisk/local-agent-core-raw-envd-direct-vs-proxy-20260508.json`
adds a direct request to the sandbox's envd HTTP listener before the domain
proxy request. In that storage-isolated smoke, raw direct envd HTTP first
command was `43.72ms`, raw domain-proxy envd first command was `43.20ms`, SDK
c1 command was `44.20ms`, and direct adapter first stdout was `7.83ms`. Treat
those as smoke-only attribution samples, not stable percentiles: the earlier
raw-proxy-only run was `26.15ms`. The useful conclusion is narrower than
"proxy is slow": direct envd HTTP and proxied envd HTTP are the same shape in
the latest run, so the next seam is envd HTTP process serving, finite-output
stream semantics, and first-command runtime pressure before blaming the SDK or
domain proxy hop.

The envd-health diagnostic
`target/firkin-live-evidence/ramdisk/local-agent-core-envd-health-direct-vs-proxy-20260508.json`
isolates envd HTTP round-trip latency from envd process execution. Direct
`GET /health` against the sandbox's envd listener was `0.42ms`, and the same
health check through the domain proxy was `0.50ms`. In the same run, raw direct
envd `/process.Process/Start` was `31.76ms`, raw proxied envd process start was
`30.24ms`, SDK c1 command was `47.52ms`, and direct adapter first stdout was
`7.61ms`. That makes the current factor split:

```text
envd HTTP + optional domain proxy RTT:
  sub-millisecond smoke sample

envd-served process start + finite output response:
  roughly 30ms smoke sample

SDK command wrapper plus command path:
  roughly 48ms smoke sample

fresh direct adapter exec:
  roughly 8ms smoke sample
```

The actionable target is no longer "envd latency" as a generic bucket. It is
the envd process-serving path: request decode, adapter `start_process_stream`,
runtime command start, output collection, stream frame encoding, and concurrent
first-command scheduling.

The streaming-envd optimization
`target/firkin-live-evidence/ramdisk/local-agent-core-envd-streaming-20260508.json`
adds a Firkin runtime streaming command path behind envd `/process.Process/Start`
instead of relying on the trait's finite-output fallback. Compared with
`local-agent-core-envd-health-direct-vs-proxy-20260508`, the storage-isolated
smoke moved:

```text
hot_to_first_stdout p95:
  99.47ms -> 69.07ms

exec.command_start p95:
  84.35ms -> 52.08ms

exec.first_stdout_byte p95:
  91.47ms -> 64.62ms

SDK c1 command:
  47.52ms -> 28.43ms

SDK c8 command:
  100.69ms -> 79.43ms

batch_100_small_commands:
  111.67ms -> 9.62ms
```

This is a smoke-level performance win and a topology win: envd process start no
longer has to wait for complete finite command output before returning the
HTTP stream. It is not final SLA completion. The remaining command-loop miss is
now concentrated in concurrent first-command scheduling and the gap between
raw direct envd first command (`24.99ms`) and aggregate p95 (`64.62ms`).

The follow-up null-stdin optimization
`target/firkin-live-evidence/ramdisk/local-agent-core-envd-streaming-stdin-null-20260508.json`
keeps the streamed envd process path but uses `Stdio::Null` for
non-interactive commands instead of allocating a stdin pipe that no caller will
write. Compared with the first streaming run, the storage-isolated smoke moved:

```text
hot_to_first_stdout p95:
  69.07ms -> 44.27ms

exec.command_start p95:
  52.08ms -> 40.63ms

exec.first_stdout_byte p95:
  64.62ms -> 41.67ms

SDK c8 command:
  79.43ms -> 48.65ms
```

The first-stdout-split diagnostic
`target/firkin-live-evidence/ramdisk/local-agent-core-envd-first-stdout-split-20260508.json`
renames the raw envd probes to measure the first stdout frame instead of
waiting for the complete streaming response body. The run reports direct envd
health RTT `0.47ms`, proxied envd health RTT `0.36ms`, raw direct envd first
stdout `27.44ms`, raw proxied envd first stdout `28.93ms`, aggregate
command-start p95 `47.67ms`, and aggregate first-stdout p95 `49.35ms`.

The envd factor split is now explicit enough for optimization:

```text
direct/proxied envd health RTT:
  about 0.5ms smoke sample

raw direct/proxied envd first stdout:
  about 27-29ms smoke sample

full aggregate command p95:
  about 47-49ms fast-iteration sample
```

So "envd latency" should not be treated as one bucket. Transport/proxy RTT is
already sub-millisecond. Raw envd process serving is a distinct tens-of-ms
factor. The product-path miss above that is adapter, shell, concurrency, and
scorecard mix. Isolating envd internals further requires guest/envd-side
timestamps, not just host-side probes.

## Product UX Targets

The product should be judged by agent computers, not by shell-only sandboxes.
The default loop is browser + database + CLI. It needs to spin up bursts, tear
down quickly, and adapt its footprint to the host.

```text
single hot/resumed agent computer:
  product.agent_computer_ready_ms p95 < 250ms
  CLI first useful stdout p95 < 75ms
  browser ready p95 < 150ms
  database ready p95 < 150ms

create burst of 8 hot/resumed CLI sandboxes:
  first useful stdout p95 < 250ms
  no unknown failures
  no cleanup leftovers

create burst of 4 full agent computers:
  product.agent_computer_ready_ms p95 < 500ms
  no unknown failures
  no cleanup leftovers

destroy 8 active sandboxes:
  cleanup p95 < 500ms
  capacity released to the scheduler immediately
  cleanup.leftover_bytes = 0

destroy 4 full agent computers:
  product.agent_computer_destroy_ms p95 < 750ms
  browser/db/CLI processes classified on exit
  capacity released to the scheduler immediately
  cleanup.leftover_bytes = 0

burst create followed by burst destroy:
  no active-session ledger leaks
  no stuck VM markers
  no unknown failures
  no host free-space drop after fstrim/destroy settles

idle shrink:
  idle memory reclaimed within 5s
  disk fstrim queued within 30s
  warm pool never holds the host below reserved free-space floors
  browser and database sidecars stop before VM envelope is destroyed

demand rise:
  hot pool consumed first
  resumed snapshots used second
  warm/prepared state created third
  cold unprepared work only runs as background replenishment
```

The UX target is "fast enough that users do not watch infrastructure." A miss
in one internal phase should degrade predictably: use another lifecycle class,
refuse new warm entries, or return a classified capacity error. It should not
turn into an unknown timeout.

## Dynamic Capacity Policy

Firkin should opportunistically use free host capacity while preserving host
safety. The scheduler should treat active user work as more important than warm
pool comfort.

The runtime should behave like a permanent job queue for agent computers:

```text
ready queue:
  keeps hot/resumed agent computers on deck while capacity is free

pressure controller:
  shrinks idle capacity when the host needs resources back

refill controller:
  rebuilds the ready queue when pressure clears or demand rises

admission controller:
  gives active jobs priority and refuses new work before harming active work
```

Recommended floor policy:

```text
reserved_free_disk = max(20GiB, 10% host disk)
reserved_free_memory = max(8GiB, 15% host memory)
```

Recommended warm-pool target:

```text
warm_pool_target =
  min(configured_max, capacity_available_after_reserves)
```

Recommended agent-computer ready target:

```text
agent_computer_ready_target =
  min(configured_max_agent_computers, capacity_available_after_reserves)
```

When shrinking:

```text
1. stop refilling warm pool
2. stop idle browser/database sidecars
3. evict idle warm entries
4. fstrim idle disks
5. suspend or snapshot idle agent computers
6. destroy coldest prepared state
7. refuse new work with classified capacity errors
8. never kill active sessions for pool comfort
```

When growing:

```text
1. consume hot pool entries
2. resume existing snapshots
3. restart browser/database sidecars for ready agent computers
4. create warm/prepared entries in the background
5. build cold templates only when reserves still hold
```

Capacity metrics should eventually include:

```text
capacity.active_sessions
capacity.ready_agent_computers
capacity.hot_pool_entries
capacity.warm_pool_entries
capacity.reserved_free_disk_bytes
capacity.available_after_reserves_bytes
capacity.pressure_state
capacity.pool_shrink_latency_ms
capacity.pool_refill_latency_ms
capacity.agent_computer_suspend_latency_ms
capacity.agent_computer_resume_latency_ms
capacity.capacity_refusal_rate
```

These are supporting telemetry. They should not crowd the public scorecard
unless they become user-visible bottlenecks.

## Autoscale Efficiency Metrics

Autoscale optimization needs its own measurement board before controller work
starts. The goal is not simply to keep more warm machines around. The goal is:

```text
serve demand from already-ready agent computers
use most of the safe spare host capacity
shrink quickly under pressure
refill quickly when pressure clears
never steal capacity from active work
never hide cleanup, bloat, or failure debt
```

### Autoscale Scorecard

These metrics are the first-class autoscale board. They should be reported for
the default `browser + database + CLI` product loop and, separately, for the
shell-only fast path.

| Area | Metric | Dummy-fast target | Stretch target | Meaning |
| --- | --- | ---: | ---: | --- |
| ready hit rate | `autoscale.ready_queue_hit_rate_pct` | `>90%` | `>98%` | percent of requests served from hot/resumed ready capacity instead of warm/cold creation |
| ready latency | `product.agent_computer_ready_ms` | `<250ms` | `<150ms` | external request to browser + database + CLI ready |
| resume latency | `product.agent_computer_resume_ms` | `<75ms` | `<35ms` | pressure-suspended agent computer back to product ready |
| safe spare use | `autoscale.safe_spare_limiting_utilization_pct` | `>70%` | `>85%` | limiting-resource percent of host capacity safely usable after reserves that is occupied by active + ready work |
| shrink reaction | `autoscale.pressure_to_safe_floor_ms` | `<5s` | `<2s` | time from pressure detection to all configured reserve floors satisfied |
| refill reaction | `autoscale.pressure_clear_to_ready_target_ms` | `<10s` | `<3s` | time from pressure clear or demand rise to ready queue target restored |
| density | `density.max_agent_computers_before_ready_p95_doubles` | `>=4` | `>=8` | full browser + database + CLI agent computers before ready p95 doubles |
| prestarted slot density | `density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles` | `>=4` | `>=8` | already-running browser + database + CLI slots before checkout-ready p95 doubles; does not replace add/start density |
| prestarted slot FIFO | `density.prestarted_agent_slot_fifo_acceptance_p95_ms` | `<5ms` | `<2ms` | p95 request-acceptance latency for FIFO dispatch into already-running prestarted agent slots |
| active protection | `autoscale.active_evictions_due_to_pool_pressure` | `0` | `0` | active sessions killed or suspended only to protect pool comfort |
| floor safety | `autoscale.reserve_floor_violations` | `0` | `0` | times the controller drove disk/memory below configured reserves |
| cleanup | `cleanup.leftover_bytes` | `0` | `0` | Firkin-owned bytes left after destroy/reclaim |
| reliability | `reliability.unknown_failure_rate` | `0%` | `0%` | failures not classified into known buckets |

`autoscale.safe_spare_utilization_pct` should be calculated per resource and as
the limiting resource:

```text
safe_spare_resource =
  total_resource
  - active_user_resource
  - reserved_floor_resource

ready_queue_resource =
  hot_agent_computer_resource
  + resumed_agent_computer_resource
  + warm_pool_resource

autoscale.safe_spare_utilization_pct =
  ready_queue_resource / safe_spare_resource * 100
```

Report at least:

```text
autoscale.safe_spare_cpu_utilization_pct
autoscale.safe_spare_memory_utilization_pct
autoscale.safe_spare_disk_utilization_pct
autoscale.safe_spare_limiting_utilization_pct
```

The limiting utilization is the scorecard row. The per-resource rows explain
whether CPU, memory, or disk is blocking scale.

### Demand Metrics

Demand metrics explain what the autoscaler was trying to satisfy.

```text
demand.request_arrival_rate_per_s
demand.burst_size
demand.pending_agent_computers
demand.pending_cli_sandboxes
demand.queue_wait_ms
demand.queue_timeout_rate_pct
demand.cancelled_while_queued_rate_pct
demand.priority_class
```

Required labels:

```text
workload = cli_only | agent_computer
profile = fast_agent | browser_db_cli
priority = interactive | background | refill
queue_policy
```

The autoscaler cannot be evaluated without demand. A low ready queue is good
under no demand and bad during a burst.

### Supply Metrics

Supply metrics describe what capacity exists and how much of it is usable.

```text
capacity.active_agent_computers
capacity.ready_agent_computers
capacity.hot_agent_computers
capacity.resumed_agent_computers
capacity.warm_pool_entries
capacity.prepared_templates
capacity.cold_templates_available
capacity.ready_target
capacity.ready_deficit
capacity.ready_surplus
capacity.active_sessions
capacity.pending_active_sessions
capacity.max_configured_agent_computers
```

Required labels:

```text
template_key
runtime_profile
trust_class
resource_class
```

`capacity.ready_deficit` is the fastest operator signal that demand will soon
fall off the hot/resume path.

### Pressure Metrics

Pressure metrics explain why the controller shrank, refused, or delayed work.

```text
pressure.state
pressure.reason
pressure.disk_available_bytes
pressure.disk_reserved_floor_bytes
pressure.memory_available_bytes
pressure.memory_reserved_floor_bytes
pressure.cpu_load_pct
pressure.cpu_pressure_pct
pressure.io_pressure_pct
pressure.thermal_state
pressure.power_state
pressure.low_power_mode
pressure.sample_age_ms
```

Recommended pressure state values:

```text
nominal
watch
shrink
critical
recovery
```

Recommended pressure reasons:

```text
disk_floor
memory_floor
cpu_pressure
io_pressure
thermal
power
demand_spike
manual_limit
```

Do not collapse pressure into a boolean. The controller needs to know whether
it should fstrim, drop idle sidecars, stop refilling, reduce CPU pressure, or
refuse new work.

### Controller Decision Metrics

Controller metrics explain the actual autoscale decisions. They are necessary
for debugging thrash and underfill.

```text
autoscale.tick_ms
autoscale.decision_latency_ms
autoscale.decision
autoscale.decision_reason
autoscale.target_ready_agent_computers
autoscale.previous_ready_agent_computers
autoscale.target_delta
autoscale.hysteresis_hold_ms
autoscale.cooldown_remaining_ms
autoscale.last_pressure_transition_age_ms
```

Recommended decisions:

```text
hold
refill_hot
resume_snapshot
start_warm
stop_refill
stop_sidecars
evict_warm
trim_idle_disk
suspend_idle_agent_computer
destroy_prepared
refuse_new_work
```

Each decision sample must include `pressure.state`, `pressure.reason`, demand
queue depth, and ready target. Otherwise a bad autoscale decision cannot be
distinguished from a correct response to pressure.

### Actuator Metrics

Actuator metrics measure whether the controller's chosen operation actually
moved the system.

```text
autoscale.stop_sidecars_ms
autoscale.restart_sidecars_ms
autoscale.evict_warm_entry_ms
autoscale.fstrim_idle_disk_ms
autoscale.suspend_agent_computer_ms
autoscale.resume_agent_computer_ms
autoscale.destroy_prepared_state_ms
autoscale.refill_hot_agent_computer_ms
autoscale.refill_warm_entry_ms
autoscale.capacity_release_ms
```

Each actuator needs before/after resource samples:

```text
memory_available_before_bytes
memory_available_after_bytes
disk_available_before_bytes
disk_available_after_bytes
cpu_pressure_before_pct
cpu_pressure_after_pct
ready_agent_computers_before
ready_agent_computers_after
```

This prevents fake shrink wins where an operation ran quickly but did not
return usable capacity.

### Efficiency And Waste Metrics

Autoscale should be efficient, not just aggressive.

```text
autoscale.idle_ready_agent_computer_ms
autoscale.ready_agent_computer_used_before_eviction_pct
autoscale.unused_ready_eviction_rate_pct
autoscale.overfill_pct
autoscale.underfill_pct
autoscale.thrash_rate_per_min
autoscale.target_oscillation_count
autoscale.refill_cancelled_due_to_pressure_rate_pct
resource.idle_agent_computer_memory_bytes
resource.idle_agent_computer_disk_allocated_bytes
resource.idle_agent_computer_cpu_pct
resource.idle_agent_computer_wakeup_rate_hz
resource.ready_queue_memory_bytes
resource.ready_queue_disk_allocated_bytes
```

Definitions:

```text
overfill =
  ready_agent_computers > target_ready_agent_computers

underfill =
  ready_agent_computers < target_ready_agent_computers

thrash =
  shrink/refill/shrink for the same resource within the hysteresis window
```

Targets:

```text
autoscale.thrash_rate_per_min = 0 during steady-state runs
autoscale.target_oscillation_count = 0 during steady-state runs
autoscale.unused_ready_eviction_rate_pct < 20%
autoscale.underfill_pct < 5% during demand bursts
autoscale.overfill_pct < 5% after pressure settles
```

### Correctness And Safety Metrics

These are hard gates. They are not optimization targets.

```text
autoscale.active_evictions_due_to_pool_pressure = 0
autoscale.reserve_floor_violations = 0
autoscale.unknown_decision_reason_rate = 0%
autoscale.unclassified_pressure_rate = 0%
cleanup.leftover_bytes = 0
reliability.unknown_failure_rate = 0%
```

If any of these fail, the autoscaler is not production-ready regardless of
latency.

### Autoscale Validation Scenarios

Autoscale metrics should be proven by scenario, not only by passive observation.

```text
steady_idle:
  demand = 0
  pressure = nominal
  expected: ready queue holds target, no thrash, safe spare utilization near policy target

interactive_burst:
  demand jumps from 0 to N agent computers
  expected: high ready hit rate, bounded queue wait, no unknown failures

pressure_shrink:
  disk or memory pressure crosses shrink threshold
  expected: refill stops, idle capacity shrinks, reserve floors restored within SLA

pressure_recovery:
  pressure clears
  expected: ready target restored within SLA without overshoot/thrash

mixed_active_and_idle:
  active work continues while idle ready entries exist
  expected: idle entries shrink first, active sessions are not evicted

churn:
  repeated create/destroy/refill/shrink for 30-60 minutes
  expected: no capacity leaks, no cleanup leftovers, no increasing sparse bloat
```

Each scenario should emit one compact JSON artifact with:

```text
scenario
policy
machine
pressure_samples
demand_samples
controller_decisions
actuator_results
scorecard_summaries
failure_classification
```

That artifact is what makes autoscale optimization decision-grade.

An autoscale scorecard artifact has three separate gates:

```text
valid:
  required metric presence, shape, and sample count

promotable:
  `promotion_blockers=0`; no proxy or unit-only coverage remains

snappy:
  `snappy_target_status=pass`; p95 targets on the dummy-fast board pass
```

`validate-autoscale-scorecard` proves required metric presence, shape, and
sample count. It must also print `promotion_blockers=0` before any autoscale
SLA is enforceable. A blocker on `unit_validated_only` or proxy coverage means
the metric is useful for harness development, but not yet optimization truth.
CI or release checks that need enforceable evidence should run
`validate-autoscale-scorecard --require-promotable --require-snappy` so both
trust blockers and target misses become hard failures. Use
`--require-promotable` without `--require-snappy` when validating measurement
truth before the optimization pass has actually made the numbers fast.

Current signed-live autoscale smoke artifact:
`target/firkin-live-evidence/local-autoscale-scorecard-pressure-promotes-superfast.json`.
It structurally validates and passes `--require-promotable` with
`promotion_blockers=0`: product ready, product resume, ready-hit-rate,
safe-spare utilization, pressure shrink/refill, product add/start density,
prestarted-slot checkout density, cleanup, unknown-failure, active-eviction,
and reserve-floor rows are present with signed-live or signed-live scoped
product-path boundaries where applicable.

This clears the metric-truth gate for autoscale scorecard shape, not the
decision-grade sample gate. The current autoscale rows are `count=3`
`superfast_iteration` evidence. The pressure rows come from a scoped host
disk-pressure scenario: an elevated runtime disk floor triggers
`PressureDetected`, a normal reserve-floor probe gates
`SafeFloorRestored`, and a ready-queue hit probe gates `ReadyTargetRestored`.
Before enforcing p95 pressure SLAs, rerun this path at the required sample
floor and add a production-controller scenario if controller behavior changes.

The release baseline helper captures these autoscale facts when run in
`agent-computer` mode:

```bash
FIRKIN_BASELINE_NAME=local-agent-computer-product-pod-narrow-start-gate-20260508 \
FIRKIN_BASELINE_DURATION=60s \
scripts/run-firkin-decision-baseline.sh agent-computer
```

It writes:

```text
<baseline>.autoscale-scorecard.json
<baseline>.autoscale-scorecard.txt
<baseline>.autoscale-scorecard-validate.txt
<baseline>.autoscale-scorecard-promotable.txt
```

The promotability file is intentionally non-fatal for the baseline loop so
known blockers are preserved as inspectable evidence while product-path
benchmarks continue to run. Enforcement still requires
`validate-autoscale-scorecard --require-promotable` to exit successfully.

By default the helper runs the autoscale proof with
`FIRKIN_BASELINE_AUTOSCALE_REPEATS=3` and validates it with
`--min-samples 3`, which is the `superfast_iteration` floor. For p95-grade
autoscale evidence, run with both `FIRKIN_BASELINE_AUTOSCALE_REPEATS` and
`FIRKIN_BASELINE_AUTOSCALE_MIN_SAMPLES` set to the required sample floor.

Density coverage is deliberately explicit in the same helper so a cheap
iteration run cannot be mistaken for an 8-way density proof:

```text
FIRKIN_BASELINE_SHELL_DENSITY_LEVELS=1,2
FIRKIN_BASELINE_PRODUCT_POD_READY_DECK_DENSITY_LEVELS=1,2,4
FIRKIN_BASELINE_PRODUCT_POD_PRESTARTED_AGENT_SLOT_DENSITY_LEVELS=1,2,4
```

Use `1,2,4,8` for the relevant variable when validating the snappy density
target. The harness records the tested `concurrency_levels`, `baseline_p95_ms`,
and `threshold_p95_ms` on the density sample so the report shows whether the
run actually attempted the target tier.

## Density Targets

Density is the graduation path from a demo to agent infrastructure.

```text
density.max_active_before_hot_to_first_stdout_p95_doubles:
  next target: 4
  dummy-fast: 8
  stretch: 16

density.max_agent_computers_before_ready_p95_doubles:
  next target: 4
  dummy-fast: 4
  stretch: 8

density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles:
  next target: 4
  dummy-fast: 4
  stretch: 8
```

Also track degradation curves:

```text
p95 degradation at 4 active sandboxes:  <1.25x
p95 degradation at 8 active sandboxes:  <1.5x
p95 degradation at 16 active sandboxes: <2.0x
```

The density metric must always say which workload doubled. The active P0 metric
is specifically hot-to-first-stdout p95. Do not average hot, warm, resumed, and
cold paths together.

Full agent-computer density is stricter than shell-only density because each
unit includes browser + database + CLI. Report it separately so shell wins do
not hide product-path contention.

Current reliable smoke evidence for the full add/start path:

- Focused proof:
  `target/firkin-live-evidence/local-product-pod-ready-deck-density-narrow-start-gate.json`.
  It reports breakpoint `4` for levels `1,2,4`, baseline p95 `55.44ms`,
  threshold p95 `110.89ms`, and max trace `107.15ms`.
- Representative scorecard:
  `target/firkin-live-evidence/local-agent-computer-product-pod-narrow-start-gate-20260508.json`.
  It reports breakpoint max `4`, count `3`, and confidence
  `superfast_iteration`.

These remain low-count evidence, not decision-grade density. An ungated
prepared guest-path start previously produced a faster breakpoint `4` smoke
artifact, but the representative baseline exposed a repeatable vminitd
concurrent-start failure. The implementation now reserves pod container IDs
under the pod lock, prepares the container rootfs outside the lock, serializes
only the fragile prepared guest-path `startProcess` RPC behind the per-pod
start gate, retries transient vmexec ENOENT from that exact operation, and
commits the started container back under the pod lock. The next optimization
target is promoting breakpoint `4` to decision-grade sample counts, then
stretching to breakpoint `8`.

Prestarted slot density is a different autoscale metric. It measures already
running agent slots that are kept on deck and checked out under load. It must
carry `measurement_boundary=prestarted_slot_checkout`,
`slot_surface=prestarted_agent_slot`, `excludes_container_add=true`,
`ready_signal=request_fifo_acceptance`, and `output_wait_preserved=true`, and it
must not replace `density.max_agent_computers_before_ready_p95_doubles`, which
continues to measure add/start contention.

Current smoke evidence is
`target/firkin-live-evidence/local-product-pod-prestarted-agent-slot-density-after-fast-spawn.json`.
It reports breakpoint `2` for levels `1,2,4`. The absolute numbers are snappy:
single-slot checkout acceptance p95 `0.96ms`, level-4 max checkout acceptance
`2.30ms`, and full output max `19.05ms`. The remaining miss is the doubled-p95
density rule on a sub-millisecond baseline, not container add/start. The next
optimization target is reducing concurrent FIFO dispatch overhead enough for
level 4 to stay below the doubled baseline.

## Disk And Cleanup Targets

Disk bloat is urgent because a fast sandbox that leaves massive sparse-image
bloat is not fast over time.

Every representative baseline should also record host storage topology. If the
repo, runtime state root, benchmark artifact root, pod-store images, and
snapshot staging all live on the same APFS volume, density can be limited by
shared storage contention rather than only CPU, memory, or VZ behavior. The
baseline script emits `<baseline>.storage.txt` with filesystem, mount, total
bytes, available bytes, and same-volume booleans so the density report can be
read with that context.

For storage-isolated attribution, run the ramdisk wrapper:

```bash
FIRKIN_RAMDISK_PREFLIGHT_ONLY=1 \
FIRKIN_RAMDISK_SIZE_GIB=40 \
FIRKIN_BASELINE_DURATION=60s \
FIRKIN_BASELINE_SHELL_DENSITY_LEVELS=1,2,4,8 \
scripts/run-firkin-ramdisk-decision-baseline.sh agent-core

FIRKIN_RAMDISK_SIZE_GIB=40 \
FIRKIN_BASELINE_DURATION=60s \
FIRKIN_BASELINE_SHELL_DENSITY_LEVELS=1,2,4,8 \
scripts/run-firkin-ramdisk-decision-baseline.sh agent-core
```

The preflight line is side-effect-free and reports the current state size,
benchmark disk floor, live working-set headroom, recommended RAM-disk size, and
whether the requested RAM disk is obviously too small. The wrapper copies the
current Firkin state to an APFS ramdisk, co-locates state, benchmark roots, and
artifacts there, delegates to the normal decision baseline script, and copies
evidence back to `target/firkin-live-evidence/ramdisk`. Use it as an
attribution baseline, not a replacement for the host-volume SLA. The normal SLA
still has to pass on realistic storage.

Snapshot restore rootfs staging records `rootfs_stage_method` as `clone`,
`copy`, or `rebuild`. A cross-device restore must report `copy`; same-device
APFS restore may report `clone`. Cross-device copy fallback is expected when
state is moved to a ramdisk while older snapshot source paths still point at
the host volume. It should be visible in restore timing artifacts without
polluting benchmark logs.

```text
disk.sparse_bloat_after_trim:
  hard fail: > 1.5x
  dummy-fast: < 1.25x
  stretch: < 1.10x

disk.trim_reclaim_effectiveness_pct:
  dummy-fast: > 85%
  stretch: > 95%

disk.trim_duration_ms:
  p95 < 500ms
  p99 < 1s

cleanup.leftover_bytes:
  exactly 0
```

The staged disk report should preserve:

```text
host_allocated_before_task
guest_used_before_task
host_allocated_after_task
guest_used_after_task
host_allocated_after_delete_inside_guest
guest_used_after_delete_inside_guest
host_allocated_after_fstrim
guest_used_after_fstrim
host_allocated_after_destroy
cleanup_leftover_bytes
```

Derived disk metrics:

```text
disk.sparse_bloat_after_task
disk.sparse_bloat_after_delete
disk.sparse_bloat_after_trim
disk.sparse_bloat_after_destroy
disk.host_bytes_reclaimed_after_trim
disk.trim_reclaim_effectiveness_pct
disk.trim_duration_ms
cleanup.leftover_bytes
```

`disk.sparse_bloat_after_trim` and `cleanup.leftover_bytes` stay in the public
scorecard. The intermediate stages explain which storage phase regressed.

## Disk Performance Guardrails

These are guardrails, not first-line targets unless real repo/package-manager
workloads show they are the bottleneck.

```text
disk.fsync_p99_us:
  balanced profile: < 10_000us
  stretch: < 5_000us

metadata_create_stat_unlink:
  p95 < 1.25x Linux block baseline
  hard fail > 1.5x baseline

git_status_ms:
  p95 < 1.25x baseline

package_install_overhead:
  p95 < 1.5x baseline
```

If startup and exec are already under target but real workloads feel slow, use
these guardrails to identify whether the problem is filesystem metadata, fsync,
package-manager behavior, or host pressure.

## Pod Targets

Pods are not the first optimization target for the snappy path, but they matter
when the common product workload becomes `agent + browser + db + tool server`.
For the default browser + database + CLI loop, same-trust pods are the product
shape rather than a later feature.

```text
pod.spawn_sidecar_to_ready_ms:
  p95 < 30ms
  stretch < 15ms

pod.spawn_sidecar_to_first_stdout_ms:
  p95 < 50ms
  stretch < 25ms

pod.localhost_rtt_us:
  p95 < 500us
  stretch < 250us

pod.memory_savings_vs_vm_per_container:
  > 40% for 3-process pod
  stretch > 60%

pod.sidecar_crash_classification:
  100% classified

pod.cross_trust_colocation:
  0 by default
```

Pods should never hide trust decisions. Sidecars that share a VM must be part of
a deliberate same-trust placement policy.

Agent-computer pod readiness should include:

```text
product.browser_ready_ms:
  p95 < 150ms
  stretch < 75ms

product.database_ready_ms:
  p95 < 150ms
  stretch < 75ms

product.cli_first_stdout_ms:
  p95 < 75ms
  stretch < 35ms
```

There are now two agent-computer measurement classes:

- The live `agent-computer` scorecard now uses the product-pod ready-deck path
  for `product.agent_computer_ready_ms`, `product.agent_computer_resume_ms`,
  and the product drilldowns. The representative artifact
  `target/firkin-live-evidence/local-agent-computer-product-pod-narrow-start-gate-20260508.json`
  validates with `promotion_blockers=0` under
  `validate-agent-computer-scorecard --require-promotable`, with ready max
  `55.78ms`, explicit resume max `67.86ms`, and product density breakpoint
  max `4` at count `3`. That proves the promotable shape, real product
  boundaries, and the first density target in superfast-iteration evidence,
  but not a decision-grade sample count.
- Legacy proxy artifacts are useful pressure telemetry only. They tag DB
  readiness as `measurement_boundary=sqlite_proxy_not_db_sidecar` and must carry
  `cli_boundary=code_interpreter_exec`,
  `browser_boundary=code_interpreter_health`, and
  `database_boundary=sqlite_proxy_not_db_sidecar`.
- Product-pod ready-deck artifacts are the canonical product-path proof for
  the snappy loop. They tag `cli_boundary=real_cli`,
  `browser_boundary=real_browser_sidecar`, and
  `database_boundary=real_db_sidecar`; the canonical refresh path is
  `FIRKIN_LIVE_PRODUCT_POD_READY_DECK_REPEATS=100 FIRKIN_LIVE_PRODUCT_POD_READY_DECK_ARTIFACT=target/firkin-live-evidence/local-product-pod-ready-deck-sh-c-repeats100-phases.json scripts/run-signed-live-runtime-test.sh live_runtime_product_pod_ready_deck_writes_real_boundary_resume_sample`, followed by `target/release/fk benchmark report-agent-computer-traces <artifact>`.

Autoscale promotion requires the ready-deck boundaries, so an exact outer event
pair cannot hide proxy CLI, browser, or database probes.
Product add/start density promotion also requires
`excludes_container_add=false` and
`ready_signal=agent_computer_ready_after_container_add`; prestarted slot density
uses the separate checkout predicate with `excludes_container_add=true` and
`ready_signal=request_fifo_acceptance`.

The standalone signed-live DB-sidecar proof writes
`target/firkin-live-evidence/local-db-sidecar-readiness-smoke.json` with
`product.database_ready_ms`, `measurement_boundary=db_sidecar`, and
`database_boundary=real_db_sidecar`. It is intentionally a cold product-pod
sidecar component proof, not the integrated browser + database + CLI scorecard.
The browser-sidecar proof writes
`target/firkin-live-evidence/local-browser-sidecar-readiness-smoke.json` with
`product.browser_ready_ms`, `measurement_boundary=browser_sidecar`, and
`browser_boundary=real_browser_sidecar` under the same cold product-pod caveat.

## Real Repo Workloads

The snappy board starts with `tiny_exec`, but public bragging needs at least one
real workload because users do not only run `true`.

Required real-workload checks before public claims:

```text
repo_git_status:
  p95 < 1.25x baseline

workspace_import_small:
  p95 < 250ms after hot/resume sandbox availability

cargo_build_small:
  p95 < 1.5x baseline

npm_install_small:
  p95 < 1.5x baseline

batch_100_tiny_commands:
  p95 < 500ms
```

These workloads can be supporting telemetry until they block the product path.
If a real workload violates its guardrail, optimize the phase it points to
before chasing smaller hot-start wins.

## Ready To Brag

Internal brag bar:

```text
start.resume_to_first_stdout_ms p95 < 35ms
start.hot_to_first_stdout_ms p95 < 75ms
exec.first_stdout_byte_ms p95 < 25ms
start.warm_to_first_stdout_ms p95 < 350ms
product.agent_computer_ready_ms p95 < 250ms
product.agent_computer_resume_ms p95 < 75ms
density.max_active_before_hot_to_first_stdout_p95_doubles >= 8
density.max_agent_computers_before_ready_p95_doubles >= 4
disk.sparse_bloat_after_trim < 1.25x
cleanup.leftover_bytes = 0
reliability.unknown_failure_rate = 0
```

Public brag bar:

```text
same numbers hold with n >= 100 fast-path runs
same numbers hold across 3 separate benchmark batches
same numbers hold on a clean machine and a loaded machine
same numbers hold for at least one real repo workload
hot path has no hidden image, rootfs, or template preparation work
p99 is tracked and not contradicted by obvious tail instability
all failures are classified
cleanup leftovers remain zero
```

## Simplest Target Set

If the team needs the smallest possible version of the target board, use this:

```text
1. hot/resume -> first stdout: p95 < 75ms
2. direct exec -> first stdout: p95 < 25ms
3. agent computer ready: p95 < 250ms
4. warm -> first stdout: p95 < 350ms
5. batch 100 tiny commands: p95 < 500ms
6. shell density breakpoint: >= 8 active sandboxes
7. agent-computer density breakpoint: >= 4 full environments
8. sparse bloat after trim: < 1.25x
9. cleanup leftover bytes: 0
10. unknown failure rate: 0
```

That is the dummy-fast bar. Everything else is diagnostic, capacity policy, or
future workload coverage.

## First Optimization Order

The first optimization pass should follow this order:

```text
1. make hot/resume -> first useful stdout snappy
2. raise the product-pod ready-deck scorecard from smoke to decision-grade sample counts
3. make product.agent_computer_ready_ms snappy from real browser + database + CLI traces
4. make batch tiny commands snappy
5. make warm misses tolerable
6. fix disk bloat after trim
7. add dynamic pressure shrink/refill policy
8. raise shell density breakpoint to 4, then 8
9. validate full agent-computer density at decision-grade sample counts, then push from 4 to 8
10. validate prestarted slot density at decision-grade sample counts, then push from 4 to 8
11. add real-repo workload guardrails
```

This order keeps the work pointed at product UX instead of optimizing a metric
that is already good enough.
