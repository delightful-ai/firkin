# S6 — findings

**Status**: 🟢 PASSED.
**Host**: macOS 26.3, arm64 (Apple Silicon), rustc 1.95-nightly.
**Spike code**: `~/tmp/rust-rewrite-spikes/s6-vmnet-entitlements/` (~290 LOC Rust + ~50 LOC C init).
**Time**: ~45 min.

## The headline

On **macOS 26+**, `VZVmnetNetworkDeviceAttachment` (IP-per-container networking)
works with the **same minimum entitlement as NAT**:

- `com.apple.security.virtualization`
- ad-hoc signed (`codesign --force --sign - --entitlements ...`)
- no paid Apple Developer Program
- no provisioning profile

The historical wisdom that "vmnet needs `com.apple.vm.networking`" is
**wrong for this class**. That entitlement is required only by
`VZBridgedNetworkDeviceAttachment` (bridge a physical en0/en1 to a VM) —
verbatim from Apple's class doc in the Obj-C header:
> Using a VZBridgedNetworkDeviceAttachment requires the app to have the
> "com.apple.vm.networking" entitlement.

`com.apple.vm.networking` IS a **profile-restricted entitlement** — AMFI
refuses to honor it on ad-hoc-signed or even Apple-Development-cert-signed
binaries unless a matching provisioning profile is present. That's
Apple-Developer-Program gated. So:

- v1 can ship NAT **and** vmnet with ad-hoc signing. No paid dev program.
- Bridged-to-physical-NIC is not viable locally. Defer to Phase 3 and only
  for distributable builds.

## Acceptance — pass/fail

| Criterion | Result | Evidence |
|---|---|---|
| NAT case: signed + entitled binary, VM boots with NAT | ✅ | Probe A. Matches S1 baseline. |
| vmnet: clear answer on entitlement / prov-profile requirements | ✅ | This whole doc. |
| Bonus: actual working Rust that boots a VM with vmnet | ✅ | Probe B. Guest `/sys/class/net` shows `lo + eth0`. |
| `sign-and-run.sh` exits 0 cold | ✅ | Probes A, B, NAT with virt-only ent. |
| Debug + release builds clean | ✅ | `cargo clean && cargo build{,--release}` no warnings. |

## Probes — complete table with verbatim evidence

All probes run on the same binary, re-signed per probe.

| # | Network probe | Entitlements | Signer | Result |
|---|---|---|---|---|
| A | NAT | virt-only | ad-hoc (`-`) | ✅ Boots. Guest prints hello, powers off. |
| B | vmnet | virt-only | ad-hoc (`-`) | ✅ **KEY RESULT**: Boots. Guest sees `eth0`. |
| C | vmnet | empty (no entitlements) | ad-hoc (`-`) | ❌ `vmnet_network_create` → `VMNET_MEM_FAILURE (1002)` |
| D | NAT | empty | ad-hoc (`-`) | ❌ Validation error (see below) |
| E | vmnet | virt + `com.apple.vm.networking` | ad-hoc (`-`) | ❌ AMFI kills at exec (SIGKILL, exit 137) |
| F | vmnet | virt + `com.apple.vm.networking` | Apple Development cert | ❌ AMFI kills: no matching prov-profile |
| — | control (none probe) | virt-only | ad-hoc | ✅ Guest sees only `lo` — baseline |

### Verbatim error: Probe D (missing virt entitlement, validation boundary)

```
[host] VALIDATE_FAILED: NSError domain=VZErrorDomain code=2
desc=Invalid virtual machine configuration. The process doesn't have
the "com.apple.security.virtualization" entitlement.
```

### Verbatim error: Probe C (missing virt entitlement, vmnet boundary)

```
[host] VMNET_PROBE_FAILED: vmnet_network_create returned NULL;
status=1002 (VMNET_MEM_FAILURE)
```

Counter-intuitive — `vmnet.framework` reports "memory failure" rather than
`VMNET_NOT_AUTHORIZED (1010)` when the process lacks the virtualization
entitlement. Don't pattern-match on the code number alone.

### Verbatim evidence: Probe E (`com.apple.vm.networking` on ad-hoc signature)

Process is `kill -9`'d at exec. `log show` output:

```
AMFI: '/Users/darin/tmp/rust-rewrite-spikes/s6-vmnet-entitlements/
  target/debug/s6-vmnet-entitlements' is adhoc signed.
amfid: ...s6-vmnet-entitlements not valid: Error
  Domain=AppleMobileFileIntegrityError Code=-420
  "The signature on the file is invalid"
AMFI: Code has restricted entitlements, but the validation of
  its code signature failed.
Unsatisfied Entitlements:
ASP: Security policy would not allow process
```

### Verbatim evidence: Probe F (`com.apple.vm.networking` on Apple Dev cert)

Process is also `kill -9`'d at exec. `log show` output:

```
taskgated-helper: Disallowing s6-vmnet-entitlements because no
  eligible provisioning profiles found
amfid: Error Domain=AppleMobileFileIntegrityError Code=-413
  "No matching profile found"
AMFI: Code has restricted entitlements, but the validation of
  its code signature failed.
Unsatisfied Entitlements:
```

## The working incantation (for v1 library code)

```xml
<!-- entitlements.plist -->
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.virtualization</key>
    <true/>
</dict>
</plist>
```

```bash
codesign --force --sign - --timestamp=none \
    --entitlements entitlements.plist path/to/binary
```

Re-sign after every `cargo build` (cargo strips signatures).

Verify:

```bash
codesign -d --entitlements :- path/to/binary
```

## The working Rust pattern for vmnet

`VZVmnetNetworkDeviceAttachment`'s only initializer (`-initWithNetwork:`) is
marked **unavailable** in `objc2-virtualization` 0.3.2's generated bindings.
Call it directly via `msg_send!`.

The Obj-C runtime's selector metadata requires exactly the type encoding
`^{vmnet_network=}` for the argument. `*mut AnyObject` encodes as `@`
(rejected). `*mut c_void` encodes as `^v` (rejected). Define a named ZST
with a custom `RefEncode`:

```rust
use objc2::encode::{Encoding, RefEncode};

#[repr(C)]
struct VmnetNetwork { _priv: [u8; 0] }
unsafe impl RefEncode for VmnetNetwork {
    const ENCODING_REF: Encoding =
        Encoding::Pointer(&Encoding::Struct("vmnet_network", &[]));
}
type VmnetNetworkRef = *mut VmnetNetwork;

#[link(name = "vmnet", kind = "framework")]
extern "C" {
    fn vmnet_network_configuration_create(mode: u32, status: *mut u32)
        -> *mut VmnetNetworkConfiguration;   // analogous type
    fn vmnet_network_configuration_disable_dhcp(cfg: *mut VmnetNetworkConfiguration);
    fn vmnet_network_create(cfg: *mut VmnetNetworkConfiguration, status: *mut u32)
        -> VmnetNetworkRef;
}

const VMNET_SHARED_MODE: u32 = 1001;

fn build_vmnet_attachment() -> Result<Retained<VZVmnetNetworkDeviceAttachment>, String> {
    let mut status = 0u32;
    let cfg = unsafe { vmnet_network_configuration_create(VMNET_SHARED_MODE, &mut status) };
    if cfg.is_null() { return Err(format!("ncc failed: {status}")); }
    unsafe { vmnet_network_configuration_disable_dhcp(cfg); }

    let mut status = 0u32;
    let net: VmnetNetworkRef = unsafe { vmnet_network_create(cfg, &mut status) };
    if net.is_null() { return Err(format!("nc failed: {status}")); }

    let attach: Option<Retained<VZVmnetNetworkDeviceAttachment>> = unsafe {
        msg_send![
            VZVmnetNetworkDeviceAttachment::alloc(),
            initWithNetwork: net,
        ]
    };
    attach.ok_or_else(|| "initWithNetwork: nil".into())
}
```

Then drop the returned attachment onto a `VZVirtioNetworkDeviceConfiguration`
via `setAttachment` (upcasting through `VZNetworkDeviceAttachment`), and
`setNetworkDevices` on the VM config.

**Availability guard**: `vmnet_network_create` and friends are macOS 26.0+.
The library must gate this behind `#[cfg(target_os = "macos")]` + a runtime
check (VZ's own `VZLinuxRosettaDirectoryShare::availability` pattern) or
just document "macOS 26+ only". For < macOS 26 hosts, fall back to NAT.

## What surprised me

1. **apple/containerization's own `vz.entitlements` is the minimal one.**
   I expected to find a longer entitlement list there given that the
   project clearly ships vmnet support; instead the library relies on the
   macOS-26+ `vmnet_network_*` path which isn't entitlement-gated beyond
   `com.apple.security.virtualization`. The Swift-side `import vmnet`
   machinery does the same thing we do manually.
2. **`vmnet_network_create` fails with `VMNET_MEM_FAILURE (1002)`, not
   `VMNET_NOT_AUTHORIZED (1010)`, when the process lacks the virtualization
   entitlement.** Misleading — don't pattern-match on the error code to
   infer entitlement status.
3. **`com.apple.vm.networking` is genuinely profile-restricted.** Even a
   legitimate Apple Development identity can't satisfy it without a
   provisioning profile that lists the entitlement — AMFI is strict.
   That's a bigger gate than "paid dev program"; it's "registered app ID
   with that capability enabled on the dev portal, provisioning profile
   generated, embedded in the app bundle."
4. **objc2's type-encoding check saved us from an afternoon of debugging.**
   The runtime panic at `msg_send!` with "expected `^{vmnet_network=}`,
   found `@`" pointed directly at the fix. If objc2 accepted the mismatched
   argument silently we'd have passed a nil or garbage pointer and spent
   hours diagnosing a crash.

## Proposed PRO_TIPS additions

Flagging for the curator to fold into shared `PRO_TIPS.md` — do not edit
shared docs directly per spike rules. Suggested sections:

### §28 — vmnet on macOS 26+ in Rust (from S6)

- `VZVmnetNetworkDeviceAttachment`'s `-initWithNetwork:` selector isn't
  bridged by `objc2-virtualization 0.3.2` (init/new are "unavailable").
  Call via `msg_send!`.
- The argument type must encode to `^{vmnet_network=}`. Neither `*mut
  AnyObject` nor `*mut c_void` will satisfy objc2's encoding check. Define
  a ZST with custom `RefEncode`:

  ```rust
  #[repr(C)] struct VmnetNetwork { _priv: [u8; 0] }
  unsafe impl RefEncode for VmnetNetwork {
      const ENCODING_REF: Encoding =
          Encoding::Pointer(&Encoding::Struct("vmnet_network", &[]));
  }
  ```

- `vmnet_network_create` and friends are macOS 26.0+. Gate the code path;
  fall back to NAT on older hosts.
- `vmnet_network_create` returns `VMNET_MEM_FAILURE (1002)` when the
  process lacks `com.apple.security.virtualization`. Not
  `VMNET_NOT_AUTHORIZED`. Don't pattern-match on the code.

### §29 — entitlements matrix for VZ network attachments (from S6)

| Attachment | Min entitlements | Signing | Paid dev program? |
|---|---|---|---|
| `VZNATNetworkDeviceAttachment` | `com.apple.security.virtualization` | ad-hoc OK | no |
| `VZVmnetNetworkDeviceAttachment` (macOS 26+) | `com.apple.security.virtualization` | ad-hoc OK | no |
| `VZBridgedNetworkDeviceAttachment` | `com.apple.security.virtualization` + `com.apple.vm.networking` | prov-profile required | **yes** |
| `VZFileHandleNetworkDeviceAttachment` | `com.apple.security.virtualization` | ad-hoc OK | no |

`com.apple.vm.networking` is a **restricted** entitlement: AMFI refuses
ad-hoc-signed binaries entirely and Apple-Dev-cert-signed binaries without
a matching provisioning profile. SIGKILL at exec, not a runtime error.

Console debug: `log show --last 30s --predicate 'eventMessage CONTAINS "AMFI"'`.

## Reusable patterns

These lift verbatim into the real `vmm` / `network` crate:

- **`build_vmnet_attachment()`** — in main.rs. Spike-sized helper; in the
  library it'd grow DHCP config, subnet setters, and an `AddressAllocator`
  ala `Sources/Containerization/VmnetNetwork.swift`. The hard part (the
  `^{vmnet_network=}` encoding + msg_send shape) is solved here.
- **`SPIKE_PROBE` env switch + `SPIKE_ENT` entitlement file switch +
  `SPIKE_SIGN_ID` signer** — pattern for any future entitlement / signing
  investigation. Makes regression checks a 30-second loop.

## Known loose ends (not spike-blocking)

- We didn't actually exercise IP traffic through the vmnet interface.
  Guest sees `eth0` but no DHCP + no `ip` tooling in the minimal initrd.
  A full network smoke test would need a richer rootfs (ping the host
  gateway, etc.) — but S4's E2E harness already has that, and it's
  orthogonal to S6's entitlement question. Defer to a network integration
  test in Phase 1.
- We didn't test `configureSubnet` (custom CIDR) — the Swift API supports
  it; straightforward port, not entitlement-affecting.
- We didn't stress-test DHCP-enabled vmnet (we call
  `vmnet_network_configuration_disable_dhcp`, matching what the Swift code
  does). Enabling DHCP might surface different auth paths; would need to
  probe if we change that later.
- We didn't try the `VZBridgedNetworkDeviceAttachment` path end-to-end.
  The class docstring and AMFI evidence are strong enough that we know
  it requires a paid dev program + provisioning profile; the rest of the
  code is straightforward. Confirm before Phase 3.

## Implications for library design

Phase 1 `network` crate can offer, ad-hoc-signed, no-paid-dev-program:

- NAT attachment (trivial)
- vmnet attachment with `VMNET_SHARED_MODE`, DHCP off, custom subnet, IP
  allocation per container (macOS 26+ only; gate with availability check)

Phase 3 / distributable builds (requires a paid-dev-program + CI-side
provisioning profile):

- Bridged attachment to a physical host NIC

This matches the plan's "NAT-only MVP" fallback almost exactly — except we
get vmnet "for free" at the same entitlement level on macOS 26+. Ship vmnet
in v1.
