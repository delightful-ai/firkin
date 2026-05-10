# Rust rewrite — running notes

Question being answered: *can we build a Rust library equivalent to Apple's `Containerization` Swift package (the library, not the `container` CLI) that runs OCI containers in microVMs on Apple Silicon via `Virtualization.framework`?*

Short answer: **yes, with one hand-port (EXT4 writer) and one small transport shim (vsock ↔ tonic).** Everything else has a usable Rust crate or can be reused verbatim from this repo.

---

## Status as of 2026-04-20 — **evidenced, not speculative**

Nine spikes (see `02-spike-plan.md`). As of commit `bcaec8aa`:

| Spike | State | Evidence |
|---|---|---|
| S1 — Boot VZ VM from Rust | ✅ | 46 ms guest boot, clean shutdown via delegate |
| S2 — Vsock ↔ tonic | ✅ | ~400 µs RTT, no fd leaks over 1000 iters |
| S3 — Cross-build vminitd | ✅ | vminitd on vsock 1024, reproducible toolchain recipe |
| S4 — End-to-end container exec | ✅ | `echo hello` runs in a real busybox container via inverse-vsock stdio |
| S5 — EXT4 writer | ✅ | Tiers 1–4 all passed; depth-1 extents, whiteouts, opaque-dir markers, guest-mount probes |
| S6 — Entitlements / vmnet | ✅ | ad-hoc + base virt entitlement unlocks vmnet shared on macOS 26+ |
| S7 — Rosetta | ✅ | amd64 `uname -m` → `x86_64` in arm64 guest |
| S8 — Bundling benchmark | ✅ | embed 131 MiB vminitd ELF (not init.block); fallback C behind feature |
| S9 — vmnet reachability | ✅ | container gets IP via vminitd netlink RPCs; host→container 0.2 ms; outside-world reaches; DNS works |
| S10 — VZ snapshot/restore for vminitd shape | — | not started; load-bearing for v1 snapshot feature + integration-test speedup (10–20×/test) |

**Every scary engineering question has been answered on this machine.** Nothing in Phase 1 is speculative.

### Key architectural decisions that fell out of spikes

See [`DECISIONS.md`](./DECISIONS.md) for the full list with rationale. Top ones:

- **macOS 26+ only.** Matches apple/container's floor; unlocks vmnet shared under ad-hoc codesigning.
- **Ad-hoc signing + `com.apple.security.virtualization` entitlement only** for the full v1 feature set (NAT + vmnet shared + Rosetta). No paid Apple Developer Program.
- **Ship the 131 MiB vminitd ELF, not the 384 MiB `init.block`.** The `ext4` crate synthesizes `init.block` on-host from the ELF — 2.4× cold-build speedup, 2.9× final-binary-size cut.
- **Container stdio is inverse-vsock** (guest dials back to host-side `VZVirtioSocketListener`s), not streaming RPC. The library needs a listener-delegate pattern alongside its connector.
- **Network configuration is via vminitd netlink RPCs**, not DHCP in the container. Five-RPC sequence (`IpLinkSet` / `IpAddrAdd` / `IpLinkSet` / `IpRouteAddDefault` / `ConfigureDns`) runs in <100 ms. No `dhclient` / `udhcpc` in container rootfs images.
- **One VM per container.** Multiple containers on the same vmnet network share a `vmnet_network_ref`; each container's VM has its own `VZVmnetNetworkDeviceAttachment` pointing at it.

### Deliverables durably on disk (in this directory)

- **`PRO_TIPS.md`** — 33 sections of gotchas and patterns. Read first when touching Rust/VZ/vminitd.
- **`SPIKE_RUNBOOK.md`** — conventions for running spikes (stub-file rule, watchdog pattern, parallelization rules).
- **`CONCEPTS.md`** — grounding glossary (vmnet modes, virtio-\* devices, initramfs vs rootfs, codesigning vs entitlements vs profiles).
- **`DECISIONS.md`** — ADR-style log of architectural decisions.
- **`04-phase1-plan.md`** — execution plan for shipping v0.1.0. Walking skeleton, crate build sequence, rewrite checklist for spike→library translation, risk register, deferrals, first-PR sequence.
- **`spike-template/`** — materializable scaffold (`scaffold.sh <N> <topic>`). Tested cold.
- **`spike-logs/s{1..9}-*/`** — JOURNAL / STATUS / FINDINGS per spike.

---

---

## Target = the library

This repo ships **Swift packages**, not a CLI:

```
Sources/
  Containerization           ← VZ lifecycle + vminitd client (the facade)
  ContainerizationOCI        ← registry + image spec
  ContainerizationEXT4       ← rootfs writer (4,439 LOC — the rabbit hole)
  ContainerizationArchive    ← tar handling
  ContainerizationNetlink    ← in-guest net config
  ContainerizationIO / OS    ← plumbing
  cctl                       ← test CLI (not the product)
vminitd/                     ← PID-1 guest agent
kernel/                      ← optimized Linux kernel config
```

The shipping `container` CLI lives in `apple/container` and depends on this as a Swift package. A Rust port should mirror the same split at the **crate** level; a CLI is a second, much smaller project on top.

---

## Why VZ (not Hypervisor.framework)?

- VZ is higher-level: `VZVirtualMachine`, `VZLinuxBootLoader`, virtio device configs, vsock device, Rosetta share, vmnet attachments — all ready-made.
- VZ is **Objective-C**, not Swift-only. Every `VZ*` type is `NSObject`-subclassed with message-send ABI → directly reachable from Rust via `objc2` with **zero Swift in the middle**.
- Hypervisor.framework is the layer *below* VZ — raw vCPU/memory, no devices. That's what libkrun/krunkit use, and they rebuild device/boot/virtio emulation themselves.

If we want "boot a Linux VM with virtio block/net/vsock and a bootloader," VZ is the right altitude. Hypervisor.framework is the right choice only if we want total control (and a lot more code).

---

## Component map: what Rust has, what we write

### Free (reuse verbatim)
- **`vminitd`** — Swift-compiled static Linux ELF. PID 1 inside the guest. Exposes gRPC over vsock. Handles mount/env/network/process-lifecycle via `SandboxContext.proto`. Wraps `runc` for actual OCI execution. We build it once, ship the binary, never touch it.
- **`kernel/`** — optimized kernel config for sub-second boot.
- **`SandboxContext.proto`** — stable guest API.

### Rust crates (piggyback)
- **`objc2-virtualization`** (part of madsmtm/objc2) — auto-generated, maintained bindings to `Virtualization.framework`. Covers `VZVirtualMachine`, bootloaders, all device configs, delegate protocols. **This is the keystone.**
- **`oci-client`** (was `oci-distribution`, now at oras-project/rust-oci-client) — registry client.
- **`oci-spec`** — OCI spec types.
- **`rtnetlink` / `netlink-packet-route`** — netlink on the host side if needed.
- **`tonic`** + `prost` — gRPC client for `SandboxContext.proto`.
- **`tokio`** — obviously.
- **Maybe**: `erofs-rs` / `squashfs-tools-rs` — as an *alternative* to porting EXT4 writer (see Gotchas).

### Rust crates we probably don't use but should know about
- **`virtualization-rs`** (suzusuzu) — older hand-rolled bindings, predates objc2's framework crates. Historical interest.
- **`virt-fwk`** — another safe-wrapper take. Smaller/less proven than objc2-virtualization.
- **`applevisor`** (Impalabs) / **`xhypervisor`** (RWTH-OS) — Hypervisor.framework bindings. Wrong altitude for us.
- **`youki`** — Rust OCI runtime. Runs *inside* Linux. Could eventually replace runc inside vminitd; doesn't help host-side.
- **`bollard`** — Docker API client. Talks to an existing daemon. Not what we're building.

### Prior art we're not using but should mentally benchmark against
- **libkrun + krunkit** (containers org) — Rust/C library+CLI that runs OCI containers in microVMs on macOS via **Hypervisor.framework**, not VZ. Closest existing project to our goal. Uses a different guest agent and a different rootfs strategy. Worth reading for architectural hints; not a drop-in base.
- **vfkit** — Red Hat Go wrapper around VZ. Tiny. Good mental model for what the "just boot a VM" surface looks like.

### New Rust code we have to write
1. **Host VM driver** (`objc2-virtualization` wrapper). ~a week of Rust-side surface.
2. **vsock ↔ hyper transport for tonic.** Small but nobody's done it: VZ's host-side vsock is `VZVirtioSocketDevice.connect(toPort:)` returning a file descriptor via delegate callback. Wrap that fd as `tokio::io::AsyncRead+AsyncWrite`, feed to hyper as a custom `Connector`. ~200 lines.
3. **OCI pull + layer extraction orchestration.** Off-the-shelf crates but glue and whiteout handling. ~1000 lines.
4. **EXT4 writer.** **4,439 Swift LOC equivalent.** Either port `ContainerizationEXT4` or sidestep with read-only squashfs + tmpfs overlay. This is the single biggest unknown.
5. **Orchestrator.** State machine: pull image → build rootfs → boot VM → await vsock → stage mounts/env/net via gRPC → `CreateProcess` + stream stdio → `WaitProcess` → teardown.
6. **Networking.** `VZNATNetworkDeviceAttachment` is free. IP-per-container needs `vmnet.framework` (Obj-C, objc2-reachable) + entitlements. This repo's `VmnetNetwork.swift` is a reference.
7. **`VsockListener`/`VsockProxy` analogues.** See `Sources/Containerization/VsockListener.swift`.

---

## Gotchas ranked by pain

1. **EXT4 image authoring.** 4,439 LOC to port, ~1,298 of which is the formatter. No good Rust writer exists. Escape hatch: use erofs/squashfs read-only + overlayfs tmpfs inside guest, accept that perf/compat might differ.
2. **vsock ↔ tonic transport.** Small but unsolved. One-off glue.
3. **vmnet entitlements + code signing** for the IP-per-container story. Solvable but bureaucratic.
4. **Cross-arch (Rosetta).** Needs `VZLinuxRosettaDirectoryShare` on host + `binfmt_misc` registration inside guest (vminitd's `SetupEmulator` RPC already handles the guest side).

Everything else is straightforward plumbing.

---

## Rough sizing

- Single-dev MVP ("pull an image and run `echo hello`"): ~2 weeks.
- Real Docker-compatible CLI: ~2–3 months, dominated by image/rootfs edge cases and networking polish. Not by VZ or vminitd.

---

## Settled decisions

Architectural decisions live in [`DECISIONS.md`](./DECISIONS.md) (D-001..D-027 as of 2026-04-21). Crate layout lives in [`03-project-layout.md`](./03-project-layout.md). Public API surface lives in [`04-library-surface/`](./04-library-surface/). Execution plan lives in [`04-phase1-plan.md`](./04-phase1-plan.md).

Headline: **library-first** Rust workspace named **`firkin`** (D-014); bundled vminitd ELF resolved via pinned download (D-017); `ext4` an independently publishable crate (D-004); macOS 26+ floor (D-001) with ad-hoc signing + single entitlement (D-002) covers NAT + vmnet-shared + Rosetta; MSRV policy in [`04-library-surface/09-cross-cutting.md § 5`](./04-library-surface/09-cross-cutting.md).

## Index

- `00-notes.md` — this file. Architecture overview.
- `01-ecosystem-verification.md` — crates cloned, cargo-checked, VZ symbol coverage audit.
- `02-spike-plan.md` — the spikes we run before writing library code (S1–S10).
- `03-project-layout.md` — workspace shape, crate responsibilities, build/bundle/release, CI, risk register.
- `04-library-surface/` — public API surface design (directory). `README.md` is the landing doc; per-concern files for Container, VM, stdio, value types, errors, ext4, oci, vmm boundary, cross-cutting concerns, and non-goals.
- `CONCEPTS.md` — grounding vocabulary for VZ, virtio devices, vsock, initramfs vs rootfs, codesigning / entitlements / profiles.
- `DECISIONS.md` — ADR-style log of architectural decisions (D-001..D-027).
- `PRO_TIPS.md` — dense gotchas from spikes; read before touching threads, `define_class!`, codesigning, vsock, or vmnet.
- `SPIKE_RUNBOOK.md` — conventions for running spikes (stub-file rule, watchdog pattern, parallelization rules).
- `spike-logs/` — per-spike JOURNAL / STATUS / FINDINGS.
- `spike-template/` — `scaffold.sh <N> <topic>` for a known-good starting binary.
