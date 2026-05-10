# Virtual machine surface

> Covers: `VmConfig`, `VirtualMachine<NotBooted>` / `<Running>`, attachments, ownership story (borrow-bound vs Arc-shared), pause/resume, snapshot/restore, vsock dial and listen, drop semantics.
>
> Prerequisites: [`README.md`](./README.md) and [`01-container-surface.md`](./01-container-surface.md).

---

## 1. Overview

The VM surface sits below the container surface and exists for three user scenarios:

1. **Multi-container per VM.** Q2/C "pods without a Pod" — user boots one VM, spawns N containers on it. Shared-kernel semantics for mounts; shared resource pool; single shutdown verb.
2. **Just a microVM.** Users who want VZ-backed Linux VMs for purposes other than OCI containers (vfkit-shaped use cases: boot a kernel, run a single-binary guest, talk over vsock). They import `vmm` directly, use `VirtualMachine` without ever touching `Container`.
3. **Fine-grained config.** Container users who need VM-level knobs (custom kernel, memory ballooning, explicit network subnet) that aren't in the `ContainerBuilder<ImplicitVm>` defaults.

Three visible types:

| Type | Role |
|---|---|
| `VmConfig` (+ `VmConfigBuilder`) | Value-typed VM configuration, validated at `build()` time. |
| `VirtualMachine<NotBooted>` | Pre-boot VM handle. Owns the config. Consumed by `.boot()`. |
| `VirtualMachine<Running>` | Booted VM handle. Exposes container-spawning, vsock, pause/resume, stop. |

All types live in the `vmm` crate and are re-exported from `core` for consumer ergonomics.

---

## 2. `VmConfig` — configuration value

Every field has a sensible default; no typestate required on the builder. Validation happens in `.build()`, returns `Result<VmConfig, vmm::Error>`.

### 2.1 Construction

```rust
pub struct VmConfig { /* private */ }

impl VmConfig {
    pub fn builder() -> VmConfigBuilder;
    pub fn default() -> Self;                // 4 CPUs, 1 GiB RAM, NAT, bundled kernel, no extras
}

pub struct VmConfigBuilder { /* private */ }

impl VmConfigBuilder {
    // ─── resources ────────────────────────────────────────────────────────
    pub fn cpus(self, n: NonZeroU32) -> Self;              // default NonZeroU32::new(4).unwrap()
    pub fn memory(self, size: Size) -> Self;               // default Size::gib(1); min 128 MiB

    // ─── networking ───────────────────────────────────────────────────────
    pub fn network(self, n: Network) -> Self;              // append; multiple -> eth0, eth1, ...
    pub fn networks(self, ns: impl IntoIterator<Item = Network>) -> Self;

    // ─── storage / filesystem shares ──────────────────────────────────────
    pub fn virtiofs_share(
        self,
        tag: impl Into<VirtiofsTag>,
        host: impl Into<PathBuf>,
    ) -> Self;

    /// Pre-declare a block device; returns the updated builder plus a typed
    /// `BlockDeviceId` handle. Hand the handle to `Rootfs::block_device(id)`
    /// on a container built with `vm.container(id)` / `vm.container_shared(id)`
    /// (D-022, D-023). Required for the multi-container-per-VM path: every
    /// rootfs on an `OnVm`/`OnVmArc` builder must reference an id obtained
    /// this way. The single-use `Container::builder(id)` → `ImplicitVm` path
    /// declares its rootfs at `.spawn()` time and does *not* use this method.
    ///
    /// Can be called multiple times; each invocation attaches one block
    /// device. Order is preserved (first declared → `/dev/vdb`, second →
    /// `/dev/vdc`, …; `/dev/vda` is reserved for `init.block`).
    pub fn block_device(self, path: impl Into<PathBuf>) -> (Self, BlockDeviceId);

    // ─── cross-arch ───────────────────────────────────────────────────────
    pub fn rosetta(self, enabled: bool) -> Self;
    // Attaches VZLinuxRosettaDirectoryShare and wires SetupEmulator on the guest.
    // Default false; callers opt in with `.rosetta(true)`.

    // ─── power-user knobs ─────────────────────────────────────────────────
    pub fn nested_virtualization(self, on: bool) -> Self;  // default false
    pub fn boot_log(self, b: BootLog) -> Self;             // serial-console sink; default None
    pub fn kernel(self, k: KernelImage) -> Self;           // default KernelImage::bundled()
    pub fn cmdline_extra(self, s: impl Into<String>) -> Self; // append to kernel cmdline

    // ─── memory ballooning (requires `balloon` Cargo feature) ────────────
    #[cfg(feature = "balloon")]
    pub fn memory_balloon(self, enabled: bool) -> Self;

    // ─── finalization ────────────────────────────────────────────────────
    pub fn build(self) -> Result<VmConfig, vmm::Error>;    // validates; returns Config error
}
```

### 2.2 Validation done in `build()`

- Zero CPUs is a compile error (type is `NonZeroU32`); no runtime variant exists.
- `memory < 128 MiB` → `Error::InvalidConfig { reason: "memory must be ≥128 MiB" }`.
- Duplicate virtiofs tags → `Error::InvalidConfig { reason: "virtiofs tag X used twice" }`.
- More than 8 network attachments (VZ limit; empirically less on older macOS) → `Error::InvalidConfig`.
- More than 63 block devices (VZ virtio-blk limit) → `Error::InvalidConfig`.
- `block_device(path)` where `path` does not exist or is not readable → `Error::InvalidConfig { reason: "block_device: <path> not accessible" }`. Reported eagerly at `build()` so the `BlockDeviceId` handed back only exists for a validated declaration.
- `nested_virtualization(true)` on an unsupported machine → deferred to `boot()`, surfaces as `Error::NestedVirtNotSupported`.

Validation is *eager where cheap*, *lazy where it requires VZ interaction*. Catches fat-finger bugs early without requiring a VZ round-trip just to validate.

### 2.3 Referenced value types

Defined in full in [`04-value-types.md`](./04-value-types.md); listed here for orientation:

| Type | What it represents | Summary |
|---|---|---|
| `Network` | A single NIC attachment | Enum: `Nat`, `VmnetShared { subnet }`, future `Bridged` |
| `VirtiofsTag` | Tag for a virtiofs share | Newtype; printable ASCII, ≤36 bytes. Compile-time `virtiofs_tag!(…)` macro for literals (D-027) |
| `BootLog` | Serial-console sink | Enum: `File(PathBuf)`, `Writer(Box<dyn AsyncWrite>)` |
| `KernelImage` | Kernel image to boot | Opaque type; `bundled()` or `from_file(path)` |
| `BlockDeviceId` | Typed handle returned by `VmConfigBuilder::block_device(path)` | Newtype `Copy`; consumed by `Rootfs::block_device(id)` on `OnVm` builders (D-022) |

---

## 3. `VirtualMachine<S>` — typestate for boot state

### 3.1 Types

```rust
pub struct VirtualMachine<S = NotBooted> { /* private */ }

// Marker types (sealed in the crate; users can't implement).
pub struct NotBooted;
pub struct Running;
```

The `Paused` state is **runtime-tracked inside `Running`**, not a third typestate. Rationale in §4 of this file.

### 3.2 Constructor

```rust
impl VirtualMachine<NotBooted> {
    pub fn new(config: VmConfig) -> Self;
}
```

Cheap: just wraps the config. Acquires no resources. Dropping a `NotBooted` is free.

### 3.3 Boot transition

```rust
impl VirtualMachine<NotBooted> {
    pub async fn boot(self) -> Result<VirtualMachine<Running>, vmm::Error>;
}
```

`.boot()` does, in order:

1. Assemble the `VZVirtualMachineConfiguration` on the serial dispatch queue ([D-006](../DECISIONS.md#d-006--single-serial-dispatch-queue-per-vm)), using the kernel + init.block paths that `VmConfig` already carries. (The caller populates these; see "Caller's side" below.)
2. Call VZ's `validateWithError:` — any config issues surface here as `Error::InvalidDeviceConfig { reason }`.
3. Construct the `VZVirtualMachine`, install the delegate subclass, start on the queue with completion-handler bridged to `tokio::sync::oneshot`.
4. Wait for the VZ delegate to report `running` state.
5. Dial vminitd on vsock 1024. Wait for the gRPC service to become responsive (short timeout; usually sub-second).
6. Return `VirtualMachine<Running>`.

If any step fails, cleanup runs in reverse: stop VM, release delegate, free dispatch queue if owned, return the error with context.

**Caller's side** (what happens *before* `.boot()` is called — lives in [`firkin-core`](./08-vmm-crate.md#1-scope), not `vmm`):

- Resolve the kernel image (bundled lookup or from-file).
- Synthesize `init.block` from the bundled vminitd ELF via the `ext4` crate if no cache entry exists ([D-003](../DECISIONS.md#d-003--embed-vminitd-elf-not-initblock)). Cache key is SHA-256 of the vminitd ELF. If cache entry exists, this step is O(stat).
- Populate `VmConfig` with concrete filesystem paths for the resolved kernel and init.block.

This split respects the crate dependency graph: `vmm` doesn't depend on `ext4` or `vminitd-bytes`. The bundled-artifact-resolution and init.block-synthesis pipeline is `core`'s responsibility; `vmm::boot()` receives ready-to-boot paths. In a library-consumer use-case that doesn't need container runtime (e.g., "just a microVM"), the consumer bypasses the init.block step entirely and boots with whatever rootfs they want.

### 3.4 Boot-from-snapshot variant

Requires the `snapshot` Cargo feature (off by default, pending S10 verification — see [`02-spike-plan.md § S10`](../02-spike-plan.md)).

```rust
#[cfg(feature = "snapshot")]
impl VirtualMachine<NotBooted> {
    /// Construct VM from config; if the snapshot file exists and matches the config,
    /// restore from it instead of cold-booting. Falls back to cold boot if the snapshot
    /// is missing or incompatible.
    pub async fn boot_or_restore(
        self,
        snapshot_path: impl AsRef<Path>,
    ) -> Result<VirtualMachine<Running>, vmm::Error>;
}
```

Typically 10–30× faster than cold boot for our workload (sub-100ms vs ~400ms). Primary use case: integration-test fixtures (save snapshot once, every test restores). Secondary: user-facing "warm start" workflows.

Incompatibility cases that trigger fallback-to-cold-boot:
- VmConfig field mismatch (different memory size, different kernel).
- VZ snapshot format incompatibility (macOS version differences).
- Snapshot corrupted.

All fallbacks log `tracing::warn!` with the reason; boot still succeeds.

---

## 4. Running VM — methods

### 4.1 Identity and introspection

```rust
impl VirtualMachine<Running> {
    pub fn id(&self) -> &VmId;                    // opaque UUID; library-generated
    pub fn cpus(&self) -> NonZeroU32;
    pub fn memory(&self) -> Size;
    pub fn config(&self) -> &VmConfig;            // returns the effective config as booted
    pub fn is_paused(&self) -> bool;              // runtime-tracked phase
    pub fn state(&self) -> VmPhase;               // running | paused | stopping
}

pub enum VmPhase { Running, Paused, Stopping }
```

### 4.2 Container factory — the two variants (D-018)

This is the load-bearing design choice from [`README.md § 2.4`](./README.md) and [01-container-surface.md § 2.2 entry points](./01-container-surface.md). Two factory methods, two ownership stories. Both live on the sealed **`CoreContainerFactory`** extension trait defined in `firkin-core` — *not* on `VirtualMachine<Running>`'s inherent impl.

**Why a trait?** `ContainerBuilder` is defined in `firkin-core`. `VirtualMachine<Running>` is defined in `firkin-vmm`. `core → vmm` in the dep graph; `vmm` cannot import `ContainerBuilder`. The same orphan-rule dance used for `StoppableAsync` (see [`09-cross-cutting.md § 2.2`](./09-cross-cutting.md)) applies here: the trait *and* its impls both live in `firkin-core`, which is the only crate that sees both sides. The trait is re-exported from `firkin`, so `use firkin::*;` brings it into scope automatically for downstream users.

```rust
// in firkin-core (re-exported from firkin):
mod sealed { pub trait Sealed {} }

pub trait CoreContainerFactory: sealed::Sealed {
    /// Borrow-bound container. The resulting ContainerBuilder borrows &'a self;
    /// after spawn, Container holds a shared reference whose lifetime is tied to
    /// the VM's local scope. Default, ergonomic, natural Rust ownership semantics.
    fn container<'a>(&'a self, id: impl Into<ContainerId>)
        -> ContainerBuilder<OnVm<'a>, Init>;

    /// Arc-shared container. Requires self: &Arc<Self>; the resulting Container
    /// holds an independent Arc clone of the VM state, allowing cross-scope sharing.
    /// User writes `Arc::new(vm)` explicitly — Arc semantics are visible in user code.
    fn container_shared(self: &Arc<Self>, id: impl Into<ContainerId>)
        -> ContainerBuilder<OnVmArc, Init>;
}

impl sealed::Sealed for VirtualMachine<Running> {}
impl CoreContainerFactory for VirtualMachine<Running> { /* … */ }
```

**Import requirement:** users must have `CoreContainerFactory` in scope to call `vm.container()`. The typical `use firkin::*;` or `use firkin::prelude::*;` pattern handles it. A library consumer that imports only `firkin-vmm` ("just a microVM" use case, no containers) does *not* get these methods — which is correct: without `firkin-core` in their dep tree they have no `Container` type anyway.

**v0.1 constraint on rootfs declarations (per [D-019](../DECISIONS.md#d-019--multi-container-vms-require-pre-declared-block-devices-in-v01), enforced by types via [D-022](../DECISIONS.md#d-022--blockdeviceid-replaces-stringly-paired-block_devicepath--rootfsext4_imagepath) + [D-023](../DECISIONS.md#d-023--rootfs-split-by-vm-context-rootfs-vs-vmrootfs)):** every rootfs for a container built via `vm.container(id)` / `vm.container_shared(id)` must already exist in the running VM. It is either a `BlockDeviceId` returned from `VmConfig::builder().block_device(path)` or a validated `GuestPath` such as a rootfs directory under a mounted pod store. The `OnVm` / `OnVmArc` impls of `.rootfs()` accept only `VmRootfs`, not the `Rootfs` enum — `Rootfs::OciBundle(...)` is unreachable on these builders at the type level, not at runtime. Runtime VZ storage attach is not the first pod path.

**Usage pattern — borrow-bound (default, most cases):**

```rust
use firkin::CoreContainerFactory;                        // brings the trait in scope

let (builder, foo_bd) = VmConfig::builder()
    .block_device("/srv/foo.ext4");                      // D-022 returns typed handle
let vm = VirtualMachine::new(builder.build()?).boot().await?;

let c = vm.container("foo")
    .rootfs(Rootfs::block_device(foo_bd))                // typed; no string match
    .spawn().await?;
// `c` borrows from `vm`. drop(vm) is a compile error while `c` lives.
c.wait().await?;
vm.stop().await?;
```

**Usage pattern — Arc-shared (cross-scope, registries):**

```rust
use firkin::CoreContainerFactory;

let (builder, foo_bd) = VmConfig::builder().block_device("/srv/foo.ext4");
let vm = Arc::new(VirtualMachine::new(builder.build()?).boot().await?);
let c = vm.container_shared("foo")
    .rootfs(Rootfs::block_device(foo_bd))
    .spawn().await?;
// `c` holds an Arc<VmCore> internally; VM lives until last Container drops.
registry.insert(c.id().clone(), c);
drop(vm);                // fine; Arc::clone inside c keeps VM alive.
// registry owns containers; shutdown later via iterating and stop()-ing each.
```

**Why the split between borrow-bound and Arc-shared**:
- Borrow-bound is *natural Rust*. Ownership is visible; VM outlives Containers via borrow checker; no surprises.
- Arc-shared is *visible opt-in*. User typed `Arc::new`; they know they're in Arc-semantics territory. No hidden Arc behavior.
- Neither path has the "surprising deferred cleanup" failure mode of an implicit Arc approach.

### 4.3 Vsock operations

```rust
impl VirtualMachine<Running> {
    /// Dial an arbitrary port in the guest. Host is the active party.
    /// VsockStream: AsyncRead + AsyncWrite + Send + Unpin.
    pub async fn dial(&self, port: VsockPort) -> Result<VsockStream, vmm::Error>;

    /// Listen on a host-side vsock port for guest-initiated connections.
    /// Used internally for container stdio (D-005 inverse-vsock); also available
    /// to users running custom guest daemons that dial back.
    pub fn listen(&self, port: VsockPort) -> Result<VsockListener, vmm::Error>;
}
```

`VsockListener` is an async stream of `VsockStream`s — see [`03-stdio-pty-vsock.md § VsockListener`](./03-stdio-pty-vsock.md).

**Reserved ports**: calling `dial()` or `listen()` with a port in the library-reserved range (`0x1000_0000`-`0x2000_0000` for stdio/relay use; `1024` for vminitd gRPC) returns `Error::ReservedPort { port, reason: &'static str }`. Reserved ranges documented in [`03-stdio-pty-vsock.md § reserved ports`](./03-stdio-pty-vsock.md).

### 4.4 Pause, resume, statistics

```rust
impl VirtualMachine<Running> {
    /// Pause VZ execution. Every container on this VM freezes simultaneously.
    /// Idempotent: pausing an already-paused VM is Ok(()), not an error.
    ///
    /// Not typestated. Reason: Arc-shared ownership means multiple references exist;
    /// typestated transitions would need to consume self, which breaks Arc sharing.
    /// The op-set difference between Running and Paused is small enough that runtime
    /// checks (returning Error::VmPaused from container methods) are cleaner than
    /// a parallel Paused typestate.
    pub async fn pause(&self) -> Result<(), vmm::Error>;

    pub async fn resume(&self) -> Result<(), vmm::Error>;

    /// Per-VM statistics. For per-container stats, call Container::statistics().
    pub async fn statistics(&self) -> Result<VmStatistics, vmm::Error>;
}
```

On paused VMs, most container operations return `Error::VmPaused` at their next RPC boundary. Operations that are purely host-side (like `container.id()`) still work. Operations that would deadlock waiting for a guest response (`wait`, `exec`, `copy_in/out`, `statistics`, `dial_vsock`) return `Error::VmPaused`.

### 4.5 Snapshot save (requires `snapshot` feature)

```rust
#[cfg(feature = "snapshot")]
impl VirtualMachine<Running> {
    /// Save the VM state to `path` as a binary snapshot file. Size ≈ VM RAM + metadata.
    /// VM continues running after the snapshot — it's not a stop-the-world operation
    /// beyond a brief VZ pause-snapshot-resume cycle.
    pub async fn save_snapshot(&self, path: impl AsRef<Path>) -> Result<(), vmm::Error>;
}
```

Pairs with `VirtualMachine::<NotBooted>::boot_or_restore()` in §3.4.

### 4.6 Stop

```rust
impl VirtualMachine<Running> {
    /// Graceful shutdown: cascade-cancel in-flight ops on all contained Containers,
    /// kill container init processes, unmount rootfses, VZ shutdown.
    /// Consumes `self`. Idempotent via the internal Arc ref count.
    pub async fn stop(self) -> Result<(), vmm::Error>;

    pub async fn stop_with_grace(self, grace: std::time::Duration) -> Result<(), vmm::Error>;
}
```

`vm.stop()` cancels the internal VM-level `CancellationToken`, which cascades through the Container tokens to in-flight operation tokens. Operations observe cancellation at their next RPC boundary and return `Error::Cancelled { reason: CancelReason::VmStopped }`. See [`09-cross-cutting.md § cancellation`](./09-cross-cutting.md).

---

## 5. Drop semantics

Same pattern as `Container` (see [`01-container-surface.md § 5`](./01-container-surface.md)), applied at the VM level:

### 5.1 `Drop` does not `.stop()`

```rust
// What happens when VirtualMachine<Running> is dropped:
//   1. Aborts internal relay tasks (sync).
//   2. Closes host-side vsock fds (sync).
//   3. Decrements internal Arc<VmCore> (sync).
//      - If this was the last Arc, a best-effort cleanup task is spawned on the
//        current runtime to stop the VM.
//      - If no runtime is available (runtime shutting down), logs warn and leaks
//        the VZ VM until process exit.
//   4. Logs tracing::warn! if the VM was still running.
```

### 5.2 `AbortOnDrop<VirtualMachine<Running>>`

For users who want drop-means-stop semantics (most useful with the Arc-shared path):

```rust
pub struct AbortOnDrop<T>(Option<T>);

impl AbortOnDrop<VirtualMachine<Running>> {
    pub fn new(vm: VirtualMachine<Running>) -> Self;
    pub fn into_inner(self) -> VirtualMachine<Running>;
}
```

Same Drop pattern as `AbortOnDrop<Container>` — spawns a task to run `stop()`, logs warning if no runtime.

### 5.3 Borrow-bound default: no surprises

Because `vm.container(id)` borrows `&vm`, **you literally cannot drop `vm` while `Container<'_>` handles are still live — it's a compile error**. The borrow checker enforces the invariant.

```rust
let vm = VirtualMachine::new(cfg).boot().await?;
let c = vm.container("foo").rootfs(rfs).spawn().await?;
drop(vm);         // compile error: `vm` borrowed by `c`
c.wait().await?;
```

This is why the default is borrow-bound rather than Arc-shared: the sharp corner (dropping VM without stopping while containers live) is impossible, not just "documented."

---

## 6. Worked examples — non-obvious flows

### 6.1 One VM, many containers, graceful staged shutdown

```rust
use firkin::vmm::{VirtualMachine, VmConfig, Network};
use firkin::{Rootfs, Size, CoreContainerFactory};     // D-018: ext trait

let (builder, web_bd) = VmConfig::builder()
    .cpus(8)
    .memory(Size::gib(4))
    .network(Network::vmnet_shared())
    .block_device("/srv/web.ext4");                    // D-022 typed handle
let (builder, db_bd)  = builder.block_device("/srv/db.ext4");
let vm = VirtualMachine::new(builder.build()?).boot().await?;

let web = vm.container("web").rootfs(Rootfs::block_device(web_bd))
    .command(["/usr/sbin/nginx", "-g", "daemon off;"])
    .spawn().await?;
let db = vm.container("db").rootfs(Rootfs::block_device(db_bd))
    .command(["/usr/bin/postgres", "-D", "/var/lib/postgres"])
    .spawn().await?;

// Wait for one to exit or a shutdown signal...
tokio::select! {
    _ = web.wait()      => tracing::info!("web exited first"),
    _ = db.wait()       => tracing::info!("db exited first"),
    _ = tokio::signal::ctrl_c() => tracing::info!("ctrl-c received"),
}

// Graceful: stop web first (drains requests), then db, then VM.
// Each stop() cascade-cancels only its own in-flight operations.
web.stop().await?;
db.stop().await?;
vm.stop().await?;
```

### 6.2 Just a microVM — no containers

```rust
use firkin::{Size, vmm::{VirtualMachine, VmConfig, KernelImage, BootLog}};

let vm = VirtualMachine::new(
    VmConfig::builder()
        .cpus(2)
        .memory(Size::mib(512))
        .kernel(KernelImage::from_file("/path/to/my-kernel"))
        .cmdline_extra("console=hvc0 panic=-1")
        .boot_log(BootLog::File("/tmp/vm-serial.log".into()))
        .build()?
)
.boot().await?;

// Open a vsock connection to your own guest daemon listening on port 4242.
let mut stream = vm.dial(VsockPort::new(4242)).await?;
// Use stream as an AsyncRead + AsyncWrite...

vm.stop().await?;
```

### 6.3 Snapshot-for-integration-tests pattern

```rust
#[cfg(feature = "snapshot")]
mod fixtures {
    use once_cell::sync::Lazy;
    use std::path::PathBuf;

    static SNAPSHOT_PATH: Lazy<PathBuf> = Lazy::new(|| {
        let path = std::env::temp_dir().join("test-fixture.snap");
        // Cold-boot once, snapshot, stop.
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let vm = VirtualMachine::new(test_config()).boot().await.unwrap();
            vm.save_snapshot(&path).await.unwrap();
            vm.stop().await.unwrap();
        });
        path
    });

    pub async fn fresh_vm() -> VirtualMachine<Running> {
        VirtualMachine::new(test_config())
            .boot_or_restore(&*SNAPSHOT_PATH)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn my_test() {
    let vm = fixtures::fresh_vm().await;   // ~30ms via snapshot, not ~400ms cold boot
    let c = vm.container("test").rootfs(test_rootfs()).spawn().await.unwrap();
    assert!(c.wait().await.unwrap().success());
    vm.stop().await.unwrap();
}
```

### 6.4 Pause, inspect, resume

```rust
let vm = VirtualMachine::new(cfg).boot().await?;
let c = vm.container("worker").rootfs(rfs).command(["/usr/bin/long-job"]).spawn().await?;

// Periodic snapshot-like "inspect" loop.
loop {
    tokio::time::sleep(Duration::from_secs(30)).await;
    vm.pause().await?;
    let stats = c.statistics(StatCategory::all()).await
        .or_else(|e| if matches!(e, Error::VmPaused) { Ok(/*...*/) } else { Err(e) })?;
    // statistics while paused returns Error::VmPaused — request before pausing instead.
    // The above `.or_else` is illustrative; the right call order is stats-then-pause.
    vm.resume().await?;
}
```

---

## 7. Relationship to the `vmm` crate boundary

The `vmm` crate (full spec in [`08-vmm-crate.md`](./08-vmm-crate.md)) *is* the home of every type in this file. Everything on this surface is exported from `vmm::*` and re-exported from `core::vmm::*` for consumer ergonomics.

What this file specifies is "what `vmm` offers to users of the library." What [`08-vmm-crate.md`](./08-vmm-crate.md) specifies is "how `vmm` shields its VZ-coupled internals from the rest of the workspace." No type defined on this surface exposes `objc2`, `Retained<_>`, `DispatchQueue`, or any other Obj-C-bridge type.

---

## 8. Invariants worth locking

1. `VmConfig::default()` produces a usable config; every field is defaulted.
2. `VmConfig::builder().build()` validates eagerly; returns `vmm::Error::InvalidConfig` on rejection.
3. `VirtualMachine<NotBooted>` → `.boot()` → `VirtualMachine<Running>` is the only way to get a running VM. No `.new_running()` or equivalent shortcut.
4. `pause`/`resume` are **runtime-tracked**, not typestated, because Arc-shared typestate doesn't compose.
5. `vm.container()` (borrow-bound) is the default; `vm.container_shared()` (Arc-shared) is explicit opt-in. Both are defined on the `CoreContainerFactory` extension trait in `firkin-core` (D-018), not as inherent methods on `VirtualMachine<Running>`.
6. `.boot()` is VZ-only; init.block synthesis (D-003 + D-004) happens in `firkin-core` before `.boot()` is called. `vmm` has no `ext4` or `vminitd-bytes` dependency.
7. `.stop()` cascades cancellation via internal token tree to all Container handles on this VM.
8. `Drop` never calls `async fn`. `AbortOnDrop<VirtualMachine<Running>>` wrapper for opt-in auto-stop.
9. No `objc2::*` types in any public `vmm` signature.
10. Snapshot/restore (`#[cfg(feature = "snapshot")]`) is v1 pending S10 verification.
11. `VsockStream` / `VsockListener` / `VsockPeer` returned by `dial()` / `listen()` come from `firkin-vsock` (D-016); `vmm` depends on `vsock` and re-exports them.
12. `block_device(path)` on `VmConfigBuilder` returns `(Self, BlockDeviceId)` (D-022). The handle is consumed by `Rootfs::block_device(id)` on `OnVm`/`OnVmArc` containers. No stringly-typed path matching.
13. `OnVm`/`OnVmArc` containers accept only `VmRootfs` via `.rootfs()` (D-023): either `BlockDevice(BlockDeviceId)` or `GuestPath(GuestPath)`. The `Rootfs` enum's `OciBundle`/`Ext4Image`/`RawBlock` variants are unreachable on these builders at the type level. `ImplicitVm` builders accept the full `Rootfs` enum.
14. `VmConfigBuilder::memory` takes `Size` (D-026); no `memory_mib(u64)` convenience. Users write `Size::mib(2048)` or `Size::gib(2)`.

Proceed to [`03-stdio-pty-vsock.md`](./03-stdio-pty-vsock.md) for streaming surfaces.
