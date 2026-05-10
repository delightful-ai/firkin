# firkin-vmm

`firkin-vmm` owns VM configuration and the Apple
Virtualization.framework backend.

It provides:

- typed VM configuration for kernel, init block, disks, CPU, memory, and vsock;
- NAT and shared vmnet network attachment setup;
- virtiofs Rosetta directory sharing for amd64 guest process support;
- runtime handles for boot, pause, resume, stop, statistics, vsock dial, and
  vsock listen;
- codesigning resources for live test binaries that need virtualization
  entitlements.

The crate contains the small amount of `unsafe` needed for Objective-C framework
interop. Other workspace crates keep `unsafe_code` forbidden.
