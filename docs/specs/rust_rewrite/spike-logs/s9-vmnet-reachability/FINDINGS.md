# S9 — findings

**Result**: 🟢 **FULL acceptance.** Container behind
`VZVmnetNetworkDeviceAttachment` (shared mode) gets an IP on the vmnet
subnet, host can ping it, container can reach `8.8.8.8`. ~250 LOC delta
on top of S4's harness.

## The headline

**Path A works, end-to-end, no surprises.** The library-to-vminitd
contract for guest network configuration is four unremarkable RPCs in
the existing `SandboxContext.proto`:

| RPC | Argument shape | What it does in vminitd |
|---|---|---|
| `IpLinkSet` | `{interface, up, mtu?}` | `ContainerizationNetlink.linkSet` |
| `IpAddrAdd` | `{interface, ipv4Address}` where `ipv4Address` is a **CIDR string** (e.g. `"192.168.64.2/24"`, not a bare IP) | `addrAdd` |
| `IpRouteAddDefault` | `{interface, ipv4Gateway}` | `routeAddDefault` |
| `ConfigureDns` | `{location, nameservers[], domain?, searchDomains[], options[]}` — `location` is the guest path whose `/etc/resolv.conf` will be written | writes `<location>/etc/resolv.conf` |

Sequence, from `LinuxContainer.swift:594-617`: `addressAdd` → `up` →
`routeAddDefault` → `configureDNS`. `lo` is brought up separately in
`standardSetup()`. That's the whole thing.

The vmnet host-side attachment (S6) and the exec vertical (S4) are
actually the hard bits. Once both were proven, S9 was mechanical: read
the Swift call site, mirror it in Rust.

## What worked exactly as expected

- **S6's `build_vmnet_attachment` pattern**: extended by ~30 LOC to
  also return subnet metadata via `vmnet_network_get_ipv4_subnet`
  (mirrors Swift `VmnetNetwork.getSubnet`). No new objc2 / msg_send
  dance needed — same `^{vmnet_network=}` type-encoding hack that S6
  pioneered.
- **S4's full harness**: ran unmodified except for swapping the
  container's probe shell script and adding four network RPCs before
  the existing Mount/CreateProcess flow. The stdio-listener, dial_vsock,
  block-device wiring, and stdout collection were all unchanged.
- **Container shares vminitd's netns**: S4's OCI spec unshares
  pid/mount/ipc/uts — but **not** network. So when vminitd configures
  eth0 in its own netns, the container sees the same eth0. This is
  exactly what apple/container does (see `LinuxContainer.swift`
  namespace defaults). No extra work needed.
- **vmnet subnet selection is dynamic**: each `vmnet_network_create`
  with `VMNET_SHARED_MODE` + `disable_dhcp` picks a fresh subnet (we
  saw `.64.0/24` through `.69.0/24` across six runs). Swift's
  `AddressAllocator` handles this by pulling the subnet back out via
  `get_ipv4_subnet` and allocating relative to it — we do the same.
- **Container IP = `subnet.lower + 2`**: matches Swift's
  `UInt32.rotatingAllocator(lower: cidr.lower.value + 2, size: ...)`.
  `.0` is the network address, `.1` is conventionally the vmnet
  gateway, `.2` and up are containers.
- **`CAP_NET_RAW` in the OCI capability set**: needed for busybox's
  `ping` to open a raw socket. Without it, ping fails with "permission
  denied". S4's spec didn't include it.

## Gotchas (one proposed PRO_TIPS addition)

### 1. `VZMACAddress` is **required** for a vmnet-attached network config

S6's spike didn't exercise this because its guest initrd never
configured `eth0` — it just probed whether the adapter appeared in
`/sys/class/net`. Add vmnet + actually validate the config, and:

```
config.validate() → NSError VZErrorDomain code=2
  "Invalid virtual machine configuration. The MAC address of the
   network device must not be nil."
```

Fix is one line:

```rust
let mac: Retained<VZMACAddress> =
    unsafe { VZMACAddress::randomLocallyAdministeredAddress() };
unsafe { net_cfg.setMACAddress(&mac) };
```

NAT and bridged also require a MAC, but `VZNATNetworkDeviceAttachment`
quietly falls back to an auto-generated one whereas the vmnet path
doesn't. Swift's `VmnetNetwork.Interface.device()` sets this explicitly
too — the library can (and probably should) let callers supply a
stable MAC for reproducibility. For the spike, random-local is fine.

### 2. `IpAddrAdd.ipv4Address` is a **CIDR string**, not a bare IP

The proto field is just `string ipv4Address`. You might assume "IP"
means "dotted quad". Reading vminitd's Swift-side validator shows it
calls `CIDRv4(string:)` which REQUIRES the `/<prefix>`. A bare
`192.168.64.2` deserializes to an RPCError (not a nice one). The Swift
library builds it as `CIDRv4(description: "\(ip.description)/\(prefix)")`.

### 3. The S4 busybox rootfs symlinks only a handful of applets

S4's rootfs-build script in `FINDINGS.md` has this line:

```
for applet in echo sh ls cat sleep true false env printf; do ...; done
```

`ip`, `ping`, `ifconfig`, `nslookup`, `wget` are all compiled into
`busybox.static` (we verified in a fresh alpine container — see
busybox's `--list`) but not on-disk as symlinks. Easiest workaround
for the spike: invoke through the multiplexer directly:

```sh
/bin/busybox ip -4 addr show eth0
/bin/busybox ping -c 2 8.8.8.8
```

Real fix for future spikes: extend S4's rootfs build to symlink the
full set we need. Flagged in "Handoff" — see below.

### 4. Host-side first-ping may "No route to host" once per new subnet

On a fresh vmnet subnet the macOS side needs a moment for its own arp
table to populate / bridge to come up. Our `host_ping` helper retries 5
times with 500 ms sleep; in practice one retry is enough. Added a
pre-ping 500 ms delay; that reduces the retry count to zero most runs,
but we keep the retry loop as a belt-and-braces.

## Reusable patterns for Phase 1

### `build_vmnet_setup() -> (attachment, subnet_lower, netmask, prefix, gateway)`

Lifts ~95 LOC verbatim into `crates/network/src/vmnet.rs`. Add DHCP
toggle and explicit subnet knob later (Swift supports both).

### Guest network configuration module (~60 LOC)

```rust
async fn configure_guest_interface(
    client: &mut SandboxContextClient,
    name: &str,                   // "eth0"
    cidr: &str,                   // "192.168.64.2/24"
    gateway: &str,                // "192.168.64.1"
    dns_location: &str,           // rootfs path
    nameservers: &[&str],
) -> Result<()> {
    client.ip_link_set(IpLinkSetRequest { interface: "lo".into(), up: true, mtu: None }).await?;
    client.ip_addr_add(IpAddrAddRequest { interface: name.into(), ipv4_address: cidr.into() }).await?;
    client.ip_link_set(IpLinkSetRequest { interface: name.into(), up: true, mtu: None }).await?;
    client.ip_route_add_default(IpRouteAddDefaultRequest {
        interface: name.into(), ipv4_gateway: gateway.into(),
    }).await?;
    client.configure_dns(ConfigureDnsRequest {
        location: dns_location.into(),
        nameservers: nameservers.iter().map(|s| s.to_string()).collect(),
        domain: None, search_domains: vec![], options: vec![],
    }).await?;
    Ok(())
}
```

### OCI spec additions

- `process.capabilities.{bounding,effective,permitted}` += `"CAP_NET_RAW"`
  when the container will do raw-socket stuff (ping, traceroute). Library
  should probably add it by default when the container has a network
  interface at all.
- `linux.namespaces` **must NOT include `network`** if the container
  is to share vminitd's netns (which is the model we're following).
  If the real library wants per-container netns isolation later, it
  needs to push `addressAdd` etc. inside the container's netns via
  `nsenter` — untested; flagged as open work.

## Known loose ends (not spike-blocking)

- **Two-container stretch**: NOT attempted. apple/container's model is
  one VM per container, each with its own `VZVmnetNetworkDeviceAttachment`
  pointing at the same shared `vmnet_network_ref`. Would require a
  second harness VM — significantly more work than the 1-2 h budget.
- **MTU override**: `IpLinkSet` has an optional `mtu` field; untested.
- **ConfigureHosts**: untested. Proto shape is symmetric to
  `ConfigureDns`.
- **IPv6**: vmnet shared mode doesn't advertise IPv6 by default;
  untested. The proto only has `ipv4Address` fields — extending to v6
  is a proto-level change.
- **Route to gateway outside subnet** (`routeAddLink` code path in
  `LinuxContainer.swift:600-603`): untested. The vmnet subnets we saw
  were well-formed with gateway IN the subnet, so we never exercised
  the "add a link-scope route first" branch.
- **`setNetworkDevices` drop behavior**: we `Box::leak` the attachment
  + net_cfg + MAC so they outlive the VM. Should be fine for the
  process lifetime; real library will want a proper ownership story
  (probably: hold in the VM wrapper struct's fields).

## Proposed PRO_TIPS additions

For the curator to fold into `PRO_TIPS.md` — do not touch shared docs
per SPIKE_RUNBOOK.md. Suggested sections:

### Proposed §31 — vmnet-attached VMs need an explicit MAC address

Extending §29 (S6). Once you actually validate + boot a VM whose
network device's attachment is `VZVmnetNetworkDeviceAttachment`:

```
VZErrorDomain code=2 "The MAC address of the network device must not be nil."
```

One-line fix:

```rust
let mac = unsafe { VZMACAddress::randomLocallyAdministeredAddress() };
unsafe { net_cfg.setMACAddress(&mac) };
```

NAT attachments happen to auto-generate a MAC; vmnet ones don't. Apply
before `validateWithError`.

### Proposed §32 — vminitd SandboxContext networking RPCs (from S9)

Driving guest-side network configuration over vminitd's gRPC, the full
sequence (from `LinuxContainer.swift:594-617`):

```rust
// 1. Bring up loopback (part of "standard setup" — once per vminitd).
client.ip_link_set(IpLinkSetRequest {
    interface: "lo".into(), up: true, mtu: None,
}).await?;

// 2. Assign an IP. NB: ipv4Address field is a CIDR string.
client.ip_addr_add(IpAddrAddRequest {
    interface: "eth0".into(),
    ipv4_address: "192.168.64.2/24".into(),  // NOT "192.168.64.2"
}).await?;

// 3. Bring up eth0.
client.ip_link_set(IpLinkSetRequest {
    interface: "eth0".into(), up: true, mtu: None,
}).await?;

// 4. Default route via the vmnet gateway (the .1 of the subnet in
//    shared mode).
client.ip_route_add_default(IpRouteAddDefaultRequest {
    interface: "eth0".into(),
    ipv4_gateway: "192.168.64.1".into(),
}).await?;

// 5. DNS. `location` is the guest filesystem path whose
//    `<location>/etc/resolv.conf` will be written.
client.configure_dns(ConfigureDnsRequest {
    location: "/run/container/<id>/rootfs".into(),
    nameservers: vec!["192.168.64.1".into(), "8.8.8.8".into()],
    ..Default::default()
}).await?;
```

Gotcha: `ipv4Address` is a CIDR, not a bare IP. vminitd's Swift-side
CIDRv4 parser rejects `192.168.64.2` with no prefix (RPC error on the
wire is terse; easy to waste time on).

If the container's OCI spec doesn't unshare the `network` namespace,
the container sees the same `eth0` vminitd just configured. (That's how
apple/container's `LinuxContainer` works by default.)

### Proposed §33 — vmnet subnet discovery from Rust (from S9)

Extending S6's extern bindings:

```rust
#[link(name = "vmnet", kind = "framework")]
extern "C" {
    fn vmnet_network_get_ipv4_subnet(
        net: VmnetNetworkRef,
        subnet: *mut InAddr,  // network byte order
        mask: *mut InAddr,    // network byte order
    );
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InAddr { s_addr: u32 }
unsafe impl objc2::encode::RefEncode for InAddr {
    const ENCODING_REF: objc2::encode::Encoding =
        objc2::encode::Encoding::Pointer(
            &objc2::encode::Encoding::Struct("in_addr", &[objc2::encode::Encoding::UInt]));
}

// In host byte order:
let net = u32::from_be(subnet.s_addr);
let mask = u32::from_be(mask.s_addr);
let prefix = mask.count_ones() as u8;
let gateway = (net & mask) | 1;   // vmnet uses .1 as the gateway
```

The subnet vmnet picks is deterministic within a run but rotates across
runs (we saw `.64.0/24` through `.69.0/24` across six invocations).
Always pull it back out; don't hardcode.

## Time to solve

~1 h 10 min focused work. Breakdown:

- ~20 min reading Swift (VmnetNetwork, Vminitd.swift, LinuxContainer.swift,
  SandboxContext.proto). No guesswork on RPC shapes.
- ~15 min scaffolding + lifting S4 and S6 bits.
- ~10 min writing the new code (vmnet subnet extraction, 4 RPCs,
  host-ping helper, probe script).
- ~5 min on the first compile → fix tokio features.
- ~10 min on the first-run failure (`ip` applet missing) → switch to
  busybox multiplexer invocation.
- ~10 min re-runs (debug + release, three times each) + cleanup of dead
  code + release profile check.
- ~15 min on JOURNAL / STATUS / FINDINGS.

Well under the 1-2 hour budget, primarily because S4 and S6 had already
paid the hard costs.
