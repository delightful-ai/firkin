# Production Apple/VZ Substrate Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build Firkin into the production Apple/VZ substrate described in `docs/specs/rust_rewrite/07-production-substrate-goal.md`.

**Architecture:** CubeAPI owns E2B/Cube API semantics; Firkin owns Apple/VZ VM, container, rootfs, snapshot, capacity, and substrate mechanics. Snapshot restore is the primary session-create path. Firkin overhead must be measured separately from real VM/container/rootfs/snapshot resource cost.

**Tech Stack:** Rust 1.95, `firkin-substrate` control models, `firkin-core` VM/container orchestration, `firkin-cli`, `firkin-e2b` contract helpers, Apple Virtualization.framework, VZ snapshots, `jj` for version control.

---

### Task 1: Capacity Ledger Release And Promotion

**Files:**
- Modify: `crates/substrate/src/lib.rs`
- Modify: `crates/substrate/tests/capacity.rs`
- Modify: `crates/cli/src/main.rs`

**Step 1: Write failing tests**

Add tests beside the existing `capacity_ledger_*` tests:

```rust
#[test]
fn capacity_ledger_releases_active_and_warm_pool_reservations() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(32), Size::gib(500)));
    ledger.reserve_active(ResourceBudget::new(4, Size::gib(16), Size::gib(120))).unwrap();
    ledger.reserve_warm_pool(ResourceBudget::new(2, Size::gib(8), Size::gib(80))).unwrap();

    ledger.release_active(ResourceBudget::new(2, Size::gib(8), Size::gib(20)));
    ledger.release_warm_pool(ResourceBudget::new(1, Size::gib(4), Size::gib(30)));

    assert_eq!(ledger.active(), ResourceBudget::new(2, Size::gib(8), Size::gib(100)));
    assert_eq!(ledger.warm_pool(), ResourceBudget::new(1, Size::gib(4), Size::gib(50)));
}

#[test]
fn capacity_ledger_promotes_warm_pool_to_active_without_double_counting() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(32), Size::gib(500)));
    let request = ResourceBudget::new(2, Size::gib(8), Size::gib(80));
    ledger.reserve_warm_pool(request).unwrap();
    ledger.promote_warm_pool_to_active(request).unwrap();

    assert_eq!(ledger.warm_pool(), ResourceBudget::new(0, Size::bytes(0), Size::bytes(0)));
    assert_eq!(ledger.active(), request);
    assert_eq!(ledger.used(), request);
}
```

**Step 2: Run failing tests**

Run:

```bash
df -g /System/Volumes/Data /Users /tmp
RUSTUP_TOOLCHAIN=1.95.0 CARGO_TARGET_DIR=/tmp/firkin-target cargo test -q -p firkin-substrate capacity_ledger
```

Expected: fail because `release_active`, `release_warm_pool`, and `promote_warm_pool_to_active` do not exist.

**Step 3: Implement minimal API**

Add methods to `CapacityLedger`:

```rust
pub fn release_active(&mut self, budget: ResourceBudget) {
    self.active = self.active - budget;
}

pub fn release_warm_pool(&mut self, budget: ResourceBudget) {
    self.warm_pool = self.warm_pool - budget;
}

pub fn promote_warm_pool_to_active(
    &mut self,
    budget: ResourceBudget,
) -> std::result::Result<(), CapacityError> {
    if budget.cpus() > self.warm_pool.cpus() {
        return Err(CapacityError::Cpu { requested: budget.cpus(), available: self.warm_pool.cpus() });
    }
    if budget.memory() > self.warm_pool.memory() {
        return Err(CapacityError::Memory { requested: budget.memory(), available: self.warm_pool.memory() });
    }
    if budget.disk() > self.warm_pool.disk() {
        return Err(CapacityError::Disk { requested: budget.disk(), available: self.warm_pool.disk() });
    }
    self.warm_pool = self.warm_pool - budget;
    self.active = self.active + budget;
    Ok(())
}
```

Update `fk substrate acceptance-checklist` only if status meaning changes.

**Step 4: Verify**

Run:

```bash
RUSTUP_TOOLCHAIN=1.95.0 CARGO_TARGET_DIR=/tmp/firkin-target cargo test -q -p firkin-substrate capacity_ledger
RUSTUP_TOOLCHAIN=1.95.0 CARGO_TARGET_DIR=/tmp/firkin-target cargo fmt --check
```

Expected: capacity tests pass, formatting clean.

**Step 5: Commit**

```bash
jj describe -m "feat: extend Firkin capacity ledger"
jj new
```

---

### Task 2: Snapshot Artifact Manifest Types

**Files:**
- Modify: `crates/substrate/src/lib.rs`
- Create or modify: `crates/substrate/tests/snapshot_manifest.rs`
- Modify: `crates/cli/src/main.rs`

**Step 1: Write failing tests**

Add tests for a durable snapshot manifest:

```rust
#[test]
fn snapshot_manifest_distinguishes_base_and_continuation_snapshots() {
    let base = SnapshotArtifactManifest::base("repo-main", "/snapshots/base.vzstate");
    let continuation = SnapshotArtifactManifest::continuation("session-1", "/snapshots/followup.vzstate");

    assert_eq!(base.kind(), SnapshotArtifactKind::BaseTemplate);
    assert_eq!(continuation.kind(), SnapshotArtifactKind::Continuation);
}
```

**Step 2: Run failing tests**

Run targeted core test. Expected: missing types.

**Step 3: Implement minimal types**

Add public types for `SnapshotArtifactKind` and `SnapshotArtifactManifest` with getters for kind, logical id, path, and created timestamp placeholder. Keep persistence out of this task.

**Step 4: Update acceptance checklist**

Keep `template_build_snapshot` as `missing`; add evidence note that manifest substrate exists, not build execution.

**Step 5: Verify and commit**

Run targeted tests and `cargo fmt --check`, then:

```bash
jj describe -m "feat: add snapshot artifact manifest"
jj new
```

---

### Task 3: Warm Pool Model

**Files:**
- Modify: `crates/substrate/src/lib.rs`
- Create or modify: `crates/substrate/tests/warm_pool.rs`
- Modify: `crates/cli/src/main.rs`

**Step 1: Write failing tests**

Cover:
- warm pool key is repo/template/runtime-profile
- checkout promotes capacity from warm-pool to active
- expiration releases warm-pool capacity

**Step 2: Run failing tests**

Run targeted warm-pool tests. Expected: missing model.

**Step 3: Implement minimal model**

Add:
- `WarmPoolKey`
- `WarmPoolEntry`
- `WarmPoolLedger`

Use `CapacityLedger` for reservation and promotion. No VZ runtime calls yet.

**Step 4: Update acceptance checklist**

Move `warm_pool_lifecycle` from `missing` to `substrate_model_defined` only after tests cover maintain, checkout, and expire.

**Step 5: Verify and commit**

Run targeted tests, CLI checklist test, `cargo fmt --check`, then commit with:

```bash
jj describe -m "feat: model Firkin warm pools"
jj new
```

---

### Task 4: Template Build Job Model

**Files:**
- Modify: `crates/substrate/src/lib.rs`
- Create or modify: `crates/substrate/tests/template_build.rs`
- Modify: `docs/specs/rust_rewrite/07-production-substrate-goal.md`

**Step 1: Write failing tests**

Model a build plan with:
- repo URL/path
- checkout ref
- setup commands
- cache-warming commands
- snapshot output path

**Step 2: Run failing tests**

Expected: missing build job types.

**Step 3: Implement model only**

Add immutable value types. Do not clone repos or execute commands yet.

**Step 4: Verify and commit**

Run targeted tests and `cargo fmt --check`; commit:

```bash
jj describe -m "feat: model template build jobs"
jj new
```

---

### Task 5: Benchmark Result Schema

**Files:**
- Modify: `crates/substrate/src/lib.rs`
- Modify: `crates/cli/src/main.rs`
- Create or modify: `crates/substrate/tests/benchmarks.rs`

**Step 1: Write failing tests**

Test benchmark result records for:
- lifecycle latency
- Firkin overhead
- VM/container workload resources
- p50/p95 aggregation input shape

**Step 2: Run failing tests**

Expected: missing schema.

**Step 3: Implement schema**

Add structs only; no live measurement yet.

**Step 4: CLI output**

Add `fk substrate benchmark-schema` if useful, or keep schema internal until a runner exists.

**Step 5: Verify and commit**

Run targeted tests, CLI tests if touched, `cargo fmt --check`, then commit:

```bash
jj describe -m "feat: add substrate benchmark schema"
jj new
```

---

### Task 6: Restart Reconciliation Plan And Stubs

**Files:**
- Modify: `crates/substrate/src/lib.rs`
- Create or modify: `crates/substrate/tests/reconciliation.rs`
- Modify: `docs/specs/rust_rewrite/07-production-substrate-goal.md`

**Step 1: Write failing tests**

Represent restart state records for:
- active VM
- snapshot artifact
- log stream
- stale runtime process

**Step 2: Run failing tests**

Expected: missing reconciliation types.

**Step 3: Implement stubs**

Add reconciliation data types and decisions:
- recover
- cleanup
- quarantine

No host process scanning yet.

**Step 4: Verify and commit**

Run targeted tests and `cargo fmt --check`; commit:

```bash
jj describe -m "feat: model restart reconciliation"
jj new
```

---

## Execution Notes

- Check disk before each command. If `/System/Volumes/Data` has less than 10 GiB free, stop and clean disposable build output before continuing.
- Prefer targeted tests. Full workspace tests and live VZ smokes are expensive and should run only after the relevant slice is complete.
- Use `CARGO_TARGET_DIR=/tmp/firkin-target` for Rust checks in this checkout, then delete that directory if it threatens the disk floor.
- Commit every completed task with `jj describe ... && jj new`.
- Do not mark the production-substrate goal complete until `fk substrate acceptance-checklist` has no `missing`, `target_defined`, or weakly evidenced items.
