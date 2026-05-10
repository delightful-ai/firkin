# S6 — status snapshot

Last updated: 2026-04-20

## State
- 🟢 **Passed** — vmnet works with the SAME `com.apple.security.virtualization`
  ad-hoc entitlement as NAT. No paid Apple Developer Program required.

## Summary

- **NAT** (S1 already answered): ✅ ad-hoc + `com.apple.security.virtualization`.
- **vmnet (`VZVmnetNetworkDeviceAttachment`)**: ✅ same — ad-hoc +
  `com.apple.security.virtualization`. Guest's `/sys/class/net` shows `eth0`.
  Proven on macOS 26.3 arm64.
- **Bridged (`VZBridgedNetworkDeviceAttachment`)**: ⛔ requires the
  `com.apple.vm.networking` entitlement, which AMFI will ONLY honor when
  backed by a provisioning profile that whitelists it — i.e. paid Apple
  Developer Program. Not tested end-to-end; evidence documented in FINDINGS.

## Repro

```bash
cd ~/tmp/rust-rewrite-spikes/s6-vmnet-entitlements

# Control: no net attached. Guest prints `lo` only.
SPIKE_TIMEOUT_SECS=15 SPIKE_PROBE=none ./sign-and-run.sh

# NAT: works with virt-only entitlements.
SPIKE_TIMEOUT_SECS=15 SPIKE_PROBE=nat ./sign-and-run.sh

# vmnet: ALSO works with virt-only entitlements. Guest sees `lo` + `eth0`.
SPIKE_TIMEOUT_SECS=15 SPIKE_PROBE=vmnet ./sign-and-run.sh

# Evidence of the opposite: try adding com.apple.vm.networking —
# AMFI kills the process.
SPIKE_TIMEOUT_SECS=15 SPIKE_PROBE=vmnet SPIKE_ENT=networking ./sign-and-run.sh
# (and check Console: amfid says "The signature on the file is invalid",
#  AMFI says "Unsatisfied Entitlements".)
```

## Done checklist

- [x] Acceptance criteria met: "we have a clear written answer for what
      additional entitlement / provisioning profile is required, even if
      we defer actually implementing vmnet to Phase 3" — answered: NO
      additional entitlement needed for vmnet; bridged needs paid dev program.
- [x] `sign-and-run.sh` exits 0 cold (NAT and vmnet probes).
- [x] Debug + release builds clean (`cargo clean && cargo build` +
      `cargo build --release` both pass, no warnings).
- [x] JOURNAL.md final resolution entry written.
- [x] FINDINGS.md written — what worked, what surprised.
- [x] State line above reads "🟢 Passed".
- [ ] spike-logs/README.md index updated — flagged for curator (shared doc).
- [x] PRO_TIPS.md additions flagged — in FINDINGS under
      "Proposed PRO_TIPS additions".

## Handoff notes

- `src/main.rs` has a working `build_vmnet_attachment()` helper that:
  1. Links vmnet.framework.
  2. Defines `#[repr(C)] struct VmnetNetwork { _priv: [u8; 0]; }` with
     a `RefEncode` impl encoding to `^{vmnet_network=}`.
  3. Calls `vmnet_network_create` and then
     `msg_send![VZVmnetNetworkDeviceAttachment::alloc(), initWithNetwork: net]`.
  This pattern lifts cleanly into the real library. ~40 LOC.
- `entitlements-networking.plist` and `entitlements-hypervisor.plist` are
  negative probes. Keep them in the spike dir so future agents can re-run
  the AMFI-kill evidence quickly.
- No blocking issues for Phase 1. Library can design around NAT+vmnet
  without paid dev program; bridged is Phase 3 / distributable-builds-only.
