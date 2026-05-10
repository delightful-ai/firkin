# S1 — status snapshot

Last updated: 2026-04-20

## State
- 🟢 **Passed.** All acceptance criteria met (see FINDINGS.md).

## Repro (for anyone coming back to this)

```bash
cd ~/tmp/rust-rewrite-spikes/s1-boot
./sign-and-run.sh        # debug; builds, ad-hoc-signs, runs, exits 0
PROFILE=release ./sign-and-run.sh
```

Expected output tail:
```
[host] VM started
...kernel printk lines...
SPIKE-S1: hello from init
SPIKE-S1: powering off
[    0.0xxxxx] reboot: Power down
[delegate] guestDidStopVirtualMachine
[host] exiting with code 0
```

## Assets
- `~/tmp/rust-rewrite-spikes/s1-boot/assets/vmlinux` — 56 MB arm64 Linux Image (Ubuntu 24.04 linux-image-virtual, gunzipped).
- `~/tmp/rust-rewrite-spikes/s1-boot/assets/initrd.cpio` — 66 KB newc-cpio with static musl `/init` (see `init/init.c`).
- `entitlements.plist` — grants `com.apple.security.virtualization`.

## Key code paths (the bits that will survive into the library)

- `src/main.rs::StopDelegate` — `define_class!`-based `VZVirtualMachineDelegate`.
- `src/main.rs::main` lines for config build — boot loader + platform + serial port wiring.
- `sign-and-run.sh` — ad-hoc codesigning incantation (this is S6's answer for NAT-only).

## Handoff to other claudes

### S2 (vsock ↔ tonic)
- **Can lift**: `StopDelegate` pattern verbatim. `dispatch_main()` loop is fine as
  harness; just add a tokio runtime on a background thread for the tonic client
  half, pull guest-side vsock fd into it.
- **Add**: `VZVirtioSocketDeviceConfiguration` to `config.setSocketDevices(&[...])`.
  After `startWithCompletionHandler` succeeds, pull `vm.socketDevices().firstObject()`
  and cast to `VZVirtioSocketDevice`. Call `connectToPort_completionHandler` with a
  `block2::RcBlock` that hands the `VZVirtioSocketConnection` to a oneshot channel.
  `fileDescriptor()` on the connection gives the raw fd — wrap it in `tokio::io::unix::AsyncFd`
  or `UnixStream::from_raw_fd`.
- **Gotcha**: completion handler may fire on VZ queue (main in our setup). Don't
  block on it; `tx.send(...)` and return.
- **Gotcha**: `VZVirtioSocketConnection`'s docs say it closes the underlying fd
  when the connection is released. Either dup() the fd, or take ownership of the
  connection Retained and keep it alive as long as the tokio half is using the fd.

### S3 (cross-build vminitd + boot it)
- **Can lift**: the whole VM harness. Swap kernel path for apple/containerization
  kernel (once built or downloaded) and add a `VZVirtioBlockDeviceConfiguration`
  pointing at `bin/init.block` — kernel cmdline becomes
  `console=hvc0 root=/dev/vda init=/sbin/vminitd panic=-1` (check actual path).
- **Can lift**: kernel-acquisition recipe in FINDINGS.md if vminitd build fails.

### S6 (entitlements)
- **Already answered for NAT case**: `entitlements.plist` contains
  `com.apple.security.virtualization`, ad-hoc signing works.
- **Still needed**: the vmnet question. That's S6 proper.

### S4, S5, S7, S8
- No blockers from S1. S4 depends on S2+S3. S5 is purely Rust-side, doesn't
  need a VM harness until the final validation step. S7/S8 layer on top of S4.

## Open questions NOT answered by S1
- Full end-to-end fd-lifetime audit: we leak; a real library must not.
- Behavior when `VZVirtualMachine::isSupported` returns false (untested path).
- Is there a race between `startWithCompletionHandler`'s completion block and
  `guestDidStopVirtualMachine:`? Probably not on a serial queue, but worth
  confirming with an instrumented spike if S2 needs to know ordering.
