# Spike S6 — vmnet-entitlements

**Spike code**: `~/tmp/rust-rewrite-spikes/s6-vmnet-entitlements/`
**Started**: 2026-04-20
**Resolved**: 2026-04-20 (🟢 passed, ~45 min)

## The question

From `02-spike-plan.md` §S6:
> What's the minimum codesigning incantation to run VZ + vmnet from Rust in
> dev? Can we avoid paid Apple Developer Program for local use?

## Acceptance

- NAT case: signed + entitled binary, VM networks via NAT. (S1 already did this.)
- vmnet case: clear written answer for what entitlement/provisioning profile
  is needed, even if we defer implementing vmnet to Phase 3.

## Plan

1. Extend S1 harness with `VZVmnetNetworkDeviceAttachment`.
2. Probe entitlement variants: virt-only (ad-hoc), +com.apple.vm.networking
   (ad-hoc), +com.apple.vm.networking (Apple Development cert).
3. Capture the exact error at each boundary: `vmnet_network_create`,
   `config.validateWithError`, `vm.startWithCompletionHandler`, AMFI kill.
4. Document.

## Events

- 2026-04-20 15:10 — `scaffold.sh 6 vmnet-entitlements` run. Stock boot
  harness works; output matches S1.
- 2026-04-20 15:20 — API reconnaissance:
  - `signing/vz.entitlements` (apple/containerization itself) contains ONLY
    `com.apple.security.virtualization` — and the Swift code at
    `Sources/Containerization/VmnetNetwork.swift` creates a `vmnet_network_ref`
    via `vmnet_network_create` and hands it to `VZVmnetNetworkDeviceAttachment(network:)`.
    This is a macOS 26.0+ API path.
  - `~/vendor/github.com/madsmtm/objc2/generated/Virtualization/VZVmnetNetworkDeviceAttachment.rs`
    has `+new` and `-init` marked **unavailable**; no public initializer is
    bridged. `-initWithNetwork:` exists in the Obj-C runtime but isn't in the
    generated bindings — we'll have to call it via `msg_send!`.
  - By contrast, `VZBridgedNetworkDeviceAttachment.rs` says (verbatim):
    "Using a VZBridgedNetworkDeviceAttachment requires the app to have the
    `com.apple.vm.networking` entitlement." That explains where that
    entitlement comes from. Vmnet doesn't document needing it.
  - `/Applications/Xcode.app/.../vmnet.framework/Headers/vmnet.h` shows
    `vmnet_network_configuration_create`, `vmnet_network_create`, etc. are
    all `API_AVAILABLE(macos(26.0))` — brand new. No `API_UNAVAILABLE`
    entitlement gate documented.
- 2026-04-20 15:35 — Wrote `src/main.rs` with a `SPIKE_PROBE` env var to
  switch between {none, nat, vmnet} attachment modes, plus a `SPIKE_ENT`
  env var (via sign-and-run.sh) to switch between {virt-only, networking,
  hypervisor, empty} entitlement plists and `SPIKE_SIGN_ID` to switch
  signing identity.
- 2026-04-20 15:42 — First build failure: `Allocated<T>::assume_init` doesn't
  exist. Switched to `msg_send![VZVmnetNetworkDeviceAttachment::alloc(),
  initWithNetwork: net as *mut AnyObject]` — compiles.
- 2026-04-20 15:46 — **Probe A (NAT, virt-only ent, ad-hoc)**: VM boots,
  guest prints "hello", powers off clean. Exit 0. (Matches S1 baseline.)
- 2026-04-20 15:48 — **Probe B (vmnet, virt-only ent, ad-hoc)**: runtime
  panic from objc2's type-encoding check:
  ```
  invalid message send to -[VZVmnetNetworkDeviceAttachment initWithNetwork:]:
  expected argument at index 0 to have type code '^{vmnet_network=}',
  but found '@'
  ```
  Passing as `*mut AnyObject` encodes to `@` (id). Changed to `*mut c_void`,
  got `'^v'`. Wrong encoding.
- 2026-04-20 15:52 — Defined a ZST `struct VmnetNetwork { _priv: [u8; 0] }`
  with `unsafe impl RefEncode for VmnetNetwork { const ENCODING_REF: Encoding
  = Encoding::Pointer(&Encoding::Struct("vmnet_network", &[])); }` — that
  encodes to `^{vmnet_network=}`, matching the runtime's selector metadata.
- 2026-04-20 15:54 — **Probe B retry (vmnet, virt-only ent, ad-hoc)** —
  FULL SUCCESS:
  ```
  [vmnet] network_configuration_create OK
  [vmnet] network_create OK (ref=0x102c49a90)
  [vmnet] -[VZVmnetNetworkDeviceAttachment initWithNetwork:] OK
  [host] vmnet attachment added to VM config
  [host] configuration validated
  [host] VM started
  SPIKE: hello from init
  [delegate] guestDidStopVirtualMachine
  [host] exiting with code 0
  ```
- 2026-04-20 15:57 — **Probe C (vmnet, empty ent, ad-hoc)**: exits at
  `vmnet_network_create` with status=1002 (VMNET_MEM_FAILURE). Counter-
  intuitively NOT `VMNET_NOT_AUTHORIZED` (1010). Captured verbatim.
- 2026-04-20 15:59 — **Probe D (NAT, empty ent, ad-hoc)** (canonical
  missing-virt-entitlement error for the journal):
  ```
  NSError domain=VZErrorDomain code=2 desc=Invalid virtual machine
  configuration. The process doesn't have the "com.apple.security.
  virtualization" entitlement.
  ```
- 2026-04-20 16:04 — **Probe E (vmnet, virt + com.apple.vm.networking,
  ad-hoc)**: process is SIGKILL'd at exec with exit code 137. Console log:
  ```
  AMFI: '...s6-vmnet-entitlements' is adhoc signed.
  amfid: Error Domain=AppleMobileFileIntegrityError Code=-420
    "The signature on the file is invalid"
  AMFI: Code has restricted entitlements, but the validation of its
    code signature failed. Unsatisfied Entitlements:
  ASP: Security policy would not allow process
  ```
  So `com.apple.vm.networking` is a **restricted entitlement** —
  ad-hoc signing can't satisfy it.
- 2026-04-20 16:06 — **Probe F (vmnet, virt + com.apple.vm.networking,
  Apple Development cert signed)**: also SIGKILL'd. Console log:
  ```
  taskgated-helper: Disallowing s6-vmnet-entitlements because no
    eligible provisioning profiles found
  amfid: Error Domain=AppleMobileFileIntegrityError Code=-413
    "No matching profile found"
  AMFI: Code has restricted entitlements, but the validation of its
    code signature failed. Unsatisfied Entitlements:
  ```
  So even with a valid Apple Dev cert, `com.apple.vm.networking` requires
  a **provisioning profile** that lists it — i.e. a paid Developer
  Program with an app ID that declares that entitlement capability.
- 2026-04-20 16:10 — Verified kernel-side proof: extended init.c to mount
  sysfs and dump `/sys/class/net`. With `SPIKE_PROBE=vmnet` the guest sees
  `lo` and `eth0`. With `SPIKE_PROBE=none` the guest sees only `lo`. Device
  really is wired through.
- 2026-04-20 16:14 — Cleaned warnings. Cold `cargo clean && cargo build` +
  `cargo build --release` both green.

## Resolution

**🟢 Passed.** vmnet works with the SAME minimum entitlement as NAT
(`com.apple.security.virtualization`), ad-hoc signed. No paid Apple
Developer Program required for `VZVmnetNetworkDeviceAttachment`. The
`com.apple.vm.networking` entitlement that S6 worried about is a separate
gate that applies only to **`VZBridgedNetworkDeviceAttachment`** (Apple
says so explicitly in the objc2 bindings' class doc) — and THAT entitlement
IS the restricted one that requires a provisioning profile + paid dev
program.

Practical implication: v1 can ship both NAT and vmnet (IP-per-container
networking) with ad-hoc signing. Only `VZBridgedNetworkDeviceAttachment`
(bridge an en0/en1 physical interface to the VM) is paid-only.
