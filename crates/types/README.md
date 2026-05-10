# firkin-types

`firkin-types` contains shared validated value types for the workspace.

Examples include container IDs, process IDs, VM IDs, hostnames, E2B-style
`{port}-{sandboxID}.{domain}` proxy host routes, E2B-style sandbox network
policy fields, vsock ports, virtiofs tags, sizes, platforms, architectures,
operating systems, and Linux namespace kinds.

This crate is intentionally leaf-shaped. Later crates use it to avoid passing
unchecked strings and integers across crate boundaries.
