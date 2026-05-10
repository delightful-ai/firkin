# firkin-benchmark

`firkin-benchmark` owns the benchmark execution surface above raw trace
samples and validated evidence schemas. It is the place for named benchmark
suites, runtime-facing writers, and future live harness orchestration.

Current surfaces:

- `BENCHMARK_SUITES` is the stable suite catalog.
- `benchmark_suites()` lists every suite.
- `benchmark_suite(id)` looks up one suite.
- `benchmark_cases_for_metric(metric)` shows which cases produce a metric.
- `RuntimeAgentScorecardEvidenceWriter` validates raw `BenchmarkSample`s and
  writes an agent scorecard evidence artifact.
- `RuntimeAutoscaleScorecardEvidenceWriter` validates raw `BenchmarkSample`s
  and writes the autoscale efficiency scorecard artifact for the
  browser/database/CLI product path.
- `RuntimeAgentComputerScorecardEvidenceWriter` validates raw `BenchmarkSample`s
  and writes the five-metric product-path scorecard artifact without requiring
  autoscale pressure/controller rows.
- `HostMemoryAttributionCollector` is the promotion seam for exact P0 memory
  metrics. The current `CurrentProcessVmmapCollector` deliberately reports a
  process-wide proxy scope, not an exact VM scope.

The suite catalog is intentionally broader than the current live runners. Each
case declares an execution mode so callers can distinguish host-only probes from
live VM, guest-agent, external-tool, and manual benchmark work.

Current signed-live benchmark gates:

- `just live-runtime-benchmark-representative`: three-sample lifecycle latency
  evidence plus SLO validation.
- `just live-runtime-overhead-representative`: three-sample host-side overhead
  evidence.
- `just live-apple-vz-benchmark-suite`: representative lifecycle, overhead,
  one-second soak, and product pod ASIF smoke through the signed harness.

Sprint loop operator path:

```text
cargo run -p firkin-cli -- benchmark doctor --mode signed-live
cargo run -p firkin-cli -- benchmark run agent-core --mode signed-live --duration 30s --out target/firkin-live-evidence/current-30s.json
cargo run -p firkin-cli -- benchmark report lifecycle target/firkin-live-evidence/current-30s.json
cargo run -p firkin-cli -- benchmark run overhead --mode signed-live --duration 30s --out target/firkin-live-evidence/overhead-30s.json
cargo run -p firkin-cli -- benchmark coverage --strict --artifact target/firkin-live-evidence/current-30s.json --artifact target/firkin-live-evidence/overhead-30s.json
```

Use the 30s loop as fast smoke after each focused edit. Promote promising
changes to the 60s loop before accepting them:

```text
cargo run -p firkin-cli -- benchmark run agent-core --mode signed-live --duration 60s --out target/firkin-live-evidence/current-60s.json
cargo run -p firkin-cli -- benchmark run overhead --mode signed-live --duration 60s --out target/firkin-live-evidence/overhead-60s.json
```

The next two-hour sprint loop is useful now with the 14 exact signed-live P0
metrics. Treat strict coverage failure as M4 prerequisite readback, not as a
reason to skip the sprint:

- `sandbox.mem.idle_host_footprint_bytes`
- `sandbox.mem.post_task_residual_bytes`
- `sandbox.mem.reclaim_effectiveness_ratio`
- `sandbox.pressure.io_full_avg10`

Sprint decisions may use the exact lifecycle, disk, density, reliability, and
cleanup rows. Do not optimize from the three `sandbox.mem.*` proxy metrics, and
do not infer I/O pressure from `sandbox.pressure.io_full_avg10` while the guest
lacks `/proc/pressure/io`.

Guest PSI readiness is an explicit doctor surface:

```text
cargo run -p firkin-cli -- benchmark doctor --mode signed-live
```

The `check=guest_psi` row reports whether `kernel/config-arm64` enables
`CONFIG_PSI=y`, whether PSI is default-enabled, whether `bin/vmlinux` is current
with that config, and the exact missing prerequisite. Do not mark
`sandbox.pressure.io_full_avg10` exact until the signed-live harness emits JSON
read from `/proc/pressure/io`.

M4 prerequisites:

- Guest PSI capability reporting and enabling: expose whether
  `/proc/pressure/io` is available in the signed-live guest, make the absence
  operator-visible, and enable guest PSI before promoting
  `sandbox.pressure.io_full_avg10` to an exact sprint signal.
- Memory attribution collector: replace host-process `vmmap` deltas with an
  exact VM or sandbox-scoped attribution source before promoting the three
  `sandbox.mem.*` rows to optimization targets.

The ignored `#[ignore]` tests are not dead code. They are live Apple/VZ tests
that must be signed before execution and should be run through the Justfile or
`scripts/run-signed-live-runtime-test.sh`.

Memory attribution boundary:

- `crates/benchmark/src/memory.rs` uses `vmmap -summary` against the current
  Firkin host process and derives deltas around sandbox create, task, and
  delete phases.
- Those deltas are proxy evidence for
  `sandbox.mem.idle_host_footprint_bytes`,
  `sandbox.mem.post_task_residual_bytes`, and
  `sandbox.mem.reclaim_effectiveness_ratio`.
- Do not mark those metrics exact until the Apple/VZ runtime exposes a
  trustworthy VM-process footprint or task-scoped host footprint source.
  Verified blocker on macOS 26.2 SDK headers: `VZVirtualMachine` exposes
  configured memory, `memoryBalloonDevices`, and balloon target controls, but
  not a VM pid, per-VM resident footprint, or per-VM host-memory statistics.
  `task_info(TASK_VM_INFO).phys_footprint` and `vmmap -summary` are process/task
  scoped; in the current in-process runtime they include all VZ VMs plus host
  bookkeeping. Guest cgroup memory stats remain guest-scoped and cannot
  attribute host VZ backing pages.
- Smallest exact-attribution spike: run exactly one VZ VM in a signed helper
  process, sample that helper's `TASK_VM_INFO`/`vmmap` footprint, and pair the
  host task delta with guest free/compact plus VZ balloon target changes. That
  gives a truthful one-VM-per-task host attribution path without promoting the
  current process-wide proxy.
- Proof-visible blocker: `fk benchmark memory-attribution` prints the required
  collector contract and the reason the current collector cannot promote the
  three P0 memory metrics.

Suites:

- `agent-core`: required P0 dashboard metrics for readiness, first exec, disk
  metadata, fsync tail latency, memory floor/reclaim, I/O pressure, disk bloat,
  density breakpoints, reliability, and cleanup leftovers.
- `agent-computer`: the product path for a ready browser + database + CLI
  environment, including ready/resume latency, density, cleanup, and unknown
  failure guardrails.
- `autoscale`: the autoscale efficiency dashboard for ready hit rate, product
  ready/resume latency, safe spare utilization, pressure shrink/refill, density,
  active-session protection, reserve-floor protection, cleanup, and unknown
  failures.
- `startup`: image, rootfs, disk attach, VM, guest-agent, network, DNS, mount,
  cgroup, first exec, and first stdout phases.
- `disk`: throughput, random I/O, fsync/fdatasync, metadata workload, developer
  workload, sparse bloat, trim, and reclaim metrics.
- `memory`: host, guest, cgroup, balloon, retention, reclaim, and recycle
  metrics.
- `cpu`: host and guest CPU, cgroup usage, throttling, context switches, and
  idle tax.
- `pressure`: guest PSI and pressure event metrics.
- `network`: DNS, connect, TLS, HTTP, RTT, vsock, packet loss, and policy
  denial metrics.
- `pod`: pod VM readiness, container spawn, marginal memory, shared volume,
  localhost RTT, and amortization metrics.
- `agent-control`: exec RPC, stream lag, stdin, signal, and log backpressure.
- `cleanup`: teardown, fstrim, disk/memory reclaim, leftover bytes, orphan
  count, and long-run leak slope.
- `isolation`: host mount exposure, capability count, network profile, risk
  flags, and secret exposure duration.
- `cache`: hit ratio, lookup/restore/save, retained bytes, evictions, and
  corruption.
- `density`: p95 under concurrency, idle and active breakpoints, per-GB
  density, throughput, and tail degradation.
- `power`: hot-pool idle CPU, wakeups, battery drain estimate, and thermal
  pressure.
- `abuse`: pids, OOM, disk fill, output flood, ignored SIGTERM, and cleanup
  abuse cases.
- `agent-realism`: repo import/index, first useful tool, first test, full task,
  patch apply, and artifact export.

Topology rules:

- Raw sample primitives stay in `firkin-trace`.
- Evidence schemas and metric-law validation stay in `firkin-evidence`.
- This crate may depend upward into runtime composition when implementing live
  runners, but lower runtime/library crates must not depend on it.
