# Firkin Agent Benchmark Scorecard Surface Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the agent-sandbox benchmark scorecard a first-class Firkin surface: typed metric catalog, benchmark suite catalog, scorecard evidence artifacts, CLI reporting, and validation for the core requested metrics.

**Architecture:** `firkin-trace` remains the only owner of raw `BenchmarkSample`s. `firkin-evidence` owns validated evidence/report schemas and metric definitions. `firkin-benchmark` owns benchmark-suite declarations and artifact writers. `firkin-cli` exposes operator commands for listing suites, validating scorecard artifacts, and reporting p50/p90/p95/p99/max summaries.

**Tech Stack:** Rust workspace crates `firkin-trace`, `firkin-evidence`, `firkin-benchmark`, `firkin-cli`; serde JSON artifacts; existing `BenchmarkSummary` percentile logic and SLO gates.

---

## Success Criteria

- `firkin-evidence` exposes a stable catalog for the requested metric families: startup/readiness, exec/control-plane, host/guest/cgroup/balloon memory, CPU/pressure, disk throughput/metadata/reclaim, network, pids, pod, cache, isolation, cleanup, density, agent task, and power/thermal.
- `firkin-benchmark` exposes named suite definitions for `agent-core`, `startup`, `disk`, `memory`, `network`, `density`, `pod`, `agent-control`, `cleanup`, `isolation`, `cache`, `power`, and `abuse`.
- A scorecard artifact can be built from `BenchmarkSample`s and validates the required P0 dashboard metrics with count and percentile summaries.
- CLI commands can list the catalog/suites, write a scorecard artifact from sample JSON, validate a scorecard artifact, and print p50/p90/p95/p99/max report lines.
- Docs make clear what is runnable now versus live-VM/guest-agent probes that are declared and ready for harness implementations.
- Focused tests cover schema validation, suite coverage, CLI parsing/output, and artifact round-trip.
- Final verification runs `cargo fmt --check`, focused tests, `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets`, and `git diff --check`.

## Task 1: Metric Catalog

**Files:**
- Create: `crates/evidence/src/catalog.rs`
- Modify: `crates/evidence/src/lib.rs`
- Test: `crates/evidence/src/catalog.rs`

**Steps:**
1. Add `BenchmarkMetricGroup`, `BenchmarkRequirementLevel`, `BenchmarkMetricDefinition`, and static catalog arrays.
2. Include stable metric names from the requested P0 dashboard and drilldowns.
3. Add helpers for required scorecard metrics and lookup by name.
4. Test that all P0 metrics exist and names are unique.

## Task 2: Scorecard Evidence

**Files:**
- Create: `crates/evidence/src/scorecard.rs`
- Modify: `crates/evidence/src/lib.rs`
- Test: `crates/evidence/src/scorecard.rs`

**Steps:**
1. Add `AgentBenchmarkScorecardReport` with required metrics and `BenchmarkSummary`s.
2. Validate missing metric, wrong unit/kind, and sample count failures.
3. Add JSON artifact read/write helpers.
4. Test percentile summaries include p50/p90/p95/p99/max.

## Task 3: Suite Catalog

**Files:**
- Create: `crates/benchmark/src/suite.rs`
- Modify: `crates/benchmark/src/lib.rs`
- Test: `crates/benchmark/tests/benchmarks.rs`

**Steps:**
1. Add `BenchmarkSuiteDefinition` and `BenchmarkCaseDefinition`.
2. Declare suites for the requested startup, disk, memory, network, density, pod, control, cleanup, isolation, cache, power, abuse, and agent-realism surfaces.
3. Mark case execution as `HostRunnable`, `LiveVmRequired`, `GuestAgentRequired`, or `ExternalToolRequired`.
4. Test that `agent-core` contains all P0 scorecard metrics and that suite IDs are unique.

## Task 4: CLI Surface

**Files:**
- Modify: `crates/cli/src/main.rs`
- Test: `crates/cli/src/main.rs`

**Steps:**
1. Add `fk benchmark catalog`, `fk benchmark suites`, `fk benchmark write-scorecard`, `fk benchmark validate-scorecard`, and `fk benchmark report-scorecard`.
2. Keep existing lifecycle/overhead commands intact.
3. Emit compact line-oriented output for operator workflows.
4. Test parsing and representative output.

## Task 5: Documentation

**Files:**
- Modify: `crates/benchmark/README.md` if present or `crates/cli/README.md`
- Modify: `crates/evidence/README.md` if present or root relevant docs

**Steps:**
1. Document the default scorecard and suite list.
2. State which probes are fully runnable now and which require live VM/guest integration.
3. Link scorecard commands to sample and evidence artifact flow.

## Task 6: Verification

**Commands:**
- `cargo fmt --check`
- `cargo test -p firkin-evidence`
- `cargo test -p firkin-benchmark`
- `cargo test -p firkin-cli benchmark`
- `cargo check --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets`
- `git diff --check`
