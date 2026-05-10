---
name: firkin-profiling
description: Use when iterating quickly on Firkin performance, local profiling, stage timing, bottleneck attribution, smoke evidence, or benchmark-derived instrumentation in the Firkin checkout.
---

# Firkin Profiling

Use this skill inside the standalone Firkin checkout when the goal is to iterate
quickly on Firkin performance. This is not primarily about publishable benchmark
results. The default mode is fast local attribution: find what got slower, add
the smallest useful stage timing, rerun, compare, repeat.

## Mental Model

Profiling answers:

```text
where is time / memory / disk / pressure being spent right now?
did this edit move the bottleneck?
what stage should we inspect next?
```

Benchmarking answers:

```text
is this result stable enough to publish or gate?
```

Start with profiling. Escalate to benchmark gates only when the result is worth
validating.

Debug-build benchmark runs are harness checks only. They do not count as
performance baselines. Use release-built `fk` for any baseline that will guide
optimization decisions.

## Topology

Keep ownership strict:

- `firkin-trace`: raw `BenchmarkSample` primitives, metric kinds, units, tags.
- runtime/core/template/vminitd-facing crates: emit samples where the work
  happens.
- `firkin-evidence`: metric catalog, required sets, coverage/trust labels, SLOs,
  and phase ownership. It validates; it does not run VMs.
- `firkin-benchmark`: suite definitions and run orchestration.
- `firkin-cli`: operator commands, reports, compare, sprint records.

Do not add generic profiling helpers in `lib.rs`, `utils`, or `common`. Put the
timing at the seam that owns the work.

## Fast Profiling Loop

Doctor only when running signed-live work:

```bash
cargo run -q -p firkin-cli -- benchmark doctor --mode signed-live
```

Fast smoke run:

```bash
cargo run -p firkin-cli -- benchmark run agent-core \
  --mode signed-live \
  --duration 30s \
  --out target/firkin-live-evidence/profile-current-30s.json
```

Inspect one artifact:

```bash
cargo run -q -p firkin-cli -- benchmark report lifecycle \
  target/firkin-live-evidence/profile-current-30s.json
```

Compare against a known artifact or baseline:

```bash
cargo run -q -p firkin-cli -- benchmark compare \
  /Users/darin/.firkin/benchmarks/baselines/local-agent-core-60s.json \
  target/firkin-live-evidence/profile-current-30s.json \
  --rank bottlenecks
```

Use strict coverage only when the contract itself matters:

```bash
cargo run -q -p firkin-cli -- benchmark coverage --strict \
  --artifact target/firkin-live-evidence/profile-current-30s.json \
  --artifact target/firkin-live-evidence/overhead-60s.json
```

For most profiling turns, `report lifecycle` and `compare --rank bottlenecks`
are enough. Do not burn time on full workspace tests or 60s live runs until a
candidate optimization is real.

## Baselines

Treat CLI `--duration` as repeat policy, not a wall-clock timeout. Current code
maps duration to repeats as:

```text
repeats = ceil(duration_seconds / 20)
```

So `--duration 60s` means 3 repeats, and `--duration 10m` means 30 repeats.
Live Apple/VZ wall-clock time can be much longer.

When moving fast, prefer 1-2 repeats:

```text
--duration 20s -> 1 repeat
--duration 30s -> 2 repeats
--duration 60s -> 3 repeats
```

Before starting any signed-live run, check for process contention:

```bash
pgrep -fl 'live_snapshot_restore|fk benchmark run|run-signed-live' || true
```

If a fast run looks odd, rerun once before drawing conclusions. Treat 1-2 repeat
results as directional iteration signal, not a publication baseline.

Proper local baseline:

```bash
cargo build --release -p firkin-cli
target/release/fk benchmark doctor --mode signed-live --min-free-bytes 10737418240
target/release/fk benchmark run agent-core \
  --mode signed-live \
  --duration 60s \
  --out target/firkin-live-evidence/baseline-agent-core-60s-release.json
target/release/fk benchmark baseline save \
  target/firkin-live-evidence/baseline-agent-core-60s-release.json \
  --name local-agent-core-60s-release
target/release/fk benchmark report lifecycle \
  target/firkin-live-evidence/baseline-agent-core-60s-release.json
```

Use a debug run only to prove the harness still reaches the signed-live path.
If a debug benchmark is accidentally running while trying to establish a
baseline, kill it and rerun release.

The signed-live script supports direct release test execution:

```bash
scripts/run-signed-live-runtime-test.sh --release --no-build \
  live_runtime_benchmark_evidence_writes_required_lifecycle_artifact
```

`--release` selects `target/release/deps/<test>-*`; `--no-build` skips Cargo and
uses the newest already-built matching test binary. This is the fastest path
when a release test binary already exists and you only need to rerun the live
benchmark.

## CPU Profiling With Samply

`samply` is available on this Mac and can write Firefox Profiler JSON:

```bash
samply record --save-only --unstable-presymbolicate \
  -o target/profiles/fk-profile.json \
  -- target/debug/fk benchmark report lifecycle \
    target/firkin-live-evidence/profile-current-30s.json
```

Outputs:

- `target/profiles/fk-profile.json`: Firefox Profiler profile tables.
- `target/profiles/fk-profile.syms.json`: symbol sidecar when
  `--unstable-presymbolicate` is used.

Use `--unstable-presymbolicate` for machine parsing. The main JSON often stores
frames as addresses; the `.syms.json` sidecar maps known addresses to Rust
function names and source lines.

Mac sharp edge: `samply` cannot profile Apple/system binaries such as `/bin/sh`
because system signing blocks the task-port mechanism. It works on locally built
Rust binaries such as `target/debug/fk` and test binaries under
`target/debug/deps/`.

Fast smoke for the machinery:

```bash
cargo test -p firkin-runtime runtime_adapter_command_start_records_command_latency_samples --no-run
samply record --save-only --unstable-presymbolicate \
  -o target/profiles/e2b-adapter-test.json \
  -- target/debug/deps/e2b_adapter-<hash> \
    runtime_adapter_command_start_records_command_latency_samples --nocapture
```

Tiny commands may produce valid but empty profiles. For useful hot paths, profile
a command that runs long enough to collect samples, such as a signed-live smoke,
a focused expensive test, or a looped local repro.

Minimal machine-parse shape:

```text
profile JSON: threads[].samples, stackTable, frameTable, funcTable, stringArray
syms JSON: data[].known_addresses -> data[].symbol_table -> string_table
```

The parser should count leaf and inclusive samples by walking
`samples.stack -> stackTable.prefix/frame -> frameTable.address`, then resolving
addresses through the symbol sidecar.

Repo-local parser:

```bash
scripts/samply-hot.py target/profiles/fk-profile.json --top 30
scripts/samply-hot.py target/profiles/fk-profile.json --repo-only --json \
  > target/profiles/fk-profile-summary.json
```

One-command profile, parse, and summarize:

```bash
scripts/profile-firkin.py \
  --profile target/profiles/fk-help.json \
  --summary target/profiles/fk-help-summary.json \
  --repo-only \
  --group-by module \
  --module-depth 4 \
  --collapse-generics \
  --top-threads 8 \
  --save-baseline bounded-runtime-adapter \
  -- target/debug/fk --help
```

This writes:

- profile JSON from `samply`.
- symbol sidecar when `--unstable-presymbolicate` works.
- `*-stages.json` with a coarse `command` stage.
- summary JSON from `scripts/samply-hot.py`.

Grouping:

```bash
scripts/samply-hot.py target/profiles/fk-profile.json --repo-only --group-by crate
scripts/samply-hot.py target/profiles/fk-profile.json --repo-only --group-by module --module-depth 3
scripts/samply-hot.py target/profiles/fk-profile.json --repo-only --group-by file
```

For stage attribution, prefer `--group-by module --module-depth 3` or
`--module-depth 4`. Crate grouping is only a sanity rollup and is usually too
coarse to guide an optimization.

Useful filters:

```bash
scripts/samply-hot.py target/profiles/fk-profile.json --include 'firkin|vminitd|tokio'
scripts/samply-hot.py target/profiles/fk-profile.json --exclude 'test::|std::rt'
scripts/samply-hot.py target/profiles/fk-profile.json --thread 'tokio|runtime' --top-threads 8
```

Use `--collapse-generics` when monomorphized Rust names are splitting the same
logical code path across many rows.

Diff two summaries or raw profiles:

```bash
scripts/samply-hot.py --diff \
  target/profiles/baselines/bounded-runtime-adapter-summary.json \
  target/profiles/after-summary.json \
  --diff-table grouped_inclusive \
  --top 30
```

The benchmark evidence CLI has its own baseline store:

```bash
cargo run -q -p firkin-cli -- benchmark baseline list
cargo run -q -p firkin-cli -- benchmark baseline save \
  target/firkin-live-evidence/current-60s.json \
  --name local-agent-core-60s
```

Use evidence baselines for P0 metric regressions. Use profiling baselines for
hot-path attribution after a benchmark or focused test points at a phase.

Stage correlation:

```bash
scripts/samply-hot.py target/profiles/fk-profile.json \
  --stages target/profiles/fk-profile-stages.json \
  --repo-only \
  --group-by crate
```

Stage files use profile-relative milliseconds:

```json
{
  "schema": "firkin.samply.stages.v1",
  "time_unit": "ms",
  "time_origin": "samply_profile_start",
  "stages": [
    { "name": "command", "start_ms": 0.0, "end_ms": 1000.0 }
  ]
}
```

The wrapper generates a coarse `command` stage automatically. For fine-grained
runtime attribution, emit stage ranges from the owner seam into the same schema
and pass them with `--stages`; the parser will slice samples into per-stage hot
tables.

The parser auto-discovers `target/profiles/fk-profile.syms.json` next to the
profile. Pass `--syms <path>` if the sidecar was moved.

## Useful Current Metrics

Startup/tool loop:

- `agent_task_ready_ms`
- `sandbox.start.cold_ready_ms`
- `sandbox.start.warm_ready_ms`
- `sandbox.start.hot_pool_checkout_ms`
- `sandbox.start.resume_snapshot_to_first_stdout_ms`
- `warm_snapshot_restore`
- `cold_template_build`
- `snapshot_save`
- `ready_probe`

Command path:

- `command_start`
- `first_stdout_byte`
- `sandbox.exec.first_latency_ms`
- `sandbox.exec.first_stdout_ms`

Disk/workspace:

- `sandbox.disk.metadata_create_stat_unlink_ms`
- `sandbox.disk.fsync_p99_us`
- `sandbox.disk.sparse_bloat_ratio`
- `sandbox.disk.trim_reclaim_bytes_per_sec`

Density/reliability/cleanup:

- `concurrent_create`
- `sandbox.density.max_active_before_p95_doubles`
- `sandbox.reliability.boot_failure_rate`
- `sandbox.reliability.unknown_failure_rate`
- `kill_delete`
- `sandbox.cleanup.leftover_bytes`

Inspect ownership and next-action hints:

```bash
cargo run -q -p firkin-cli -- benchmark phase-owners
```

## Adding Stage-Local Instrumentation

Add a metric when a top-level number is too big to explain.

Decision rule:

```text
need quick local attribution only -> emit sample + catalog Core/Drilldown
need compare output to route ownership -> add phase ownership
need smoke artifact completeness -> add lifecycle required metric
need dashboard headline -> add P0 scorecard + suite + coverage
```

Start with the least contractual surface that answers the profiling question.
Promote later if the stage becomes a stable dashboard metric.

Emitter pattern:

```rust
let started = Instant::now();
// stage work
samples.push(BenchmarkSample::new(
    "sandbox.start.some_stage_ms",
    BenchmarkMetricKind::LifecycleLatency,
    BenchmarkUnit::Milliseconds,
    started.elapsed().as_secs_f64() * 1000.0,
));
```

For composed spans, store the upstream `Instant` on runtime/session state and
emit once when the downstream event happens. The current example is
`sandbox.start.resume_snapshot_to_first_stdout_ms` in
`crates/runtime/src/adapter.rs`.

Likely owner files:

- template/rootfs/snapshot build: `crates/runtime/src/template_build.rs`
- restore/readiness/create route: `crates/runtime/src/restore.rs`,
  `crates/runtime/src/adapter.rs`
- exec/stdout: `crates/runtime/src/session.rs`,
  `crates/runtime/src/adapter.rs`
- disk workload probes: benchmark disk harness or the relevant core/vminitd seam
- catalog/trust/ownership: `crates/evidence/src/catalog.rs`
- required lifecycle gates: `crates/evidence/src/lifecycle.rs`
- suite membership: `crates/benchmark/src/suite.rs`

## Sharp Edges

- `duration 30s` is smoke data. Treat it as directionally useful, not truth.
- Signed-live `agent-core` may take several minutes because it compiles/signs
  the harness and boots/restores VMs.
- `benchmark compare` currently wants a resolved baseline artifact path;
  `sprint-record` accepts a baseline name.
- Contract changes stale old artifacts. If you add a required metric, rerun the
  lifecycle artifact before strict coverage.
- Disk pressure is common after full builds. Preserve live evidence and clear
  rebuildable outputs:

```bash
rm -rf target/debug target/tests
```

- `doctor` has a 16 GiB default free-space floor in signed-live mode.
- Do not weaken disk-pressure tests to make profiling convenient.
- Markdown artifacts under `docs/artifacts/` are tracked. Generated HTML proof
  previews are ignored.

## Verification By Scope

For instrumentation-only edits:

```bash
cargo fmt --check
cargo test -p firkin-runtime <focused_test> -- --nocapture
cargo test -p firkin-evidence catalog -- --nocapture
cargo test -p firkin-benchmark suite -- --nocapture
cargo check -p firkin-cli --all-targets
git diff --check
```

For a profiling smoke:

```bash
cargo run -p firkin-cli -- benchmark run agent-core \
  --mode signed-live \
  --duration 30s \
  --out target/firkin-live-evidence/profile-current-30s.json

cargo run -q -p firkin-cli -- benchmark report lifecycle \
  target/firkin-live-evidence/profile-current-30s.json
```

For promotion after an optimization looks good:

```bash
cargo clippy -p firkin-runtime -p firkin-evidence -p firkin-benchmark -p firkin-cli --all-targets -- -D warnings
cargo run -p firkin-cli -- benchmark run agent-core --mode signed-live --duration 60s --out target/firkin-live-evidence/current-60s.json
cargo run -q -p firkin-cli -- benchmark coverage --strict --artifact target/firkin-live-evidence/current-60s.json --artifact target/firkin-live-evidence/overhead-60s.json
```

Use full workspace gates only when closing a branch, not during tight profiling.

## Reporting

For profiling updates, report:

- metric and stage
- artifact path
- count
- p50/p95 if useful
- compare delta if available
- confidence label, usually smoke-only
- next owner/seam to inspect

Avoid publishable language unless the sample count, live conditions, and strict
coverage actually support it.
