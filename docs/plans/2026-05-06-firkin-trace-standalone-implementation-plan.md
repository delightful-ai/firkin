# Firkin Trace Standalone Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a standalone, spec-faithful `firkin-trace` leaf crate that owns benchmark samples, recorder spans, samplers, checkpoints, shared trace tags, and lifecycle phase names, then hard-cut existing benchmark primitive consumers to the new crate.

**Architecture:** `firkin-trace` is the narrow measurement waist below substrate/evidence and above no runtime domain type. It exports typed `BenchmarkSample`s and a low-overhead `Recorder` bus; `firkin-substrate` keeps summary/SLO/evidence logic and depends inward on `firkin-trace`.

**Tech Stack:** Rust 2024, `serde`, `thiserror`, `parking_lot`, `smallvec`, `async-trait`, `tokio`, current Cargo workspace.

---

## Success Criteria

- `crates/trace/` exists as workspace crate `firkin-trace`.
- `BenchmarkSample`, `BenchmarkMetricKind`, and `BenchmarkUnit` live in `firkin-trace`, not `firkin-substrate`.
- `BenchmarkSample` keeps the primitive metric/kind/unit/value shape and adds skip-empty per-sample `tags`.
- `BenchmarkSummary` reports count, p50, p90, p95, p99, and max. New evidence artifacts are a hard-cut schema; regenerate old summary-only artifacts.
- `Recorder::disabled()` is cheap and produces no samples.
- `Recorder::enabled(profile, tags)` records spans, direct samples, samplers, and checkpoints into one bus.
- `Span::finish_ok`, `Span::finish_error`, `Span::discard`, and drop-cancel semantics are tested.
- `attach_sampler` returns `RecorderError::NoRuntime` outside tokio and `close_and_drain()` aborts sampler tasks before drain.
- Shared tags live on `RecordedTrace`; `into_samples()` does not flatten them, and `into_flat_samples()` does.
- Tag cardinality/value limits and sample-cap overflow are enforced and observable through trace counters.
- `phase::*` constants cover the foundation design's first lifecycle spine.
- `firkin-substrate`, `firkin-template`, `firkin-runtime`, `firkin`, and CLI/test consumers compile against `firkin-trace`.
- Targeted verification passes:
  - `cargo test -p firkin-trace`
  - `cargo test -p firkin-substrate benchmarks`
  - `cargo test -p firkin-cli benchmark`
  - `cargo test -p firkin-template`
  - `cargo check -p firkin-runtime`
  - `cargo check -p firkin-cli`

## Crate DAG

Allowed new edge:

```text
firkin-trace -> serde, thiserror, parking_lot, smallvec, async-trait, tokio
firkin-substrate -> firkin-trace
firkin-template -> firkin-trace
firkin-runtime -> firkin-trace
firkin -> firkin-trace
```

Forbidden edges:

```text
firkin-trace -/> firkin-substrate
firkin-trace -/> firkin-runtime
firkin-trace -/> firkin-core
firkin-trace -/> firkin-vmm
firkin-trace -/> firkin-oci
firkin-trace -/> firkin-vminitd-client
```

## Task 1: Trace Crate Red Tests

**Files:**
- Create: `crates/trace/Cargo.toml`
- Create: `crates/trace/src/lib.rs`
- Create: `crates/trace/tests/recorder.rs`
- Modify: `Cargo.toml`

**Step 1: Write failing tests**

Add tests for sample JSON compatibility, static/dynamic tags, span outcomes, drop cancellation, disabled recorders, shared-tag flattening, checkpoint pairs, sample overflow, no-runtime sampler attach, and async sampler close/drain.

**Step 2: Verify red**

Run:

```bash
cargo test -p firkin-trace
```

Expected: compile failure or test failures because the production API is not implemented.

## Task 2: Implement `firkin-trace`

**Files:**
- Modify: `crates/trace/src/lib.rs`

**Step 1: Implement benchmark primitives**

Add `BenchmarkMetricKind`, `BenchmarkUnit`, `BenchmarkSample`, `SampleTag`, and `SampleTags`. Keep `BenchmarkSample::new(...)` source-compatible while adding `from_static`, `with_static_tag`, and `with_dynamic_tag`.

**Step 2: Implement recorder bus**

Add `BenchProfile`, `Recorder`, `EnabledRecorder`, `RecordedTrace`, `RecorderStats`, `RecorderError`, `SampleClass`, `SamplerId`, `Tags`, and bounded bus insertion.

**Step 3: Implement drivers**

Add `Span`, `SpanOutcome`, `FailureClass`, `Sampler`, sampler task management, `Checkpoint`, and `phase` constants.

**Step 4: Verify green**

Run:

```bash
cargo test -p firkin-trace
```

Expected: all trace crate tests pass.

## Task 3: Hard-Cut Primitive Ownership

**Files:**
- Modify: `crates/substrate/Cargo.toml`
- Modify: `crates/substrate/src/lib.rs`
- Modify: `crates/substrate/tests/benchmarks.rs`

**Step 1: Move ownership**

Remove benchmark primitive definitions from substrate and import them from `firkin_trace`. Update summary/evidence internals to use public getters only. Extend `BenchmarkSummary` with p90, p99, and max while keeping current SLO gates p95-based.

**Step 2: Update tests**

Use `firkin_trace::{BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit}` directly in substrate tests. Assert nearest-rank p50, p90, p95, p99, and max behavior on a 100-sample series.

**Step 3: Verify substrate**

Run:

```bash
cargo test -p firkin-substrate benchmarks
```

Expected: benchmark summary, SLO, and evidence tests pass.

## Task 4: Update Workspace Consumers

**Files:**
- Modify: `crates/template/Cargo.toml`
- Modify: `crates/template/src/lib.rs`
- Modify: `crates/template/tests/executor.rs`
- Modify: `crates/runtime/Cargo.toml`
- Modify: `crates/runtime/src/lib.rs`
- Modify: `crates/runtime/tests/benchmarks.rs`
- Modify: `crates/runtime/tests/e2b_adapter.rs`
- Modify: `crates/runtime/tests/live_snapshot_restore.rs`
- Modify: `crates/firkin/Cargo.toml`
- Modify: `crates/firkin/src/lib.rs`
- Modify: `crates/cli/src/main.rs`

**Step 1: Update imports**

Consumers that construct samples import primitives from `firkin_trace`. The facade crate re-exports `firkin_trace` and exposes benchmark primitives at `firkin::{BenchmarkSample, BenchmarkMetricKind, BenchmarkUnit}`. CLI benchmark reports print p50, p90, p95, p99, and max for lifecycle and overhead artifacts.

**Step 2: Verify consumers**

Run:

```bash
cargo test -p firkin-template
cargo check -p firkin-runtime
cargo check -p firkin-cli
```

Expected: all pass without benchmark primitive imports from substrate.

## Task 5: Final Integrity Checks

**Files:**
- All modified files.

**Step 1: Check for old ownership**

Run:

```bash
rg -n "pub enum BenchmarkMetricKind|pub enum BenchmarkUnit|pub struct BenchmarkSample|firkin_substrate::\\{[^}]*Benchmark" crates
```

Expected: primitive definitions only in `crates/trace/src/lib.rs`; direct construction imports use `firkin_trace` or facade re-exports.

**Step 2: Check formatting and whitespace**

Run:

```bash
cargo fmt --check
git diff --check
```

Expected: clean.
