# Architectural Decisions — rust_rewrite

Short ADR-style log. Each entry: the decision, why we made it, when, and what evidence supports it. When a decision gets reversed, leave the old entry in place and add a new one referencing it — don't rewrite history.

---

## D-001 — macOS 26+ only

**Decision**: the library supports macOS 26.0 and later. No pre-26 compatibility scaffolding.
**Rationale**:
- apple/container's own CLI has the same floor.
- vmnet shared-mode ad-hoc signing only works on 26+ (S6 evidence); on 13–15 it required `com.apple.vm.networking` + a provisioning profile + paid dev program.
- Keeps the `vmm` crate simple: one VZ behavior set to code against, one signing model, one network-attachment flow.
**Date**: 2026-04-20.
**Superseded by**: —
**Evidence**: `spike-logs/s6-vmnet-entitlements/FINDINGS.md`.

---

## D-002 — Ad-hoc codesigning, base virt entitlement only

**Decision**: the library and its CLI are signed ad-hoc (`codesign --sign -`) with an `entitlements.plist` containing exactly `com.apple.security.virtualization`. No Apple Developer Program membership required for the v1 feature set.
**Why this is defensible**:
- Covers: `VZNATNetworkDeviceAttachment`, `VZVmnetNetworkDeviceAttachment` (shared), `VZLinuxRosettaDirectoryShare`, all storage / console / vsock / entropy devices.
- Does *not* cover `VZBridgedNetworkDeviceAttachment` (bridged-to-physical-NIC) — requires `com.apple.vm.networking` which is a **restricted entitlement**: AMFI refuses any binary declaring it unless signed with a provisioning profile that explicitly grants that entitlement to that app ID. A cert alone is not sufficient; you also need Apple to have granted your team access to the entitlement.
**Rationale**:
- Anybody can build from source. No Apple account required.
- No CI signing-identity secrets to manage.
- Keeps the project open to contributors.
**Date**: 2026-04-20.
**Superseded by**: — (will need revision if/when Phase 3 wants bridged networking).
**Evidence**: PRO_TIPS §29 entitlements matrix; `spike-logs/s6-vmnet-entitlements/FINDINGS.md` probe F.
**Note**: the author has an Apple Development cert (Team `X7B3K399TD`) available if Phase 3 bridged work needs it, but that alone doesn't unblock `com.apple.vm.networking` — that needs a matching provisioning profile and Apple-team entitlement approval. Decision can be revisited for a separately-released "bridged-networking" feature crate.

---

## D-003 — Embed vminitd ELF, not `init.block`

**Decision**: `core/build.rs` embeds the ~131 MiB `vminitd` ELF via `include_bytes!` (in a dedicated leaf crate `vminitd-bytes`). The `ext4` crate synthesizes `init.block` on-host at first VM boot, caches the result in `$XDG_CACHE_HOME` keyed by the vminitd-ELF SHA-256.
**Rationale**:
- Embedding the 384 MiB `init.block` busts the warm-rebuild budget (20–40 s) and peak RSS (7 GB). The 131 MiB ELF hits every tolerance.
- Consumers of `ext4` who never instantiate a VM pay 0 bytes (ld dead-strips unreferenced `include_bytes!` consts).
- `init.block` is a deterministic function of the ELF: same ELF in → same ext4 bytes out (modulo filesystem-UUID which we pin). Cache works.
- Aligns the `ext4` crate's mission with a real use case beyond container rootfs.
**Date**: 2026-04-20.
**Evidence**: `spike-logs/s8-bundling-bench/FINDINGS.md`; convergence with `spike-logs/s5-ext4/`.

---

## D-004 — `ext4` crate is the source of truth for both init.block and container rootfs

**Decision**: one Rust EXT4 writer, used for (a) synthesizing `init.block` from the bundled vminitd ELF and (b) composing container rootfs images from OCI layers.
**Rationale**:
- Two EXT4-writer implementations would drift. One implementation, two consumers.
- Keeps `ext4` as an independently useful crate (may publish separately).
- The idiomatic-Rust style (newtypes, thiserror, `#[repr(C)]`+bytemuck) established in S5 applies directly.
**Date**: 2026-04-20.
**Evidence**: `spike-logs/s5-ext4/FINDINGS.md` + `spike-logs/s8-bundling-bench/` convergence.
**Consequence**: `ext4` has no dependency on `vmm`, `vsock`, or anything macOS-specific. It can be cross-tested on Linux CI if desired.

---

## D-005 — Inverse-vsock listener for container stdio

**Decision**: the `vmm` crate exposes a listener-delegate API (built on `VZVirtioSocketListener` + `VZVirtioSocketListenerDelegate`) in addition to the connector used for outbound RPC. The `core` / `vminitd-client` layer uses this listener for container stdio — vminitd's `CreateProcess` takes vsock port numbers in `stdin`/`stdout`/`stderr`, and the guest connects back to those ports.
**Rationale**:
- Stdio throughput wants streaming fd-level semantics, not tonic RPC framing. Passing raw vsock connections keeps latency low and avoids re-copying.
- apple/containerization's Swift side does the same (`StandardIO.start()` in vminitd + host-side acceptors).
- We *only* discovered this because S4 hit it in practice — worth documenting so it doesn't get re-invented.
**Date**: 2026-04-20.
**Evidence**: `spike-logs/s4-e2e/FINDINGS.md` §1; PRO_TIPS §20.

---

## D-006 — Single serial dispatch queue per VM

**Decision**: each `VirtualMachine` in the library is bound to one serial dispatch queue. All VZ calls and delegate callbacks happen on that queue. Rust code talking to the VM goes through a tokio channel or equivalent.
**Rationale**:
- VZ requires per-VM serial queue affinity; crossing queues is undefined behavior.
- For CLI-shaped consumers the queue can be the main queue + `dispatch_main()` (simpler, no cross-thread Send gymnastics).
- For library consumers running alongside other frameworks, the queue can be a dedicated custom queue; use `VzSend<T>` (`unsafe impl Send`) for the !Send `Retained<VZ*>` handles that need to cross thread boundaries, guarded by the convention that the inner value is only touched on that queue.
**Date**: 2026-04-20.
**Evidence**: PRO_TIPS §1, §2; applies uniformly across S1, S2, S4, S6, S7.

---

## D-007 — beads-rs philosophy for Rust style

**Decision**: we apply the design filters from `~/src/personal/beads-rs/docs/philosophy/` (type, error, trait, test design) to all Rust code in this project. Key operational implications:
- **Newtypes for every number-with-meaning** (`BlockNumber`, `InodeNumber`, `ContainerId`, `VsockPort`). Not `u32` all the way down.
- **Domain-named error variants** via `thiserror`. No `Io(io::Error)` god-catchalls; underlying crate errors live in `#[source]`.
- **No trait unless two distinct implementations exist.** "Most traits shouldn't exist." Don't invent capability seams that aren't real yet.
- **One capability error enum per trait.** `OneOf`-style internal error sets allowed for precision; collapse to the canonical enum at boundaries.
- **Tests are executable claims.** Four shapes only: law, example, scenario, regression. Every test must kill a family of wrong implementations.
**Date**: 2026-04-20.
**Evidence**: `spike-logs/s5-ext4/FINDINGS.md` — newtypes caught a compile-time bug in `encode_inline_extent`; domain-named error variants made xattr + directory-block bugs cheap to diagnose. Style held up in anger.

---

## D-008 — Proto vendored at a pinned SHA, not a submodule

**Decision**: `SandboxContext.proto` is copied into `crates/vminitd-client/proto/` at a specific apple/containerization git SHA. The SHA is part of `pin.toml`. Regenerating the stubs is a deliberate act triggered by bumping the pin.
**Rationale**:
- Reproducibility > freshness. CI must produce the same bytes on the same input regardless of when it runs.
- Submodules add ceremony (`git submodule update --init --depth 1`) that confuses new contributors — we already hit this with `objc2-generated` (PRO_TIPS §8).
- apple/containerization may bump proto in breaking ways; we want deliberate adoption, not silent drift.
**Date**: 2026-04-20.
**Evidence**: PRO_TIPS §8 (objc2-generated submodule trap).

---

## D-009 — No CLI as product in v1

**Decision**: v1 ships as a **library**. The CLI in the same workspace is for exercising the library during development; it is not a Docker-compatible user-facing product.
**Rationale**:
- The original brief is a library port of apple/containerization's Swift packages; a CLI is a separate product (like apple/container).
- Keeping CLI surface thin means we're not litigating UX decisions while still proving the library shape.
- A Docker-compatible CLI can land as a separate crate later, on top of `core`.
**Date**: 2026-04-20.
**Evidence**: `00-notes.md` "target = the library" framing from the pre-spike brief; spike CLI at `crates/cli/` in `03-project-layout.md` is intentionally labeled "dev-facing".

---

## D-010 — `rust-rewrite-spikes` jj bookmark pins the proof

**Decision**: all spike-phase commits (runbook, PRO_TIPS, spike-logs, template, this decisions log) land as commits on a jj bookmark named `rust-rewrite-spikes`, stacked on apple/containerization's `main`. Never pushed to `origin`.
**Rationale**:
- The upstream `main` is apple's; we don't have permission to push there.
- Local jj history preserves the full trail with `jj op log` undo safety.
- A clear bookmark makes it obvious which commits are "our spike work" vs the upstream apple commits we sit on top of.
**Date**: 2026-04-20.
**Evidence**: `jj log -r 'trunk()..rust-rewrite-spikes'`.

---

## D-011 — Network config via vminitd netlink RPCs, not guest-side DHCP

**Decision**: the library configures the container's network interface via vminitd's `IpLinkSet` / `IpAddrAdd` / `IpRouteAddDefault` / `ConfigureDns` RPCs (SandboxContext v3). No DHCP client in the container rootfs.
**Rationale**:
- Matches apple/containerization's Swift side verbatim (`LinuxContainer.swift:594-617`).
- Keeps the container rootfs smaller — no dhclient/udhcpc/NetworkManager to carry.
- Configuration is deterministic and synchronous: we know the IP the library assigned before the container's first instruction runs; no "did DHCP finish yet?" races.
- vmnet decides the subnet at attachment time — we learn it via `vmnet_network_get_ipv4_subnet` and allocate IPs from it.
**Consequence**: the `vminitd-client` crate's network module is a set of small typed wrappers; the `core` crate's lifecycle step "bring container online" issues five RPCs in a known sequence (see PRO_TIPS §32). DHCP + related guest-side machinery not required.
**Date**: 2026-04-20.
**Evidence**: `spike-logs/s9-vmnet-reachability/FINDINGS.md`.

---

## D-012 — One VM per container (matches apple/container's model)

**Decision**: each container gets its own VZ virtual machine. Multiple containers on the same vmnet network share a `vmnet_network_ref`; each VM has its own `VZVmnetNetworkDeviceAttachment` pointing at that shared network.
**Rationale**:
- Mirrors apple/container's architecture — same mental model, same operational characteristics.
- Containers get strong isolation (separate kernel, separate namespaces-inside-VM, separate cgroup tree) without extra library work.
- Sharing one VM across containers requires vminitd to juggle multiple cgroups and netns trees; apple/containerization deliberately doesn't do this, and for good reason (complexity, security attack surface).
- vmnet naturally supports many attachments on one network, so this composes cleanly with D-011's networking.
**Cost**: per-container VM overhead (memory, boot time). kata 3.17.0 + small vminitd gets us sub-second boot; the overhead is real but affordable, especially for Mac-native dev-loop containers (typically N<10 concurrent).
**Date**: 2026-04-20.
**Evidence**: `spike-logs/s9-vmnet-reachability/FINDINGS.md` ("two-container stretch" note); reference: `Sources/Containerization/VmnetNetwork.swift`.

---

## D-013 — Async, tokio-committed

**Decision**: the library is fully async. Public API types use concrete `tokio::io::AsyncRead` / `tokio::io::AsyncWrite`, not runtime-agnostic traits like `futures::io::*`. Users need a tokio runtime to use the library.
**Rationale**:
- VZ is event-driven via Obj-C delegates and completion blocks. Every VZ operation is "fire it on the dispatch queue, get a callback later." The natural Rust bridge is `oneshot::Receiver` (awaited); a sync surface would mean `.recv()` blocking inside methods, which on the wrong thread deadlocks against the VZ serial queue itself (D-006).
- tonic is async-only. The vminitd RPC client *is* async. A sync facade would `runtime.block_on()` inside every method — hidden tokio runtime, with the classic "runtime inside runtime" panic when called from the user's runtime.
- Every spike (S1–S9) is already async tokio. The proven-on-this-machine code path is async end-to-end; retrofitting a sync facade is pure cost against a path that already works.
- Library-internal concurrency is real: host vsock listeners for stdio (D-005), the RPC client, `copy_in` / `copy_out` streams, unix-socket relays, VM state listeners. Sync doesn't eliminate them — it moves them to threads, which is worse for this workload.
- `tokio::process::Child` is the shape we model `Container` on; consistency is a UX win.
- S2's vsock fd wrapper uses `tokio::io::unix::AsyncFd`. Committing to tokio avoids wrapping through `async-io` or similar — no extra layer, no extra surface.
**Consequence**: public signatures use concrete tokio types (e.g., `ChildStdout: AsyncRead` where `AsyncRead` is `tokio::io::AsyncRead`). Users who need a sync facade write ~100 LOC of `block_on`-based wrapping in their own code; we don't ship a `blocking` submodule in v1 (possibly later if demand shows up).
**Date**: 2026-04-20.
**Evidence**: spike-logs S1/S2/S4/S6/S7/S9 (all tokio); [`04-library-surface/09-cross-cutting.md § 10`](./04-library-surface/09-cross-cutting.md).

---

## D-014 — CLI binary is `fk`, not `firkin`

**Decision**: the dev-facing CLI (package `firkin-cli`) installs a binary named `fk`, not `firkin`.
**Rationale**:
- The library crate is `firkin`; the CLI crate is `firkin-cli`. Cargo resolves both `cargo doc` output paths to `<target>/doc/firkin/index.html` when both crates expose a target named `firkin`, which triggers rust-lang/cargo#6313 and breaks `cargo doc -D warnings` in CI.
- Naming the CLI binary `fk` avoids the collision entirely (two-letter invocations match Rust-ecosystem convention: `rg`, `fd`, `bat`, `jj`).
- The CLI is explicitly dev-facing per [D-009](#d-009--no-cli-as-product-in-v1); there is no marketing or UX reason to insist on the name `firkin` for the binary.
**Date**: 2026-04-20.
**Superseded by**: —
**Evidence**: `cargo doc --workspace --no-deps` collision observed during workspace scaffold; fix verified by renaming the `[[bin]]` in `crates/cli/Cargo.toml`.
**Consequence**: docs that reference the CLI invocation say `fk run` / `fk pull`, not `firkin run`. The `scripts/sign.sh` example in the root README signs `target/release/fk`.

---

## D-015 — `firkin-types` leaf crate for shared value types

**Decision**: introduce a dedicated leaf crate `firkin-types` that holds value types shared across crates: `ContainerId`, `ProcessId`, `VsockPort`, `VirtiofsTag`, `VmId`, `Size`, `Hostname`, `Platform` / `Os` / `Arch`, `NamespaceKind`, and their `InvalidX` error types. `firkin-types` has no dependencies on any other workspace crate; every other crate that needs these types depends on it.
**Rationale**:
- `firkin-vminitd-client` needs `ContainerId` and `NamespaceKind` but cannot depend on `core` (cycle); it also needs `VsockPort` but should not depend on `vmm` (which is macOS-only and heavy).
- `firkin-vsock` needs `VsockPort` but should remain portable (see D-016).
- `firkin-oci` defines `Platform` / `Os` / `Arch` for manifest-list selection; `core` references them through `oci`, but they are cheap value types that logically belong at the bottom of the graph.
- Folding these into `firkin-ext4` would conflate two missions (EXT4 writer vs. shared value types) and force `ext4` consumers to drag in types they don't need.
**Consequence**: `04-library-surface/04-value-types.md` annotates each type's owning crate. Re-exports from `firkin` (the facade) mean users still write `firkin::ContainerId`. Containers-still-are-containers; nothing moves at the user surface. The split is purely internal.
**Date**: 2026-04-21.
**Superseded by**: —
**Evidence**: 2026-04-21 plan-pass surfaced the cycle (vminitd-client cannot depend on core; must still use `ContainerId`).

---

## D-016 — `firkin-vsock` owns stream/listener types; `vmm` depends on `vsock`

**Decision**: flip the previously-planned dep direction. `firkin-vsock` owns the public `VsockStream` / `VsockListener` / `VsockPeer` types (wrappers around `OwnedFd` via `tokio::io::unix::AsyncFd`). `firkin-vmm` depends on `firkin-vsock` and hands it `OwnedFd`s produced by VZ's connect and listener-delegate machinery. `VsockPort` lives in `firkin-types` (D-015).
**Rationale**:
- 04-phase1-plan committed to loopback-testing `vsock` with `tokio-vsock` listeners (no VM required). That only works if `vsock` is portable, i.e. does not import `objc2-virtualization` transitively.
- `VsockStream` is conceptually "an async duplex over an OwnedFd" — its impl has nothing to do with VZ. VZ is *one* way to acquire the FD; another is `tokio-vsock` loopback; a third is a future Linux vhost-vsock path if anyone wants it.
- The original spec text (`03-stdio-pty-vsock.md §5`: "these types live in the vmm crate") conflicted with the project-layout graph (`vsock → vmm`) and with the phase1-plan's testing strategy. Flipping resolves all three.
**Consequence**:
- `firkin-vsock` is portable: no `objc2-*`, no macOS-specific deps. Builds on Linux CI.
- `firkin-vmm` depends on `firkin-vsock`. The VZ connect/listener machinery produces `OwnedFd`s that vsock then wraps.
- `VsockStream` is re-exported from `firkin-vmm` (for users who want "just a microVM" without separately importing `vsock`) and from `firkin` (the facade).
**Date**: 2026-04-21.
**Superseded by**: —
**Evidence**: 2026-04-21 plan-pass surfaced the contradiction between spec sections.

---

## D-017 — vminitd ELF distributed via pinned download, not checked-in

**Decision**: the pinned vminitd ELF is fetched by `build-tools/build-vminitd/fetch.sh` (and `firkin-vminitd-bytes`'s `build.rs`) from the pinned GitHub release asset, verified by SHA-256, and cached under `$CARGO_TARGET_DIR/firkin-vminitd/<sha256>/`. No >100 MiB file is committed to git. The previously-default "checked-in at `vendor/vminitd/...`" path moves behind a non-default `vendored-vminitd` Cargo feature for air-gapped users willing to set up git-LFS themselves.
**Rationale**:
- The ELF is ~131 MiB (S8). GitHub rejects pushes of single files >100 MiB. Committing would force git-LFS, which adds a second-clone step and friction for every contributor, not just those exercising the VM.
- First-build download (once per machine, cached forever) costs ~11 s on a typical 100 Mbps link — well inside tolerance (04-phase1-plan cold-build budget is 60 s).
- Determinism is preserved: `pin.toml` carries the exact sha256; mismatch fails the build loudly.
- Escape hatch for offline environments: `FIRKIN_VMINITD_PATH=<path>` env var at build time skips the download and uses the user-provided ELF (still sha256-verified).
**Consequence**:
- [D-003](#d-003--embed-vminitd-elf-not-initblock)'s "embed via `include_bytes!` from a vendor-checked-in path" is superseded in the path-resolution dimension only: the ELF is *still* embedded via `include_bytes!`, but the path now resolves to `$OUT_DIR/vminitd-<target>` populated by `build.rs`, not `vendor/vminitd/...`.
- `runtime-download` feature (03-project-layout §B) and `vendored-vminitd` feature are now mutually exclusive escape hatches off the new default.
- `.gitignore` excludes `vendor/vminitd/**`; the `vendor/vminitd/README.md` documents how to populate it manually for the `vendored-vminitd` feature.
**Implementation note, 2026-05-03**: public `apple/containerization` releases currently do not publish `vminitd`/`vmexec` runtime assets. `pin.toml` carries empty URL slots until a real release asset exists; default builds still work from `vminitd/bin/` or explicit env overrides, and fresh clones fail loudly rather than silently fetching from an unpinned location.
**Date**: 2026-04-21.
**Superseded by**: —
**Supersedes**: the path-resolution aspect of D-003 (the embedding strategy is unchanged).
**Evidence**: GitHub's 100 MiB per-file limit; S8 measured first-run latency +0.6s loopback / ~11s @ 100 Mbps.

---

## D-018 — Container factory exposed via `CoreContainerFactory` extension trait

**Decision**: `VirtualMachine<Running>::container(id)` and `::container_shared(id)` are *not* inherent methods on `firkin-vmm`'s `VirtualMachine<Running>` (which would force `vmm` to know about `ContainerBuilder`, a cross-crate cycle). Instead, a sealed extension trait `CoreContainerFactory` is defined in `firkin-core`, implemented for `VirtualMachine<Running>` in `firkin-core`, and re-exported from `firkin`. Users import it via `use firkin::prelude::*;` or the facade crate's default re-exports and call `vm.container(id)` the same way.
**Rationale**:
- `VirtualMachine<Running>` lives in `firkin-vmm`. `ContainerBuilder<OnVm<'_>, Init>` lives in `firkin-core`. `vmm` cannot import `core` (the dep graph is `core → vmm`, not the reverse).
- This is the same orphan-rule-respecting pattern already used for `StoppableAsync` + `AbortOnDrop<VirtualMachine<Running>>` ([`09-cross-cutting.md §2.2`](./04-library-surface/09-cross-cutting.md)) — the trait + impl both live in `core` because `core` is the only crate that depends on both sides.
- The trait is **sealed** (private supertrait) so consumers cannot opt third-party types into it; only `VirtualMachine<Running>` (and, internally, the Arc variant) implement it.
**Consequence**:
- Users must have `firkin` (the facade) or `firkin-core` in scope to call `vm.container()`; a consumer who imports *only* `firkin-vmm` (just-a-microVM use case) does not get the container factory — which is correct, because without `core` they have no `Container` type to call.
- [`02-vm-surface.md §4.2`](./04-library-surface/02-vm-surface.md) documents the factory alongside the inherent `VirtualMachine<Running>` methods but flags the trait import requirement.
- Same treatment applies to any other "vmm type → core type" method we find during implementation.
**Date**: 2026-04-21.
**Superseded by**: —
**Evidence**: 2026-04-21 plan-pass surfaced the cycle.

---

## D-019 — Multi-container VMs require pre-declared block devices in v0.1

**Decision**: when a user uses `vm.container(id)` / `vm.container_shared(id)` (the multi-container-per-VM escape hatch), Firkin does not attach new VZ storage at container-spawn time. The rootfs is either a `BlockDeviceId` declared on `VmConfig::builder().block_device(path)` before VM boot, or an already-mounted guest path represented by `VmRootfs::GuestPath`. Runtime block-device attach (`VZVirtualMachine.attachDevice:completionHandler:`) is **not** exposed in v0.1 and is not the pod elasticity path.
**Rationale**:
- The public storage hotplug path found locally is USB-controller attach, not general `VZVirtualMachine.attachDevice` storage attach. Building pod elasticity around one new VZ disk per container is the wrong substrate.
- The common Q2/C multi-container workload (e.g., `10 cargo builds + shared cache`) is well-served by predeclared storage. For dynamic pods, one preboot pod-store disk can hold many guest-path rootfs directories.
- The single-use-VM path (`Container::builder(id)` → `ImplicitVm`) is unaffected: that VM is constructed with exactly the one rootfs the container needs, atomically at `.spawn()`.
- Documenting the v0.1 constraint explicitly is cheaper than shipping a runtime-attach API we'll then have to support forever.
**Consequence**:
- `VmConfigBuilder::block_device(path)` (already present in `02-vm-surface.md` under §2.1's attachment builders — verify/add) is the declaration site.
- `Rootfs::OciBundle(bundle)` on a multi-container VM is **not supported in v0.1** on the `OnVm` builder. Users either pre-assemble rootfses as predeclared block devices, or materialize rootfs directories inside a mounted pod store and pass `VmRootfs::guest_path(...)`.
- `10-non-goals.md` keeps general runtime block-device attach out of the first pod path.
**Date**: 2026-04-21.
**Superseded by**: partially — the **matching mechanism** (path-string runtime match) is superseded by [D-022](#d-022--blockdeviceid-replaces-stringly-paired-block_devicepath--rootfsext4_imagepath). The **block-device-only rootfs constraint** is superseded by [D-023](#d-023--rootfs-split-by-vm-context-rootfs-vs-vmrootfs), which adds `VmRootfs::GuestPath` for mounted pod-store rootfses.
**Evidence**: 2026-04-21 plan-pass surfaced that `vm.container()` had no attach path. 2026-05-06 ASIF/pod-store prerequisite work proved the preboot pod-store + guest-path rootfs path with signed live smokes.

---

## D-020 — `oci::Bundle` renamed to `oci::ImageBundle`

**Decision**: the type today spec'd as `oci::Bundle` (the on-disk pulled-image artifact: manifest + config + layers under `$cache/bundles/<digest>/`) is named `oci::ImageBundle`. The shorter `Bundle` name is reserved for future use in `firkin-core` should we ever expose an OCI *runtime* bundle type (the `config.json` + `rootfs/` layout that `runc` consumes).
**Rationale**:
- OCI specs distinguish **image bundle** (registry artifact) from **runtime bundle** (filesystem layout handed to runc). Both are colloquially called "Bundle." Collision would confuse readers and make error messages ambiguous.
- Even if we never materialize a `core::RuntimeBundle` as a public type (current plan: we don't, the spec writing happens inside `vminitd-client` wrappers), reserving the unqualified name costs nothing and prevents a future refactor.
- `Rootfs::OciBundle(oci::ImageBundle)` reads fine; `Rootfs::OciBundle(oci::Bundle)` would force the reader to mentally qualify "which bundle?"
**Consequence**:
- `07-oci-crate.md § Bundle` renames to `§ ImageBundle`.
- `04-value-types.md § Rootfs::OciBundle(oci::Bundle)` → `Rootfs::OciBundle(oci::ImageBundle)`.
- `README.md §2.2` narrative uses `ImageBundle`.
- Rename is pure find-replace at PR time; no semantic change.
**Date**: 2026-04-21.
**Superseded by**: —
**Evidence**: 2026-04-21 plan-pass flagged the collision preemptively.

---

## D-021 — `ContainerBuilder::output()` / `.status()` are the one-shot terminals

**Decision**: `ContainerBuilder<Vm, Ready>` and `ContainerBuilder<Vm, ReadyPty>` expose terminal methods `output()` and `status()` in addition to `spawn()`. `output()` spawns, drains stdout/stderr (auto-piping if unset), waits, returns `Output`. `status()` spawns, waits, returns `ExitStatus`. `spawn()` stays for the "I need the Container handle to interact" case.
**Rationale**:
- The hello-world path `spawn().await?.wait_with_output().await?` fails three of scatter.md's six disciplines at once: noise (four operators for one domain operation), scatter (operation split across two method calls), implicit (`wait_with_output` is the pair that drains; plain `wait` deadlocks on a piped unclosed buffer).
- `tokio::process::Command::output()` / `::status()` solved the identical tension in the stdlib lineage. We inherit the shape.
- Eliminates the pit-of-failure where `.stdout(Stdio::piped())` + `.wait()` silently hangs.
**Date**: 2026-04-21.
**Superseded by**: —
**Evidence**: 2026-04-21 scatter.md review; `tokio::process::Command::output/status` precedent.

---

## D-022 — `BlockDeviceId` replaces stringly-paired `block_device(path)` / `Rootfs::ext4_image(path)`

**Decision**: `VmConfigBuilder::block_device(path)` returns `(Self, BlockDeviceId)`; the `BlockDeviceId` is the handle passed to `Rootfs::block_device(id)` on `OnVm`/`OnVmArc` builders. Runtime path-string matching (previously specified in [D-019](#d-019--multi-container-vms-require-pre-declared-block-devices-in-v01)) is replaced by a typed handle.
**Rationale**:
- D-019's "match by path string at runtime" violated D-007's newtype discipline: a typo produced `ConfigError::RootfsNotPreDeclared` at `.spawn()` time instead of a compile error.
- A typed handle threads the declaration through the VmConfigBuilder → VM → Container chain without stringly-typed indirection.
- Keeps D-019's underlying constraint (rootfses pre-declared at boot; runtime attach is Phase 2) intact.
**Consequence**:
- `VmConfigBuilder::block_device(…)` signature changes to `(Self, BlockDeviceId)` tuple return. The consuming-self builder pattern is preserved.
- `Rootfs` gains a `BlockDevice(BlockDeviceId)` variant; the existing `Ext4Image(PathBuf)` and `RawBlock(PathBuf)` variants stay for the `ImplicitVm` path.
- Invalid cross-VM use of a handle (passing one VM's id to another VM's container) is a runtime check, not a compile check (would require phantom-lifetime parameterization; cost > benefit at this level).
**Date**: 2026-04-21.
**Superseded by**: —
**Supersedes**: the path-matching mechanism in [D-019](#d-019--multi-container-vms-require-pre-declared-block-devices-in-v01). D-019's pre-declaration-requirement invariant holds; only the matching shape changes.
**Evidence**: 2026-04-21 scatter.md review surfaced the stringly-typed pairing.

---

## D-023 — `Rootfs` split by VM context: `Rootfs` vs `VmRootfs`

**Decision**: the `Rootfs` enum (used on `ContainerBuilder<ImplicitVm, _>`) contains `Ext4Image(PathBuf)`, `OciBundle(oci::ImageBundle)`, `RawBlock(PathBuf)`. A distinct type `VmRootfs` (used on `ContainerBuilder<OnVm<'_>, _>` / `OnVmArc`) is a sum type with `BlockDevice(BlockDeviceId)` and `GuestPath(GuestPath)`. `ContainerBuilder::rootfs` is overloaded at the type level: the `ImplicitVm` impl takes `impl Into<Rootfs>`; the `OnVm`/`OnVmArc` impl takes `impl Into<VmRootfs>`.
**Rationale**:
- [D-019](#d-019--multi-container-vms-require-pre-declared-block-devices-in-v01) made `Rootfs::OciBundle` work on `ImplicitVm` but fail-at-spawn on `OnVm`. An enum variant that's valid in some contexts and invalid in others is a class of unsoundness the type system can reject.
- OCI-bundle assembly happens at `.spawn()` time; on `OnVm` the VM has already booted and its block attachments are pre-declared (D-019). No code path ties an `ImageBundle` to an already-booted VM in v0.1; encoding that impossibility in types is cheaper than runtime-erroring it.
- Two types, two method impls, one reader-mental-model: "builders in VM context only accept rootfses that already exist in the running VM, either as predeclared block devices or mounted guest paths."
**Consequence**:
- `04-value-types.md` specifies `Rootfs` (enum, ImplicitVm variants) and `VmRootfs` (`BlockDevice` or `GuestPath`) as two distinct types.
- `01-container-surface.md §2.3.1` splits the `.rootfs()` method into two impls.
- The README narrative in §2.4/§2.5 uses `Rootfs::block_device(id)` (returning `VmRootfs`) — no string-path-matched `.ext4_image()` call.
- Pod elasticity uses `VmRootfs::GuestPath` after the pod store is mounted and the rootfs directory is materialized in the guest. General runtime VZ storage attach remains out of this decision.
**Date**: 2026-04-21.
**Superseded by**: —
**Evidence**: 2026-04-21 scatter.md review — "`Rootfs::OciBundle` works in some contexts but not others" named as Translation (shape) failure.

---

## D-024 — `ext4::OciLayerSource` trait decouples `oci` from ext4 name in method signatures

**Decision**: `ext4` defines a sealed trait `OciLayerSource` with one method (`layers() -> impl Iterator<Item = (&Path, LayerCompression)>`). `ext4::Writer::write_oci_layers` accepts `impl OciLayerSource`. `firkin-oci` implements `OciLayerSource` for `ImageBundle` in its own crate (orphan rule: `oci` owns `ImageBundle`, imports the trait from `ext4`). The previously-spec'd `ImageBundle::layers_for_ext4()` method name is dropped.
**Rationale**:
- `bundle.layers_for_ext4()` named a downstream consumer in the producer's public API — a layering leak. `oci` knew `ext4` existed *at the method-name level*, not just the dep graph level.
- A trait-dispatched signature (`write_oci_layers(&bundle)`) reads cleaner at the call site and lets `ext4` accept fabricated layer sources in tests without fabricating an `ImageBundle`.
- Keeps the `oci → ext4` dep direction (already correct) and `ext4 → oci` non-dep (still correct).
**Consequence**:
- `ext4::Writer::write_oci_layers` has two signatures: the trait-dispatched form (`&impl OciLayerSource`) and a low-level `write_layers_raw<I, P>(I) where I: IntoIterator<Item = (P, LayerCompression)>` for the test path.
- `ImageBundle` exposes `layers() -> &[Layer]` and `Layer::compression() -> ext4::LayerCompression`; no `layers_for_ext4` method.
- README §2.2 becomes `Writer::new(...).write_oci_layers(&bundle)?.finalize()?`.
**Date**: 2026-04-21.
**Superseded by**: —
**Evidence**: 2026-04-21 scatter.md review identified the name-leak; RPITIT stabilized in Rust 1.75 enables the trait method shape.

---

## D-025 — `Container<S>` typestate: `Streams` vs `Pty`

**Decision**: `Container` is generic over a sealed marker type `S: ContainerStdio`, with markers `Streams` (default) and `Pty`. `ContainerBuilder<_, Ready>::spawn()` returns `Container<Streams>`; `ContainerBuilder<_, ReadyPty>::spawn()` returns `Container<Pty>`. `Container<Streams>` exposes `stdin` / `stdout` / `stderr` accessors; `Container<Pty>` exposes `stdin` + `pty`. The `pty()` accessor is infallible (returns `&mut Pty`, not `Option<&mut Pty>`) because the typestate has already proven a pty exists.
**Rationale**:
- The previous shape had `Container::pty(&mut self) -> Option<&mut Pty>` even when the builder typestate had guaranteed a pty. The README `.expect("builder guaranteed a pty")` was evidence the type lied.
- Consistency with `ContainerBuilder`'s `Ready`/`ReadyPty` typestate: what we enforced at builder-time we maintain post-spawn.
- `Process` (exec'd-process handle) gets the same treatment: `Process<S>` with the same markers.
**Consequence**:
- `01-container-surface.md §3` splits the `impl Container` block into `impl Container<Streams>` and `impl Container<Pty>`; common methods (`id`, `pid`, `wait`, `kill`, `stop`, `pause`, `statistics`, `exec`, `copy_in/out`, `dial_vsock`, `virtual_machine`) stay under `impl<S> Container<S>`.
- `04-value-types.md` adds `Streams` / `Pty` marker types and the sealed `ContainerStdio` trait.
- README §2.3 drops the `.expect("builder guaranteed a pty")` — `c.pty()` is infallible.
- `take_pty` stays on `Container<Pty>` as `Option<Pty>` — the `Option` there is honest ("might have been taken already"), not a lie about presence.
**Date**: 2026-04-21.
**Superseded by**: —
**Evidence**: 2026-04-21 scatter.md review — `.expect(...)` flagged as a true/lies discipline failure.

---

## D-026 — `Size` is the one memory setter; `memory_mib(u64)` removed

**Decision**: `VmConfigBuilder::memory(Size)` and `ContainerBuilder::memory(Size)` are the only memory setters. The previously-spec'd `memory_mib(u64)` convenience is dropped. Callers write `Size::mib(2048)` or `Size::gib(2)`.
**Rationale**:
- [D-007](#d-007--beads-rs-philosophy-for-rust-style) committed to newtype-per-number. `memory_mib(u64)` was a Swift-ism with suffix-typing that snuck through, inconsistent with `Size::mib` + newtype arithmetic elsewhere in the surface.
- Two ways to do the same thing is drift (scatter.md). One newtype, one setter, one reader-mental-model.
- Conversion cost at call sites is `Size::mib(N)` vs `.memory_mib(N)` — one method call each; the newtype form is more honest and composable (`.memory(total - reserved)` type-checks; `.memory_mib(total_mib - reserved_mib)` requires hand math).
**Consequence**:
- `01-container-surface.md §2.3.3` drops `memory_mib` from `ContainerBuilder`.
- `02-vm-surface.md §2.1` drops `memory_mib` from `VmConfigBuilder`.
- README §2.4 / §2.5 examples replace `.memory_mib(2048)` with `.memory(Size::mib(2048))` (or `Size::gib(2)`).
- The `Size::as_mib()` / `as_gib()` readers stay — converting out is a distinct direction.
**Date**: 2026-04-21.
**Superseded by**: —
**Evidence**: 2026-04-21 scatter.md review — flagged as inconsistent newtype discipline.

---

## D-027 — Compile-time validated `virtiofs_tag!` / `container_id!` / `hostname!` literal macros

**Decision**: `firkin-types` exports `virtiofs_tag!`, `container_id!`, and `hostname!` macros that validate string literals at compile time and construct the corresponding newtype infallibly. Runtime construction via `VirtiofsTag::new(s)?` / `ContainerId::new(s)?` / `Hostname::new(s)?` remains available for dynamic strings.
**Rationale**:
- The 99% case for these newtypes is a string literal (a statically-known tag name like `"cargo-cache"`, a statically-known container id like `"web"`). Forcing `.new("...")?` or `.unwrap()` on every literal is noise that buries the actual content.
- A declarative `virtiofs_tag!("cargo-cache")` evaluates validation in a `const` context; invalid literals fail at compile time — earlier and louder than any runtime path.
- Keeps the fallible runtime constructor for the actual dynamic case (config file input, user-provided strings); no collision.
**Consequence**:
- `04-value-types.md` §2.5 / §2.1 / §2.6 document the macros alongside the runtime constructors.
- README §2.5 replaces `VirtiofsTag::new("cargo-cache")?` with `virtiofs_tag!("cargo-cache")`.
- Implementation detail (private): macro expands to a const-fn validation call plus the unchecked internal constructor. No `unsafe`; const-eval panic is a compile error.
**Date**: 2026-04-21.
**Superseded by**: —
**Evidence**: 2026-04-21 scatter.md review — `.new(literal)?` noise flagged as minimal-discipline (noise) failure for the common case.

---

## Template for new entries

```
## D-NNN — <short title>

**Decision**: one sentence, imperative.
**Rationale**: bullets of why, with evidence.
**Date**: YYYY-MM-DD.
**Superseded by**: — (or D-MMM).
**Evidence**: pointers to spike-logs, PRO_TIPS sections, external docs.
```
