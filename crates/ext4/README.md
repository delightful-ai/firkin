# firkin-ext4

`firkin-ext4` writes ext4 images used by the Rust runtime.

It provides:

- a structured image builder for files, directories, symlinks, device nodes,
  permissions, and timestamps;
- OCI layer extraction with whiteout and opaque directory handling;
- deterministic `init.block` synthesis for vminitd and vmexec payloads.

The crate is a pure Rust storage component. It does not know about VM boot,
OCI registries, or vminitd RPCs.
