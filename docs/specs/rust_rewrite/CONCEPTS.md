# Concepts — the macOS-virtualization vocabulary this project speaks

Plain-English grounding for terms that appear throughout `PRO_TIPS.md`, `02-spike-plan.md`, and the spike findings. Skim this first if you haven't worked with macOS `Virtualization.framework` / OCI containers / virtio before.

Not a spec — a vocabulary. For the authoritative definitions, chase the links.

---

## The big picture

On macOS we want to run Linux containers. The stack:

```
┌───────────────────────────────────────────────────────────────┐
│  Your Rust binary (on macOS)                                  │
│  ├─ vmm crate       → talks to VZ (Virtualization.framework)  │
│  ├─ vsock crate     → wraps VZ's vsock into tonic Channels    │
│  ├─ vminitd-client  → gRPC stubs for SandboxContext.proto     │
│  ├─ ext4 crate      → writes ext4 images, on host             │
│  └─ oci crate       → pulls images, extracts layers           │
└──────────────┬────────────────────────────────────────────────┘
               │ VZ = Obj-C framework; we reach it via objc2
               ▼
┌───────────────────────────────────────────────────────────────┐
│  VZ (Virtualization.framework) — Apple-provided hypervisor UI │
└──────────────┬────────────────────────────────────────────────┘
               │ configures a KVM-like VM
               ▼
┌───────────────────────────────────────────────────────────────┐
│  Linux guest VM                                               │
│  ├─ vminitd (PID 1, Swift-compiled static Linux ELF)          │
│  │   ↓ fork+exec per container                                │
│  └─ runc + container rootfs (YOUR container's process)        │
└───────────────────────────────────────────────────────────────┘
```

The library's job is to compose the top half so the bottom half runs your container.

---

## VZ — `Virtualization.framework`

Apple's framework for creating + running virtual machines on macOS. Higher-level than `Hypervisor.framework` (which is just raw vCPU + memory). VZ gives you ready-made virtio devices, bootloaders, and a delegate protocol for lifecycle events.

Every VZ type is `NSObject`-subclassed Objective-C, so it's reachable from Rust via `objc2` with no Swift in between.

---

## Virtio devices

"virtio" is a standard for paravirtualized devices: the guest and host agree on a message format that's efficient because the guest *knows* it's virtualized. VZ exposes a virtio device per bus type:

| Device | Purpose | VZ class |
|---|---|---|
| `virtio-console` | A serial port. Kernel boot log + `console=hvc0` in cmdline both end up here. | `VZVirtioConsoleDeviceSerialPortConfiguration` |
| `virtio-block` | A disk. Guest sees it as `/dev/vda`, `/dev/vdb`, … in attach order. | `VZVirtioBlockDeviceConfiguration` + `VZDiskImageStorageDeviceAttachment` |
| `virtio-net` | An ethernet NIC. Guest sees `eth0`. Backed by a host-side attachment (NAT / vmnet / bridged). | `VZVirtioNetworkDeviceConfiguration` |
| `virtio-vsock` | A socket-like device for host↔guest communication on a "context ID + port" address. No IP stack needed; the framework handles framing. | `VZVirtioSocketDeviceConfiguration` + `VZVirtioSocketDevice` + `VZVirtioSocketListener` |
| `virtiofs` | A shared directory visible in the guest as a filesystem. We use this to hand Rosetta into the guest. | `VZVirtioFileSystemDeviceConfiguration` |

The guest mounts or uses them via the standard Linux drivers (ext4 over virtio-block, `AF_VSOCK` over virtio-vsock, etc.).

---

## Vsock — virtio-vsock

A guest/host communication channel that looks like a socket but doesn't route through IP. Addresses are `(CID, port)` instead of `(IP, port)` — CID 2 is the host, CID 3+ are guests.

- Host **dialing in**: `VZVirtioSocketDevice::connectToPort_completionHandler`. Completion block hands you a `VZVirtioSocketConnection`; call `.fileDescriptor()`, `dup()` it, and you have a real SOCK_STREAM fd you can hand to tokio. (`PRO_TIPS §13`.)
- Guest **dialing back to host** (our inverse case for container stdio): host runs a `VZVirtioSocketListener` with a delegate that accepts new connections on a given port number. Delegate returns true → VZ hands you the fd; same dup pattern. (`PRO_TIPS §20`.)

Vminitd listens on vsock port **1024** by default for its gRPC service.

---

## vminitd — the in-guest agent

Swift-compiled static Linux ELF that serves as PID 1 inside the guest VM. Apple's own code; we build it and ship the bytes. It:

- Mounts `/run` (tmpfs), `/sys`, `/sys/fs/cgroup`, binfmt_misc at startup.
- Exposes a gRPC service (`SandboxContext`) on vsock port 1024.
- Takes RPCs: `Mount`, `WriteFile`, `CreateProcess`, `StartProcess`, `WaitProcess`, `SetupEmulator` (for Rosetta), etc.
- Under `CreateProcess`, forks + execs a `vmexec` helper that `chroot`s into the container rootfs and execs the user's process via runc-spec rules.

Key quirk: vminitd **computes the container bundle path itself** (hardcoded `/run/container/<id>`) and writes its own `config.json`. Our `WriteFile(config.json)` is functionally a no-op — useful as a smoke test but the guest uses its own copy. See `PRO_TIPS §21` for the full list of vminitd quirks to budget for.

---

## Networking — attachment types

VZ gives you three ways to connect a VM's NIC to the outside world. Pick one per VM.

### `VZNATNetworkDeviceAttachment`

VZ's built-in NAT. All VMs on the host share one hidden private subnet. Simple, ad-hoc-signable, but: no per-container IPs, everything-through-host-IP, port-collision hell for multi-service apps.

### `VZVmnetNetworkDeviceAttachment` — shared mode (v1 default)

Uses `vmnet.framework` under the hood. Host creates (or uses the shared) vmnet "network" — a subnet like `192.168.64.0/24`. Each attached VM gets its own IP from vmnet's DHCP server. Guest can reach the outside via NAT. External hosts cannot initiate connections to the guest.

**This is what apple/container ships with and what v1 uses.** On macOS 26+, ad-hoc signing + `com.apple.security.virtualization` is sufficient — no paid dev program.

### `VZVmnetNetworkDeviceAttachment` — bridged mode

The VM's NIC is bridged to a real physical NIC on the host. Guest gets an IP from the physical-LAN DHCP server; other machines on the office/home network can `ping` it. Requires `com.apple.vm.networking` restricted entitlement + matching provisioning profile + paid Apple Developer Program. Deferred to Phase 3. The type is technically `VZBridgedNetworkDeviceAttachment` — a distinct class, same entitlement family.

### `VZVmnetNetworkDeviceAttachment` — host-only mode

Isolated — guest can only talk to the host. Niche. Not used here.

### What "IP-per-container" actually means

With vmnet shared mode, each container (in our case, each VM, because apple/container's model is one VM per container) gets its own IP. Two containers can reach each other via TCP directly (`curl http://192.168.64.3:5432`) without any host-side port forwarding. This is the user-facing networking expectation Docker users bring, and is the reason vmnet matters for a serious container runtime.

---

## initramfs vs. rootfs

Two different kinds of "filesystem the guest sees":

- **initramfs** (a.k.a. initrd) — a tiny cpio archive the kernel loads into memory at boot. The kernel mounts it as the *initial* root fs, finds `/init`, and execs it. Commonly used to set up devices / load modules before pivoting to the real rootfs.
- **rootfs** — the long-lived filesystem your init runs inside. For us, the guest's rootfs is `init.block` (ext4, read-only) containing vminitd. Each container then gets its *own* rootfs as a second `virtio-block` device (`/dev/vdb`), which vminitd mounts at `/run/container/<id>/rootfs`.

Early spikes (S1) used initramfs for simplicity — a tiny `init.c` prints "hello" and powers off. Real spikes (S3, S4, S7, S9) use block-device rootfs + vminitd.

---

## Linux boot loader config

VZ's `VZLinuxBootLoader` takes three inputs:

- `kernelURL` — path to an uncompressed Linux kernel image. On arm64, this is the raw `Image` format (ARMd magic + MZ/PE stub). **Uncompressed** — a gzipped `vmlinuz.gz` won't boot; decompress first.
- `initialRamdiskURL` — optional initrd/initramfs cpio. If set, kernel loads it into memory + looks for `/init` inside.
- `commandLine` — Linux kernel cmdline. Typical:
  - `console=hvc0` — send kernel log to virtio-console (host stdout in our case).
  - `root=/dev/vda` — mount first virtio-block as root. Skip if using initramfs.
  - `rootfstype=ext4` — explicit, avoids probe surprises.
  - `ro` — mount root read-only (vminitd expects this).
  - `init=/sbin/vminitd` — which binary to exec as PID 1.
  - `panic=-1` — reboot on kernel panic, instead of hanging.

---

## Codesigning, entitlements, provisioning profiles — three distinct things

These get conflated a lot. They aren't the same:

### Codesigning (`codesign`)

Attaches a cryptographic signature to a Mach-O binary claiming "this binary is what the signer says it is". Three modes:

- **Ad-hoc** (`codesign --sign -`): uses no identity; the signature says nothing about who signed it. Sufficient for most local-dev VZ use.
- **Apple Development cert**: a developer identity issued by Apple to a specific team. Required for signing binaries for internal distribution or TestFlight.
- **Developer ID cert**: for signing binaries distributed outside the App Store. Different flavor.

### Entitlements (`entitlements.plist`)

An XML file listing *capabilities* the binary is requesting:

```xml
<dict>
  <key>com.apple.security.virtualization</key><true/>
</dict>
```

Entitlements come in two flavors:

- **Unrestricted**: any signed binary can claim them; the OS trusts the claim. `com.apple.security.virtualization` is in this class on macOS 26+.
- **Restricted**: the OS only honors the claim if the binary is also signed with a *provisioning profile* that explicitly grants the entitlement. `com.apple.vm.networking` (bridged networking) is the one we care about — it's restricted.

### Provisioning profile

A file signed by Apple that:
- Names one or more cert thumbprints that may use it.
- Names app ID(s) it applies to.
- Lists restricted entitlements the holder is authorized to claim.

Provisioning profiles are generated via developer.apple.com after Apple has approved your team for the entitlement in question. A cert alone does *not* give you a profile. Unfortunately.

### What this means for our project

For v1 (macOS 26+, ad-hoc, NAT + vmnet shared + Rosetta), we need: codesigning (ad-hoc is fine), entitlements (`com.apple.security.virtualization` only), no provisioning profile.

For hypothetical Phase 3 (bridged networking), we'd need: codesigning (Apple Dev or Developer ID cert), entitlements (add `com.apple.vm.networking`), AND a provisioning profile that grants `com.apple.vm.networking` to a specific app ID — requires Apple to have approved your team for that entitlement.

See `DECISIONS.md` D-002 and `PRO_TIPS.md` §29 for the version this project actually uses.

---

## OCI — the image format

"Open Container Initiative" — the set of specs that define what "a Docker image" is without being tied to Docker. Two we use:

- **OCI image spec**: what an image looks like on a registry. Manifest (JSON) + config (JSON, describes env/cmd/layers) + layers (tar.gz). The `oci-client` crate pulls these; `oci-spec` types them.
- **OCI runtime spec**: what a container's runtime config looks like. `config.json` next to a rootfs directory, describes `process.args`, `mounts`, `linux.namespaces`, `root.readonly`, etc. runc (and thus vminitd) reads this.

Our flow: `oci-client` pulls image → `ext4` crate builds a container rootfs from the extracted layers → `vminitd-client` sends a `Mount` for the rootfs + `CreateProcess` with an OCI runtime-spec JSON derived from the image config.

---

## Rosetta — amd64 on arm64

Apple's dynamic binary translator for running amd64 binaries on arm64 Macs. Inside a Linux guest, you get Rosetta via:

1. Host attaches `VZLinuxRosettaDirectoryShare` as a virtiofs mount.
2. Guest mounts it (e.g. at `/run/rosetta`).
3. Guest registers an entry in `/proc/sys/fs/binfmt_misc` pointing at `/run/rosetta/rosetta` with the `F` (fix-binary) flag — kernel now recognizes amd64 ELFs and routes execution through Rosetta.

For us: vminitd exposes a `SetupEmulator` RPC that does the binfmt_misc registration. See `PRO_TIPS §28` for the 4-step wiring.

---

## `objc2` / `objc2-virtualization`

Rust crate family for calling Obj-C APIs safely. The `objc2-virtualization` crate specifically exposes every VZ class we need.

- **Path deps recommended**: we vendor `madsmtm/objc2` at `~/vendor/github.com/madsmtm/objc2/`. The `generated/` subdir is a git submodule — if cargo-check complains about missing symbols on a fresh clone, run `git submodule update --init --depth 1`.
- **Everything VZ is `!Send` and `!Sync`** by default. See `PRO_TIPS.md` §9 for the cheat sheet and §1 for the `VzSend<T>` wrapper pattern.
- **`define_class!` + `msg_send!` + `block2::RcBlock`** is the Rust-side incantation for subclassing + calling + completion blocks. Details in `PRO_TIPS.md` §2.

---

## `beads-rs` philosophy

The design docs at `~/src/personal/beads-rs/docs/philosophy/` that we apply to Rust code in this project:

- `type_design.md` — newtypes over raw `u32`s; unrepresentable > invalid.
- `error_design.md` — `thiserror` enums with domain-named variants; `Result<T, CapabilityError>` in traits; no god-errors.
- `trait_design.md` — most traits shouldn't exist; no trait without two real implementations.
- `test_design.md` — four test shapes (law, example, scenario, regression); every test kills a family of wrong implementations.

See `DECISIONS.md` D-007 and `spike-logs/s5-ext4/` for a worked example.

---

## When you see a term you don't recognize

- Grep `PRO_TIPS.md` first; most VZ/objc2/dispatch gotchas live there with concrete code.
- Spike findings (`spike-logs/s*/FINDINGS.md`) have worked examples.
- Apple's developer docs (search `VZ<ClassName>`) are authoritative for VZ.
- The `generated/Virtualization/` directory under the vendored `objc2` repo is the authoritative Rust API surface — more reliable than any Rust-side doc.
