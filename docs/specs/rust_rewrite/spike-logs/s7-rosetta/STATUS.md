# S7 — status snapshot

Last updated: 2026-04-20

## State
- 🟢 **Passed — FULL acceptance.** `/bin/uname -m` inside an amd64
  busybox container returns `x86_64\n` (7 bytes) on an aarch64 VZ VM.
  `WaitProcess.exit_code = 0`. Debug + release builds both green.

## Repro

```bash
# 1. Install Rosetta for Linux once (no-op if already installed).
#    On a fresh machine, the first call may prompt for license acceptance.
#    On this M4 Max the call succeeded non-interactively.
cd /Users/darin/vendor/github.com/https:/github.com/apple/containerization
docs/specs/rust_rewrite/spike-template/scaffold.sh 7 rosetta   # if not yet
cp ~/tmp/rust-rewrite-spikes/s4-e2e/src/main.rs \
   ~/tmp/rust-rewrite-spikes/s7-rosetta/src/main.rs   # then edit per this spike
cp ~/tmp/rust-rewrite-spikes/s4-e2e/{Cargo.toml,build.rs,sign-and-run.sh,entitlements.plist} \
   ~/tmp/rust-rewrite-spikes/s7-rosetta/
cp -r ~/tmp/rust-rewrite-spikes/s4-e2e/proto \
   ~/tmp/rust-rewrite-spikes/s7-rosetta/proto
cp ~/tmp/rust-rewrite-spikes/s4-e2e/assets/init.block \
   ~/tmp/rust-rewrite-spikes/s7-rosetta/assets/init.block
ln -sf ~/tmp/rust-rewrite-spikes/s3-vminitd-build/assets/vmlinux \
       ~/tmp/rust-rewrite-spikes/s7-rosetta/assets/vmlinux

# 2. Build an amd64 rootfs (not arm64 — different from S4).
mkdir -p /tmp/s7-rootfs-build
cat > /tmp/s7-rootfs-build/build.sh <<'EOF'
#!/bin/sh
set -eux
apk add --no-cache e2fsprogs util-linux busybox-static
rm -rf /build/rootfs
mkdir -p /build/rootfs/bin /build/rootfs/etc /build/rootfs/proc \
         /build/rootfs/sys /build/rootfs/dev /build/rootfs/tmp \
         /build/rootfs/root /build/rootfs/run/rosetta
cp /bin/busybox.static /build/rootfs/bin/busybox
cd /build/rootfs
for a in uname echo sh ls cat sleep true false env printf; do ln -sf /bin/busybox bin/$a; done
echo 'root:x:0:0:root:/root:/bin/sh' > /build/rootfs/etc/passwd
echo 'root:x:0:'                      > /build/rootfs/etc/group
dd if=/dev/zero of=/out/rootfs.ext4 bs=1M count=64
mkfs.ext4 -F -L s7rootfs -d /build/rootfs /out/rootfs.ext4
e2fsck -fy /out/rootfs.ext4 || true
EOF
docker run --rm --platform linux/amd64 \
    -v /tmp/s7-rootfs-build:/work \
    -v ~/tmp/rust-rewrite-spikes/s7-rosetta/assets:/out \
    alpine:3.20 sh -c 'cd /work && sh build.sh'

# 3. (Once only) Install Rosetta for Linux.
cd ~/tmp/rust-rewrite-spikes/s7-rosetta
cargo build --bin install-rosetta
codesign --force --sign - --entitlements entitlements.plist target/debug/install-rosetta
./target/debug/install-rosetta   # prints availability before/after

# 4. Run (debug + release both green).
SPIKE_TIMEOUT_SECS=30 ./sign-and-run.sh
SPIKE_TIMEOUT_SECS=30 PROFILE=release ./sign-and-run.sh
```

Expected tail:
```
[ACC/target] Mount(/dev/vdb -> /run/container/container-0/rootfs) OK
[ACC/rosetta] Mkdir(/run/rosetta) OK
[ACC/rosetta] Mount(rosetta -> /run/rosetta) OK
[ACC/rosetta] SetupEmulator(amd64) OK
[ACC/target] CreateProcess OK
[ACC/target] StartProcess OK; pid=79
[ACC/target] WaitProcess returned exit=0
[ACC/full] stdout (7B) = "x86_64\n"
[ACC] FULL acceptance met — amd64 uname -m returned x86_64
```

## Acceptance criteria (from 02-spike-plan.md §S7)

- [x] `uname -m` run via the amd64 busybox returns `x86_64` inside an aarch64 VM.
- [x] Exit code propagates (WaitProcess.exit_code=0).
- [x] Debug + release Rust builds clean.
- [x] `SPIKE_TIMEOUT_SECS=30 ./sign-and-run.sh` exits 0.

## RPCs that round-trip (on top of S4's set)

- S4's nine RPCs all still work unchanged.
- **New**: `Mount(type="virtiofs", source="rosetta", destination="/run/rosetta")` (target — guest mounts the shared rosetta directory).
- **New**: `SetupEmulator(binary_path, name, type, offset, magic, mask, flags)` (target — registers a binfmt_misc entry).

## Done checklist

- [x] Acceptance criteria met (see above)
- [x] `sign-and-run.sh` exits 0 (cold + warm, both profiles)
- [x] Debug + release builds clean
- [x] JOURNAL.md has a final resolution entry
- [x] FINDINGS.md written
- [x] State line above reads 🟢 Passed
- [ ] spike-logs/README.md index update — **flagging for curator** (shared doc)
- [x] PRO_TIPS additions flagged in FINDINGS.md

## Handoff notes

### For the real library (Phase 1)

- Gate Rosetta on `cfg(target_arch = "aarch64")` + `availability() != NotSupported` before appending the directory-sharing device. Mirror Apple's Swift `VZVirtualMachineInstance.prestart()` + `installRosetta()` call — their flow: `prestart()` kicks the installer if `.notInstalled`; `start()` then proceeds and the VM boots even if the user declined (it'll error at share-time with an actionable message).
- The guest-side sequence is exactly: `Mount(virtiofs)` → `SetupEmulator(...)`. Worth exposing as a single `enable_rosetta()` helper that mirrors `Vminitd+Rosetta.swift`.
- Magic + mask for amd64 ELF are fixed strings; lift them verbatim from `Binfmt.swift::amd64()` into a `const` in the Rust port.
- `F` (fix-binary) flag in the binfmt registration means the kernel opens `/run/rosetta/rosetta` at register time and keeps the fd; it does **not** need to be visible from the container's mount namespace at exec time. So you do NOT need to bind-mount `/run/rosetta` into the container rootfs — the spec's `mounts` stay unchanged vs. non-rosetta containers.

### Unknowns still open

- **License-acceptance prompt**: on this machine, `installRosettaWithCompletionHandler:` returned success with no dialog. Either (a) the license had been pre-accepted at the macOS level, or (b) VZ's install path auto-accepts for programmatic callers that hold the `com.apple.security.virtualization` entitlement. A fresh Mac (or one that has never run any Rosetta-using VM) would be a more honest test. `apple/containerization`'s own flow assumes a GUI prompt is possible (`ContainerizationError` thrown with a user-facing message). Worth documenting but not blocking — the "happy path" works.
- **Rosetta CachingOptions**: `VZLinuxRosettaCachingOptions` (`VZLinuxRosettaAbstractSocketCachingOptions` / `VZLinuxRosettaUnixSocketCachingOptions`) are *not* wired; the default (no caching) is fine for `uname -m`, but AOT translation cache boosts throughput for long-running amd64 workloads. Orthogonal to S7 acceptance.
