# Spike S7 — rosetta

**Spike code**: `~/tmp/rust-rewrite-spikes/s7-rosetta/`
**Started**: 2026-04-20

## The question

> Does `VZLinuxRosettaDirectoryShare` + vminitd's `SetupEmulator` RPC
> actually run amd64 binaries on an arm64 host+guest, end-to-end?

## Acceptance

- `uname -m` run via the amd64 busybox returns `x86_64` inside an
  aarch64 VM.
- Exit codes propagate.

## Plan

- Lift S4's harness verbatim (vsock + stdio listeners + spec shape).
- Add `VZLinuxRosettaDirectoryShare` → `VZVirtioFileSystemDeviceConfiguration(tag: "rosetta")` → `config.setDirectorySharingDevices([…])`.
- Guest side: `Mount(virtiofs, rosetta, /run/rosetta)` then `SetupEmulator(binary_path="/run/rosetta/rosetta", …amd64 magic/mask…)`.
- Build an amd64 busybox rootfs (same docker recipe as S4 but `--platform linux/amd64`).
- Expect `/bin/uname -m` to print `x86_64\n`.

## Current status

🟢 FULL acceptance — `uname -m` inside amd64 container returned `x86_64\n`.

## Events

- 2026-04-20 — `scaffold.sh` run. Harness boots a VM; extending from here.
- 2026-04-20 — Lifted S4 main.rs + Cargo.toml + build.rs + proto/. Renamed crate to `s7-rosetta`. Re-linked `assets/vmlinux` to S3's kata kernel (vsock + ext4 + virtio_blk built in).
- 2026-04-20 — Built amd64 rootfs via `docker run --platform linux/amd64 alpine:3.20`. Verified busybox is `ELF 64-bit LSB x86-64` via debugfs extract.
- 2026-04-20 — `VZLinuxRosettaDirectoryShare::availability()` → `NotInstalled` on fresh M4 Max. Wrote a small `install-rosetta` binary (in `src/bin/`) that calls `installRosettaWithCompletionHandler:`. Ran it once; install succeeded without any interactive prompt (EULA likely pre-accepted machine-wide). Subsequent `availability()` → `Installed`.
- 2026-04-20 — Wired in `setDirectorySharingDevices([VZVirtioFileSystemDeviceConfiguration(tag:"rosetta", share: VZLinuxRosettaDirectoryShare())])`. Config validated first try.
- 2026-04-20 — Added guest-side `Mount(virtiofs, "rosetta", "/run/rosetta")` + `SetupEmulator(binary_path="/run/rosetta/rosetta", magic=…, mask=…)` — magic/mask lifted verbatim from `Sources/ContainerizationOS/Linux/Binfmt.swift::Binfmt.Entry.amd64()` (with the `\x` escape sequences intact — binfmt_misc parses them, we just pass-through).
- 2026-04-20 — First run: `/bin/uname -m` ⇒ `x86_64\n` (7 bytes). `WaitProcess.exit_code = 0`. Release build clean; debug + release both green under `SPIKE_TIMEOUT_SECS=30`.
- 2026-04-20 — Spike DONE in well under budget (< 1 hour of focused work after S4 was lifted).
