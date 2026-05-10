# Firkin P0 Benchmark Iteration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

> **Status Update:** This plan captured the first P0 benchmark loop. For the
> next e2e measurement-readiness pass, use
> `docs/plans/2026-05-07-firkin-decision-grade-metrics-implementation-plan.md`
> as the authoritative implementation plan. That follow-up hard-cuts ambiguous
> smoke-era metric names, derives headline metrics from one event trace, and
> adds sample-count, fault-injection, disk-stage, and stability gates.

**Goal:** Make Firkin's P0 benchmark loop seamless enough that an optimization sprint can start from one documented command sequence, produce trusted evidence, rank the next code owner to change, and leave a durable markdown record.

**Architecture:** Keep raw samples and phases in `firkin-trace`, scorecard definitions and ownership policy in `firkin-evidence`, suite declarations and benchmark helpers in `firkin-benchmark`, and operator orchestration in `firkin-cli`. Use a hard cutover metric contract: one P0 table, one suite list, one coverage gate, no aliases or compatibility names.

**Tech Stack:** Rust workspace crates `firkin-trace`, `firkin-evidence`, `firkin-benchmark`, `firkin-cli`; signed-live Apple/VZ harness through `scripts/run-signed-live-runtime-test.sh`; markdown sprint record under `docs/artifacts/`.

---

## Objective Behavior Validation

The work is done when these commands give the operator all information needed to iterate on P0 without rediscovering the workflow:

```bash
cargo run -q -p firkin-cli -- benchmark doctor --mode signed-live
cargo run -q -p firkin-cli -- benchmark p0-contract
cargo run -q -p firkin-cli -- benchmark phase-owners
cargo run -q -p firkin-cli -- benchmark sprint-record \
  --suite agent-core \
  --baseline local-agent-core-60s \
  --current-artifact target/firkin-live-evidence/current-60s.json \
  --overhead-artifact target/firkin-live-evidence/overhead-60s.json \
  --out docs/artifacts/firkin-p0-sprint-current.md
```

Expected pass/fail signal:

- `doctor` prints every prerequisite bucket and fails before writing benchmark artifacts when roots, disk, VZ/signing, guest PSI, or harness files are missing.
- `p0-contract` prints the single P0 metric set used by catalog, coverage, and `agent-core`.
- `phase-owners` prints metric prefix or exact metric ownership plus phase labels, so compare output is not a hidden CLI-only heuristic.
- `sprint-record` writes a markdown artifact with doctor summary, coverage command, compare command, current bottleneck, sample confidence, residual risks, and the first 30s command to run next.

Residual risks:

- Signed-live Apple/VZ commands remain machine-state sensitive.
- Existing proof artifacts may contain stale transcripts; the sprint markdown is the current working artifact.
- p95/p99 are optimization-grade only when sample thresholds are met.

## Task 1: Hard-Cut P0 Metric Contract

**Files:**

- Modify: `docs/plans/archived/2026-05-07-firkin-trustworthy-performance-loop-spec.md`
- Modify: `docs/plans/archived/2026-05-07-firkin-trustworthy-benchmark-milestones.md`
- Modify: `crates/evidence/src/catalog.rs`
- Modify: `crates/benchmark/src/suite.rs`
- Modify: `crates/cli/src/main.rs`

**Steps:**

1. Pick the implemented names as the canonical P0 surface unless a metric is missing from implementation and is required for iteration.
2. Add `sandbox.cleanup.orphan_count` to P0 only if it has a real artifact source and strict coverage can enforce it immediately; otherwise keep it `Core` and update the spec table to the 18 implemented P0 metrics.
3. Add a CLI command `benchmark p0-contract` that prints one line per P0 metric: name, group, kind, unit, suite case id, measurement source, and current accepted status.
4. Add tests proving `P0_SCORECARD_METRICS`, `AGENT_CORE_CASES`, and `p0-contract` contain exactly the same P0 set.

**Verification:**

```bash
cargo test -p firkin-evidence catalog
cargo test -p firkin-benchmark suite
cargo test -p firkin-cli p0_contract
```

## Task 2: First-Class Phase Ownership

**Files:**

- Modify: `crates/evidence/src/catalog.rs`
- Modify: `crates/cli/src/main.rs`
- Modify: `docs/plans/archived/2026-05-07-firkin-trustworthy-performance-loop-spec.md`

**Steps:**

1. Move the compare ownership policy out of ad hoc CLI prefix matching into a small explicit table or function exposed by `firkin-evidence`.
2. Each row must include metric match, phase label, owner crate/module string, and next-action hint.
3. Update compare output to print `phase=... owner=... next_action=...`.
4. Add `benchmark phase-owners` to print the table directly.
5. Add tests for exact metrics, prefix metrics, and fallback ownership.

**Verification:**

```bash
cargo test -p firkin-evidence ownership
cargo test -p firkin-cli benchmark_compare
cargo run -q -p firkin-cli -- benchmark phase-owners
```

## Task 3: Doctor and Artifact Policy Documentation

**Files:**

- Modify: `docs/plans/archived/2026-05-07-firkin-trustworthy-benchmark-milestones.md`
- Modify: `.gitignore`
- Create: `docs/artifacts/README.md`

**Steps:**

1. Document exactly what `benchmark doctor` checks today: configured roots, free disk, VZ host, signing/entitlements, signed-live harness files, guest PSI readiness, and vminitd bytes.
2. Configure ignore policy so generated live evidence under `target/` stays ignored, generated proof HTML under `docs/artifacts/*.html` is ignored, and committed markdown proof/sprint records remain trackable.
3. Document that `docs/artifacts/*.md` is the durable operator record format unless the task specifically needs HTML.

**Verification:**

```bash
git check-ignore -v docs/artifacts/firkin-benchmark-m3-proof.html
git check-ignore -v docs/artifacts/firkin-p0-sprint-current.md || true
git diff --check
```

## Task 4: Markdown Sprint Record Command

**Files:**

- Modify: `crates/cli/src/main.rs`
- Create or update: `docs/artifacts/firkin-p0-sprint-current.md`

**Steps:**

1. Add `benchmark sprint-record` with `--suite`, `--baseline`, `--current-artifact`, `--overhead-artifact`, optional `--scorecard-artifact`, `--out`, and `--min-free-bytes`.
2. The command should run or embed doctor, overhead SLO, strict coverage, compare, and sprint-ready logic without changing live artifacts.
3. The markdown must include exact commands, artifact paths, top bottleneck row, confidence label, residual risks, and next 30s command.
4. Missing artifact paths must fail with a useful error before writing a passing record.

**Verification:**

```bash
cargo test -p firkin-cli sprint_record
cargo run -q -p firkin-cli -- benchmark sprint-record \
  --suite agent-core \
  --baseline local-agent-core-60s \
  --current-artifact target/firkin-live-evidence/current-60s.json \
  --overhead-artifact target/firkin-live-evidence/overhead-60s.json \
  --out docs/artifacts/firkin-p0-sprint-current.md
```

## Task 5: Final Gates

**Commands:**

```bash
cargo fmt --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
scripts/check-firkin-crate-graph.sh
git diff --check
```

For live closure, also run:

```bash
cargo run -p firkin-cli -- benchmark run agent-core --mode signed-live --duration 60s --out target/firkin-live-evidence/current-60s.json
cargo run -p firkin-cli -- benchmark run overhead --mode signed-live --duration 60s --out target/firkin-live-evidence/overhead-60s.json
cargo run -q -p firkin-cli -- benchmark sprint-record --suite agent-core --baseline local-agent-core-60s --current-artifact target/firkin-live-evidence/current-60s.json --overhead-artifact target/firkin-live-evidence/overhead-60s.json --out docs/artifacts/firkin-p0-sprint-current.md
```
