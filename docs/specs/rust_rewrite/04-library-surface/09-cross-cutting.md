# Cross-cutting concerns

> Covers: Send/Sync guarantees, Drop semantics, cancellation model, `tracing` conventions, Cargo features across crates, MSRV policy, versioning policy, lint profile, testing discipline. The invariants that span every crate.
>
> Sources: [`scatter.md`](../../../../../../src/personal/beads-rs/docs/philosophy/scatter.md), [`error_design.md`](../../../../../../src/personal/beads-rs/docs/philosophy/error_design.md), [`type_design.md`](../../../../../../src/personal/beads-rs/docs/philosophy/type_design.md), [`trait_design.md`](../../../../../../src/personal/beads-rs/docs/philosophy/trait_design.md), [`test_design.md`](../../../../../../src/personal/beads-rs/docs/philosophy/test_design.md), [D-006](../DECISIONS.md#d-006--single-serial-dispatch-queue-per-vm), [D-007](../DECISIONS.md#d-007--beads-rs-philosophy-for-rust-style), [D-013](../DECISIONS.md#d-013--async-tokio-committed).

---

## 1. Send / Sync story

### 1.1 Public type matrix

| Type | `Send` | `Sync` | Notes |
|---|---|---|---|
| `Container` | ✓ | ✓ | Arc-internal state; ships across `tokio::spawn`, stored in registries |
| `ContainerBuilder<Vm, S>` | ✓ | ✗ | Consuming-self; no shared-state access pattern |
| `Process` | ✓ | ✓ | Same pattern as Container |
| `ExecConfig` | ✓ | ✓ | Plain value |
| `VirtualMachine<NotBooted>` | ✓ | ✗ | Owns a `VmConfig`; no concurrent access |
| `VirtualMachine<Running>` | ✓ | ✓ | Arc-internal; multiple readers via `&self` |
| `VmConfig`, `VmConfigBuilder` | ✓ | ✓ | Plain values |
| `Pty` | ✓ | ✗ | `AsyncRead`/`AsyncWrite` methods take `&mut self` |
| `ChildStdin`, `ChildStdout`, `ChildStderr` | ✓ | ✗ | Same reason |
| `VsockStream` | ✓ | ✗ | Same reason |
| `VsockListener` | ✓ | ✓ | `accept()` is on `&self`; concurrent accept is valid |
| `oci::Client`, `oci::ImageBundle`, `Reference` | ✓ | ✓ | Plain data; `Client`'s inner reqwest is `Send + Sync` |
| `ext4::Writer` | ✓ | ✗ | Consuming-self; no concurrent-write pattern |
| `oci::Error`, `vmm::Error`, `core::Error`, `ext4::Error` | ✓ | ✓ | `thiserror` + `#[source]` chains hold `Box<dyn Error + Send + Sync>` |
| All value types (`Size`, `Mount`, `Network`, etc.) | ✓ | ✓ | Value semantics |
| `AbortOnDrop<T>` | ✓ | ✓ where `T: Send + Sync` | Inherits from `T` |

### 1.2 Internal D-006 invariant

Every call into `objc2-virtualization` happens on `vmm`'s **serial dispatch queue** per [D-006](../DECISIONS.md#d-006--single-serial-dispatch-queue-per-vm). The crate's internal `Arc<VmCore>` mediates access from Rust; public types talk to the queue via tokio channels (completion-handler → `oneshot::Receiver`).

`VzSend<T>` (PRO_TIPS §1) wraps `!Send` `Retained<VZ*>` values that must cross thread boundaries under the "only touched on that queue" invariant. **`VzSend` is never public.** The public Rust API never sees an Obj-C retained directly; it's always behind internal state that enforces queue affinity.

### 1.3 Why concrete types, not traits

Per [`trait_design.md § most traits shouldn't exist`](../../../../../../src/personal/beads-rs/docs/philosophy/trait_design.md): a trait earns its existence when two distinct, real implementations exist. For `Container`, `VirtualMachine`, `Process`, there's one implementation; defining them as traits for mocking is exactly the kind of premature abstraction that spreads lies. Consumers mock at the layer above if they need test doubles.

---

## 2. Drop and cancellation — consolidated

### 2.1 Drop semantics: no async work

**`Drop` never calls `async fn`** on any public type. Reasons:

1. `Drop` is synchronous. Async cleanup requires either `block_on` (deadlocks on the same runtime) or `tokio::spawn` (silent failure on runtime shutdown).
2. `Drop` has no way to propagate errors. Silent "stop failed" is worse than no stop.
3. A user who dropped a live resource already has a misaligned mental model; implicit cleanup hides that rather than surfacing it.

What `Drop` does do:
- Aborts internal tokio relay tasks (sync; they self-terminate when their state drops).
- Closes fds (sync).
- Decrements internal Arc refs. Last ref triggers a best-effort async cleanup task on the current runtime, if any.
- Logs `tracing::warn!` if the resource was alive at drop time.

### 2.2 `AbortOnDrop<T>` opt-in wrapper

For users who want drop-means-stop semantics:

```rust
pub struct AbortOnDrop<T: StoppableAsync>(Option<T>);

impl<T: StoppableAsync> AbortOnDrop<T> {
    pub fn new(t: T) -> Self;
    pub fn into_inner(mut self) -> T;       // extract without triggering drop handler
}

impl<T: StoppableAsync> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        if let Some(t) = self.0.take() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Err(e) = t.stop_async().await {
                        tracing::warn!(?e, "AbortOnDrop: stop failed");
                    }
                });
            } else {
                tracing::warn!("AbortOnDrop: no tokio runtime; resource leaked until process exit");
            }
        }
    }
}
```

The bound `T: StoppableAsync` is the seam that decides which types the wrapper can wrap:

```rust
mod sealed { pub trait Sealed {} }

/// Resources that own teardown state and expose an async `stop_async()`.
/// Sealed: only `Container` and `VirtualMachine<Running>` implement it in v1.
/// Adding a new implementer is an API-surface decision, not something consumers
/// can do; that's why the supertrait is private.
pub trait StoppableAsync: sealed::Sealed + Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;
    fn stop_async(self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;
}

impl sealed::Sealed for Container {}
impl sealed::Sealed for VirtualMachine<Running> {}

impl StoppableAsync for Container {
    type Error = core::Error;
    fn stop_async(self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        async move { self.stop().await.map(|_| ()) }
    }
}

impl StoppableAsync for VirtualMachine<Running> {
    type Error = vmm::Error;
    fn stop_async(self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        self.stop()
    }
}
```

**Crate placement**: `StoppableAsync`, `sealed::Sealed`, `AbortOnDrop<T>`, and the two `impl StoppableAsync for …` blocks all live in **`firkin-core`**. This is the only crate that depends on both `firkin-vmm` (for `VirtualMachine<Running>`) and itself (for `Container`), so it's the only crate that can satisfy the orphan rule for both impls. `firkin-vmm` does *not* import `StoppableAsync` — it just exposes `VirtualMachine<Running>` as a plain type, and `firkin-core` does the wiring. Users who want `AbortOnDrop<VirtualMachine<Running>>` import it from `firkin` (the top-level crate re-exports it).

Why sealed: per [`trait_design.md § most traits shouldn't exist`](../../../../../../src/personal/beads-rs/docs/philosophy/trait_design.md), the trait only exists because we have *two* real implementers with identical drop-time semantics. Sealing it prevents consumers from opting third-party types into `AbortOnDrop`; if a consumer wants that pattern for their own type, they write their own wrapper (~20 LOC). Keeping the seam closed means we can evolve `stop_async()`'s exact signature without it being a public semver event.

Mirrors `tokio::task::AbortHandle` / `AbortOnDrop` conventions.

### 2.2a `CoreContainerFactory` — the twin pattern for `vm.container(…)` (D-018)

`VirtualMachine<Running>::container(id)` and `::container_shared(id)` return `ContainerBuilder<OnVm<'_>, Init>` and `ContainerBuilder<OnVmArc, Init>` respectively. Both `ContainerBuilder` variants are `firkin-core` types; `VirtualMachine<Running>` lives in `firkin-vmm`, which cannot import `core` (cycle). Same orphan-rule shape as `StoppableAsync`, same resolution: a sealed extension trait defined + impl'd in `core`.

```rust
// in firkin-core (re-exported from firkin):
pub trait CoreContainerFactory: sealed::Sealed {
    fn container<'a>(&'a self, id: impl Into<ContainerId>)
        -> ContainerBuilder<OnVm<'a>, Init>;

    fn container_shared(self: &Arc<Self>, id: impl Into<ContainerId>)
        -> ContainerBuilder<OnVmArc, Init>;
}

impl CoreContainerFactory for VirtualMachine<Running> { /* … */ }
```

**Sealed**, for the same reason as `StoppableAsync`: the seam exists only because two related but internal types need a shared shape (`OnVm` borrow-bound + `OnVmArc` Arc-shared); there is no third-party implementer we could imagine. Implementation is free to change signatures internally without public-semver implications.

**Import requirement**: users call `vm.container(…)` only when `CoreContainerFactory` is in scope. `use firkin::*;` or `use firkin::prelude::*;` handles it. Consumers using only `firkin-vmm` (no `core`) do not have the trait in scope, which is correct — they have no `Container` to spawn onto anyway.

**Why in `core`, not `vmm`**: the trait's return types (`ContainerBuilder<*, *>`) are `core`-owned. Putting the trait in `vmm` would require `vmm` to import `core`, which reverses the dep graph.

### 2.3 Cancellation — two modes

**Mode 1: Ad-hoc external cancellation via drop-future.**

Users wrap any `async fn` in `tokio::select!` / `tokio::time::timeout` / `tokio_util::sync::CancellationToken::run_until_cancelled`. Dropping the future cancels the operation cleanly.

**Library invariant enforcing drop-safety:**

> Every `async fn` in the public API owns its internal state in the future's stack frame. No `tokio::spawn` of tasks that outlive the parent future. RAII on port allocations, fd ownership, file locks. Dropping the future releases every held resource immediately without any work required from the caller.

This is a discipline contributors must follow for every new operation they implement. Violating it means drop-future-is-cancel stops being true, which breaks the Rust async cancellation model users expect.

**Mode 2: Cascading lifecycle cancellation via internal token tree.**

When `vm.stop()` or `container.stop()` is called, every in-flight operation in the stopped subtree observes cancellation at its next RPC boundary and returns `Error::Cancelled { reason: CancelReason }`.

The tree:
```
VirtualMachine<Running> (root CancellationToken)
 ├── Container (child)
 │   ├── operation: copy_in  (grandchild)
 │   ├── operation: exec     (grandchild)
 │   └── operation: dial_vsock (grandchild)
 └── Container (child)
     └── operation: wait     (grandchild)
```

Cancelling the root (VM stop) cascades to every child and grandchild. Cancelling a Container cancels only its operations.

**No user-facing token types in v1.** Users don't construct, clone, or cancel library-internal tokens. Cancellation is what `stop()` *is*. Users observe cascaded cancellation via `Error::Cancelled { reason }` in their error path. For ad-hoc external cancel, Mode 1 composes with standard tokio idioms.

### 2.4 Cancel-safety checklist for contributors

When implementing a new `async fn` in a public API, verify:

- [ ] All state allocated inside the function is on the future's stack frame, not in `tokio::spawn`'d tasks.
- [ ] Port allocations use RAII guards that release on drop.
- [ ] File locks use RAII (e.g., `fd4`'s `flock` or the `fs2` crate's guards).
- [ ] Any internal channels are scoped to the function (not stashed in struct state across calls).
- [ ] At every `.await` point, if the future is dropped, the library is in a consistent state.
- [ ] RPC operations that would leave partial guest-side state document what that state is (e.g., "partial file on guest at `guest_path`; caller may clean up").

---

## 3. `tracing` conventions

`tracing` is the instrumentation crate. The library emits spans + events; **the user installs a subscriber**. No library-level subscriber initialization except as the optional `vmm::logging::install_dev_subscriber()` helper.

### 3.1 Span instrumentation

Every public `async fn` is `#[tracing::instrument]`-ed at an appropriate level:

- `DEBUG` for common operations (`spawn`, `wait`, `kill`).
- `TRACE` for noisy inner ops (`dial_vsock`, per-byte copy progress).
- **Fields are structured, not formatted into messages**: `container_id`, `vm_id`, `operation`, `port`, `bytes_transferred`.

### 3.2 Event levels

| Level | When |
|---|---|
| `ERROR` | Only for bug-shaped conditions (internal invariant violated). World-state errors return `Result`; they do not log-and-continue. |
| `WARN` | Recoverable-but-notable: `container.drop.alive`, `AbortOnDrop: no runtime`, `snapshot restore: incompatible, falling back to cold boot`. |
| `INFO` | Durable state transitions: `vm.boot.success`, `container.spawn.success`, `container.stop.begin`. |
| `DEBUG` | Per-operation breadcrumbs that help debug end-to-end flows. |
| `TRACE` | Per-byte / per-frame detail. Off by default. |

### 3.3 Span / event naming

Dotted, hierarchical, searchable: `<crate>.<entity>.<event>`.

Examples:
- `vmm.vm.boot.begin`
- `vmm.vm.boot.success`
- `vmm.vm.boot.fail`
- `vmm.vsock.dial.begin`
- `vmm.vsock.accept`
- `core.container.spawn.begin`
- `core.container.exec.begin`
- `oci.client.pull.begin`
- `oci.client.pull.layer.downloaded`
- `oci.client.pull.cache_hit`
- `ext4.writer.finalize.success`

### 3.4 What we never do

- **No `eprintln!` / `println!`** in library code.
- **No `log::*` macros.** `tracing` only. (It auto-bridges from `log` via `tracing-log`, but we don't emit to `log`.)
- **No panic-then-log.** Panic is for bugs; world errors return `Result`.
- **No sensitive data in logs** without explicit user opt-in — credentials, layer contents, raw network payloads. If useful for debugging, gate behind a separate feature or env var (`FIRKIN_LOG_SECRETS=1`).

---

## 4. Feature flags — consolidated across all crates

| Crate | Feature | Default | Effect |
|---|---|---|---|
| `core` | `runtime-download` | off | vminitd fetched at runtime instead of embedded |
| `core` | `serde` | off | derives `Serialize`/`Deserialize` on public types |
| `core` | `integration-tests` | off | compiles + runs integration tier of tests |
| `vmm` | `rosetta` | off | `VZLinuxRosettaDirectoryShare` attachment + guest binfmt registration support; per-VM opt-in |
| `vmm` | `snapshot` | off | `save_snapshot` + `boot_or_restore` (pending S10 verification) |
| `vmm` | `balloon` | off | `VmConfig::memory_balloon(bool)` knob |
| `vmm` | `runtime-download` | off | same as `core`'s runtime-download |
| `vmm` | `integration-tests` | off | same as `core` |
| `oci` | `keychain` | **on (macOS only)** | `Auth::Keychain` via `Security.framework` |
| `oci` | `serde` | on | `oci-spec` types already serde; we inherit |
| `oci` | `rustls` | on | rustls TLS; alternative to system-tls |
| `oci` | `native-tls` | off | opt-in alternative |
| `ext4` | (none) | — | all features are compile-time in `Features` bitflags |

**Rule**: features toggle *attachments* or *alternative implementations*, never *semantics*. The same user code must compile regardless of which features are enabled, as long as the user doesn't call feature-gated items.

---

## 5. MSRV policy

- **Track stable. Bump freely** until v1.0.
- After v1.0: MSRV is documented in each crate's `Cargo.toml` via `rust-version` field, held at **N-2** (two stable releases behind current) for a rolling ~6-month window.
- MSRV bumps are minor-version bumps (semver-minor), not patch.
- Beta / nightly features only behind a `nightly` feature flag, off by default. **None expected in v1.**

---

## 6. Versioning policy

- **`v0.0.1-alpha` to start**. All crates. Explicit signal that the API is uncommitted and breaking changes can land in any minor bump. No `v1.0` until the API shape has burned in on real consumers.
- **Cargo semantics under `0.y.z`**: `0.y.z` → `0.y.(z+1)` is compatible (bug fixes, additions). `0.y.*` → `0.(y+1).*` is breaking. Users who want stability pin exact minor versions.
- **Per-crate version cadence** (not workspace-lockstep):
  - `ext4` ships its own version line (D-004 — independently publishable; may be adopted outside this project).
  - `oci` ships its own version line (general-purpose registry client).
  - `core`, `vmm`, `vsock`, `vminitd-client` are tightly coupled and coordinate releases; independent numbers but a given `core` release pins specific minors of the others.
- **Criteria for moving any crate to `1.0`**:
  1. Two independent real consumers have shipped non-trivial workloads on it without breaking-change pressure.
  2. The public API has gone 90+ days without a breaking-change PR merged.
  3. The `CHANGELOG.md` entries for the last three minor bumps contain no `BREAKING:` markers.
- **`1.0` doesn't have to be synchronized across crates.** `ext4` may reach 1.0 well before `core` — its surface is narrower.
- **CHANGELOG discipline**: every crate maintains its own `CHANGELOG.md`. `BREAKING:` prefix on every breaking-change line. `cargo release` or equivalent automates the bump; no manual version edits.

### 6.1 What users see in practice

```toml
# Unstable phase, user's Cargo.toml:
firkin        = "=0.3.1"    # exact pin for reproducibility
firkin-ext4   = "0.5"       # tolerate patch updates only
```

Bumping `firkin` from `0.3.1` to `0.4.0` is a **read-the-changelog** event. Bumping `0.3.1` → `0.3.2` is safe by semver contract.

---

## 7. Lint profile

Workspace-level `[workspace.lints]` in root `Cargo.toml`:

```toml
[workspace.lints.rust]
missing_docs = "warn"                     # deny post-v1 when docs complete
unsafe_code = "deny"                      # crate-level override in vmm, vsock, ext4 byte-layout modules
unused = "deny"
rust_2018_idioms = "deny"

[workspace.lints.clippy]
correctness = "deny"
suspicious  = "deny"
pedantic    = "warn"                      # has false positives; warn not deny
complexity  = "warn"
perf        = "warn"
style       = "warn"
# Explicit opt-outs from pedantic:
module_name_repetitions = "allow"
must_use_candidate = "allow"
missing_errors_doc = "allow"              # revisit post-v1
```

Per-crate overrides:

```toml
# In vmm/Cargo.toml:
[lints.rust]
unsafe_code = "allow"                     # needed for Obj-C boundary; contained within internal modules

# In vsock/Cargo.toml:
unsafe_code = "allow"                     # raw fd manipulation

# In ext4/Cargo.toml:
[lints.rust]
unsafe_code = "allow"                     # #[repr(C)] + bytemuck at byte-layout module boundaries only
```

CI runs:
```
cargo fmt --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
```

All four must pass before a PR merges.

---

## 8. The "push logic down" discipline

A cross-cutting rule that shapes how features get implemented across every crate. From [`test_design.md § 3`](../../../../../../src/personal/beads-rs/docs/philosophy/test_design.md):

> **A fact is asserted at the lowest level where it can be expressed cleanly.** If a validation can live in a newtype constructor, it does. If a decision can be made from pure-data input and tested without VZ or the network, it's extracted. Integration tests are the last place a fact should appear, not the first.

### 8.1 Concrete applications throughout this design

| Concern | Lowest layer that can express it |
|---|---|
| "Container IDs are valid" | `ContainerId::new()` — newtype constructor; rejected at type construction |
| "Reference syntax is valid" | `Reference::parse()` — pure function, tested with fixture strings |
| "VmConfig is internally consistent" | `VmConfigBuilder::build()` — pure function over the builder state |
| "Manifest list → platform descriptor" | pure function over `(ManifestList, Platform) -> Option<Descriptor>` — unit-tested against fixtures |
| "OCI layer produces correct ext4 layout" | `ext4::Writer` unit tests against tarballs |
| "VZ config assembly matches VmConfig" | pure function over `VmConfig -> VZVirtualMachineConfiguration` — unit-tested by inspecting the assembled config |
| "Cascading cancellation works" | Mode-2 unit test over the internal token tree — no VZ needed |
| "VM actually boots and runs a container" | Integration test — necessarily |

This is what makes the integration suite small ([`08-vmm-crate.md § testing`](./08-vmm-crate.md)): most logic is tested where it's cheap. Integration tests verify only the things that genuinely need VZ.

### 8.2 Four test shapes

From [`test_design.md § 2`](../../../../../../src/personal/beads-rs/docs/philosophy/test_design.md), every test in this codebase is exactly one of:

1. **Law** — ∀ input, property P holds. (e.g., `ext4::Writer` output byte-matches mkfs.ext4 for any valid feature set)
2. **Example** — for this specific input, this specific output / error happens. (e.g., "parsing `foo:1.2.3` yields `Reference { registry: docker.io, namespace: library/foo, tag: 1.2.3 }`")
3. **Scenario** — given this starting state and action sequence, these observable outcomes result. (e.g., "boot VM, spawn container, wait, exit status is 0")
4. **Regression** — this specific combination used to break; never again. (Named after incidents / issue numbers when possible.)

If you can't tag a new test as Law / Example / Scenario / Regression, the test isn't designed yet.

---

## 9. Target platform policy — consolidated

| Target | `core` | `vmm` | `vminitd-client` | `vsock` | `oci` | `ext4` | `types` |
|---|---|---|---|---|---|---|---|
| `aarch64-apple-darwin` | ✓ tested | ✓ tested | ✓ tested | ✓ tested | ✓ tested | ✓ tested | ✓ tested |
| `x86_64-apple-darwin` | ✓ best-effort | ✓ best-effort | ✓ best-effort | ✓ best-effort | ✓ best-effort | ✓ best-effort | ✓ best-effort |
| `aarch64-unknown-linux-gnu` | ✗ | ✗ | ✗ | ✓ tested | ✓ tested | ✓ tested | ✓ tested |
| `x86_64-unknown-linux-gnu` | ✗ | ✗ | ✗ | ✓ tested | ✓ tested | ✓ tested | ✓ tested |
| `*-windows-*` | ✗ | ✗ | ✗ | ✗ (AsyncFd unix-only) | ✓ probably | ✓ | ✓ |
| `*-wasm*` | ✗ | ✗ | ✗ | ✗ | ✗ | ✗ | ✓ probably |

Portable crates:
- `types` per [D-015](../DECISIONS.md#d-015--firkin-types-leaf-crate-for-shared-value-types) — pure value types, no system deps.
- `ext4` per [D-004](../DECISIONS.md#d-004--ext4-crate-is-the-source-of-truth-for-both-initblock-and-container-rootfs) — no OS-specific deps.
- `vsock` per [D-016](../DECISIONS.md#d-016--firkin-vsock-owns-streamlistener-types-vmm-depends-on-vsock) — uses `tokio::io::unix::AsyncFd` (Unix) but no VZ; loopback-testable against `tokio-vsock`.
- `oci` uses `reqwest` + `tokio`, both cross-platform.
- `vminitd-client`, despite depending only on `tokio` / `tonic` / `vsock` at the Rust level, is macOS-only in CI because the only real way to exercise it is against a running vminitd, which requires VZ.

This enables fast CI: unit tests for portable crates run on Linux runners (cheap, fast); macOS runners are reserved for `vmm` + `core` + integration tests.

---

## 10. Async runtime — why tokio, committed

[D-013](../DECISIONS.md#d-013--async-tokio-committed) commits the library to tokio. Reasons:

1. **VZ delegate callbacks + completion-handler-to-oneshot bridge** already require a reactor; S2's `VsockConnector` uses `tokio::io::unix::AsyncFd`. Runtime-agnostic would require an extra abstraction layer with real cost.
2. **tonic + hyper** are async-only and tokio-coupled. Bridging from tonic to a non-tokio runtime is a hostile code path.
3. **Concurrency inside the library** is real: host vsock listeners, RPC client, copy tasks, socket relays, VM state listeners. Sync doesn't eliminate them — it moves them to threads, which is worse for this workload.
4. **`tokio::process::Child`** is the shape we model `Container` on. Consistency is a UX win.

**Consequence for the public API**: types use concrete `tokio::io::{AsyncRead, AsyncWrite}`, not `futures::io::*` traits. `Arc<tokio::sync::Mutex<_>>` appears in some internal types (not public API) for the D-006 dispatch-queue bridge. Users need a `tokio::runtime::Runtime` (usually via `#[tokio::main]` or `tokio::runtime::Builder`).

**Users who can't use tokio** are rare on macOS-native code. They can write their own sync-blocking facade (roughly 100 LOC; `reqwest::blocking`-style). We don't ship it in v1 because (a) users rarely need it, (b) it's easy to add as a `blocking` submodule later, (c) maintaining both doubles surface.

---

## 11. CI shape — consolidated

Every PR runs:

- `cargo fmt --check`
- `cargo clippy --workspace --all-features --all-targets -- -D warnings`
- `cargo test --workspace` (unit tests only — default features)
- `cargo doc --workspace --no-deps`
- `ext4` golden-diff tests on Linux runner (with `--features golden-diff`)
- `vmm` + `core` smoke test on macOS runner (single boot + container spawn + echo; ~3 sec)

Scheduled / tag-triggered:
- Full integration tier (`cargo test --workspace --features integration-tests`) on macOS runner
- `ext4` full feature matrix tests on Linux runner
- Cross-platform build matrix (verify `oci` + `ext4` compile on Linux/Windows)
- Release workflow: tag-triggered; builds + codesigns + publishes

---

## 12. Invariants worth locking

1. Public types are `Send + Sync` where meaningful; internal `objc2` types never leak.
2. No async work in `Drop`. `AbortOnDrop<T>` for opt-in auto-stop.
3. Cascading cancellation via internal token tree; no user-facing token types in v1.
4. `tracing` only, with structured fields and dotted naming (`crate.entity.event`).
5. Cargo features toggle attachments or shipping strategies, never semantics.
6. MSRV tracks stable pre-v1; N-2 rolling post-v1.
7. Per-crate version cadence starting at `v0.0.1-alpha`.
8. Per-crate target policy; `types`, `ext4`, `vsock`, and `oci` portable for fast CI (D-015, D-016, D-004).
9. Lint profile enforced at workspace level; `unsafe_code` only in `vmm`, `vsock` (FD wrapping), and `ext4` byte-layout modules.
10. "Push logic down" — facts live at the lowest layer that can express them.
11. Four test shapes only: Law, Example, Scenario, Regression.
12. Tokio-committed; public API types use concrete tokio traits.
13. Orphan-rule-respecting extension traits (`StoppableAsync`, `CoreContainerFactory`) live in `firkin-core`; both are sealed and re-exported from `firkin`.

Proceed to [`10-non-goals.md`](./10-non-goals.md) for the consolidated deferral catalog.
