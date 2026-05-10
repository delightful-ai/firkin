# Non-goals — consolidated deferral catalog

> Covers: every deferred item and architectural non-goal, each with its *reason* and — where applicable — its *unlock condition*. The "what we don't do and why" reference for future contributors.
>
> Rationale for keeping this as a dedicated file: maintainers six months from now should be able to read this single page and answer "why don't we support X?" without archaeology.

---

## 1. Scope deferrals (might do these later)

These are items we *could* do but chose not to in v1. Each has an **unlock condition** — what would make us reopen the decision.

| Item | Reason for deferral | Unlock condition |
|---|---|---|
| **Pods** (`Pod` type with shared-PID-ns semantics) | `LinuxPod` is experimental even in Swift; Q2/C's raw `vm.container()` covers multi-container-per-VM for 80% of real use | Real user demand for pause/shared-PID-ns/per-container-resource-override semantics that the raw-VM pattern doesn't cover |
| **Runtime block-device attach** on `VirtualMachine<Running>` | Per [D-019](../DECISIONS.md#d-019--multi-container-vms-require-pre-declared-block-devices-in-v01): the first elastic-pod path is a preboot pod-store disk plus `VmRootfs::GuestPath`, not one new VZ disk per container. | A proven USB/NBD/NVMe hotplug use case that cannot be handled by a preboot pod store |
| **`Rootfs::OciBundle(...)` on `OnVm` / `OnVmArc` builders** | OCI bundle assembly at running-VM time now belongs to pod-store materialization, which produces a guest path and then uses `VmRootfs::GuestPath`. The `Rootfs` enum remains the implicit-VM rootfs surface. | Guest-side OCI materializer with whiteout/xattr/hardlink fidelity |
| **Bridged networking** (`VZBridgedNetworkDeviceAttachment`) | Requires paid Apple Developer Program + restricted `com.apple.vm.networking` entitlement + matching provisioning profile ([D-002](../DECISIONS.md#d-002--ad-hoc-codesigning-base-virt-entitlement-only)) | Contributor with the paid-program setup; deliverable is a separate feature crate with its own signing story |
| **OCI signature verification** (sigstore / cosign / Notary) | ~500 LOC + crate surface; most v1 consumers don't gate on this | Supply-chain concern raised by a real consumer; ship as an additional `oci::verify` module or sibling crate |
| **Resumable / incremental OCI pull** | Happy path handles retries; partial resume is niche | User request with a workload that justifies it (e.g., multi-GB model layers on flaky networks) |
| **Container stats streaming** (subscribe to metrics over time) | `container.statistics()` is request/response; streaming adds a new shape | Real observability-pipeline integration asks for it |
| **VZ graphics / display attachments** | Not a container concern. | — (architectural non-goal for this library; see §2.3) |
| **Mock `VirtualMachine` for testing** | Creating a seam purely for mocking violates [D-007](../DECISIONS.md#d-007--beads-rs-philosophy-for-rust-style). Consumers mock at the layer above | — |
| **Custom test-only VM backends** | Same reason | — |
| **Non-blocking `Stdio::Inherit`-with-buffering mode** | `Inherit` relay tasks are simple today; buffered variants add complexity without clear demand | Specific perf complaint |
| **Explicit "cold VM pool"** (warm VMs kept ready for fast spawn) | Snapshot + restore covers the same perf win with less library-state-to-manage | If snapshot proves insufficient for the use case |

---

## 2. Architectural non-goals (not in this library ever, by design)

These are **not deferrals**. They're items that belong in *sibling* tools or crates, not inside `firkin`.

### 2.1 Image building

**Dockerfile interpretation and image building** are out of scope for this library, v1 and ever. A future sibling tool may use *this* library's `oci`, `ext4`, and `core` primitives to build images, but the interpreter itself does not live here.

Reasons:
- Docker and OCI already separate "build image" from "run image" into distinct subsystems. containerd (analogous to this library) doesn't interpret Dockerfiles; BuildKit does. runc doesn't; Buildah does.
- Dockerfile interpretation is a **huge** surface — COPY/ADD with URL resolution and cache-busting, RUN with shell/exec split, build args, multi-stage builds, heredocs, BuildKit-specific features (secrets, SSH mounts, cache mounts, mount types). It's effectively a scripting-language interpreter.
- Apple's own Swift `apple/containerization` library doesn't include this; `apple/container` (the CLI) has build support in a separate layer.

**What a future sibling tool would look like**: a crate `firkin-build` (or `buildah-rs`-shaped) that imports this library's `oci::Client` (to push built images), `ext4::Writer` (to assemble intermediate rootfses), and `Container::spawn` (to run build steps in ephemeral containers). Straightforward composition; no changes needed here.

### 2.2 OCI image `push`

This library is a **runtime**, not a publisher. `oci-client` (which our `oci` crate wraps) supports push; we deliberately don't expose it.

Reasons:
- Symmetric with "no image building": publish is a build-time concern, not a runtime concern.
- The `oci::Client` API is shaped for pull — adding push would double its surface and complicate error semantics (a `push` can fail in registry-side-half-complete ways that `pull` doesn't).
- A future `oci-push` crate can layer on top of our `oci::ImageBundle` type (renamed from `Bundle` per D-020) if demand arises.

### 2.3 GUI / graphics / audio / USB attachments

VZ supports these attachments for macOS-guest VMs. They're **not a container concern** — containers are headless by design, communicate via stdio/vsock/network, and don't have UIs.

If someone wanted a VZ-backed macOS-guest runtime (running full macOS inside a VM), that's a sibling library (see §3 below), not an extension of this one.

### 2.4 Docker-compatible CLI

`docker run`-shaped CLI ships as a **separate product** on top of this library, matching the `apple/container` vs `apple/containerization` split. [D-009](../DECISIONS.md#d-009--no-cli-as-product-in-v1) committed to this.

The dev-facing CLI in this workspace (`crates/cli/`) is *not* the product — it's a test harness for exercising the library during development.

### 2.5 vminitd rewrite in Rust

vminitd stays Swift; we bundle the ELF. [D-003](../DECISIONS.md#d-003--embed-vminitd-elf-not-initblock).

Reasons:
- vminitd is ~80k LOC of Swift. Porting is a separate multi-month project with zero architectural benefit.
- vminitd has a stable gRPC API (`SandboxContext.proto`) that we can pin and consume. The binary itself is the source of truth.
- Cross-building Swift to a static Linux ELF is a solved problem (S3 validated the recipe).

### 2.6 Partial-pull formats (stargz / eStargz)

Lazy-load-layers-as-used is a different runtime model from full-pull. Belongs in a parallel library if anyone wants it on macOS/VZ.

### 2.7 Manifest v1 (legacy Docker)

Deprecated ≥10 years; nearly all registries serve v2 schemas. Skipping.

### 2.8 Nested exec (`exec` into an `exec`'d Process)

Not supported. Users who want another process run `container.exec(id_2, cfg_2)` on the parent Container, not on a Process handle.

Reasons:
- Docker doesn't support nested exec either.
- `runc exec` is always against a container, not a process — the OCI runtime spec itself doesn't model nested exec.
- If a user wants tmux-like session multiplexing, they run tmux inside the container and `container.exec(…, .pty(…))` produces additional pty-carrying endpoints to the same tmux.

### 2.9 Post-boot rootfs mutation API

No "edit the ext4 image while it's mounted" verb. Users call `copy_in(host, guest)` to write into a running container (whose writable layer gets the change) or rebuild the rootfs and restart for permanent changes.

Reasons:
- Ext4 isn't designed for concurrent host + guest mutation.
- Writable-layer overlay semantics are the right primitive for "persistent in-container changes."
- `copy_in` is the one-shot equivalent.

---

## 3. Platform / target non-goals

### 3.1 Pre-macOS 26 support

No scaffolding, no runtime checks for older versions, no compile-time gates for legacy APIs. [D-001](../DECISIONS.md#d-001--macos-26-only) committed to this.

**Reasons**:
- apple/container's own CLI has the same floor.
- vmnet shared-mode ad-hoc signing only works on 26+ (S6 evidence; on 13–15 it required provisioning profile + paid developer program).
- Cutting the scaffolding once is easier than carrying it forever; this library is v0 greenfield, no prior users to support.

**Users on older macOS**: they can't use this library. They should use `apple/container` (requires 26+), `colima`, `orbstack`, or `docker-desktop`.

### 3.2 Linux host targets for `core` / `vmm`

`objc2-virtualization` is macOS-only. `core` and `vmm` don't build on Linux, intentionally.

**`ext4` and `oci` do** build and run on Linux for fast CI.

**Users who want to run OCI containers on Linux**: use runc / containerd / podman / crun — they do it better than anything Rust-shaped could.

### 3.3 Intel macOS as a tested target

`x86_64-apple-darwin` builds and probably works (VZ is available on both architectures). Not in CI's blocking test matrix; we don't actively test Intel-specific gotchas.

**Why the asymmetry**: M-series Macs dominate new developer machines; Apple Silicon is the "primary" target for anyone starting greenfield.

### 3.4 Windows / WASM / other OSes

Not applicable. VZ is Apple-only.

### 3.5 Running macOS guests

VZ supports macOS guests via `VZMacOSVirtualMachineConfiguration`. Our `vmm` crate is shaped for Linux guests:

- `VirtualMachine<_>` types assume `VZLinuxBootLoader` + vminitd on vsock 1024.
- `Container` abstraction is meaningless for macOS guests (no cgroups/namespaces/runc).
- Device attachments are Linux-oriented (virtio-block, virtio-net, virtiofs, no graphics/audio/pointing-device).

**Future extension path** (if anyone ever wants it): split `vmm` into:

- **`vmm-core`** — shared VZ primitives: dispatch queue, `VzSend<T>`, delegate subclasses, codesigning story, preflight, `VsockStream`, error types.
- **`vmm-linux`** — what's currently `vmm`: `LinuxVirtualMachine<S>`, `LinuxVmConfig`, container-oriented attachments.
- **`vmm-macos`** (new) — `MacVirtualMachine<S>`, `MacVmConfig`, macOS-specific attachments, `VZMacAuxiliaryStorage` handling, graphics/keyboard/pointing.

The `Container` / `core` stack would depend only on `vmm-linux`. A macOS-guest consumer would depend on `vmm-core` + `vmm-macos` and build their own top layer.

**This is a refactor, not a rewrite** — but the user-facing types are completely different (no Container, no ext4-rootfs model) so it's a parallel library with shared substrate, not an extension of this one.

**Unlock condition**: a concrete use case and a contributor willing to do the split.

---

## 4. Guest-OS / kernel non-goals

Our bundled kernel (verified via explorer agent during design review; source at `kernel/config-arm64`) has:

- **LSM framework OFF** (`CONFIG_SECURITY=n`). No SELinux, AppArmor, Smack, Tomoyo, Yama, Landlock, Lockdown. `selinuxLabel` / `apparmorProfile` fields pass through to runc but are **inert** — the guest kernel can't enforce them.
- **Seccomp ON** (`CONFIG_SECCOMP=y`, `CONFIG_SECCOMP_FILTER=y`). Full structured + pass-through seccomp support in v1.
- **Standard cgroup v2** — what runc / OCI containers expect.

**Users who need LSM enforcement**: they supply their own kernel via `VmConfig::kernel(KernelImage::from_file(…))` built with `CONFIG_SECURITY=y` + the LSM module of choice.

We do NOT ship alternate kernels. The escape hatch (`KernelImage::from_file`) exists precisely for this.

---

## 5. VM / hardware feature non-goals

Beyond §2.3 above:

- **USB / serial passthrough** — not a container concern.
- **Memory ballooning** — actually **in** v1 behind the `balloon` Cargo feature.
- **VM snapshot / restore** — actually **in** v1 behind the `snapshot` Cargo feature (pending S10 verification).
- **Live migration** — VZ doesn't support this; no macOS host can migrate a running VM to another host. Not applicable.
- **GPU passthrough (CUDA, Metal)** — VZ supports Metal acceleration for macOS guests; irrelevant for Linux containers. Not applicable.

---

## 6. API shape non-decisions (lockked choices that won't be reopened without cause)

- **No `Container` lifecycle typestate.** [Q3 audit](./01-container-surface.md) found typestate at the Container level doesn't earn its keep.
- **No `cancellable_*` method suffixes.** Drop-future-is-cancel for external; internal token tree for cascading stop() propagation.
- **No `Stdio::Inherit` as default.** `Null` is the only non-chatty, non-deadlocking default.
- **No stringly-typed configs.** `Mount`, `LinuxCapabilities`, `LinuxRlimit`, `Network`, `Rootfs`, `Capability`, `RlimitKind` — all enums over structured values.
- **No runtime-agnostic async.** tokio-committed per [D-013](../DECISIONS.md#d-013--async-tokio-committed).
- **No `anyhow::Error` in capability APIs.** Per-crate `thiserror` enums; anyhow only at the user's top level.
- **No god-Error across crates.** Peer capability enums; `core::Error` wraps lower crates' enums via `#[source]`.
- **No traits for single-impl types.** Per [D-007](../DECISIONS.md#d-007--beads-rs-philosophy-for-rust-style) + [`trait_design.md`](../../../../../../src/personal/beads-rs/docs/philosophy/trait_design.md).

---

## 7. Testing infrastructure non-goals

- **No mock `VirtualMachine` / `Container` / `Process`** in v1 (per §1 above and [`08-vmm-crate.md § testing`](./08-vmm-crate.md)).
- **No in-process "simulator" mode** for running tests without VZ. Integration tests need real VZ.
- **No WASM-compatible test mode.** Same reason.

---

## 8. Summary — what v1 is

Not just "the positive list of features," but also the reasons these items are out, so a maintainer six months from now can read this section and answer "why don't we support X?" without archaeology.

The positive list:

1. **Linux OCI containers** on macOS 26+ Apple Silicon (and best-effort Intel macOS).
2. **Single-use-VM and multi-container-on-one-VM** execution models.
3. **OCI pull** from standard registries (including multi-arch, zstd, Docker Hub, keychain auth).
4. **EXT4 rootfs writing** with `mkfs.ext4`-parity feature set.
5. **vminitd-backed exec**: processes, stdio, pty, vsock, copy_in/out, statistics.
6. **Networking**: NAT, vmnet-shared (with custom subnet), DNS/hosts configuration.
7. **Cross-arch execution** via Rosetta.
8. **VM snapshot + ballooning** (Cargo-feature-gated).
9. **Seccomp** (structured + pass-through).
10. **Ad-hoc codesigning** with the `com.apple.security.virtualization` entitlement only.

Not in v1:

1. Pods (type).
2. Bridged networking.
3. OCI push / image building.
4. Docker-compat CLI.
5. vminitd rewrite.
6. OCI signature verification.
7. LSM enforcement on the bundled kernel (pass-through only).
8. Pre-macOS-26 support.
9. Linux hosts for `core` / `vmm`.
10. macOS-guest VM support.
11. Runtime block-device attach on `VirtualMachine<Running>` (D-019 constrains the multi-container path to pre-declared block devices in v0.1).
12. `Rootfs::OciBundle(...)` on `OnVm` / `OnVmArc` builders (consequence of item 11).

Everything on the negative list has a reason in this file. Everything on the positive list links to the section that specifies it. No archaeology needed.

---

Continue exploration of this design in:
- [`README.md`](./README.md) for scope + narrative
- [`01-container-surface.md`](./01-container-surface.md) for the Container API
- [`02-vm-surface.md`](./02-vm-surface.md) for the VirtualMachine API
- [`05-error-model.md`](./05-error-model.md) for error discipline
- [`09-cross-cutting.md`](./09-cross-cutting.md) for Send/Sync, drop, cancellation, tracing, versioning
- Or implementation work — this design is ready for [`writing-plans`](../../../../../.claude/plugins/cache/superpowers-marketplace/superpowers/4.3.0/skills/writing-plans/) to turn into tasks.
