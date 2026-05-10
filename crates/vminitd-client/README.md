# firkin-vminitd-client

`firkin-vminitd-client` provides typed helpers over the vminitd
`SandboxContext` gRPC API.

It contains the generated protobuf client plus wrapper requests for rootfs
mounting, OCI runtime bundle creation, process execution, network setup, and
Rosetta binfmt setup.

The crate speaks over `firkin-vsock`, so higher layers can treat guest control
as typed Rust calls instead of hand-built protobuf messages.
