# firkin-vsock

`firkin-vsock` provides async vsock stream and listener primitives used by the
VM and vminitd client crates.

It wraps file-descriptor based streams in Tokio-friendly types and includes a
connector service for libraries that need to plug vsock transport into tonic or
hyper-style clients.
