# S3 — status snapshot

Last updated: 2026-04-20

## State
- 🟢 **Passed.** vminitd boots as PID 1 from an EXT4 block device and reaches gRPC-serving state (vsock port 1024). Debug + release Rust builds both clean.

## Repro

```bash
# Once per machine: install Swift 6.3 + static-linux-musl SDK.
cd /Users/darin/vendor/github.com/https:/github.com/apple/containerization
make -C vminitd cross-prep

# Build vminitd + vmexec (static musl arm64 ELFs).
make -C vminitd                              # 90 s, produces vminitd/bin/{vminitd,vmexec}

# Build cctl via swiftly's Swift 6.3 (host's /usr/bin/swift is 6.2.3, too old).
~/.swiftly/bin/swift build --product cctl -c debug --disable-automatic-resolution
install .build/arm64-apple-macosx/debug/cctl ./bin/
codesign --force --sign - --timestamp=none --entitlements=signing/vz.entitlements bin/cctl

# Produce init.block (ext4; 384 MiB). --ext4 skips the OCI image pipeline
# (the top-level `make init` only writes the tar + OCI record).
./bin/cctl rootfs create \
    --vminitd vminitd/bin/vminitd \
    --vmexec  vminitd/bin/vmexec  \
    --ext4    bin/init.block      \
    --label   org.opencontainers.image.source=https://github.com/apple/containerization \
    bin/init.rootfs.tar.gz

# Kernel: repo's own kernel/ needs apple/container CLI (not installed).
# Use the kata 3.17.0 arm64 vmlinux.container — ARM64 boot Image, 14 MiB.
mkdir -p .local bin
curl -SsL -o .local/kata.tar.gz \
    https://github.com/kata-containers/kata-containers/releases/download/3.17.0/kata-static-3.17.0-arm64.tar.xz
tar -xf .local/kata.tar.gz -C .local/ --strip-components=1
cp -L .local/opt/kata/share/kata-containers/vmlinux.container bin/vmlinux

# Stage assets for the spike.
cp bin/vmlinux    ~/tmp/rust-rewrite-spikes/s3-vminitd-build/assets/vmlinux
cp bin/init.block ~/tmp/rust-rewrite-spikes/s3-vminitd-build/assets/init.block

# Run. vminitd is long-running; sign-and-run.sh bounds it to 10s and treats
# SIGTERM/SIGKILL at timeout as success. Override via SPIKE_TIMEOUT_SECS.
cd ~/tmp/rust-rewrite-spikes/s3-vminitd-build
./sign-and-run.sh
PROFILE=release ./sign-and-run.sh
```

Expected tail — vminitd logs its gRPC-serving state before the timeout:
```
[    0.063273] EXT4-fs (vda): mounted filesystem ... ro without journal. Quota mode: disabled.
[    0.063333] VFS: Mounted root (ext4 filesystem) readonly on device 254:0.
[    0.063596] Run /sbin/vminitd as init process
...
info vminitd: vminitd booting (commit: 325d33a..., built: 2026-04-20T21:42:11Z)
info vminitd: mounting /run
info vminitd: mounting /sys
info vminitd: mounting /sys/fs/cgroup
info vminitd: Started memory monitoring
info vminitd: serving vminitd API
info vminitd: port: 1024 booting gRPC server on vsock
info vminitd: port: 1024 gRPC API serving on vsock
[run] spike killed by SIGTERM at timeout — vminitd was running (success)
```

## Assets (final, staged in the spike dir)

- `~/tmp/rust-rewrite-spikes/s3-vminitd-build/assets/vmlinux` — 14 MiB kata 3.17.0 arm64 kernel (Image format).
- `~/tmp/rust-rewrite-spikes/s3-vminitd-build/assets/init.block` — 384 MiB EXT4; contains `/sbin/vminitd` (137 MiB) + `/sbin/vmexec` (130 MiB) + `/proc/self/exe → sbin/vminitd` symlink + bin/dev/sys/proc/run/tmp/mnt/var directory tree.
- `entitlements.plist` — `com.apple.security.virtualization` (unchanged from template).

## Toolchain versions pinned

| Component | Version | Source |
|---|---|---|
| swiftly | whatever the Darwin pkg installer gives today | `https://download.swift.org/swiftly/darwin/swiftly.pkg` |
| Swift | 6.3.0 (installed via `swiftly install 6.3.0`; set as global default) | ~/.swiftly/bin/swift |
| static-linux SDK | `swift-6.3-RELEASE_static-linux-0.1.0` | https://download.swift.org/swift-6.3-release/static-sdk/swift-6.3-RELEASE/swift-6.3-RELEASE_static-linux-0.1.0.artifactbundle.tar.gz |
| SDK sha256 | `d2078b69bdeb5c31202c10e9d8a11d6f66f82938b51a4b75f032ccb35c4c286c` | pinned in `vminitd/Makefile` |
| Kernel | kata-containers 3.17.0 `vmlinux.container` | https://github.com/kata-containers/kata-containers/releases/download/3.17.0/kata-static-3.17.0-arm64.tar.xz |

## Key observations for follow-on spikes

### S4 (end-to-end) will want

- **vminitd's vsock port = 1024.** Hard-coded in `vminitd/Sources/vminitd/AgentCommand.swift:44`.
- **Default subcommand = `agent`.** `init=/sbin/vminitd` with no arguments is fine.
- **Debug vs release vminitd**: the build uses `BUILD_CONFIGURATION=debug` by default. In DEBUG mode, vminitd re-execs itself with `FOREGROUND=1` so the outer PID 1 can catch startup errors before kernel panic. Release builds skip this. Either mode serves gRPC identically.
- **Readonly ext4 root**: the top-level Makefile references `bin/init.block` and the Integration suite boots it with `options: ["ro"]`. We did the same (`initWithURL_readOnly_error(..., true)`).
- **cmdline**: `console=hvc0 root=/dev/vda rootfstype=ext4 ro init=/sbin/vminitd panic=-1`. `rootfstype=ext4` is not strictly required (kernel can detect), but is explicit and avoids probe surprises.

### Bypassing the top-level `make linux-build LIBC=musl`

On Darwin this target requires apple/container CLI (not installed here). It exists to build a Linux dev container image then invoke `make containerization && make -C vminitd`. Since we're building on arm64 macOS and the static-linux-musl SDK handles the cross-compile, we skipped the container step entirely: `make -C vminitd` does the whole thing, reusing the swiftly-installed toolchain + SDK. This matters for CI — CI on macOS-arm64 runners can use this exact recipe without `container` CLI.

### cctl CLI gotcha

`cctl rootfs create` has two output paths:
- `--image <name>`: writes to the local OCI image store. `InitImage.initBlock(at:)` lazily materializes ext4 later.
- `--ext4 <path>`: directly writes the ext4 image via `EXT4Unpacker.unpack(archive:compression:.gzip, at:ext4Path)`.

Top-level `make init` only uses `--image`; the ext4 falls out at VM-boot time. For reproducible CI artifact builds, prefer `--ext4` (what we did) so `bin/init.block` exists in the filesystem right after the build.

## Done checklist

- [x] `make cross-prep` succeeded in the apple/containerization vendored dir (via `make -C vminitd cross-prep`).
- [x] Equivalent of `make linux-build LIBC=musl` succeeded (`make -C vminitd` + cctl build). Top-level `linux-build` requires `container` CLI which isn't installed; bypassed.
- [x] `vminitd/bin/vminitd` exists; `file` reports static ELF for Linux arm64 (confirmed: "ELF 64-bit LSB executable, ARM aarch64, ..., statically linked, stripped").
- [x] `bin/init.block` exists; 384 MiB EXT4 with `/sbin/vminitd` inside (confirmed via loop mount in alpine arm64 privileged container).
- [x] Rust spike boots the VM with the kata kernel + init.block as virtio-block; vminitd startup log appears on serial.
- [x] vminitd reaches gRPC-serving state: `port: 1024 gRPC API serving on vsock`.
- [x] Debug + release Rust builds clean; `./sign-and-run.sh` and `PROFILE=release ./sign-and-run.sh` both succeed.
- [x] JOURNAL.md has a final resolution entry.
- [x] FINDINGS.md written.
- [x] State line reads "🟢 Passed" with today's date.
- [ ] spike-logs/README.md index update — flagging for curator (per SPIKE_RUNBOOK.md, shared docs are updated by curator).
- [x] PRO_TIPS additions flagged in FINDINGS.md under "Proposed PRO_TIPS additions".

## Handoff notes

### S4 (end-to-end)
- **Lift the block-device wiring from `src/main.rs` verbatim**: the 8-line `VZDiskImageStorageDeviceAttachment → VZVirtioBlockDeviceConfiguration → setStorageDevices` block is exactly what a second `rootfs.img` attachment looks like. For a container rootfs image add a second `VZVirtioBlockDeviceConfiguration` to the same array; it'll land as `/dev/vdb` inside the guest.
- **vminitd is on vsock port 1024**: add `VZVirtioSocketDeviceConfiguration` to the VM config, then use S2's connector to dial port 1024 after `VM started` fires.
- **Don't rely on kernel panic for failure detection** — the delegate's `didStopWithError` only fires if the kernel brings the VM down. vminitd errors log to stderr but keep the kernel alive; S4 will want a vsock health check on top.
- **`init.block` is read-only**: vminitd mounts tmpfs at /run, sysfs at /sys, cgroup2 at /sys/fs/cgroup, and binfmt_misc. Don't try to write to the rootfs from the guest side.

### S2 (vsock-tonic, parallel)
- No conflict. We don't touch vsock. When S2 lands, its connector can dial vminitd@1024 directly as the first real-world smoke test.

### S6 (vmnet entitlements)
- No new info. NAT still works; vmnet untested in this spike.

### S5, S7, S8
- No new blockers. S5 still standalone. S7 needs S4. S8 is measurement.

## Proposed PRO_TIPS additions

See FINDINGS.md "Proposed PRO_TIPS additions" section. Short list:
1. Swiftly install + Swift 6.3 recipe for macOS arm64.
2. Top-level `make init` vs `cctl rootfs create --ext4` distinction.
3. Top-level `make linux-build` requires apple/container CLI; bypass with `make -C vminitd`.
4. Kata kernel fetch via direct curl when `make fetch-default-kernel` breaks on weird repo paths.
5. Host `/usr/bin/swift` is too old (6.2.x on macOS 26); use `~/.swiftly/bin/swift` for host cctl builds too.
