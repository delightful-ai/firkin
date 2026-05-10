# Spike plan

Before we commit to library design, we do spikes. A spike is a small, throwaway program whose job is to answer one scary question. Each spike has a binary outcome: the approach works as predicted, or we learn something that reshapes the plan.

Spikes live in a throwaway repo (`~/tmp/rust-rewrite-spikes/`) as separate binaries. Nothing here is the "real" code. Don't over-engineer; the goal is evidence, not reusable scaffolding.

**Run order: S1 → S2 in parallel with S3, then S4/S5/S6 (parallel), then S7/S8.** S1+S2 are blocking for the whole project; the rest defer decisions.

**Platform floor**: **macOS 26+ only.** Matches apple/container's own floor. vmnet shared-mode in particular requires 26+ for the ad-hoc-signing path. Pre-26 hosts are not a ship target; don't build compatibility scaffolding for them.

---

## Before you start a spike — mandatory reading

- **[`SPIKE_RUNBOOK.md`](./SPIKE_RUNBOOK.md)** — conventions: where code
  lives, where notes live, what "done" looks like, parallelization rules.
- **[`PRO_TIPS.md`](./PRO_TIPS.md)** — every gotcha we've hit (33 sections
  as of 2026-04-20). Dense, copy-pasteable. Read before touching threads,
  `define_class!`, codesigning, or anything involving vsock / vmnet.
- **[`CONCEPTS.md`](./CONCEPTS.md)** — grounding vocabulary for VZ,
  virtio devices, vsock, initramfs vs rootfs, codesigning/entitlements/
  profiles. Read first if any term in the plan is unfamiliar.
- **[`DECISIONS.md`](./DECISIONS.md)** — ADR-style log of architectural
  decisions (macOS floor, bundling strategy, stdio model, etc.). Skim
  before making calls that might conflict with a prior decision.
- **[`spike-template/`](./spike-template/)** — the starting point.
  Run `scaffold.sh <N> <topic>` from the repo root; you get a known-good
  "boots a VM" binary plus notes stubs to extend.
- **[`spike-logs/`](./spike-logs/)** — JOURNAL / STATUS / FINDINGS for
  every spike. The durable artifact. **Always check the other spikes'
  logs before starting** — the thing you're about to hit is probably
  already documented.

## Progress

| Spike | Status | Notes |
|---|---|---|
| S1 — Boot empty Linux VM from Rust | ✅ **Passed** | [spike-logs/s1-boot/](./spike-logs/s1-boot/) |
| S2 — Vsock ↔ tonic transport | ✅ **Passed** | [spike-logs/s2-vsock-tonic/](./spike-logs/s2-vsock-tonic/) — first-call RTT ~400 µs, no fd leaks over 1000 iters |
| S3 — Cross-build vminitd | ✅ **Passed** | [spike-logs/s3-vminitd-build/](./spike-logs/s3-vminitd-build/) — vminitd reaches gRPC on vsock 1024 |
| S4 — End-to-end (pull/rootfs/exec) | ✅ **Passed (full)** | [spike-logs/s4-e2e/](./spike-logs/s4-e2e/) — busybox container runs; echo hello round-trips via inverse vsock stdio |
| S5 — EXT4 writer | ✅ **Passed (Tiers 1–4)** | [spike-logs/s5-ext4/](./spike-logs/s5-ext4/) — e2fsck clean across 7 feature flags; depth-1 extent trees; whiteouts + opaque-dir markers validated via guest-mount probes; structural diff vs `mkfs.ext4` documented |
| S6 — Entitlements & codesigning | ✅ **Passed** | [spike-logs/s6-vmnet-entitlements/](./spike-logs/s6-vmnet-entitlements/) — on macOS 26+, vmnet shared-mode works ad-hoc with only `com.apple.security.virtualization`; bridged defers to Phase 3 |
| S7 — Rosetta | ✅ **Passed** | [spike-logs/s7-rosetta/](./spike-logs/s7-rosetta/) — amd64 `uname -m` → `x86_64` in an arm64 guest; 4-step wire lifts verbatim into library |
| S8 — vminitd bundling numbers | ✅ **Passed** | [spike-logs/s8-bundling-bench/](./spike-logs/s8-bundling-bench/) — **Strategy A embedding the 131 MiB vminitd ELF** is the default; C behind `--features runtime-download` |
| S9 — vmnet end-to-end reachability | ✅ **Passed (full)** | [spike-logs/s9-vmnet-reachability/](./spike-logs/s9-vmnet-reachability/) — container gets a reachable IP via vminitd netlink RPCs; host→container 0.21–0.29 ms, container→8.8.8.8 reaches; DNS works |
| S10 — VZ snapshot/restore for vminitd shape | — | load-bearing for v1 snapshot feature + integration-test speedup (10–20× per test). Added 2026-04-20. |

---

## S1 — Boot an empty Linux VM from Rust via objc2-virtualization

> ✅ **Passed** — see [spike-logs/s1-boot/](./spike-logs/s1-boot/). Guest boots, prints hello, powers off cleanly in ~46 ms of guest time. Debug + release builds clean. Ad-hoc codesign with `com.apple.security.virtualization` works (S6's NAT case already answered). The spike's `src/main.rs` is lifted into [spike-template/](./spike-template/) as the starting point for everyone else.

**Question**: can we configure, start, and cleanly stop a VZ VM entirely from Rust? Are there undocumented Obj-C lifetime gotchas the `objc2` ecosystem hides from us?

**Risk**: low-medium. The crate compiles, but we haven't run anything.

**Setup**
- A minimal Linux kernel (borrow `kernel/` from apple/containerization or grab a vmlinuz).
- A 10-line init that writes to the console and `sync; reboot -f`s. Build as static x86_64/aarch64 ELF with `-static` musl or similar.
- Pack init into a tiny initrd (cpio).

**Steps**
1. Build `VZVirtualMachineConfiguration` with:
   - `VZLinuxBootLoader` pointing at kernel + initrd + cmdline `console=hvc0 panic=-1`
   - `VZVirtioConsoleDeviceSerialPortConfiguration` with `VZFileHandleSerialPortAttachment` attached to stdout
   - 1 vCPU, 128 MiB memory.
2. Validate (`validateWithError:`).
3. `VZVirtualMachine::initWithConfiguration`.
4. Implement `VZVirtualMachineDelegate` via `objc2`'s `define_class!` macro. Log state transitions.
5. `start()` with a completion handler (block2).
6. Wait for state = `stopped`.

**Acceptance**
- Kernel boot log appears on stdout.
- Init's "hello" string appears.
- VM reaches `stopped` state cleanly, no panic, no leaked handles.

**If it breaks**
- Codesigning: may need `com.apple.security.virtualization` entitlement + self-sign. Capture exact missing-entitlement error.
- Obj-C lifetime: the VM's strong references to device configs are easy to get wrong. If it crashes on drop, check for premature release of config objects held only by `Retained<...>`.

**Deliverable**: `spikes/s1-boot/` binary that prints guest output and exits 0. ~200 LOC Rust + ~10 LOC C for the init.

**Time**: 1–2 days.

---

## S2 — Vsock ↔ tonic transport (the least-proven piece)

> ✅ **Passed** — see [spike-logs/s2-vsock-tonic/](./spike-logs/s2-vsock-tonic/). Host-side VZ vsock fd splices cleanly into tonic via custom `tower::Service<Uri>` connector. First-call RTT ~400 µs, 1000-iter loop: 0 fd leaks / +688 KB RSS drift, cancellation works. The `VsockConnector` pattern (~30 LOC) lifts verbatim into the real library. Key gotcha: `VZVirtioSocketConnection` owns its fd — `dup()` before handing to tokio. See [PRO_TIPS §13](./PRO_TIPS.md#13-vsock--the-least-obvious-bits-from-s2).

**Question**: can we splice a VZ-delivered host-side vsock fd into tonic's hyper Channel? Is there any blocker in how hyper handles custom connectors or how tokio wraps non-stream fds?

**Risk**: high. Nobody has published this glue. Every other layer is derivative; this one is invented.

**Setup**
- Reuse the VM from S1, but add `VZVirtioSocketDeviceConfiguration`.
- Inside the guest: a tiny Rust binary (static-linux-musl) exposing a trivial `tonic` echo server bound to an `AF_VSOCK` port.
- Use `tokio-vsock` in the guest for the listener.

**Steps (host side)**
1. Get the `VZVirtioSocketDevice` from the running VM's `socketDevices` array.
2. Call `connectToPort_completionHandler` with a `block2::StackBlock` that signals a `tokio::sync::oneshot` with either the connection or the error.
3. From the `VZVirtioSocketConnection`, extract `fileDescriptor() -> c_int`.
4. Wrap the fd:
   - Set `O_NONBLOCK`.
   - `tokio::io::unix::AsyncFd::new(fd)` or wrap as a `UnixStream::from_std(std::os::unix::net::UnixStream::from_raw_fd(fd))`. Test both.
5. Implement a hyper `Service<Uri>` that returns the wrapped fd as the connection, ignoring the URI path beyond port.
6. Build a tonic `Channel` using `Endpoint::from_static("http://vsock")` + `.connect_with_connector(service)`.
7. Call the guest's echo RPC. Expect a round-trip.

**Acceptance**
- Round-trip RTT < 10 ms on first call.
- 1000-iteration loop: no fd leaks (check `lsof`), no handle growth.
- Cancellation: drop the client mid-stream, confirm guest side sees EOF and host fd is closed.

**If it breaks**
- If `AsyncFd` approach fails for stream semantics: fall back to `UnixStream::from_raw_fd` (vsock behaves like a SOCK_STREAM).
- If hyper chokes on the `http://vsock` URI: use `Uri::from_static` with a `tonic.example` authority and rely on the custom connector to ignore it.
- If block2 lifetime gets tangled: switch from `StackBlock` to `RcBlock` and extend the lifetime with a channel-based bridge.

**Deliverable**: `spikes/s2-vsock-tonic/` with host binary + guest binary + a 200-line reusable `VsockConnector` module (this will likely survive into the real project verbatim).

**Time**: 3–5 days. This is the long pole of the spike phase.

---

## S3 — Cross-build vminitd and boot it

> ✅ **Passed** — see [spike-logs/s3-vminitd-build/](./spike-logs/s3-vminitd-build/). vminitd builds in ~90s (plus ~8 min one-time SDK install), boots as PID 1 from readonly ext4 virtio-block, reaches `gRPC API serving on vsock` port **1024**. Top-level `make init` / `make linux-build LIBC=musl` / `make fetch-default-kernel` all have traps — all documented in [PRO_TIPS §15–§19](./PRO_TIPS.md#15-swift-toolchain-setup-for-make-linux-build-from-s3). Used kata 3.17.0 arm64 kernel; apple's own kernel needs the `container` CLI (separate dep).

**Question**: can we build vminitd from apple/containerization on this machine, and does a VM boot it as PID 1?

**Risk**: medium. Build process looks well-documented but depends on swiftly + static-linux-musl SDK.

**Setup**
- Install swiftly (apple installer).
- Install Swift 6.3 via swiftly: `swiftly install 6.3.0`.
- Pull apple/containerization (already vendored).

**Steps**
1. `cd apple/containerization && make cross-prep && make linux-build LIBC=musl`.
2. Inspect `vminitd/bin/vminitd` — `file` should report static ELF.
3. Also build `bin/init.block` via `make init`.
4. From the S1 harness, attach `init.block` as a virtio-block device.
5. Boot with `init=/sbin/vminitd` on the kernel cmdline (check actual path by inspecting init.block).
6. Observe vminitd's serial log — it prints on startup.
7. (Optional hint of S5) Connect to vminitd's known vsock port and `Sync` it.

**Acceptance**
- `make init` succeeds, produces `bin/init.block` of a reasonable size.
- Booted VM's serial output shows vminitd initialization lines.
- vminitd reaches its gRPC-serving state (visible in logs).

**If it breaks**
- Missing static SDK: error will tell us. Download the pinned artifact from the URL in `vminitd/Makefile`.
- Kernel / vminitd abi mismatch: confirm kernel version matches what this repo's kernel/ targets.
- init.block not mountable: use `hdiutil attach` read-only on macOS (won't mount ext4 natively — instead, loopback-mount inside another Linux VM or a dev container to inspect).

**Deliverable**: a documented recipe in `03-build-machinery.md` for reproducing vminitd locally + CI. An `init.block` artifact we can reuse in later spikes.

**Time**: 1 day if smooth, 3 days if the SDK dance bites.

---

## S4 — End-to-end: pull image, hand-build rootfs, exec container process

> ✅ **Passed (full acceptance)** — see [spike-logs/s4-e2e/](./spike-logs/s4-e2e/). `echo hello` inside a real busybox container round-trips stdout via inverse vsock to host. 9 RPCs round-tripped (Sync/Getenv/ContainerStatistics → Mkdir/Mount/WriteFile → CreateProcess/StartProcess/WaitProcess + host-side `VZVirtioSocketListener` for stdout/stderr). Biggest architectural finding: **stdio is vsock-back, not stream RPC** — see [PRO_TIPS §20](./PRO_TIPS.md#20-host-side-vsock-listener--the-other-direction-from-s4). vminitd quirks (bundle path implicit, rootfs must be writable, Codable strictness) in [§21](./PRO_TIPS.md#21-vminitd-sandboxcontext-quirks-from-s4). Portable rootfs.ext4 build recipe in [§22](./PRO_TIPS.md#22-rootfsext4-build-recipe-for-spikes-from-s4). ~1h 45min vs plan's 3–5 days.

**Question**: with vminitd working, can we drive a real `CreateProcess`/`WaitProcess` cycle and get stdout back?

**Risk**: medium. The protocol is documented in `SandboxContext.proto` but actual usage expectations (mount ordering, config.json shape, vsock port numbers) aren't.

**Setup**
- S1 + S2 + S3 all passing.
- `oci-client` pulling `busybox:latest`.
- Hand-built rootfs: extract the busybox tarball into a directory, `mkfs.ext4` on a file, copy the tree in. This is a one-off script using shelled-out tools — we're not porting EXT4 yet.

**Steps**
1. Pull `busybox:latest` with `oci-client`: get manifest, config, and the single layer.
2. Extract layer into a temp dir.
3. Shell to `dd if=/dev/zero of=rootfs.img bs=1M count=128` + `mkfs.ext4 -F rootfs.img` + loopback mount (inside a Linux VM, since macOS can't) + `cp -a` the tree.
   - **Alternative**: run the assembly step inside a short-lived Linux VM using vminitd itself. Dog food.
4. Boot VM with `init.block` (for vminitd) + `rootfs.img` (container rootfs).
5. Open vsock to vminitd, `Mount` the container rootfs at `/run/container-0`.
6. Write an OCI runtime `config.json` into the bundle path via `WriteFile`.
7. `CreateProcess` → `StartProcess` → stream stdout → `WaitProcess`.
8. Expect `echo hello` to return.

**Acceptance**
- vminitd reports process exit 0.
- Streamed stdout on host contains "hello\n".

**If it breaks**
- Mount errors: `SandboxContext.proto:Mount` wants a specific fstype; check vminitd's runc wrapper for what it expects.
- config.json rejection: compare shape to runc-spec reference; vminitd may have quirks.
- Partial RPC coverage: some RPCs may need preconditions (e.g., `Setenv` before `CreateProcess`).

**Deliverable**: `spikes/s4-e2e/` — the ugliest script in this plan, but proves the whole vertical works.

**Time**: 3–5 days.

---

## S5 — EXT4 writer: minimal-viable Rust port

> ✅ **Passed (Tier 1 MVP + Tier 2 structure + partial Tier 3)** — see [spike-logs/s5-ext4/](./spike-logs/s5-ext4/). 2709 Rust LOC (lib + CLI + tests + VM harness + init.c); `e2fsck -nf` clean across features `{ext_attr, sparse_super2, filetype, extent, flex_bg, large_file, huge_file, extra_isize}`; kata kernel boots and mounts the produced image; `cat /hello` prints `hi\n`. Idiomatic-Rust philosophy (newtypes, domain error variants, `#[repr(C)]` + bytemuck) applied per beads-rs philosophy docs; newtypes caught at least one real bug at compile time. Five new PRO_TIPS sections land: [§23 xattr offs base](./PRO_TIPS.md#23-ext4-xattr-e_value_offs-base-differs-between-inline-and-block-from-s5), [§24 dirs-always-need-a-block](./PRO_TIPS.md#24-every-ext4-directory-needs-a-data-block--even-lostfound-even-empty-from-s5), [§25 kernel capabilities](./PRO_TIPS.md#25-kernel-capability-quick-ref-extended-from-s5), [§26 bytemuck flags](./PRO_TIPS.md#26-bytemuck-feature-flags-for-ext4-shaped-types-from-s5), [§27 pod_read_unaligned](./PRO_TIPS.md#27-bytemuckpod_read_unaligned-for-reading-on-disk-structs-in-tests-from-s5). Multi-group / deep-extents / htree / csums / byte-for-byte-vs-mkfs deferred to Phase 2.

**Question**: how much of `ContainerizationEXT4` is real algorithm vs. Swift decoration? Can we produce a tiny ext4 image that `e2fsck` approves and vminitd mounts?

**Risk**: medium-high on calendar (4,500 LOC to port), low on tractability (it's deterministic byte-level work).

**Setup**
- Read `Sources/ContainerizationEXT4/EXT4.swift`, `EXT4+Types.swift`, and `EXT4+Formatter.swift` end-to-end with a highlighter.
- Install `e2fsprogs` on macOS (`brew install e2fsprogs`).

**Steps**
1. Write ~300 LOC of Rust that produces an ext4 image containing exactly one file, `/hello`, with content `hi\n`. No layer merging, no xattrs, no whiteouts — just the minimum structure: superblock, one block group, inode table, root inode, `/hello` inode, data block, directory entry.
2. Run `e2fsck -n rootfs.img`. Must be clean.
3. Use the S4 harness to mount the image in a VM and `cat /hello`.
4. Once that's passing, add features in this order, validating with e2fsck + mount each time:
   - Multiple files / directories / recursion
   - xattrs
   - Symlinks and hardlinks
   - Device nodes (for `/dev` if needed)
   - Whiteouts / opaque-dir markers (OCI layer merge semantics)
   - Large files (extent tree)
   - Sparse regions

**Acceptance**
- `e2fsck -n` clean on every test fixture.
- Images mount in a VM and contents match expected tree.
- Byte-for-byte diff of an image we produce vs. one `ContainerizationEXT4` produces from the same input (tolerating timestamp/UUID differences). **This is the most important sanity check** — if we match byte-for-byte modulo nondeterminism, we know the port is faithful.

**If it breaks**
- Extent tree is the usual suspect. Unit-test the extent allocator in isolation before integrating.
- Directory entries: rec_len alignment and hash-tree indexing can bite.

**Deliverable**: `spikes/s5-ext4/` — throwaway, but its unit tests survive into `crates/ext4/`.

**Time**: this spike produces only the minimum-viable; the full port is Phase 2. Spike deliverable in 1 week.

---

## S6 — Entitlements & codesigning dance

> ✅ **Passed (full)** — see [spike-logs/s6-vmnet-entitlements/](./spike-logs/s6-vmnet-entitlements/).
>
> On **macOS 26+**, `VZVmnetNetworkDeviceAttachment` in shared mode (IP-per-container) works with **ad-hoc codesigning and only `com.apple.security.virtualization`** — no paid Apple Developer Program, no provisioning profile, no `com.apple.vm.networking` entitlement. (Counter-intuitively, *adding* `com.apple.vm.networking` breaks things: AMFI refuses binaries with restricted entitlements that lack a matching provisioning profile.) Bridged-to-physical-NIC (`VZBridgedNetworkDeviceAttachment`) still needs the paid dev program + a profile and defers to Phase 3. See [PRO_TIPS §29](./PRO_TIPS.md#29-vmnet-networking-from-rust-on-macos-26-from-s6) for the entitlements matrix + the `msg_send!`-based init pattern needed to work around `objc2-virtualization 0.3.2`'s unavailable-marked `VZVmnetNetworkDeviceAttachment::init`.
>
> **Decision**: v1 is macOS 26+ only (matches apple/container's floor) and ships with vmnet IP-per-container from day one. NAT-fallback-for-older-macOS dropped.

**Question**: what's the minimum codesigning incantation to run VZ + vmnet from Rust in dev? Can we avoid paid Apple Developer Program for local use?

**Risk**: low technically, medium bureaucratically.

**Setup**
- An ad-hoc developer cert from Keychain (self-signed).
- A stub Rust binary linking `objc2-virtualization`.

**Steps**
1. Build the S1 spike binary.
2. `codesign --force --entitlements spike.entitlements --sign - spike` with entitlements file containing `com.apple.security.virtualization`.
3. Run. Confirm VM starts.
4. Add `VZNATNetworkDeviceAttachment`. Confirm still runs without additional entitlements.
5. Add `VZVmnetNetworkDeviceAttachment`. Capture the expected error.
6. Research: what's needed to unlock vmnet (this repo's README + SECURITY.md + `VmnetNetwork.swift` references).

**Acceptance**
- NAT case: signed + entitled binary, VM networks via NAT.
- vmnet case: we have a clear written answer in `03-build-machinery.md` for what additional entitlement / provisioning profile is required, even if we defer actually implementing vmnet to Phase 3.

**If it breaks**
- vmnet may require a provisioning profile → notarized build → paid developer program. Document this in the risks section. NAT-only MVP is acceptable for v1.

**Deliverable**: An `entitlements.plist` we can crib from, plus a `Makefile` or `cargo-make` recipe that does codesigning. Cribbed from `krunkit.entitlements` as a starting point.

**Time**: 1 day.

---

## S7 — Rosetta cross-arch execution

> ✅ **Passed** — see [spike-logs/s7-rosetta/](./spike-logs/s7-rosetta/). `/bin/uname -m` from an amd64 busybox inside an arm64 guest returns `x86_64\n`. `WaitProcess.exit_code = 0`. Full 4-step wiring recipe in [PRO_TIPS §28](./PRO_TIPS.md#28-rosetta-wiring-recipe-from-s7): host attaches `VZLinuxRosettaDirectoryShare` via virtiofs tagged "rosetta"; guest sequences `Mkdir → Mount(virtiofs) → SetupEmulator` over SandboxContext gRPC with amd64 ELF magic/mask + `F` (fix-binary) flag. License install is programmatic (`installRosettaWithCompletionHandler:`) and non-interactive where the EULA is already accepted system-wide; fresh Macs may see a one-time GUI prompt. ~60 LOC delta from S4.

**Question**: does `VZLinuxRosettaDirectoryShare` + vminitd's `SetupEmulator` actually run amd64 binaries on an arm64 host+guest, end-to-end?

**Risk**: low. Apple has documented this well; we just need the wiring.

**Setup**
- S4 harness passing.
- An amd64 static busybox to copy into the rootfs.

**Steps**
1. Add `VZLinuxRosettaDirectoryShare` to the VM config. Accept the Rosetta license programmatically (this prompts the user once; we need to test that the error is actionable).
2. Share it into the guest as a virtiofs mount at `/run/rosetta`.
3. `CreateProcess` `/run/rosetta/register` or call `SandboxContext.proto:SetupEmulator`.
4. `CreateProcess` an amd64 binary at a path the guest sees. Expect it to run.

**Acceptance**
- `uname -m` run via the amd64 busybox returns `x86_64` inside an aarch64 VM.
- Exit codes propagate.

**If it breaks**
- Rosetta license not accepted: we need an interactive dance on first use; document it.
- binfmt_misc registration: confirm `SetupEmulator` does this correctly by reading vminitd source.

**Deliverable**: one working amd64-in-arm64 container run. Notes on the license flow.

**Time**: 1 day.

---

## S9 — vmnet end-to-end reachability

> ✅ **Passed (full)** — see [spike-logs/s9-vmnet-reachability/](./spike-logs/s9-vmnet-reachability/). Container gets a CIDR-assigned IP (`192.168.70.2/24`) via vminitd's netlink RPCs — no DHCP client needed in the container rootfs. Host→container ping **0.21–0.29 ms**; container reaches `8.8.8.8` in 7–100 ms; DNS resolves public hostnames. All five network-config RPCs complete in <100 ms over gRPC. Five-step RPC sequence documented in [PRO_TIPS §32](./PRO_TIPS.md#32-vminitds-network-rpc-sequence--the-cidr-string-gotcha-from-s9); MAC-required-for-vmnet in [§31](./PRO_TIPS.md#31-vmnet-attachments-require-a-mac-address-from-s9); subnet discovery from Rust in [§33](./PRO_TIPS.md#33-vmnet-subnet-discovery-from-rust-from-s9). Two architectural decisions captured: [D-011](./DECISIONS.md#d-011--network-config-via-vminitd-netlink-rpcs-not-guest-side-dhcp) (netlink path, not DHCP) and [D-012](./DECISIONS.md#d-012--one-vm-per-container-matches-applecontainers-model) (one VM per container, shared vmnet network).

> **Original brief, kept for reference.** S6 proved the host-side attachment works. This spike proves the whole networking round-trip: container gets a real IP via vmnet, host can reach it, container can reach the outside. Until this runs, "IP-per-container" is a design claim, not an evidenced capability.

**Question**: with a container running behind a `VZVmnetNetworkDeviceAttachment` in shared mode, does the container actually get a reachable IP from the host, and can it talk to the outside world?

**Risk**: low-medium. vminitd+`ContainerizationNetlink` handle guest-side configuration in Swift; the question is whether the pieces compose when driven from Rust over the SandboxContext protocol.

**Setup**
- Lift S6's vmnet attachment + S4's full e2e container harness.
- Build a busybox rootfs that includes a DHCP client (`udhcpc` from `busybox-static`) OR rely on vminitd's netlink configuration path.
- Plan for two runs: one container, then two containers on the same vmnet network.

**Steps**
1. Configure the VM with a `VZVmnetNetworkDeviceAttachment` in shared mode (as S6 proved works).
2. Boot the VM + vminitd. Container config: `process.args = ["/bin/sh", "-c", "ip addr show eth0; ip route; cat /etc/resolv.conf; sleep 3600"]`.
3. Observe which IP the container gets. Two paths to make this happen:
   - **(a)** vminitd's `ContainerizationNetlink` path: library passes the network config (IP, gateway, DNS) to vminitd through the SandboxContext RPCs; vminitd configures eth0 statically. Read `Sources/ContainerizationNetlink/` + `Sources/Containerization/VmnetNetwork.swift` for the interface we'd call.
   - **(b)** DHCP inside the container: `udhcpc -i eth0` or equivalent. Simpler to wire in the spike but relies on the container image carrying a DHCP client.
4. From host: `ping <container-ip>`, `curl http://<container-ip>` if we run a trivial server inside, `nc -zv <container-ip> <port>`.
5. From container: `ping 8.8.8.8`, `curl -v https://example.com` (tests NAT + DNS).
6. Stretch: spin up a second container, confirm the two can reach each other by IP.

**Acceptance**
- Host stdout shows the container's eth0 IP on a vmnet subnet (e.g. `192.168.64.2`).
- `ping -c 2 <container-ip>` from the host returns 2/2 packets within 500 ms.
- Container can resolve DNS and reach an external host (ping 8.8.8.8 or curl example.com).
- No kernel panic, no AMFI denial, no entitlement drift from S6's passing state.

**If it breaks**
- **No IP assigned**: either DHCP client missing in rootfs, or vminitd didn't configure eth0. Check vminitd's Swift for the network-config RPC shape.
- **IP assigned but unreachable**: macOS firewall (`socketfilterfw`) may be blocking; check. Routing/NAT on the host side may need an explicit `sysctl net.inet.ip.forwarding=1` (though vmnet should handle this).
- **DNS fails**: resolv.conf inside container is wrong. vmnet's shared-mode DNS is usually on the subnet gateway IP (e.g. `192.168.64.1`).

**Deliverable**: ~100 LOC Rust delta over S4. Documented recipe for "how Phase 1's `core` crate wires up vmnet per-container". Proposed PRO_TIPS additions for any gotcha hit.

**Time**: 1-2 hours. If the vminitd netlink path is load-bearing and we hit proto-discovery friction, budget 3 hours.

---

## S8 — vminitd bundling + build-time numbers

> ✅ **Passed** — see [spike-logs/s8-bundling-bench/](./spike-logs/s8-bundling-bench/).
>
> **Decision**: Strategy A (embed) with the **131 MiB vminitd ELF, not the 384 MiB init.block**. Fallback: Strategy C (runtime download, cached in XDG) behind `--features runtime-download`. Strategy B (build.rs fetch) is a CI-population mechanism, not a user choice.
>
> **Numbers against plan tolerances** (cold <60 s, warm <5 s, first-run <3 s): A/vminitd hits **4.85 s cold / 4.51 s warm-lib / 0.49 s warm-main**; A/init.block busts warm-lib at 20–40 s; B busts warm-lib at 53 s; C hits **5.19 s / 0.26 s / 0.15 s** on loopback but ~32 s first-run for 384 MiB on a typical home uplink.
>
> **Architectural convergence with S5**: don't ship `init.block` at all. Embed the `vminitd` ELF; have the `ext4` crate (S5) synthesize `init.block` on-host. 2.4× cold-build speedup and 2.9× final-binary-size cut for free.
>
> Quantified embedding cost + `ld` dead-strip behavior (consumers that import `ext4` without `core` pay ~422 KB, not 131 MB) in [PRO_TIPS §30](./PRO_TIPS.md#30-include_bytes-cost-quantified-from-s8).

**Question**: the user said "if bundling has fast builds then it's fine, else pull from GH release if missing." What are the actual numbers?

**Risk**: low. Pure measurement.

**Setup**
- A placeholder vminitd binary of ~10MB (the real one is in that ballpark — measure in S3).

**Steps**
1. Strategy A (embed): `include_bytes!("../../vendor/vminitd")` in a module. Measure `cargo build --release` wall time cold and warm.
2. Strategy B (download in build.rs): build.rs fetches from a pinned GitHub release URL, caches in `$OUT_DIR`. Measure cold + warm.
3. Strategy C (runtime download with cache): crate ships without binary, fetches on first run, caches in `$XDG_CACHE_HOME`. Measure first-run startup vs. subsequent.
4. Compare. Record the target tolerances:
   - Cold `cargo build`: <60s on an M-series laptop without spike's deps.
   - Warm build: <5s incremental.
   - First-run VM boot (in C case): <3s of added latency.

**Acceptance**
- A written decision with numbers in `03-build-machinery.md`. Default recommendation is A (bundled); fall back to B if A's build times regress noticeably.

**Deliverable**: A small benchmark harness + a decision table. Becomes the `build.rs` template for the real project.

**Time**: 0.5 day.

---

## S10 — VZ snapshot/restore for the vminitd VM shape

> **Added 2026-04-20.** Load-bearing for the v1 decision to ship VM snapshot support (audit A-lift from the library-surface design doc, §4.1 + §10.3). If snapshot/restore works reliably for our specific VM shape (vminitd + vsock + virtio-block + virtiofs + optional Rosetta), the `vmm` crate gets a `snapshot` Cargo feature in v1 and integration tests get a 10–20× speedup via snapshot-per-test. If it doesn't, snapshot defers to v1.1 and integration tests eat the cold-boot cost (~500 ms/test vs. ~30 ms/test).

**Question**: do `VZVirtualMachine.saveMachineStateToURL(_:completionHandler:)` and `restoreMachineStateFromURL(_:completionHandler:)` work end-to-end for a VM that has:
- vminitd running as PID 1, reachable on vsock 1024
- a virtio-block-attached rootfs
- a live vsock RPC channel (from the host to vminitd)
- a virtio-net attachment (NAT or vmnet-shared)
- optionally: a virtiofs share, a Rosetta directory share

…and if yes, what's the save-and-restore wall time vs. a cold boot, and what (if anything) needs to be re-wired on restore?

**Risk**: medium. Apple documents save/restore on `VZVirtualMachine` but their examples target simpler VM shapes (macOS guests, minimal device lists). Our vminitd+vsock+inverse-vsock-listener shape is not in their examples. Specific concerns:
- Vsock connections are socket-like; a restored VM may or may not have the host-side fd still usable
- vminitd is a user-space process; if its internal state (gRPC server, open sockets) survives the snapshot, great — if not, we need a post-restore reconnect dance
- Virtiofs shares may reference host paths that are no longer mapped
- Rosetta binfmt registration lives in the guest kernel — probably fine

**Setup**
- S4 harness as the base (full e2e container passing). S6/S7 passing if we want to cover vmnet and Rosetta in the same probe.
- A directory to write snapshot files to (snapshots can be multi-hundred-MB; expect ~VM-RAM-size plus a bit).

**Steps**
1. Boot a VM with vminitd, dial vsock 1024, call `Sync` RPC, confirm responsive.
2. Call `saveMachineStateToURL(url, completionHandler:)`. Measure wall time and resulting file size.
3. Stop the VM (release the `VZVirtualMachine` retain). Confirm no leaked fds.
4. Construct a fresh `VZVirtualMachine` pointed at the **same config**, then call `restoreMachineStateFromURL(url, completionHandler:)`. Measure wall time.
5. After restore: dial vsock 1024 fresh, call `Sync`. Expect: either works immediately, or works after a short retry window. **Record which.**
6. Extend (optional): snapshot/restore with an active container's init process (`echo hello; sleep 60` running at snapshot time). After restore, verify the process is either still running (ideal) or cleanly terminated (acceptable) — not stuck in a zombie or half-reaped state.
7. Extend (optional): snapshot/restore with an active vmnet-shared network attachment. Check IP assignment survives; DHCP re-lease if needed.
8. Extend (optional): snapshot/restore with virtiofs + Rosetta attachments. Check mounts remain valid.

**Acceptance**
- Save completes without error for a running-vminitd VM.
- Restore completes without error.
- vminitd responds on vsock 1024 after restore (directly, or after a documented re-connect step).
- Save-and-restore round-trip < 1 second on an M-series laptop.
- At least the primary path (vminitd + rootfs + vsock) is documented. Optional extensions (active containers, vmnet, virtiofs, Rosetta) nice-to-have but not blocking.

**If it breaks**
- Save fails outright: VZ doesn't support it for our shape; snapshot defers to v1.1 (or forever). Document why, move on.
- Restore succeeds but vminitd is unreachable: likely need to re-establish vsock. Try `VZVirtioSocketDevice::connect(toPort: 1024)` again post-restore; if that works, document as the required re-wire step.
- Restore fails on a specific device: try removing devices from the config one at a time (virtiofs first, then Rosetta, then vmnet) to identify which is incompatible.
- Wall time is too long (≥ a few seconds): snapshot+restore loses its value proposition for the integration-test use case; defer to v1.1 as a user-facing-only feature.

**Deliverable**: `spikes/s10-snapshot/` binary demonstrating one save/restore cycle against the S4 harness + `spike-logs/s10-snapshot/FINDINGS.md` stating:
- Works / doesn't work for our VM shape.
- Measured wall times (save, restore, total round-trip).
- What (if anything) needs re-wiring on restore (vsock reconnect, DHCP renewal, etc.).
- Whether the integration-test speedup story is real (test-per-snapshot-restore < ~50 ms).
- Recommendation: ship `vmm` snapshot feature in v1, or defer to v1.1.

**Time**: 2–4 hours. Not blocking any other spike; run opportunistically before v1 crate layout freezes.

---

## What the spike phase produces

At the end:
1. Evidence each scary piece works on this machine.
2. A reusable `VsockConnector` (S2) that's nearly the real module.
3. A documented vminitd build/bundle path (S3 + S8).
4. A codesigning + entitlements recipe (S6).
5. A minimum-viable EXT4 writer (S5) with passing e2fsck tests — the seed of `crates/ext4/`.
6. A scrappy end-to-end demo (S4) that proves the whole vertical.

**Total spike calendar**: ~3 weeks of focused work, or ~5 weeks with normal context switching.

At that point Phase 1 (real library MVP) starts with zero unknowns and most of the scary work already prototyped.
