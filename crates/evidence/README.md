# firkin-evidence

`firkin-evidence` owns validation and durable evidence schemas for benchmark
and production-substrate proof artifacts. It consumes raw `BenchmarkSample`s
from `firkin-trace`; it does not emit spans, run VMs, or own runtime policy.

Benchmark evidence surfaces:

- `BenchmarkSummary` computes count, min, p50, p90, p95, p99, max, mean, and
  standard deviation for one metric shape.
- `BENCHMARK_METRIC_CATALOG` is the stable agent-sandbox metric catalog. It
  covers startup, exec, agent-task, disk, memory, CPU, pressure, network, pids,
  pod, cache, isolation, cleanup, reliability, density, and power metrics.
- `P0_SCORECARD_METRICS` is the required dashboard set for the first-class
  agent benchmark scorecard.
- `P0_SCORECARD_MEASUREMENT_COVERAGE` records whether each P0 metric is backed
  by an exact signed-live measurement, a signed-live proxy, unit/schema coverage
  only, or still needs a live harness.
- `AgentBenchmarkScorecardReport` validates required P0 metrics and stores
  summaries for each one.
- `AgentBenchmarkScorecardArtifact` reads and writes the scorecard JSON
  artifact.
- `AUTOSCALE_EFFICIENCY_SCORECARD_METRICS` is the required dashboard for
  autoscale efficiency and the browser/database/CLI product path.
- `AutoscaleEfficiencyScorecardReport` validates required autoscale metrics and
  stores summaries for each one.
- `AutoscaleEfficiencyScorecardArtifact` reads and writes the autoscale
  scorecard JSON artifact.
- `AGENT_COMPUTER_SCORECARD_METRICS` is the required five-metric dashboard for
  the browser/database/CLI product path before full autoscale pressure coverage.
- `AgentComputerScorecardReport` validates required agent-computer metrics and
  stores summaries for each one.
- `AgentComputerScorecardArtifact` reads and writes the agent-computer
  scorecard JSON artifact.

The scorecard requires these P0 metrics:

- `start.hot_to_first_stdout_ms`
- `start.hot_to_ready_ms`
- `start.resume_to_first_stdout_ms`
- `start.warm_to_first_stdout_ms`
- `start.agent_task_ready_ms`
- `pool.lease_ms`
- `exec.command_start_ms`
- `exec.first_stdout_byte_ms`
- `exec.batch_100_small_commands_ms`
- `density.max_active_before_hot_to_first_stdout_p95_doubles`
- `disk.sparse_bloat_after_trim`
- `disk.host_bytes_reclaimed_after_trim`
- `cleanup.leftover_bytes`
- `reliability.unknown_failure_rate`

The autoscale efficiency scorecard requires:

- `autoscale.ready_queue_hit_rate_pct`
- `product.agent_computer_ready_ms`
- `product.agent_computer_resume_ms`
- `autoscale.safe_spare_limiting_utilization_pct`
- `autoscale.pressure_to_safe_floor_ms`
- `autoscale.pressure_clear_to_ready_target_ms`
- `density.max_agent_computers_before_ready_p95_doubles`
- `autoscale.active_evictions_due_to_pool_pressure`
- `autoscale.reserve_floor_violations`
- `cleanup.leftover_bytes`
- `reliability.unknown_failure_rate`

The agent-computer scorecard requires:

- `product.agent_computer_ready_ms`
- `product.agent_computer_resume_ms`
- `density.max_agent_computers_before_ready_p95_doubles`
- `cleanup.leftover_bytes`
- `reliability.unknown_failure_rate`

Future benchmark runners should emit typed `BenchmarkSample`s and then let this
crate decide whether the evidence is complete enough to publish or gate.
Do not treat a metric as trusted just because it appears in the catalog; require
signed-live exact coverage or an explicitly accepted proxy.
