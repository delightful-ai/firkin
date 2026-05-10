# S1 — findings

**Status**: ✅ PASSED, first real run. Debug and release builds both clean.
**Host**: macOS 26.3, arm64 (Apple Silicon), SDK 26.2, rustc 1.95-nightly.
**Spike code**: `~/tmp/rust-rewrite-spikes/s1-boot/` (~230 LOC Rust + 30 LOC C init).

## Acceptance — pass/fail

| Criterion | Result | Evidence |
|---|---|---|
| Kernel boot log on host stdout | ✅ | 60+ lines of `[ 0.0xxxxx] ...` printk output routed through virtio-console hvc0. |
| Init's "hello" string appears | ✅ | `SPIKE-S1: hello from init` on stdout. |
| VM reaches `Stopped` cleanly | ✅ | `guestDidStopVirtualMachine:` delegate fired; no error delegate. |
| No panic / no leaked handles | ✅ | Process exits 0; we intentionally `Box::leak` VM + delegate because `dispatch_main()` diverges — OS reaps on exit. |
| Guest boot time | — | ~46ms kernel → init → power off. |

## What worked as planned

1. **`objc2-virtualization`** is directly usable from Rust with zero Swift in the middle. All type definitions we needed (VZVirtualMachineConfiguration, VZLinuxBootLoader, VZGenericPlatformConfiguration, VZFileHandleSerialPortAttachment, VZVirtioConsoleDeviceSerialPortConfiguration, VZVirtualMachine, VZVirtualMachineDelegate) are in the generated module and work with default features.
2. **Ad-hoc codesigning** with `com.apple.security.virtualization` is enough for local dev. `codesign --force --sign - --entitlements entitlements.plist <bin>` — no paid developer program, no provisioning profile. This is S6's answer for the NAT-only case.
3. **`VZVirtualMachineDelegate` via `define_class!`** works with the new ivars-as-struct-fields syntax (ivar methods auto-generated; access via `self.fired()`).
4. **Kernel format**: Ubuntu 24.04's `linux-image-virtual` arm64 kernel, extracted from `/boot/vmlinuz-*` and gunzipped, is a valid `Image`-format ELF with `ARMd` magic — VZ accepts it as-is.
5. **`initramfs via /init`**: cpio archive containing a statically-linked `init` at root, passed as `initialRamdiskURL`, boots with no extra cmdline (Linux auto-mounts the initramfs and execs `/init`).
6. **Serial port attachment**: `VZFileHandleSerialPortAttachment` with `NSFileHandle::initWithFileDescriptor(alloc, 1)` for writing sends all guest console output to host stdout. Read-side is `None` (we don't type into the VM).
7. **`console=hvc0 panic=-1`** cmdline: virtio-console appears as hvc0 so the kernel writes its log there; `panic=-1` reboots on panic instead of hanging.

## Obj-C / objc2 lifetime traps we hit (so S2–S8 don't)

1. **`DispatchQueue::exec_async` requires `F: Send + 'static`.** Almost nothing in `objc2-virtualization` is `Send`. Two ways out:
   - **Preferred**: run everything on the **main queue** + `dispatch2::dispatch_main()` — no threads to cross, no `Send` required on Retained<VZ*>. That's what this spike does.
   - **If you need a dedicated queue** (e.g., multiple VMs), wrap !Send types in `struct VzSend<T>(T); unsafe impl<T> Send for VzSend<T> {}` — and be prepared for **closure-capture narrowing (RFC 2229)** to trip you: if the closure only references `wrapper.0`, Rust will capture just the field, which is !Send again. Force full capture by binding the wrapper to a fresh `let` inside the closure, or by stuffing it in an `Arc` and cloning in.
2. **`DispatchQueue::current()`** is deprecated — callers should pass the queue in explicitly (capture it via a clone).
3. **`DispatchQueue::new(label, attr)`** takes `Option<&CStr>` for the label, not `&str`. Use `CStr::from_bytes_with_nul(b"my.queue\0").unwrap()`.
4. **Superclass coercion**: going from `&VZVirtioConsoleDeviceSerialPortConfiguration` to `&VZSerialPortConfiguration` (required by `setSerialPorts`) needs `objc2::ClassType::as_super()` explicitly. Bring `use objc2::ClassType;` into scope.
5. **Completion-handler blocks**: use `block2::RcBlock` (heap-allocated, refcounted). `StackBlock` won't survive past the current scope; `startWithCompletionHandler` returns immediately and the block fires later.
6. **Delegate is a weak property.** `vm.setDelegate(...)` does not retain. Keep a strong `Retained<StopDelegate>` on the Rust side (we `Box::leak` it here; in a real library, hold it as a field on the owning struct).
7. **`define_class!` ivars moved** from `#[ivars = T]` to inline struct fields (`struct T { ivars: Foo }`). `Ivars::<Self>` is the constructor type; accessor methods are auto-generated per field (`self.fired()`).

## Architectural observation for the library port

Use a **single serial dispatch queue per VM** and tie all ops + callbacks to it. On the main queue that means `dispatch_main()`. On a custom queue, the VZ-facing struct needs non-Send-safe wrappers; the public API should use channels or async/await adapters so callers don't see the queue. The library's `vmm` crate should hand out futures/channels, never raw Retained<VZ*> types.

## Kernel acquisition recipe (for S3 / later)

Until we build apple/containerization's own kernel, a perfectly good arm64
kernel falls out of:

```bash
docker run --rm --platform linux/arm64 -v /tmp/out:/out ubuntu:24.04 bash -c '
  apt-get update -qq && apt-get install -y --no-install-recommends linux-image-virtual
  cp /boot/vmlinuz-*-generic /out/vmlinuz.gz.raw
'
gunzip -c /tmp/out/vmlinuz.gz.raw > vmlinux   # it's gzip-wrapped
```

`vmlinuz` is ~18MB compressed, ~56MB uncompressed. `file` reports
"Linux kernel ARM64 boot executable Image, little-endian, 4K pages" — the
format VZ wants. Works with or without `quiet` on the kernel cmdline.

For the eventual real project we'd replace this with the apple/containerization
kernel built by S3's recipe, but for S1 (and likely S2, S4) it's sufficient.

## Reusable patterns

The following snippets would survive into the real `vmm` crate almost verbatim:

- **`StopDelegate`** — define_class! pattern for any VZVirtualMachineDelegate subclass.
- **`ns_url_file`** — path → `NSURL::fileURLWithPath(&NSString::from_str(...))`.
- **`nserror_desc`** — NSError → `String`. Should graduate into a shared `NsErrorExt` trait.
- **`build.sh` for the init** — docker-based static musl build of anything Linux (init, later vminitd).

## Known loose ends (not spike-blocking)

- The kernel we used is Ubuntu's, not apple/containerization's. Doesn't matter for S1 but means we haven't exercised the `kernel/config-arm64` sub-second-boot kernel. S3 will.
- We leak `Retained<VM>` + `Retained<StopDelegate>` before `dispatch_main()`. Clean for CLI spikes; don't copy into a library.
- `start_block` is also leaked via `std::mem::forget` to extend its lifetime past the start call. In a real library, hold RcBlocks in an owning struct field.
- Only virtio-console is attached. No block, no vsock, no network. S2 adds vsock; S3/S4 adds block.
- Config cmdline has `panic=-1` but no `console_msg_format=` — everything we saw was default.

## Time to solve

Scaffolding + kernel/initrd acquisition: ~15 min. First Rust build (against
`objc2-virtualization` cold): ~20 min (fighting define_class syntax, Send
bounds, and DispatchQueue signatures). Codesigning + run: 2 min (literally
worked first try with `codesign -s - --entitlements`). Total: well under 1 day.
Plan estimated 1–2.
