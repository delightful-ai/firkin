# `vmm` crate

> Covers: the crate boundary of `vmm` — what it exports vs hides, Cargo features, target platform matrix, codesigning + entitlements, preflight probe, relationship to `objc2-virtualization`, testing strategy.
>
> The user-facing API of `vmm` (`VirtualMachine`, `VmConfig`, `Network`, etc.) is documented in [`02-vm-surface.md`](./02-vm-surface.md); this file covers everything *around* that API that matters for crate design.

---

## 1. Scope

`vmm` is the **VZ-backed VM primitives** crate. It's the only crate in the workspace that:

- Links `objc2-virtualization` + `objc2-foundation` + `block2` + `dispatch2`.
- Owns VZ's serial dispatch queue ([D-006](../DECISIONS.md#d-006--single-serial-dispatch-queue-per-vm)) and the bridge from delegate callbacks to async Rust.
- Implements the inverse-vsock listener pattern for container stdio ([D-005](../DECISIONS.md#d-005--inverse-vsock-listener-for-container-stdio), PRO_TIPS §20).
- Carries the codesigning + entitlements manifest ([D-002](../DECISIONS.md#d-002--ad-hoc-codesigning-base-virt-entitlement-only), PRO_TIPS §29).

Everything it does is macOS-specific. `vmm` does not build or test on non-Apple platforms.

**Dependency on `firkin-vsock`** (per [D-016](../DECISIONS.md#d-016--firkin-vsock-owns-streamlistener-types-vmm-depends-on-vsock)): `vmm` produces `OwnedFd`s from VZ's connect + listener-delegate machinery and hands them to `firkin-vsock`'s constructors. The public `VsockStream` / `VsockListener` / `VsockPeer` types live in `vsock`; `vmm` re-exports them so users writing "just a microVM" code don't need to add `firkin-vsock` to their `Cargo.toml`.

**Dependency on `firkin-types`** (per [D-015](../DECISIONS.md#d-015--firkin-types-leaf-crate-for-shared-value-types)): `vmm` consumes `VsockPort`, `VirtiofsTag`, `VmId`, `Size`, etc. from the shared-leaf.

---

## 2. Public API — what users see from `vmm`

The full public surface (user-facing API spec'd in [`02-vm-surface.md`](./02-vm-surface.md)):

**Types owned by `vmm`**:
- `VirtualMachine<NotBooted>`, `VirtualMachine<Running>`
- `VmConfig`, `VmConfigBuilder`
- `Network`, `BootLog`, `KernelImage`
- `VmStatistics`, `VmPhase`
- `AbortOnDrop<VirtualMachine<Running>>`

**Types re-exported from other crates**:
- From `firkin-types` (D-015): `VirtiofsTag`, `VmId`, `VsockPort`
- From `firkin-vsock` (D-016): `VsockStream`, `VsockListener`, `VsockPeer`

**Error**: `vmm::Error`

**Traits owned by `vmm`**: **none** (concrete types only, per [D-007](../DECISIONS.md#d-007--beads-rs-philosophy-for-rust-style)).

**Traits that extend `vmm` types but live elsewhere**:
- `CoreContainerFactory` (in `firkin-core`, D-018) — adds `vm.container(…)` / `vm.container_shared(…)`.
- `StoppableAsync` (in `firkin-core`, per [`09-cross-cutting.md § 2.2`](./09-cross-cutting.md)) — powers `AbortOnDrop<VirtualMachine<Running>>`.
- Both are sealed and re-exported from `firkin`.

**Free functions**:
- `vmm::preflight() -> Result<Preflight, Error>` — capability probe (see §7).
- `vmm::signing::codesign_check(path) -> Result<CodeSignInfo, Error>` — dev helper.
- `vmm::logging::install_dev_subscriber()` — optional tracing subscriber for first-time consumers (opt-in).

---

## 3. Non-public — what stays inside `vmm`

Under no circumstances do these leak into any public signature:

- **Every `objc2-*` type.** `Retained<VZ*>`, `NSObject`, `&ProtocolObject<…>`, Obj-C class objects, autorelease pools.
- **The dispatch queue and its lifetime management.** `dispatch2::DispatchQueue`, `dispatch_main()`, queue-label CStr.
- **`VzSend<T>`** (PRO_TIPS §1) — the `unsafe impl Send` wrapper for `!Send` Obj-C retaineds crossed across thread boundaries under the "only touched on the VZ queue" invariant. Internal escape hatch; never public.
- **`VsockConnector`** (the `tower::Service<Uri>` from S2). Used by `vminitd-client`, not by library users.
- **Delegate subclasses** generated via `define_class!` — `VZVirtualMachineDelegate`, `VZVirtioSocketListenerDelegate`, `VZVirtualMachineStateDelegate` impls.
- **Internal `Arc<VmCore>`** state struct; the public `VirtualMachine<S>` wraps it.
- **Completion-handler-to-oneshot bridge helpers.**

**Enforcement**: every file in `src/` has either `mod private { … }` or `pub(crate)` on Obj-C-adjacent items. Public `pub` items go through `lib.rs`'s explicit re-export list, which a reviewer can audit at a glance.

---

## 4. Cargo features

```toml
[features]
default = []

# Enables VZLinuxRosettaDirectoryShare attachment + guest-side binfmt_misc
# registration support. VMs still opt in with VmConfig::rosetta(true).
rosetta = []

# Enables VirtualMachine<NotBooted>::boot_or_restore and
# VirtualMachine<Running>::save_snapshot. Depends on S10 spike verification
# before being marked production-stable.
snapshot = []

# Enables VmConfig::memory_balloon(bool) + runtime ballooning adjustments.
# Zero cost when unused; active knob when enabled (changes VM memory behavior).
balloon = []

# Switches the vminitd ELF from embedded (via include_bytes!) to runtime-downloaded
# from a pinned GitHub release. For binary-size-sensitive consumers. Default is
# embedded; this feature inverts to download-on-first-use.
runtime-download = []
```

**Rule**: features toggle *attachments* or *alternative shipping strategies*, never default runtime semantics. Rosetta support is not enabled by a default feature; callers opt in per VM with `VmConfig::rosetta(true)`.

### 4.1 Per-feature rationale

| Feature | Default | Why this default |
|---|---|---|
| `rosetta` | off | Cross-architecture emulation changes VM device setup and guest binfmt state; callers opt in explicitly. |
| `snapshot` | **off** | Active operations with their own failure modes; users opt in. Also awaits S10 verification. |
| `balloon` | off | Active knob that changes VM memory behavior (pages can move); users opt in |
| `runtime-download` | off | Default is to embed (D-003); this feature inverts that |

---

## 5. Target platform matrix

| Target | Status | Notes |
|---|---|---|
| `aarch64-apple-darwin` | **supported; tested** | Primary target. macOS 26+ only. |
| `x86_64-apple-darwin` | **supported; best-effort** | Builds; VZ works; Intel-macOS-specific gotchas not actively tested |
| `aarch64-unknown-linux-*` | **not supported** | Crate fails to compile. `objc2-virtualization` is macOS-only. |
| `x86_64-unknown-linux-*` | **not supported** | Same |
| `*-windows-*` | **not supported** | Same |
| `*-wasm*` | **not supported** | Same |

`Cargo.toml` sets `[package.metadata.docs.rs]` to macOS targets only so docs build correctly on docs.rs runners.

---

## 6. Codesigning + entitlements

Per [D-002](../DECISIONS.md#d-002--ad-hoc-codesigning-base-virt-entitlement-only) + [D-001](../DECISIONS.md#d-001--macos-26-only), the full feature set (NAT + vmnet-shared + Rosetta + virtiofs) works with **ad-hoc codesigning** and **only** `com.apple.security.virtualization`.

The crate ships:
- `vmm/resources/entitlements.plist` — the minimal plist.
- `vmm/resources/codesign.sh` — a reference script consumers can lift: `codesign --force --entitlements <plist> --sign - <binary>`.
- `vmm::preflight()` — runtime probe that detects missing codesigning and emits an actionable error message (see §7).

**Not auto-codesign during `cargo build`**: we don't invoke `codesign` in a build script because (a) it requires `/usr/bin/codesign` in PATH, (b) users have their own signing workflows (CI signing certs, reproducible builds, vendored tools), (c) sandboxed build environments block it, (d) it's an irreversible binary modification that should be explicit.

Instead: the crate README documents codesigning as a required final step; `preflight()` + first-boot runtime errors point at the script.

### 6.1 `vmm/resources/entitlements.plist`

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.virtualization</key>
    <true/>
</dict>
</plist>
```

That's it. No `com.apple.vm.networking` (would require a provisioning profile per D-002). No bridged entitlements.

---

## 7. `vmm::preflight()` — capability probe

```rust
pub fn preflight() -> Result<Preflight, Error>;

pub struct Preflight {
    pub macos_version: semver::Version,
    pub architecture: HostArch,
    pub nested_virtualization_supported: bool,
    pub rosetta_available: bool,
    pub codesigned: bool,
    pub has_virtualization_entitlement: bool,
}

pub enum HostArch {
    Arm64,        // Apple Silicon
    X86_64,       // Intel Mac
}
```

Useful for the *dev-facing CLI* (which can produce friendly "your Mac can't do this because X" error messages before trying to boot) and for *library consumers* who want to gate features on capability.

**What it checks**:
- `macos_version` via `sysctlbyname("kern.osproductversion")`.
- `architecture` via compile-time cfg.
- `nested_virtualization_supported` via `VZVirtualMachineConfiguration.nestedVirtualizationSupported`.
- `rosetta_available` via `VZLinuxRosettaDirectoryShare.availability`.
- `codesigned` via reading the running binary's Mach-O and looking for a `LC_CODE_SIGNATURE` load command.
- `has_virtualization_entitlement` via `SecTaskCopyValueForEntitlement` on our own task.

All checks are synchronous and cheap (< 10 ms total).

**Why in `vmm` rather than `core`**: probes things `vmm` owns (entitlements, VZ availability). Keeping it near the capability avoids duplicating the "what does this mean?" knowledge in two places (`scatter.md § local`).

---

## 8. Dependency on `objc2-virtualization`

```toml
# workspace dependencies (pinned):
[workspace.dependencies]
objc2 = "0.5"
objc2-foundation = "0.2"
objc2-virtualization = "0.3"   # pinned major; bumped deliberately via PR
block2 = "0.5"
dispatch2 = "0.2"
```

Reason for pinning: `objc2-virtualization` is auto-generated from Apple's framework headers. Minor-version bumps can surface API churn on Apple's side with real semantic impact (see PRO_TIPS §29 for the `VZVmnetNetworkDeviceAttachment::init` unavailability workaround).

Vendored locally at `~/vendor/github.com/madsmtm/objc2` per PRO_TIPS §8 (`objc2-generated` is a git submodule requiring `--recurse-submodules` on fresh clones). During development, `Cargo.toml` can use `path = "..."` deps; CI uses the crates.io version.

---

## 9. Testing strategy

### 9.1 Three tiers

From [`09-cross-cutting.md § testing`](./09-cross-cutting.md):

1. **Unit tests** (fast, runs everywhere `vmm` builds):
   - Config validation (every `VmConfigBuilder::build()` error path)
   - Typed-port arithmetic + reservation checks
   - VmId generation determinism
   - Every pure-logic function that doesn't touch VZ

2. **Integration tests** (macOS + codesigned binary, gated behind `--features integration-tests`):
   - VM boot + vsock dial (smoke)
   - Container spawn + wait
   - Pause/resume lifecycle
   - Multi-container on one VM
   - D-005 inverse-vsock stdio
   - Snapshot + restore (when `snapshot` feature on)

3. **Smoke test** (~3 seconds; runs on every PR):
   - Single boot, single container echo hello, single vsock round-trip.
   - Fast gate that catches "catastrophic regression" without waiting for full suite.

### 9.2 Speed optimization — snapshot-per-test

With the `snapshot` feature + S10 verification, integration tests become **~30 ms per test** (snapshot restore) instead of ~400 ms (cold boot). 50-test suite: ~3 s vs ~20 s.

The fixture pattern (in [`02-vm-surface.md § worked examples`](./02-vm-surface.md)) snapshots a booted VM once, restores per-test.

### 9.3 No mock `VirtualMachine`

Deliberate (applies [D-007](../DECISIONS.md#d-007--beads-rs-philosophy-for-rust-style) + [`trait_design.md § most traits shouldn't exist`](../../../../../../src/personal/beads-rs/docs/philosophy/trait_design.md)):

> We do NOT provide a test-double or mock `VirtualMachine` in v1. A mock would either re-implement most of the state machine (maintenance burden; silently drifts from real behavior) or short-circuit most of the behavior (low test value). Consumers who want to unit-test their orchestration logic should extract a layer above `VirtualMachine` with their own trait, mock that, and integration-test the real `vmm` path separately.
>
> This may look like a D-007 violation ("no trait without two impls"), but it's the opposite: we explicitly decline to create a seam that would exist only for mocking.

---

## 10. The `vmm::signing` module

Thin dev-facing helpers for codesigning invocation:

```rust
pub mod signing {
    pub fn codesign_check(binary_path: impl AsRef<Path>) -> Result<CodeSignInfo, Error>;
    pub fn codesign_with_entitlements(
        binary_path: impl AsRef<Path>,
        entitlements_plist: impl AsRef<Path>,
        identity: SigningIdentity,
    ) -> Result<(), Error>;
}

pub struct CodeSignInfo {
    pub signed: bool,
    pub identity: Option<String>,
    pub entitlements: Vec<String>,
}

pub enum SigningIdentity {
    AdHoc,
    Identity(String),       // e.g. "Apple Development: User (ABC123)"
}
```

Used by:
- The dev CLI (`cli check-codesign`, `cli codesign --target /path/to/bin`).
- Integration tests (sign test binaries at test-runner startup).

Not useful for library consumers directly (users have their own signing workflows), but harmless to expose.

---

## 11. The `vmm::logging` module

```rust
pub mod logging {
    /// Install a reasonable `tracing` subscriber that makes sense for VZ debugging:
    /// - VM state transitions at INFO
    /// - VZ delegate callback events at DEBUG
    /// - Obj-C boundary calls at TRACE (very verbose)
    ///
    /// Respects RUST_LOG env var. Opt-in; does nothing automatically.
    pub fn install_dev_subscriber();
}
```

Purely a convenience for users who don't want to hand-roll a `tracing-subscriber` setup. Library code itself uses plain `tracing::*` macros and relies on the user's subscriber.

---

## 12. The Obj-C one-way door

**Invariant**: from `vmm`'s public boundary outward, **no caller should need `objc2` in their dependency tree**. This is load-bearing:

- Every return that would ideally be `Retained<VZ*>` is wrapped in an opaque `vmm` type.
- Every `!Send` / `!Sync` VZ retained is held behind internal `Arc<Mutex<VmCore>>`; the public `VirtualMachine<Running>` is `Send + Sync`.
- Panics, errors, and bridged completion-handlers are all converted to Rust-native types before crossing the crate boundary.

**Enforcement**: if a reviewer ever sees `Retained<_>`, `DispatchQueue`, `&ProtocolObject<…>`, `block2::*`, or `objc2_foundation::*` in a `pub fn` / `pub struct` signature, that's the signal to add an internal wrapper. No exceptions.

Rationale: this is a one-way door. Once `objc2` leaks into the API, every consuming crate depends on it transitively — and `objc2`'s API model (retained pointers, main-thread-only protocols, memory management conventions) leaks into user code. We chose a Rust-native facade; we preserve that choice at the boundary.

---

## 13. Invariants worth locking

1. `vmm` public surface is exactly what's documented in [`02-vm-surface.md`](./02-vm-surface.md) + the small helpers in this file (`preflight`, `signing::*`, `logging::install_dev_subscriber`).
2. No `objc2::*`, `Retained<_>`, `DispatchQueue`, or `block2::*` in any public signature.
3. Platform matrix: `aarch64-apple-darwin` tested, `x86_64-apple-darwin` best-effort, nothing else compiles.
4. Codesigning is a documented caller responsibility, not a build-step action.
5. No mock `VirtualMachine` in v1; users mock at the layer above.
6. Cargo features toggle attachments or shipping strategies, not semantics.
7. `objc2-virtualization` pinned; minor bumps are deliberate PRs.
8. `preflight()` returns a struct, not panics — friendly capability probe.
9. `vmm` depends on `firkin-vsock` (D-016); VZ produces `OwnedFd`s, `vsock` owns the user-visible stream types.
10. `VirtualMachine<Running>::container(…)` / `::container_shared(…)` are *not* inherent methods — they are defined on the `CoreContainerFactory` trait in `firkin-core` (D-018). `vmm` does not import `core`.
11. `VmConfigBuilder::block_device(path)` is the declaration site for rootfs attachments used by the multi-container-per-VM path (D-019); runtime block-device attach is not in v0.1.

Proceed to [`09-cross-cutting.md`](./09-cross-cutting.md) for Send/Sync, drop, cancellation, tracing, versioning, and lint discipline.
