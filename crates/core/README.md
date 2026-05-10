# firkin-core

`firkin-core` is the orchestration layer for the Rust containerization library.
It connects OCI pulls, ext4 rootfs construction, vminitd RPCs, VM boot, network
setup, Rosetta setup, and process output collection.

Primary surface:

- `ContainerBuilder` builds either an implicit VM container run or a raw block
  VM run.
- `ContainerResult` carries process status, stdout, stderr, and runtime IDs.
- Live ignored tests in `tests/builder.rs` exercise the real
  Virtualization.framework path on supported Apple hosts.

Lower crates own storage, OCI, vminitd, vsock, and VM details. This crate owns
the single high-level container run sequence.
