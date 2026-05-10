# S1 — Boot Linux VM from Rust via objc2-virtualization

**Spike code**: `~/tmp/rust-rewrite-spikes/s1-boot/`
**Host**: macOS 26.3, arm64, SDK 26.2, rustc 1.95-nightly
**Started**: 2026-04-20

## The question

Can we configure, start, and cleanly stop a VZ VM entirely from Rust? Are
there undocumented Obj-C lifetime gotchas the `objc2` ecosystem hides from
us?

## Acceptance

- Kernel boot log on host stdout.
- Init's "hello" string appears.
- VM reaches `stopped` cleanly, no panic, no leaked handles.

## Plan

1. Acquire an arm64 Linux kernel (Image format). Preferred: Firecracker's
   prebuilt test kernel if it still ships a 5.10+ arm64 build. Fallback:
   build minimal kernel via apple/containerization/kernel/.
2. Build a trivial static init in C (musl) that writes "hello from init" to
   /dev/console and then `reboot(LINUX_REBOOT_CMD_POWER_OFF)`. Pack into
   cpio initrd.
3. Cargo project:
   - deps: objc2, objc2-foundation, objc2-virtualization, block2, dispatch2, tokio (multi-thread for the run loop / wait).
   - Configure `VZVirtualMachineConfiguration` with `VZLinuxBootLoader`,
     one `VZVirtioConsoleDeviceSerialPortConfiguration` attached to host
     stdout via `VZFileHandleSerialPortAttachment`, 1 vCPU, 128 MiB.
   - `validateWithError:` before `initWithConfiguration:`.
   - Subclass `NSObject` + conform to `VZVirtualMachineDelegate` via
     `define_class!`; log state transitions in `virtualMachine:didStopWithError:`
     and `guestDidStop:`.
   - `start()` with a block2 completion handler posting the result to a
     oneshot.
   - Wait for state == `.stopped`, by polling `state` on the main queue
     via KVO-ish observation (VZ requires queue affinity — use a dispatch
     queue and dispatch2).
4. Codesign ad-hoc with `com.apple.security.virtualization` entitlement.
5. Run. Iterate.

## Why VZ requires a queue

All `VZVirtualMachine` operations must run on a single serial dispatch
queue. We'll pick one dispatch queue and route all VZ calls through it via
`dispatch_async` / `dispatch_sync` (dispatch2). Rust owns the orchestration;
the VZ queue owns the VM. Delegate callbacks fire on that queue.

## Current status

🟢 **Done.** See `FINDINGS.md` and `STATUS.md`. Spike code at
`~/tmp/rust-rewrite-spikes/s1-boot/`.

## Events

- 14:08 — scaffolding dirs created.
- 14:09 — kernel acquired via docker/ubuntu, 56 MB uncompressed Image.
- 14:11 — static arm64 musl init built via docker/alpine, packed as 66 KB
  cpio initrd.
- 14:30 — first build attempt; fought define_class! syntax (ivars moved
  from attribute to inline fields), dispatch2 API (exec_sync returns unit
  only, DispatchQueue::new wants &CStr, current() deprecated), Send bounds
  on dispatch closures that don't interact well with Retained<VZ*>.
- 14:45 — switched from custom serial dispatch queue to **main queue +
  dispatch_main()**. Collapses the whole threading story into "everything
  on one thread, stop in the delegate via exit()". Clean.
- 14:55 — ad-hoc codesign with `com.apple.security.virtualization`
  entitlement worked first try; no provisioning profile needed.
- 14:56 — **first real run passed all criteria.** Debug and release builds
  both clean. Guest boot → init hello → power down → delegate fires →
  exit 0 in ~46ms of guest time.
- 15:05 — wrote up FINDINGS.md and STATUS.md with reusable patterns,
  gotchas for S2/S3/S4 claudes, and repro recipe.
