# Library surface

> Status: **implemented workspace surface, still subject to completion audit**.
> This directory records the public API shape for the Rust rewrite. It began as
> the implementation design, and now tracks the landed crates plus the behavior
> those crates are expected to preserve. When these docs conflict with code, fix
> the stale doc or the stale implementation immediately; do not carry a
> compatibility shim.

---

## How to read this directory

This is a **hybrid split**: one landing file (this README) for scope and the user-level narrative, plus one file per major concern.

| File | Covers |
|---|---|
| [`README.md`](./README.md) (this file) | Scope, governing principles, non-goals, user-level narrative examples |
| [`01-container-surface.md`](./01-container-surface.md) | `ContainerBuilder`, `Container`, `Process`, exec, copy, drop semantics |
| [`02-vm-surface.md`](./02-vm-surface.md) | `VirtualMachine<NotBooted>`/`<Running>`, `VmConfig`, attachments, ownership story |
| [`03-stdio-pty-vsock.md`](./03-stdio-pty-vsock.md) | Streaming surfaces, listener-delegate pattern, reserved ports |
| [`04-value-types.md`](./04-value-types.md) | Newtypes, `Rootfs`, `Mount`, `Network`, `User`, `LinuxCapabilities`, etc. |
| [`05-error-model.md`](./05-error-model.md) | Per-crate `thiserror` enums, leaf errors, `OneOf` internals, classifiers |
| [`06-ext4-crate.md`](./06-ext4-crate.md) | `Writer`, `Features`, `init_block` synthesis, golden-diff testing |
| [`07-oci-crate.md`](./07-oci-crate.md) | `Client`, `ImageBundle`, `Reference`, `Platform`, manifest lists, zstd, auth |
| [`08-vmm-crate.md`](./08-vmm-crate.md) | Crate boundary, Cargo features, codesigning, preflight, target matrix |
| [`09-cross-cutting.md`](./09-cross-cutting.md) | Send/Sync, Drop, cancellation, tracing, features, MSRV, versioning, lints |
| [`10-non-goals.md`](./10-non-goals.md) | Deferrals with unlock conditions; architectural non-goals |

**Read order for first-time readers**: `README.md` (this) → `01-container-surface.md` → `02-vm-surface.md` → the rest as needed. The narrative examples in this README are the anchor for everything else.

**Read order for contributors implementing a specific crate**: `README.md` for principles → the crate-specific file → `05-error-model.md` → `09-cross-cutting.md`.

---

## Section 1 — Scope, governing principles, non-goals

### 1.1 Scope

This document specifies the **public API surface** of the Rust rewrite: the types, functions, and invariants that a library consumer touches. It covers four public-API crates out of the nine in the workspace:

1. **`firkin`** (the `core` crate) — the facade. `Container`, `ContainerBuilder`, re-exports of `VirtualMachine` and value types. What 90% of users import.
2. **`firkin-vmm`** — the VZ-backed VM primitives. `VirtualMachine`, attachments, listener-delegate for container stdio. Exposed for the pods path and the just-a-microVM use case.
3. **`firkin-ext4`** — the EXT4 image writer. Independently publishable per [D-004](../DECISIONS.md#d-004--ext4-crate-is-the-source-of-truth-for-both-initblock-and-container-rootfs). Public API for both container rootfs assembly and `init.block` synthesis ([D-003](../DECISIONS.md#d-003--embed-vminitd-elf-not-initblock)).
4. **`firkin-oci`** — OCI image pull + bundle assembly. Independently useful as a general OCI registry client. Exposes `ImageBundle` (per [D-020](../DECISIONS.md#d-020--ocibundle-renamed-to-ociimagebundle)).

**Partially public** (user types, but consumed through the facade):
- **`firkin-types`** — shared value-type leaf ([D-015](../DECISIONS.md#d-015--firkin-types-leaf-crate-for-shared-value-types)). Owns `ContainerId`, `ProcessId`, `VsockPort`, `VirtiofsTag`, `VmId`, `Size`, `Hostname`, `Platform`/`Os`/`Arch`, `NamespaceKind`, `BlockDeviceId` ([D-022](../DECISIONS.md#d-022--blockdeviceid-replaces-stringly-paired-block_devicepath--rootfsext4_imagepath)), and the compile-time `container_id!` / `virtiofs_tag!` / `hostname!` macros ([D-027](../DECISIONS.md#d-027--compile-time-validated-virtiofs_tag--container_id--hostname-literal-macros)). Re-exported from `firkin` so user code writes `firkin::ContainerId`; the split is internal.
- **`firkin-vsock`** — public `VsockStream` / `VsockListener` / `VsockPeer` types ([D-016](../DECISIONS.md#d-016--firkin-vsock-owns-streamlistener-types-vmm-depends-on-vsock)). Re-exported from `firkin-vmm` and from `firkin`. Portable (no `objc2`); loopback-testable.

**Out of scope of this design doc** (design decisions these make on their own, not covered here):
- **`firkin-vminitd-client`** — internal tonic-generated gRPC stubs for `SandboxContext.proto`; shape is dictated by the proto and wraps it ergonomically.
- **`firkin-vminitd-bytes`** — leaf crate holding the embedded vminitd ELF via `include_bytes!`. ELF is fetched via pinned download per [D-017](../DECISIONS.md#d-017--vminitd-elf-distributed-via-pinned-download-not-checked-in). Exists to keep the ~131 MiB blob's link-tax off every other crate (PRO_TIPS §30). No public API beyond one `pub const` per target.
- **`firkin-cli`** — dev-facing tool that exercises `firkin`; binary is named `fk` (D-014). Documented separately if/when built.

### 1.2 Governing principles

The principles below are not invented here — they're the lenses we apply, each sourced from a specific doc. Every design decision in this directory traces back to one or more.

1. **Designed for Rust users.** The Swift `apple/containerization` surface is *requirements*, not a *template*. Where Swift idioms (nested `Configuration` structs, `any Protocol`-typed injection, stringly-typed kinds) don't translate cleanly, we reshape. (Established in [`00-notes.md`](../00-notes.md) and throughout this document.)

2. **Async, tokio-committed.** Public types use concrete `tokio::io::AsyncRead`/`AsyncWrite`, not runtime-agnostic traits. See new ADR [D-013](../DECISIONS.md#d-013--async-tokio-committed). Reasons in [`09-cross-cutting.md`](./09-cross-cutting.md).

3. **Typestate where it earns its keep; plain types where it doesn't.** Builder typestate for required fields (caught at compile time, matches the `type_design.md § Radius of correctness` lens). VM boot state typestate (`<NotBooted>` → `<Running>`) because the op sets are genuinely disjoint. **`Container<S>` typestate by stdio shape** (D-025) — `Streams` vs `Pty` carry forward from the builder so the `pty()` accessor is infallible where it exists. **No container-level *lifecycle* typestate** because op sets collapse to "alive vs dead" once pause moves to the VM. Full rationale in [`01-container-surface.md`](./01-container-surface.md).

4. **Most traits shouldn't exist.** Swift protocols with one implementation become concrete Rust types. A trait earns its existence only when two distinct implementations are real, not hypothetical. Applied per [`trait_design.md § most traits shouldn't exist`](../../../../../../src/personal/beads-rs/docs/philosophy/trait_design.md) and [D-007](../DECISIONS.md#d-007--beads-rs-philosophy-for-rust-style).

5. **Errors are the other half of the map.** Per-crate `thiserror` capability enums, domain-named variants, `#[source]` chains, `terrors::OneOf` internal only. Variants are behaviors; fields are details. Applied per [`error_design.md`](../../../../../../src/personal/beads-rs/docs/philosophy/error_design.md). Full surface in [`05-error-model.md`](./05-error-model.md).

6. **Types tell the truth, especially uncomfortable truths.** Newtypes for every number-with-meaning. Sum types over booleans. Information holds its shape until consciously reshaped. Applied per [`type_design.md`](../../../../../../src/personal/beads-rs/docs/philosophy/type_design.md). Full inventory in [`04-value-types.md`](./04-value-types.md).

7. **Minimize scatter; encode what's implicit.** Required knowledge lives at the builder. The live handle exposes only what's callable now. No "go read five files to call `spawn()`." Applied per [`scatter.md`](../../../../../../src/personal/beads-rs/docs/philosophy/scatter.md) and the "six disciplines" it names (local, complete, true, consistent, shaped, minimal).

### 1.3 Non-goals

The following are **explicitly out of v1 scope**. Each has a reason and an unlock condition. Full treatment in [`10-non-goals.md`](./10-non-goals.md).

**Deferred (we might do these later):**
- **Pods** (`LinuxPod` equivalent). Multi-container-per-VM is already supported via the raw `VirtualMachine::container()` API (Q2/C); Pod as a first-class type with shared-PID-ns and per-container-resource-override semantics is deferred to v2.
- **Bridged networking** (`VZBridgedNetworkDeviceAttachment`). Requires paid Apple Developer Program + restricted entitlement + provisioning profile ([D-002](../DECISIONS.md#d-002--ad-hoc-codesigning-base-virt-entitlement-only)); deferred to a separate feature crate.
- **OCI signature verification** (sigstore / cosign / Notary). Deferred to v2.

**Architectural non-goals (not in this library ever, by design):**
- **Dockerfile / image building.** Sibling-tool concern (BuildKit / Buildah shape). A future tool would use *this* library's primitives; it wouldn't live inside `core`.
- **OCI `push`.** We're a runtime, not a publisher.
- **Docker-compatible CLI.** Ships as a separate product on top of this library (like `apple/container` vs `apple/containerization`). ([D-009](../DECISIONS.md#d-009--no-cli-as-product-in-v1).)
- **vminitd rewrite in Rust.** Stays Swift; we bundle the ELF. ([D-003](../DECISIONS.md#d-003--embed-vminitd-elf-not-initblock).)

**Platform / target scope:**
- **Pre-macOS 26 support.** ([D-001](../DECISIONS.md#d-001--macos-26-only).)
- **Linux host targets** for `core`/`vmm` (`objc2-virtualization` is macOS-only). `ext4` and `oci` do build and test on Linux for fast CI.
- **Intel macOS as a tested target.** `x86_64-apple-darwin` builds; not in CI's blocking matrix.

**Hardware / VM feature non-goals:**
- USB / serial / GUI / audio device attachments. Not a container concern.
- Running macOS guests (`VZMacOSVirtualMachineConfiguration`). Out of `vmm`'s Linux-guest-typed shape; if desired later, split `vmm-core` / `vmm-linux` / `vmm-macos` — full path sketched in [`10-non-goals.md`](./10-non-goals.md).

See [`10-non-goals.md`](./10-non-goals.md) for the full catalog with unlock conditions.

---

## Section 2 — User-level narrative

Six code examples, fully typed, that anchor everything downstream. If these don't read clean, nothing in Sections 3–11 will feel right. Every type shown here commits us to its shape; later sections pin the exact definitions.

> **Narrative lead** (per D-021 + [scatter.md § shape](../../../../../../src/personal/beads-rs/docs/philosophy/scatter.md)): the builder's *terminal* comes in three flavors, chosen by intent.
>
> | You want… | Terminal |
> |---|---|
> | Captured output in one call | `.output().await?` |
> | Exit code only, in one call | `.status().await?` |
> | A live handle to interact with | `.spawn().await?` |
>
> Examples §2.1 and §2.2 use the one-call terminals; §2.3–§2.6 use `.spawn()` because they need the handle.

### 2.1 Hello world — captured output

```rust
use firkin::{Container, Rootfs};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let output = Container::builder("hello")
        .rootfs(Rootfs::ext4_image("/tmp/busybox.ext4"))
        .command(["/bin/echo", "hello from the container"])
        .output().await?;

    assert!(output.status.success());
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}
```

**Anchors:**
- `Container::builder(id)` returns `ContainerBuilder<ImplicitVm, Init>` — parameterized over the VM-context (implicit single-use) and the typestate (rootfs not yet set).
- `.rootfs(…)` transitions typestate to `ContainerBuilder<ImplicitVm, Ready>`. Only `<Ready>` and `<ReadyPty>` variants expose the terminal methods. Forgetting to set a rootfs is a compile error, not a runtime one. (Applies [`type_design.md § Radius of correctness`](../../../../../../src/personal/beads-rs/docs/philosophy/type_design.md): errors from bad configuration land at compile time where possible.)
- `.command(…)` accepts `impl IntoIterator<Item = impl Into<OsString>>` so non-UTF-8 args work.
- `.output().await?` (D-021): boots an implicit single-use VM, starts the init process with stdout+stderr auto-piped, drains both, waits, returns `Output { status, stdout, stderr }`. One call, one await, no deadlock footgun. Mirrors `tokio::process::Command::output`.
- Equivalent long form for the interactive case is `.spawn().await?.wait_with_output().await?` — covered in [`01-container-surface.md § 3.3`](./01-container-surface.md).

### 2.2 Pull an OCI image, build its rootfs, run it

```rust
use firkin::{Container, Rootfs, Size};
use firkin::oci::{Client, Reference};
use firkin::ext4::Writer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let image  = Reference::parse("docker.io/library/busybox:latest")?;
    let bundle = Client::default().pull(&image).await?;   // -> oci::ImageBundle (D-020)

    // Assemble the container rootfs from OCI layers via the ext4 crate.
    // Writer is a consuming-self builder (see 06-ext4-crate.md).
    // ImageBundle implements ext4::OciLayerSource (D-024) — pass by reference.
    let rootfs_path = Writer::new("/tmp/busybox.ext4", Size::mib(64))
        .write_oci_layers(&bundle)?
        .finalize()?;

    let status = Container::builder("busybox")
        .image_config(bundle.config())         // env/cmd/cwd/user defaults from OCI
        .rootfs(Rootfs::ext4_image(rootfs_path))
        .status().await?;                      // spawn + wait, one call (D-021)

    std::process::exit(status.code().unwrap_or(0));
}
```

**Anchors:**
- `oci::Client::default()` builds an unauth'd registry client. `Client::builder()` for auth/TLS/concurrency knobs.
- `oci::ImageBundle` (per [D-020](../DECISIONS.md#d-020--ocibundle-renamed-to-ociimagebundle)) holds layers + config on disk in a content-addressable cache. No re-copying between `oci` and `ext4`.
- `&bundle` satisfies `ext4::OciLayerSource` (D-024); `oci::Layer::compression()` maps each `MediaType` to the right `LayerCompression` variant on the oci side of the boundary, so `ext4` doesn't depend on `oci-spec`.
- `ext4::Writer::new(path, Size::mib(64))` — `Size` is a typed quantity, not `u64` ([D-007](../DECISIONS.md#d-007--beads-rs-philosophy-for-rust-style); `Size::kib`, `Size::mib`, `Size::gib`, `Size::tib` constructors). Per [D-026](../DECISIONS.md#d-026--size-is-the-one-memory-setter-memory_mib-removed), `Size` is also the one memory setter on `VmConfigBuilder` and `ContainerBuilder`.
- `.image_config(bundle.config())` pulls OCI `ImageConfig` defaults into the builder; explicit setters override field-by-field.
- `.status().await?` — one-call variant of `.spawn().await?.wait().await?`. `.output()` is the captured-stdout variant.

### 2.3 Interactive pty session

```rust
use firkin::{Container, Rootfs};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut c = Container::builder("shell")
        .rootfs(Rootfs::ext4_image("/tmp/busybox.ext4"))
        .command(["/bin/sh", "-i"])
        .pty((120, 40))
        //   ^^^^^^^^^^
        //   After this call, the builder's typestate is ContainerBuilder<_, ReadyPty>.
        //   Calling .stderr(...) on that type is a compile error — no such method.
        //   `.pty()` takes `impl Into<PtyConfig>`: tuples, PtyConfig::new(...), or
        //   PtyConfig::default() all work.
        .spawn().await?;                  // returns Container<Pty> (D-025)

    let pty = c.pty();                    // infallible — typestate guarantees it
    // host-terminal ↔ pty forwarding elided; see examples/pty.rs in the repo.
    pty.resize((80, 24)).await?;

    c.wait().await?;
    Ok(())
}
```

**Anchors:**
- `.pty((cols, rows))` accepts `impl Into<PtyConfig>`; the `From<(u16, u16)>` impl makes the tuple form read cleanly. `PtyConfig::default()` is 80×24.
- Terminal-XOR-stderr is enforced at **compile time** via builder typestate ([`01-container-surface.md § ContainerBuilder`](./01-container-surface.md)). Swift's equivalent is a runtime error; Rust's moves it earlier.
- `.spawn()` on a `ReadyPty` builder returns `Container<Pty>` (D-025). `c.pty()` on `Container<Pty>` returns `&mut Pty` — infallible, no `.expect(…)`. A `Container<Streams>` has no `.pty()` method at all; calling it is a "method not found" error.
- `Pty: AsyncRead + AsyncWrite + Unpin + Send` — one duplex stream, not split halves. If a user wants split, they call `tokio::io::split(c.pty())`.
- `pty.resize(cfg)` is on the `Pty` handle, not on `Container` — the pty is the thing that knows its own size.

### 2.4 Multi-container on one VM — "pods without a Pod"

The Q2/C power path: users who need multiple containers in one VM get it today via the raw `VirtualMachine` API. A dedicated `Pod` type is deferred to v2 (see [`10-non-goals.md § Pods`](./10-non-goals.md)); the capability is not.

> **How §2.1's shorthand relates to this:** `Container::builder(id)` from §2.1 is conceptually `VirtualMachine::new(VmConfig::implicit_for(&builder)).boot().await?.container(id)` with some conveniences folded in. Same type family, same methods — just with the VM context inferred. The explicit form below gives you the VM handle to pause, dial vsock, statistics, and attach multiple containers to.

**v0.1 constraint (per [D-019](../DECISIONS.md#d-019--multi-container-vms-require-pre-declared-block-devices-in-v01), enforced by [D-022](../DECISIONS.md#d-022--blockdeviceid-replaces-stringly-paired-block_devicepath--rootfsext4_imagepath) + [D-023](../DECISIONS.md#d-023--rootfs-split-by-vm-context-rootfs-vs-vmrootfs)):** rootfses for `vm.container(id)` come from typed `BlockDeviceId` handles returned by `VmConfig::builder().block_device(path)`. The `Rootfs::OciBundle(...)` variant is *not convertible* to `VmRootfs` — using it on an `OnVm` builder is a compile error, not a runtime error. Users who want OCI-backed multi-container VMs pre-assemble via `ext4::Writer` + pre-declare. Runtime block-device attach is Phase 2.

```rust
use firkin::{Container, Rootfs, Size};
use firkin::vmm::{VirtualMachine, VmConfig, Network};
use firkin::CoreContainerFactory;              // D-018: ext trait for vm.container(…)

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (builder, web_bd) = VmConfig::builder()
        .cpus(4)
        .memory(Size::gib(2))                      // D-026: Size is the one setter
        .network(Network::vmnet_shared())
        .block_device("/srv/web.ext4");            // D-022: typed handle returned
    let (builder, db_bd)  = builder.block_device("/srv/postgres.ext4");
    let vm = VirtualMachine::new(builder.build()?).boot().await?;
    //                                                // <NotBooted> -> <Running>

    let web = vm.container("web")                    // from CoreContainerFactory trait in scope
        .rootfs(Rootfs::block_device(web_bd))        // D-023: VmRootfs, not Rootfs
        .command(["/usr/sbin/nginx", "-g", "daemon off;"])
        .spawn().await?;

    let db = vm.container("db")
        .rootfs(Rootfs::block_device(db_bd))
        .command(["/usr/bin/postgres", "-D", "/var/lib/postgres"])
        .spawn().await?;

    tokio::try_join!(web.wait(), db.wait())?;
    vm.stop().await?;
    Ok(())
}
```

**Anchors:**
- `VirtualMachine::new(cfg)` returns `VirtualMachine<NotBooted>`; `.boot().await?` returns `VirtualMachine<Running>`. Only `<Running>` exposes `.dial()`, `.listen()`, `.pause()`, `.statistics()`. Calling any of those on `<NotBooted>` is a compile error.
- `vm.container(id)` comes from the `CoreContainerFactory` extension trait (per [D-018](../DECISIONS.md#d-018--container-factory-exposed-via-corecontainerfactory-extension-trait)): the trait is defined in `firkin-core` and impl'd for `VirtualMachine<Running>` there, because `ContainerBuilder` is a `core` type and `firkin-vmm` cannot know about it. The trait is re-exported from `firkin`, so the typical `use firkin::*;` brings it into scope automatically. It returns the *same* `ContainerBuilder` type family as §2.1's `Container::builder`, parameterized on `OnVm<'vm>` rather than `ImplicitVm`. VM-config methods (`.network()`, `.virtiofs_share()`) exist only on `ImplicitVm` builders.
- `block_device(path)` returns `(Self, BlockDeviceId)`. Pass the handle to `Rootfs::block_device(id)` on containers. A typo in the path fails at `build()` time (the path must be readable); a cross-VM misuse fails at `spawn()` time with `Error::Config(ConfigError::ForeignBlockDevice { id })`. There is no stringly-typed path match.
- `vm.stop().await?` consumes the VM, cascade-cancels any in-flight operations on its containers ([`09-cross-cutting.md § cancellation`](./09-cross-cutting.md)), kills container init processes, unmounts their rootfses, shuts down the VM.

### 2.5 Ten parallel builds, one shared compiler cache

The canonical use case the design must support cleanly. Combines Q2/C multi-container-per-VM with virtiofs shares so the builders share a cache directory under real POSIX semantics.

```rust
use firkin::{Container, Rootfs, Mount, Size, virtiofs_tag, CoreContainerFactory};
use firkin::vmm::{VirtualMachine, VmConfig, Network};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    const CACHE: firkin::VirtiofsTag = virtiofs_tag!("cargo-cache");     // compile-time validated (D-027)

    // All 10 builds share the same rootfs image — VZ lets multiple containers
    // mount the same pre-declared block device read-only (with per-container
    // writable overlay via Mount::overlay or writable_layer). Pre-declare the
    // image once per D-019; typed handle from D-022.
    let (builder, builder_bd) = VmConfig::builder()
        .cpus(8)
        .memory(Size::gib(16))                                   // D-026
        .network(Network::nat())
        .virtiofs_share(CACHE, "/Users/darin/.cache/cargo")       // host dir -> tag
        .block_device("/srv/rust-builder.ext4");                  // D-022 typed handle
    let vm = VirtualMachine::new(builder.build()?).boot().await?;

    let mut builds = Vec::new();
    for i in 0..10 {
        let c = vm.container(&format!("build-{i}"))
            .rootfs(Rootfs::block_device(builder_bd))             // D-023 VmRootfs
            .command(["/usr/bin/cargo", "build", "--release"])
            .mount(Mount::virtiofs(CACHE, "/root/.cargo"))        // tag -> guest path
            .env("CARGO_TARGET_DIR", "/root/.cargo/target")
            .spawn().await?;
        builds.push(c);
    }

    // Wait for all ten to finish; stop VM.
    for c in builds {
        let status = c.wait().await?;
        eprintln!("build {} exited {:?}", c.id(), status);
    }
    vm.stop().await?;
    Ok(())
}
```

**Anchors + honest caveats:**
- **One kernel, N containers, POSIX locking works.** Because all 10 containers run on the same guest Linux kernel, `flock` / `cargo`'s internal lockfile / `sccache`'s atomic writes all behave identically to bare metal.
- **Naive `cargo build --target-dir=<shared>` will still corrupt with 10 concurrent builders** — `cargo` assumes one builder per target dir. The right fixes are `sccache`, `cargo`'s own build-server socket, or per-builder target dirs with a post-build hardlink pool. The *library* supports this use case; the *build discipline* is the user's job. Documented in [`01-container-surface.md § Mount — copy_in vs virtiofs`](./01-container-surface.md).
- **Alternative layout B** (10 VMs each virtiofs-sharing the same host path) doesn't have the single-kernel property: `flock` across VMs is NFS-like (advisory-within-VM, not-coordinated-across-VMs). Fine for content-addressable caches (sccache, bazel remote cache). Corruption gallery for naive uses. Layout A (above) is what this design optimizes for.

### 2.6 Error handling — the caller's decision surface

```rust
use firkin::{Container, Rootfs};
use firkin::core::{Error, CancelReason};

async fn try_run(id: &str, rootfs: &std::path::Path) -> Result<(), Error> {
    match Container::builder(id).rootfs(Rootfs::ext4_image(rootfs)).status().await {
        Ok(_status) => Ok(()),

        // Bad configuration — don't retry; fix inputs.
        Err(Error::Config(e))                          => Err(Error::Config(e)),

        // Transient VM boot failure — caller may retry with backoff.
        Err(Error::VmBoot(e)) if e.is_transient()      => Err(Error::VmBoot(e)),

        // Guest agent reachability issue — might be transient, worth one retry.
        Err(Error::GuestAgentUnreachable { port, source }) => {
            tracing::warn!(?port, ?source, "agent unreachable; retrying once");
            Err(Error::GuestAgentUnreachable { port, source })
        }

        // Library asked to cancel (e.g., our VM is shutting down) — don't retry.
        Err(Error::Cancelled { reason: CancelReason::VmStopped }) => Ok(()),

        Err(e) => Err(e),
    }
}
```

**Anchors:**
- `core::Error` is the single top-level capability enum users import from `core`. Variants name *behaviors* (`Config`, `VmBoot`, `GuestAgent`, `ImagePull`, …), not crates (`error_design.md § 4.3`: no library names; underlying causes in `#[source]`).
- Classification helpers (`is_transient()`, `is_config()`, `is_auth()`, `is_not_found()`) live on specific types where they drive policy decisions. Not attached everywhere — only where behavior depends on them. Full matrix in [`05-error-model.md § classifiers`](./05-error-model.md).

---

### Narrative omissions — intentional

Things used by some consumers but excluded from the landing narrative because they're *detail*, not *shape*:

- `container.dial_vsock(port) -> VsockStream` — covered in [`03-stdio-pty-vsock.md`](./03-stdio-pty-vsock.md).
- `container.copy_in(host, guest) / copy_out(…)` — covered in [`01-container-surface.md`](./01-container-surface.md).
- `container.exec(id, cfg)` — auxiliary processes; in [`01-container-surface.md § Process`](./01-container-surface.md).
- `container.statistics(categories)` — resource metrics; [`01-container-surface.md`](./01-container-surface.md).
- `container.pause() / resume()` — delegates to VM; [`01-container-surface.md`](./01-container-surface.md) and [`02-vm-surface.md`](./02-vm-surface.md).
- `vm.save_snapshot(path) / VirtualMachine::new(cfg).boot_or_restore(path)` — behind `snapshot` Cargo feature; [`02-vm-surface.md`](./02-vm-surface.md) and [`08-vmm-crate.md § Cargo features`](./08-vmm-crate.md).
- `Network::vmnet_shared_with_subnet(…)` — [`04-value-types.md § Network`](./04-value-types.md).
- Rosetta enabling (`VmConfig::rosetta(true)`) — [`02-vm-surface.md § attachments`](./02-vm-surface.md).

All of these are v1 public API. The landing narrative just doesn't lead with them.

---

## Summary — what locking this README commits

Every other file in this directory derives from what's in here:

1. Public API is designed for Rust users, not transcribed from Swift.
2. `VirtualMachine` and `Container` are both public types; common path is `Container::builder(id).…output()` with implicit single-use VM; power path is explicit `VirtualMachine::boot` + `vm.container()` (via `CoreContainerFactory` ext trait — D-018). The implicit form is a narrative shorthand for the explicit form; same type family, fewer lines.
3. Builder typestate for required fields + pty-XOR-stderr; VM boot typestate (`<NotBooted>`/`<Running>`); `Container<S>` typestate by stdio shape (D-025) carries forward from the builder.
4. Three terminals on the builder: `.output()` / `.status()` / `.spawn()` (D-021). Pick by intent; no long-form `spawn().await?.wait_with_output().await?` needed for the common case.
5. Async, tokio-committed, concrete tokio types in public API.
6. Stdio defaults to `Null`; `Piped` and `Inherit` are opt-in; `.pty((cols, rows))` excludes `.stderr()` at compile time and returns an infallible `Container<Pty>::pty() -> &mut Pty` post-spawn.
7. Pod deferred to v2; raw `vm.container()` covers multi-container-per-VM. Multi-container rootfses come from typed `BlockDeviceId` handles (D-022) consumed via `Rootfs::block_device(id)` → `VmRootfs` (D-023). No stringly-typed path match; `Rootfs::OciBundle` on `OnVm` is a compile error.
8. Rootfs assembly via `ext4` crate (`Writer::write_oci_layers(&bundle)` via `ext4::OciLayerSource` — D-024); OCI pull via `oci` crate (exposes `ImageBundle` — D-020); both independently publishable.
9. Errors per-crate `thiserror` enums; variants are behaviors, never crate names.
10. Drop doesn't auto-stop; `AbortOnDrop<Container<S>>` wrappers for opt-in; internal cascading cancellation via token tree.
11. Value types live in `firkin-types` leaf (D-015), including `BlockDeviceId`, `Streams`/`Pty` markers, and the `container_id!` / `virtiofs_tag!` / `hostname!` literal macros (D-027); `firkin-vsock` owns stream types (D-016); `firkin-vminitd-bytes` fetches ELF via pinned download (D-017).
12. `Size` is the one memory setter (D-026); `memory_mib(u64)` is gone.
13. No Dockerfile / image building / OCI push / Docker-compat CLI in this library.

Proceed to [`01-container-surface.md`](./01-container-surface.md) for the full `Container` and `Process` API.
