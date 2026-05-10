# firkin

`firkin` is the facade crate for the Rust containerization workspace.

It exposes a curated public surface so callers can depend on one library crate
without inheriting every internal workspace type at the root path:

- sandbox orchestration from `firkin::sandbox`;
- local Apple/VZ sandbox backend construction from `firkin::sandbox::apple_vz`;
- container and VM mechanics from explicit modules such as `firkin::core`,
  `firkin::vmm`, `firkin::vsock`, and `firkin::types`;
- E2B/Cube compatibility types from `firkin::e2b`;
- envd process/filesystem protocol contracts from `firkin::envd`;
- benchmark, evidence, and trace surfaces from `firkin::benchmark`,
  `firkin::evidence`, and `firkin::trace`;
- OCI bundle and platform types from `firkin-oci` and `firkin-types`;
- Apple/VZ local runtime capability reporting, including explicit unsupported
  E2B/Cube features and a hard-fail helper for required capabilities.

Examples live in `examples/` and are checked with `cargo check -p firkin
--examples`.
