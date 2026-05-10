# S9 — status snapshot

Last updated: 2026-04-20

## State
- 🟢 **Passed — FULL acceptance.** Container gets an IP on the vmnet subnet
  (dynamic, e.g. `192.168.69.2/24`), host pings it at ~0.2–0.3 ms RTT, and
  the container reaches `8.8.8.8` at ~7–100 ms RTT. Path A — vminitd
  `IpAddrAdd` / `IpLinkSet` / `IpRouteAddDefault` / `ConfigureDns` RPCs
  driven directly from Rust. No DHCP client in the rootfs.

## Summary

- **Guest-config path used**: A (vminitd netlink RPCs over SandboxContext
  gRPC). No udhcpc.
- **RPC sequence** (mirrors `Sources/Containerization/LinuxContainer.swift:594`):
  `IpLinkSet(lo, up)` → `IpAddrAdd(eth0, "<ip>/<prefix>")` →
  `IpLinkSet(eth0, up)` → `IpRouteAddDefault(eth0, <gateway>)` →
  `ConfigureDns(<rootfs>, [gateway, 8.8.8.8, 1.1.1.1])`.
- **Container shares vminitd's netns** (S4's spec unshares
  pid/mount/ipc/uts but NOT network). eth0 that vminitd configured is the
  eth0 the container sees.
- **Subnet is allocated by vmnet dynamically** — different across runs
  (observed .64.0/24, .65.0/24, .66.0/24, .67.0/24, .68.0/24, .69.0/24 on
  successive invocations). Picking container IP via `subnet.lower + 2`
  (matching Swift's `VmnetNetwork.Allocator`) works every time.
- **No AMFI / entitlements drift from S6**: same `entitlements.plist`
  (only `com.apple.security.virtualization`), ad-hoc signing.

## Acceptance (from 02-spike-plan.md §S9)

- [x] Container's eth0 has an IP on the vmnet subnet — yes, logged via
      container stdout (`## eth0 addr ##` block with `inet 192.168.69.2/24`).
- [x] `ping -c 2 <container-ip>` from host returns 2/2 packets within
      500 ms — yes, 0.21–0.29 ms avg RTT across three runs.
- [x] Container reaches an external host — yes, `ping -c 2 8.8.8.8`
      succeeds inside container (0% loss, ~7–100 ms RTT).
- [x] `SPIKE_TIMEOUT_SECS=30 ./sign-and-run.sh` exits 0 — yes, both
      debug and release.
- [x] No AMFI denial; entitlement state unchanged from S6.

## Repro

```bash
# Prereq: S3 done (kata kernel + init.block in place), S4 done (rootfs.ext4
# staged under s4-e2e/assets). macOS 26+.

cd /Users/darin/vendor/github.com/https:/github.com/apple/containerization
docs/specs/rust_rewrite/spike-template/scaffold.sh 9 vmnet-reachability

# Lift S4's harness + proto + Cargo.toml + build.rs.
cd ~/tmp/rust-rewrite-spikes/s9-vmnet-reachability
cp ~/tmp/rust-rewrite-spikes/s4-e2e/src/main.rs src/main.rs
cp ~/tmp/rust-rewrite-spikes/s4-e2e/Cargo.toml Cargo.toml
cp ~/tmp/rust-rewrite-spikes/s4-e2e/build.rs build.rs
cp -R ~/tmp/rust-rewrite-spikes/s4-e2e/proto ./proto
ln -sf ~/tmp/rust-rewrite-spikes/s4-e2e/assets/init.block assets/init.block
ln -sf ~/tmp/rust-rewrite-spikes/s4-e2e/assets/rootfs.ext4 assets/rootfs.ext4
# (Fix package name in Cargo.toml, add `process` to tokio features.)

# Then layer in S6's build_vmnet_attachment() pattern — see src/main.rs.

SPIKE_TIMEOUT_SECS=30 ./sign-and-run.sh
SPIKE_TIMEOUT_SECS=30 PROFILE=release ./sign-and-run.sh
```

Expected final lines:
```
[ACC/1] eth0 has IP 192.168.<N>.2/24
[ACC/2] host ping RTT 0.2x ms (<= 500 ms)
[ACC/3] container reached external host (EXT_OK marker seen)
[ACC] FULL acceptance met — vmnet end-to-end reachability
```

## RPCs added over S4's set

Just four, all in `SandboxContext.proto` already:

- `IpLinkSet { interface, up, mtu? }` — brings `lo` and `eth0` up.
- `IpAddrAdd { interface, ipv4Address }` — CIDR string, e.g.
  `"192.168.69.2/24"`. NOT a bare IP.
- `IpRouteAddDefault { interface, ipv4Gateway }` — default route via
  vmnet's gateway (.1 of subnet).
- `ConfigureDns { location, nameservers, domain?, searchDomains[],
  options[] }` — vminitd writes `<location>/etc/resolv.conf`. Pass the
  container's mounted rootfs path as `location`.

`ConfigureHosts` was available but not needed for the smoke test.

## Done checklist

- [x] Acceptance criteria met (quoted above, each ticked)
- [x] `sign-and-run.sh` exits 0 cold (both debug and release)
- [x] Debug + release builds clean (`cargo clean && cargo build{,--release}`)
- [x] JOURNAL.md has a final resolution entry
- [x] FINDINGS.md written (what worked, what surprised, reusable patterns)
- [x] State line above reads "🟢 Passed"
- [ ] spike-logs/README.md index updated — flagged for curator (shared doc).
- [x] PRO_TIPS additions flagged in FINDINGS.md under "Proposed PRO_TIPS
      additions".

## Handoff notes

### For Phase 1 `network` / `core` crates

- The four RPC calls in `run_tests()` (lines 612–665 of the spike
  `main.rs`) lift verbatim: call shape, argument shape, order are all
  production. Wrap them in a `GuestNetworkConfigurator` trait that takes
  a gRPC client + a `VmnetInterface` descriptor.
- The `build_vmnet_setup()` helper grew `vmnet_network_get_ipv4_subnet`
  over S6's version — that's the function Phase 1's `AddressAllocator`
  will use. `vmnet_network_get_ipv4_subnet` writes two `in_addr`s (both
  network byte order), and the host-order conversion is a `u32::from_be`.
- We set the network device's MAC via
  `VZMACAddress::randomLocallyAdministeredAddress()`. Without it, VZ's
  config validation fails with "MAC address must be non-nil" for vmnet
  attachments. S6 didn't hit this because its rootfs didn't have the
  container doing anything with the interface.
- Container IP selection is `subnet.lower + 2` — matches
  `VmnetNetwork.Allocator.allocate` in Swift which starts at `cidr.lower.value + 2`.
- Container must share netns with vminitd OR the library must push
  `IpAddrAdd` into the container's netns. S4/S9 shared; Phase 1 should
  decide per design.

### Unknowns still open

- **Two-container stretch**: not attempted. In apple/container's model,
  each container gets its own VM, with each VM pointing at the same
  shared `vmnet_network_ref` — not exercised here (single-VM harness).
- **MTU**: we left it at the default (1500). vminitd's `up` RPC supports
  an MTU override; untested.
- **ConfigureHosts**: untested. Same shape as `ConfigureDns`.
- **IPv6**: vmnet doesn't advertise IPv6 in shared mode by default;
  untested.
- **Container netns unshare + address-in-ns**: see above.
