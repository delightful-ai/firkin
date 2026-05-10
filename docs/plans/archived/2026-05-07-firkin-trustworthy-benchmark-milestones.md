# Firkin Trustworthy Benchmark Milestones

Date: 2026-05-07

Status: implementation plan

Related specs:

- `docs/plans/2026-05-07-firkin-decision-grade-metrics-implementation-plan.md`
- `docs/plans/archived/2026-05-07-firkin-trustworthy-performance-loop-spec.md`
- `docs/plans/2026-05-07-firkin-agent-benchmark-scorecard-surface.md`
- `docs/plans/2026-05-06-firkin-trace-foundation-design.md`

Decision-grade follow-up: this milestones document describes the first
operator-safe benchmark loop and signed-live P0 proof. The next implementation
pass is the decision-grade metric plan above, which hard-cuts ambiguous
smoke-era names and adds raw event traces, lifecycle/workload labels,
sample-confidence floors, metric validation, disk-stage semantics, and
batch-stability gates before optimization work starts.

## Goal

Get Firkin to a state where a two-hour performance sprint can start with
trusted live numbers, short feedback loops, cleanup safety, and proof artifacts
that make completion observable without needing to read every line of code.

## Completion Rule

A milestone is not complete when the code compiles. A milestone is complete only
when its user-visible behavior can be reproduced from commands and captured in
the milestone HTML artifact.

Every feature below has at least one observable behavior. The proof artifact for
the milestone must include:

1. The exact command.
2. The exit status.
3. The artifact path written by the command.
4. The fields or lines proving the behavior.
5. A short residual-risk note if anything is still proxy, smoke-only, or
   environment-dependent.

## Proof Artifacts

The proof artifacts are intentionally plain HTML so they can be opened directly:

| Milestone | Artifact |
| --- | --- |
| M1 | `docs/artifacts/firkin-benchmark-m1-proof.html` |
| M2 | `docs/artifacts/firkin-benchmark-m2-proof.html` |
| M3 | `docs/artifacts/firkin-benchmark-m3-proof.html` |

These files start as proof contracts. When each milestone lands, update the
corresponding HTML file with command output, screenshots if useful, links to
generated JSON artifacts, and a pass/fail summary.

## M1: Operator-Safe Benchmark Foundation

Purpose: make live benchmarking safe to start repeatedly on this machine without
silently using bad prerequisites, filling `/tmp`, or deleting the wrong files.

### Feature M1.1: Benchmark Doctor

Behavior:

```text
fk benchmark doctor --mode signed-live
```

Success criteria:

- Exits 0 when configured roots, disk floor, Apple Virtualization host,
  signed-live harness files, guest PSI readiness, and embedded vminitd bytes
  pass.
- Exits non-zero before running any benchmark when a required preflight fails.
- Prints root paths, disk, VZ host, signing/entitlement status, harness file
  presence, guest PSI readiness, and vminitd byte availability in a compact
  line-oriented format.
- Records whether live Apple/VZ benchmarks are allowed, blocked, or degraded.

Current doctor checks:

- `state_root`, `cache_root`, and `benchmark_root` exist or can be created and
  accept a write/delete probe.
- `benchmark_root` has at least `--min-free-bytes` free.
- Signed-live mode runs the Apple Virtualization host preflight and requires an
  arm64 host. It reports current executable signing and Virtualization
  entitlement fields without treating false signing fields as standalone
  failures.
- Signed-live mode requires `scripts/run-signed-live-runtime-test.sh` and
  `signing/vz.entitlements`.
- Signed-live mode requires guest PSI readiness for
  `sandbox.pressure.io_full_avg10`: `kernel/config-arm64` enables PSI/default
  PSI and `bin/vmlinux` is current against that config.
- Signed-live mode reports embedded vminitd bytes as available.

Observable proof:

- M1 HTML contains a transcript showing a passing doctor run.
- M1 HTML contains one deliberately blocked doctor case, such as a missing or
  unwritable state root, with a non-zero exit and no benchmark artifact written.

### Feature M1.2: Configurable State and Cache Roots

Behavior:

```text
fk config show
fk benchmark doctor --state-root ~/.firkin/state --cache-root ~/.firkin/cache
```

Success criteria:

- Default runtime state root is `~/.firkin/state`.
- Default runtime cache root is `~/.firkin/cache`.
- Default benchmark artifact root is `~/.firkin/benchmarks`.
- Library configuration can override these roots without using global process
  state.
- Live benchmark artifacts record the selected roots in their config
  fingerprint.

Observable proof:

- M1 HTML shows `fk config show` or equivalent output with the default roots.
- M1 HTML shows an override run and the artifact/config field proving the
  override was honored.

### Feature M1.3: Firkin-Owned Cleanup

Behavior:

```text
fk clear --dry-run --state --cache --benchmarks
fk clear --state --older-than 24h
```

Success criteria:

- Dry-run lists only Firkin-owned paths.
- Clear refuses to delete paths outside configured Firkin-owned roots.
- Clear reports bytes and path counts before deletion.
- Clear is safe when roots do not exist.

Observable proof:

- M1 HTML contains a dry-run transcript.
- M1 HTML contains a safety refusal transcript for a non-Firkin path.
- M1 HTML contains a cleanup report showing zero unexpected paths touched.

### Feature M1.4: Proof Artifact Generation

Behavior:

```text
fk benchmark proof m1 --from target/firkin-live-evidence/m1.json \
  --out docs/artifacts/firkin-benchmark-m1-proof.html
```

Success criteria:

- Produces a self-contained HTML file.
- Shows every M1 feature as pass/fail.
- Links or names all evidence JSON files used.
- Marks missing commands or failed checks as failed, not omitted.

Observable proof:

- Opening `docs/artifacts/firkin-benchmark-m1-proof.html` shows all M1 checks
  with command transcripts and pass/fail state.

## M2: Trusted Signed-Live P0 Evidence

Purpose: produce exact live numbers for the core agent-sandbox lifecycle and
refuse to treat proxy or small-sample data as optimization truth.

### Feature M2.1: Exact `agent_task_ready_ms`

Behavior:

```text
fk benchmark run agent-core --mode signed-live --duration 60s \
  --out target/firkin-live-evidence/agent-core-60s.json
```

Success criteria:

- Measures from host API request acceptance to host-observed first stdout byte.
- Includes phase spans for image/rootfs, disk, VM, guest, network, workspace,
  cgroup, first exec, first stdout, and cleanup.
- Does not report VM-start-only latency as readiness.
- Tags the metric as `signed_live_exact`.

Observable proof:

- M2 HTML shows the run command and exit status.
- M2 HTML shows the artifact path.
- M2 HTML shows the `agent_task_ready_ms` raw sample count, trust label, and
  endpoint definition.

### Feature M2.2: Percentile Honesty

Behavior:

```text
fk benchmark report target/firkin-live-evidence/agent-core-60s.json
```

Success criteria:

- Report includes count, min, p50, p90, p95, p99, max, mean, and variance label.
- p90, p95, and p99 are shown as optimization-grade only when sample-count
  thresholds are met.
- Small-n p99 is labeled smoke-only.

Observable proof:

- M2 HTML includes a report excerpt with a metric that has enough samples.
- M2 HTML includes a report excerpt where p99 is smoke-only when count is below
  100.

### Feature M2.3: Trust and Coverage Gate

Behavior:

```text
fk benchmark coverage --strict \
  --artifact target/firkin-live-evidence/agent-core-60s.json \
  --artifact target/firkin-live-evidence/overhead.json \
  --artifact target/firkin-live-evidence/scorecard.json
```

Success criteria:

- P0 metrics appear as optimization signals only with accepted trust labels.
- `schema_only` and `untrusted` metrics are shown as missing or blocked, not
  optimized.
- Proxy metrics include the proxy reason and cannot silently satisfy exact P0
  requirements.
- Strict coverage can resolve exact source metrics across lifecycle, overhead,
  and scorecard artifacts, but still fails if a required P0 metric is missing,
  proxy-only, schema-only, or below the artifact's sample floor.

Observable proof:

- M2 HTML shows strict coverage output with exact, proxy, and missing buckets.
- M2 HTML names every P0 metric that remains blocked.

### Feature M2.4: Trace Overhead Gate

Behavior:

```text
fk benchmark run overhead --mode signed-live \
  --out target/firkin-live-evidence/overhead.json
```

Success criteria:

- Runs tracing-on/tracing-off or equivalent A/B measurement.
- Reports recorder wall overhead, allocation overhead, and retained memory.
- Blocks close-call performance conclusions when overhead exceeds budget.

Observable proof:

- M2 HTML includes overhead report lines and pass/fail budget state.

## M3: Sprint-Ready Optimization Loop

Purpose: make the benchmark loop directly useful for a focused two-hour
performance sprint.

Current audit note: M3 is now strict sprint-ready evidence. The proof retains a
completed 60s signed-live lifecycle artifact, a completed 60s signed-live
overhead artifact, a baseline saved from the 60s lifecycle artifact, strict
multi-artifact P0 coverage, overhead SLO output, compare output, and a passing
`sprint-ready` transcript. Memory attribution is exact for the overhead harness
by excluding setup VZ task IDs and summing the measured exclusive
`com.apple.Virtualization.VirtualMachine` task set. Guest PSI is exact because
the rebuilt signed-live kernel artifact emits `/proc/pressure/io` samples into
the lifecycle artifact.

### Feature M3.1: Baseline Save and Compare

Behavior:

```text
fk benchmark baseline save target/firkin-live-evidence/agent-core-60s.json \
  --name local-agent-core

fk benchmark compare ~/.firkin/benchmarks/baselines/local-agent-core.json \
  target/firkin-live-evidence/current.json
```

Success criteria:

- Baseline keys include machine, macOS, image/rootfs digest, storage backend,
  cache mode, sync mode, network profile, suite, and mode.
- Compare refuses or labels cross-environment comparisons.
- Compare prints top bottlenecks, regressions, improvements, trust failures,
  sample-size failures, environment instability, and cleanup leaks.

Observable proof:

- M3 HTML shows a baseline save transcript.
- M3 HTML shows a compare transcript with at least one ranked phase and owner.

### Feature M3.2: 30s and 60s Loop Commands

Behavior:

```text
fk benchmark run agent-core --mode signed-live --duration 30s --out current-30s.json
fk benchmark run agent-core --mode signed-live --duration 60s --out current-60s.json
```

Success criteria:

- 30s loop is fast enough for every focused performance edit.
- 60s loop confirms or rejects promising changes.
- Both loops write artifacts with raw samples, trust labels, cleanup report, and
  environment fingerprint.
- Current signed-live strict P0 coverage is 18/18 exact when run against
  `current-60s.json` and `overhead-60s.json`.
- The 18 P0 rows are the hard-cut implemented contract defined by
  `P0_SCORECARD_METRICS` and `AGENT_CORE_CASES`;
  `sandbox.cleanup.orphan_count` remains Core and is not part of P0 coverage.
- The three `sandbox.mem.*` rows are exact only for artifacts emitted by the
  signed-live overhead harness after the exclusive VZ task-set collector landed.
  Stale overhead artifacts or scorecard-only memory summaries must not promote
  those rows.
- `sandbox.pressure.io_full_avg10` promotes only from lifecycle artifact
  evidence containing the guest `/proc/pressure/io` metric. Doctor readiness is
  a prerequisite, not metric evidence by itself.

Observable proof:

- M3 HTML shows elapsed time for both loops.
- M3 HTML shows the command to move from 30s smoke to 60s control.
- M3 HTML shows 60s lifecycle and overhead commands, strict 18/18 P0 coverage,
  and the passing sprint-ready transcript.

### Feature M3.3: Phase-to-Code Ownership

Behavior:

```text
fk benchmark compare baseline.json current.json --rank bottlenecks
```

Success criteria:

- Every ranked lifecycle bottleneck names phase, owner crate, owner module or
  operation, p95/p99 availability, and confidence label.
- Report identifies when the next action is "collect more samples" instead of
  "optimize code".

Observable proof:

- M3 HTML shows a bottleneck table with phase and owner columns.
- M3 HTML includes at least one residual-risk row for low sample count, high
  variance, overhead, or trust failure.

### Feature M3.4: Two-Hour Sprint Readiness Gate

Behavior:

```text
fk benchmark sprint-ready --suite agent-core --baseline local-agent-core \
  --current-artifact target/firkin-live-evidence/agent-core-60s.json \
  --overhead-artifact target/firkin-live-evidence/overhead.json \
  --scorecard-artifact target/firkin-live-evidence/scorecard.json
```

Success criteria:

- Fails unless doctor passes, overhead SLO passes, optional scorecard validation
  passes, baseline exists, compare works, and strict multi-artifact P0 coverage
  is exact.
- Prints the exact first benchmark command to run for the sprint.
- Prints the current largest trusted bottleneck.

Observable proof:

- M3 HTML shows `sprint-ready=passed` only when every required exact P0 metric
  is present; otherwise it records a partial or blocked failure.
- M3 HTML shows the first recommended 30s command and the current bottleneck.
- `sprint-ready` prints a passing strict gate only when lifecycle and overhead
  artifacts together provide all 18 exact P0 metrics. The current proof reaches
  that state with `current-60s.json` and `overhead-60s.json`.

## M4: Guardrails for Keeping P0 Metrics Exact

Purpose: preserve the exactness boundaries that made M3 sprint-ready, so future
agents do not accidentally promote stale, proxy, or doctor-only readback.

### Feature M4.1: Guest PSI Capability Reporting and Enabling

Behavior:

```text
fk benchmark doctor --mode signed-live
fk benchmark coverage --strict --artifact current-60s.json --artifact overhead-60s.json
```

Success criteria:

- Doctor reports whether guest PSI is available and whether
  `/proc/pressure/io` exists in the signed-live guest.
- Coverage reports `sandbox.pressure.io_full_avg10` as unsupported when the
  guest lacks `/proc/pressure/io` or when no signed-live artifact contains the
  emitted metric. Coverage reports the PSI row as exact only when artifact
  evidence contains `sandbox.pressure.io_full_avg10`.
- Enabling PSI in the guest and emitting it in the lifecycle artifact is the
  only path to promoting `sandbox.pressure.io_full_avg10` to an exact
  optimization signal.

### Feature M4.2: Memory Attribution Collector

Behavior:

```text
fk benchmark run overhead --mode signed-live --duration 60s --out overhead-60s.json
fk benchmark coverage --strict --artifact current-60s.json --artifact overhead-60s.json
```

Success criteria:

- The collector provides sandbox-scoped memory attribution by recording setup
  VZ task IDs, summing only the measured exclusive
  `com.apple.Virtualization.VirtualMachine` task set, and pairing that host
  task-set delta with guest reclaim evidence.
- Host-process `vmmap` deltas remain proxy readback outside the exact overhead
  harness.
- Coverage can distinguish "proxy emitted" from "exact collector present" for
  all three `sandbox.mem.*` P0 rows.

## Required Final Verification

Milestone implementation is not ready for a performance sprint until these pass:

```bash
cargo fmt --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
scripts/check-firkin-crate-graph.sh
git diff --check
```

For live milestone closure, also run:

```bash
just live-runtime-benchmark-representative
just live-runtime-overhead-representative
just live-apple-vz-benchmark-suite
```

If disk is tight, clear Firkin-owned roots or stale Cargo target directories
before weakening any benchmark, live proof, or cleanup gate.
