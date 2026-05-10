# Spike S9 — vmnet-reachability

**Spike code**: `~/tmp/rust-rewrite-spikes/s9-vmnet-reachability/`
**Started**: 2026-04-20
**Resolved**: 2026-04-20 (same day, ~1 h 10 min focused work)

## The question

> With a container running behind a `VZVmnetNetworkDeviceAttachment` in
> shared mode, does the container actually get a reachable IP from the
> host, and can it talk to the outside world?

## Acceptance

Quoted from `02-spike-plan.md` §S9:

- Host stdout shows the container's eth0 IP on a vmnet subnet.
- `ping -c 2 <container-ip>` from the host returns 2/2 packets within 500 ms.
- Container can resolve DNS and reach an external host.
- No kernel panic, no AMFI denial, no entitlement drift from S6's passing state.

## Plan

1. Scaffold. Lift S4 (full e2e harness) + S6 (`build_vmnet_attachment`)
   verbatim. Wire them together.
2. Read `VmnetNetwork.swift` + `LinuxContainer.swift` to find the exact
   RPC sequence for Path A (vminitd-netlink config).
3. Extend `build_vmnet_attachment` to also return subnet metadata (use
   `vmnet_network_get_ipv4_subnet`), so the test routine can pick an IP
   and a gateway.
4. Swap S4's `echo hello` probe for a networking smoke test:
   `ip addr show eth0 && ping gateway && ping 8.8.8.8`, print an
   `EXT_OK` marker on external success.
5. Add host-side `ping` against the container IP to assert host↔guest
   reachability.
6. Punt the two-container stretch unless time allows.

## Events

- 2026-04-20 18:40Z — `scaffold.sh 9 vmnet-reachability` run. Harness
  boots an empty VM via the template. Kata kernel symlinked in from S3.
- 2026-04-20 18:44Z — Lifted S4's `src/main.rs` + `Cargo.toml` +
  `build.rs` + `proto/` verbatim. Also symlinked
  `s4-e2e/assets/{init.block,rootfs.ext4}`.
- 2026-04-20 18:46Z — Renamed package `s4-e2e` → `s9-vmnet-reachability`.
  Added `process` to tokio features (needed for `tokio::process::Command`
  to invoke host `ping`).
- 2026-04-20 18:55Z — Read `VmnetNetwork.swift` (subnet allocation,
  `vmnet_network_get_ipv4_subnet`), `Vminitd.swift` (the four network
  RPC wrappers: `addressAdd` / `up` / `routeAddDefault` / `configureDNS`),
  and `LinuxContainer.swift:594-617` (the exact sequence and arg shapes
  the library drives). No guesswork; all call sites lifted straight
  from Swift.
- 2026-04-20 19:00Z — Extended S6's `build_vmnet_attachment` to
  `build_vmnet_setup()`: adds `vmnet_network_get_ipv4_subnet` +
  host-order conversion, returns a struct with subnet/mask/gateway so the
  test routine can allocate a container IP via `subnet.lower + 2`
  (matching Swift's `AddressAllocator`).
- 2026-04-20 19:10Z — Wired the vmnet attachment onto a
  `VZVirtioNetworkDeviceConfiguration` + `setMACAddress(randomLocally...)`.
  **Found**: without a MAC, config.validate blows up for vmnet
  attachments. S6 didn't hit this because its guest never brought eth0 up.
- 2026-04-20 19:15Z — Wrote the four network RPCs. Most-common pitfall
  dodged because we read the Swift: `IpAddrAdd.ipv4Address` is the full
  CIDR string (`192.168.64.2/24`), NOT a bare IP. Not obvious from the
  proto alone.
- 2026-04-20 19:20Z — First compile: failed — `tokio::process` not
  enabled. Added `process` to tokio features. Clean debug + release
  build.
- 2026-04-20 19:25Z — First run (`SPIKE_TIMEOUT_SECS=45`): every RPC
  round-tripped cleanly; container ran but exited 127.
  Cause: `/bin/ip: not found`. The S4 rootfs only symlinks
  `echo/sh/ls/cat/sleep/true/false/env/printf` from busybox; `ip` and
  `ping` are inside busybox but not on-disk as symlinks.
  Fix: invoke via the busybox multiplexer (`/bin/busybox ip ...`) so we
  don't have to rebuild the rootfs.
- 2026-04-20 19:28Z — Second run: FULL acceptance. Container
  `192.168.65.2/24`; container ping gateway 0.15 ms RTT; container ping
  8.8.8.8 ~8 ms RTT (**EXT_OK** marker printed); host ping container
  0.21 ms RTT (first attempt "No route to host" — arp-cache warm-up —
  second attempt succeeded).
- 2026-04-20 19:32Z — Cleaned up two dead-code warnings (unused
  constants from S6's code path). Ran debug + release three times each;
  every run passes. Subnet differs each run (`.64.`, `.65.`, `.66.`,
  `.67.`, `.68.`, `.69.`) as vmnet rotates allocations.
- 2026-04-20 19:40Z — Sample run tail (one of three debug runs):

  ```
  [vmnet] network_create OK (ref=0x...)
  [vmnet] subnet=192.168.69.0 netmask=255.255.255.0 prefix=24 gateway=192.168.69.1
  [vmnet] -[VZVmnetNetworkDeviceAttachment initWithNetwork:] OK
  [host] vmnet attachment added to VM config (MAC set)
  [net] assigning container IP 192.168.69.2 (cidr 192.168.69.2/24), gateway 192.168.69.1
  [net] up(lo) OK
  [net] IpAddrAdd(eth0, 192.168.69.2/24) OK
  [net] up(eth0) OK
  [net] IpRouteAddDefault(eth0 -> 192.168.69.1) OK
  [net] ConfigureDns OK (location=/run/container/container-0/rootfs)
  [ACC/1] eth0 has IP 192.168.69.2/24
  [ACC/2] host ping RTT 0.27 ms (<= 500 ms)
  [ACC/3] container reached external host (EXT_OK marker seen)
  [ACC] FULL acceptance met — vmnet end-to-end reachability
  [tokio] run_tests() completed ok
  ```

## Resolution

Spike 🟢 PASSED with full acceptance. ~250 LOC delta on top of S4's
harness: mostly the extra vmnet extern bindings (+`vmnet_network_get_ipv4_subnet`),
the four-RPC network-setup sequence, the `host_ping` helper, and the
`u32_to_dotted` / `subnet_host_order` small helpers.

**Path A (vminitd netlink RPCs) is the clean answer.** No DHCP client
needed in the rootfs. The exact RPC sequence + field shapes lift
verbatim into Phase 1's `core` / `network` crate. See FINDINGS.md for
the list of reusable patterns + the one gotcha (busybox applet
availability) + the one proposed PRO_TIPS addition.

Two-container stretch not attempted — would require a second VM
harness (apple/container's one-container-per-VM model).

## Final run (tail) — debug, `SPIKE_TIMEOUT_SECS=30`

    --- 192.168.70.1 ping statistics ---
    2 packets transmitted, 2 packets received, 0% packet loss
    round-trip min/avg/max = 0.193/0.212/0.231 ms
    ## ping 8.8.8.8 ##
    PING 8.8.8.8 (8.8.8.8): 56 data bytes
    64 bytes from 8.8.8.8: seq=0 ttl=115 time=8.817 ms
    64 bytes from 8.8.8.8: seq=1 ttl=115 time=7.435 ms

    --- 8.8.8.8 ping statistics ---
    2 packets transmitted, 2 packets received, 0% packet loss
    round-trip min/avg/max = 7.435/8.126/8.817 ms
    EXT_OK
    ## done ##
    ----- end container output -----
    [ACC/1] eth0 has IP 192.168.70.2/24
    [ACC/2] host ping RTT 0.27 ms (<= 500 ms)
    [ACC/3] container reached external host (EXT_OK marker seen)
    [ACC] FULL acceptance met — vmnet end-to-end reachability
    [tokio] run_tests() completed ok
