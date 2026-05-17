# Firkin Decision-Grade Metric Contract

Status: active metric contract.

This table is the human-readable copy of `DECISION_GRADE_METRICS` in
`crates/evidence/src/metric_contract.rs`. The evidence tests parse the metric
column and fail when this document and the code contract diverge.

The SLA targets for these metrics live in `firkin-dummy-fast-slas.md`. Keep
this file focused on measurement truth: what each metric means, where it starts,
where it ends, and when its percentiles are decision-grade.

## Percentile Confidence

Benchmark summaries use named sample-count tiers so iteration output is useful
without overstating percentile truth:

| Tier | Minimum samples | Intended use |
| --- | ---: | --- |
| `smoke_only` | 1 | harness reached the measurement path |
| `superfast_iteration` | 3 | fastest local check after a small edit |
| `fast_iteration` | 5 | quick local comparison before running longer loops |
| `p50_p90_decision_grade` | 30 | p50/p90 development iteration |
| `p95_decision_grade` | metric p95 floor | optimization decisions that compare p95 |
| `p99_decision_grade` | metric p99 floor | p99 claims and release-quality tail comparisons |

Strict coverage requires every P0 metric to be present, exact, and at or above
its metric-specific p95 floor. `p99_decision_grade` is reported separately and
does not block the sprint-ready gate.

## Disk Stage Telemetry

The focused dashboard keeps `disk.sparse_bloat_after_trim` and
`disk.host_bytes_reclaimed_after_trim` as headline metrics. Signed-live disk
artifacts also emit `disk.sparse_bloat_after_delete` so a report can distinguish
post-delete/pre-fstrim bloat from post-fstrim bloat. Stage samples carry raw
host/guest byte tags (`host_allocated_after_delete_bytes`,
`guest_used_after_delete_bytes`, `host_allocated_after_trim_bytes`,
`guest_used_after_trim_bytes`) so a bloat regression points at the stage that
moved instead of only reporting one final ratio.

| Metric | Start event | End event | Lifecycle | Workload | Profile | Included phases | Excluded phases | Owner | p95 min samples | p99 min samples |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | ---: | ---: |
| `start.hot_to_first_stdout_ms` | `PoolLeaseAcquired` | `FirstStdoutByte` | `hot` | `tiny_exec` | `fast_agent` | readiness check, exec request, process start, stdout wait | pool lease acquisition, template lookup, snapshot restore, cleanup | `firkin-runtime/firkin-vminitd-client` | 100 | 500 |
| `start.hot_to_ready_ms` | `PoolLeaseAcquired` | `ReadyProbePassed` | `hot` | `readiness_probe` | `fast_agent` | guest ping, workspace probe, exec probe, optional DNS probe | pool lease acquisition, first user command stdout, cleanup | `firkin-runtime` | 100 | 500 |
| `start.warm_to_first_stdout_ms` | `RequestStart` | `FirstStdoutByte` | `warm` | `tiny_exec` | `fast_agent` | warm start, readiness, exec request, stdout wait | cold image/template preparation, hot pool lease-only path, cleanup | `firkin-runtime/firkin-single-node` | 100 | 500 |
| `start.agent_task_ready_ms` | `RequestStart` | `FirstStdoutByte` | `hot` | `tiny_exec` | `fast_agent` | external API request, sandbox create, readiness, first useful stdout | post-first-stdout task wall time and cleanup | `firkin-benchmark/firkin-runtime` | 100 | 500 |
| `pool.lease_ms` | `PoolLeaseRequested` | `PoolLeaseAcquired` | `hot` | `tiny_exec` | `fast_agent` | pool lookup and lease acquisition | readiness, workspace setup, exec, stdout, cleanup | `firkin-admission/firkin-runtime` | 100 | 500 |
| `exec.direct_command_start_ms` | `ExecRequestSent` | `ProcessStarted` | `hot` | `tiny_exec` | `fast_agent` | direct exec RPC dispatch through guest process start | pool lease, readiness, stdout wait, process exit, cleanup | `firkin-vminitd-client/firkin-runtime` | 100 | 500 |
| `exec.direct_first_stdout_byte_ms` | `ExecRequestSent` | `FirstStdoutByte` | `hot` | `tiny_exec` | `fast_agent` | direct exec RPC dispatch, process start, stdout wait | pool lease, readiness, process exit, cleanup | `firkin-vminitd-client/firkin-runtime` | 100 | 500 |
| `exec.batch_100_small_commands_ms` | `ExecRequestSent` | `ProcessExited` | `hot` | `batch_100_execs` | `fast_agent` | one retained shell execution processing 100 tiny command payloads through final process exit | sandbox startup, pool lease, cleanup, independent process startup per command | `firkin-benchmark/firkin-vminitd-client` | 100 | 500 |
| `density.max_active_before_retained_shell_first_stdout_p95_doubles` | `PoolLeaseAcquired` | `FirstStdoutByte` | `hot` | `retained_shell_density` | `density` | concurrency sweep of retained-shell first-stdout p95 | other workload p95s, cold/warm/resumed lifecycles | `firkin-benchmark/firkin-runtime` | 30 | 500 |
| `disk.sparse_bloat_after_trim` | `FstrimDone` | `FstrimDone` | `hot` | `disk_bloat_reclaim` | `disk_reclaim` | host allocated bytes and guest used bytes after fstrim | pre-task and pre-trim bloat states | `firkin-benchmark/firkin-single-node` | 30 | 500 |
| `disk.host_bytes_reclaimed_after_trim` | `FstrimStart` | `FstrimDone` | `hot` | `disk_bloat_reclaim` | `disk_reclaim` | host allocated-byte delta across fstrim | guest-reported trim bytes without host allocation delta | `firkin-benchmark/firkin-single-node` | 30 | 500 |
| `cleanup.leftover_bytes` | `CleanupStart` | `CleanupDone` | `hot` | `tiny_exec` | `fast_agent` | run-scoped Firkin-owned leftover bytes after destroy | global cache/state roots and unrelated templates | `firkin-hygiene/firkin-runtime` | 30 | 500 |
| `reliability.unknown_failure_rate` | `RequestStart` | `CleanupDone` | `hot` | `tiny_exec` | `fast_agent` | classified create, readiness, exec, and cleanup attempts | known boot, agent, DNS, workspace, and OOM failures | `firkin-benchmark/firkin-runtime` | 30 | 500 |
