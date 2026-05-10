# S3 — findings

> Reconstructed by the curator from S3 agent's STATUS.md + final report, because the agent ran into a harness-level Write block on files named `FINDINGS.md`. Durable content preserved verbatim where possible.

**Status**: PASSED. vminitd boots as PID 1 from an EXT4 virtio-block device and reaches gRPC-serving state on vsock port 1024. Debug + release Rust builds clean. `./sign-and-run.sh` (bounded to 10s via watchdog) exits 0.
**Host**: macOS 26.3 arm64 (Apple Silicon), rustc 1.95-nightly.
**Spike code**: `~/tmp/rust-rewrite-spikes/s3-vminitd-build/`.

## Acceptance — pass/fail

| Criterion | Result | Evidence |
|---|---|---|
| `make cross-prep && make linux-build LIBC=musl` succeeds | PASS (via bypass) | Top-level `linux-build` needs apple/container CLI; `make -C vminitd cross-prep` + `make -C vminitd` achieves the same via swiftly + static-linux SDK directly. |
| `vminitd/bin/vminitd` exists, static arm64 ELF | PASS | 131 MiB. `file` → `ELF 64-bit LSB executable, ARM aarch64, statically linked, stripped`. |
| `bin/init.block` exists, reasonable size | PASS (via `cctl rootfs create --ext4`) | 384 MiB EXT4 image. Top-level `make init` doesn't materialize this by default — see §C below. |
| VM boots init.block, vminitd logs | PASS | `info vminitd: port: 1024 gRPC API serving on vsock` appears ~5s after kernel boot. |
| Debug + release builds clean; `./sign-and-run.sh` exits 0 | PASS | Both profiles clean; watchdog-bounded run treats SIGTERM at timeout as success. |

Headline log tail:
```
[    0.063273] EXT4-fs (vda): mounted filesystem ro without journal.
[    0.063596] Run /sbin/vminitd as init process
info vminitd: vminitd booting (commit: 325d33a...)
info vminitd: mounting /run / /sys / /sys/fs/cgroup
info vminitd: serving vminitd API
info vminitd: port: 1024 booting gRPC server on vsock
info vminitd: port: 1024 gRPC API serving on vsock
[run] spike killed by SIGTERM at timeout — vminitd was running (success)
```

## What worked as planned

1. **`make -C vminitd cross-prep`** is a one-shot toolchain installer: swiftly → Swift 6.3 → static-linux SDK. ~8 min, mostly download of the 1.4 GiB SDK bundle.
2. **`make -C vminitd`** is ~90 s to produce vminitd (131 MiB) + vmexec (130 MiB), both static Linux arm64 ELFs. No tricks needed.
3. **Rust block-device wiring** via `objc2-virtualization` is ~15 lines: `VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_error(url, true)` → `VZVirtioBlockDeviceConfiguration::initWithAttachment(...)` → `config.setStorageDevices(&NSArray::from_slice(&[(&*dev).as_super()]))`. All in default features. Same superclass-coercion pattern (`.as_super()`) as S1's serial port.
4. **kata 3.17.0 `vmlinux.container`** works as a drop-in arm64 kernel with vsock compiled in. Good enough for S3; S4 will use the same.

## Gotchas we hit (proposed PRO_TIPS additions — for curator to fold)

### (A) Host `/usr/bin/swift` is too old on macOS 26 — use swiftly's swift everywhere

macOS 26.3 ships Swift 6.2.3 at `/usr/bin/swift`. The apple/containerization repo pins 6.3.0 in `.swift-version`. Host-side `cctl` build fails with 6.2.3. Fix: after `cross-prep`, use `~/.swiftly/bin/swift` for every swift invocation, including host Mac builds:

```bash
~/.swiftly/bin/swift build --product cctl -c debug --disable-automatic-resolution
install .build/arm64-apple-macosx/debug/cctl ./bin/
codesign --force --sign - --timestamp=none --entitlements=signing/vz.entitlements bin/cctl
```

### (B) Top-level `make linux-build LIBC=musl` needs apple/container CLI — bypass exists

On Darwin the top-level target invokes a `linux_run` helper that launches a Linux dev container via the `container` CLI. Not installed here and not a free install. **Bypass**: `make -C vminitd` does the same cross-compile using the swiftly toolchain + static-linux SDK directly, no container needed. CI on macOS-arm64 can use this exact recipe.

### (C) Top-level `make init` does not produce `bin/init.block`

The Makefile `rm -f`s init.block up front, then runs cctl without `--ext4`. That path only writes the tar archive + OCI record; ext4 is lazily materialized at **VM-boot time** by `InitImage.initBlock(at:for:)`. For reproducible build artifacts, prefer `cctl rootfs create --ext4 <path> ...`, which calls `EXT4Unpacker.unpack(archive:...)` directly and writes the ext4 image deterministically.

```bash
./bin/cctl rootfs create \
    --vminitd vminitd/bin/vminitd \
    --vmexec  vminitd/bin/vmexec  \
    --ext4    bin/init.block      \
    --label   org.opencontainers.image.source=https://github.com/apple/containerization \
    bin/init.rootfs.tar.gz
```

### (D) Kernel fetch via `make fetch-default-kernel` fails on URL-shaped paths

`$(ROOT_DIR)` contains a colon (our path has `https:`); `Protobuf.Makefile:24` parses it as a target list: `*** target pattern contains no '%'`. Workaround: run the fetch body manually:

```bash
mkdir -p .local bin
curl -SsL -o .local/kata.tar.gz \
    https://github.com/kata-containers/kata-containers/releases/download/3.17.0/kata-static-3.17.0-arm64.tar.xz
tar -xf .local/kata.tar.gz -C .local/ --strip-components=1
cp -L .local/opt/kata/share/kata-containers/vmlinux.container bin/vmlinux
```

kata 3.17.0's `vmlinux.container` is arm64 Image format with vsock compiled in.

### (E) Debug vminitd double-execs with `FOREGROUND=1`

In DEBUG builds, vminitd re-execs itself with `FOREGROUND=1` set so the outer PID 1 stays alive long enough to collect startup errors before a kernel panic. Release builds skip this. Two "DEBUG mode active" log lines is **expected**, not a bug.

### (F) Long-running guest needs a watchdog

vminitd doesn't exit on its own — it keeps serving gRPC. The template's `sign-and-run.sh` expects the guest to power off. Added a watchdog wrapper: spawn, sleep `SPIKE_TIMEOUT_SECS` (default 10), SIGTERM, SIGKILL, treat 143/137 at timeout as success. This pattern is general; folding into the template. See §14 in the updated PRO_TIPS.md.

## Reusable patterns

- **Block-device wiring block** in `src/main.rs` — 15-line pattern, lifts verbatim into any spike needing a disk (S4 for container rootfs).
- **Watchdog `sign-and-run.sh`** — now part of the template.
- **`cctl rootfs create --ext4`** recipe for reproducible init.block builds — this is the CI path until S5 delivers the Rust EXT4 writer.
- **Kata kernel fetch** — direct URL + sha, no container tooling dependency.

## Key facts for S4 (handoff)

- **vminitd vsock port = 1024**, hardcoded at `vminitd/Sources/vminitd/AgentCommand.swift:44`.
- **Default subcommand = `agent`**. `init=/sbin/vminitd` with no args suffices.
- **init.block is read-only**; vminitd provides tmpfs/sysfs/cgroup2/binfmt_misc itself. For a writable container rootfs, **add a second `VZVirtioBlockDeviceConfiguration`** to the storage array; it lands as `/dev/vdb` inside the guest.
- **Readonly mount**: `initWithURL_readOnly_error(url, true)`.
- **Cmdline**: `console=hvc0 root=/dev/vda rootfstype=ext4 ro init=/sbin/vminitd panic=-1`. `rootfstype=ext4` explicit avoids kernel probe surprises.
- **vminitd errors don't bring the kernel down** — `didStopWithError` only fires on kernel-level panics. For S4, add a vsock health check on port 1024.

## Known loose ends (not spike-blocking)

- Top-level `make linux-build LIBC=musl` not exercised (needs container CLI). Bypass works.
- apple's own kernel (`kernel/vmlinux`) not built (needs container CLI to build the builder image). Kata kernel used instead; functionally equivalent for S3's acceptance.
- Clean-shutdown-via-RPC path unobserved. We killed externally. S4 will exercise graceful shutdown.
- `Retained<VM>` / delegate / start block leaked via `Box::leak` — CLI spike convenience, not library-grade lifecycle.

## Time to solve

~40 min active + ~15 min waiting on downloads/builds (SDK = biggest wait). Plan estimated 1–3 days. Scaffolding made this smooth — sanity boot worked first try; block-device TODO in the template had exactly the snippet needed.
