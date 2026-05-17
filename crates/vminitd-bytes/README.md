# firkin-vminitd-bytes

`firkin-vminitd-bytes` owns the pinned vminitd and vmexec artifacts consumed by
`firkin-core` when it builds the cached init block.

Build modes:

- default builds embed verified local artifacts from `vminitd/bin/` or explicit
  `FIRKIN_VMINITD_PATH` and `FIRKIN_VMEXEC_PATH` overrides; if the pinned URLs
  are populated, `build.rs` downloads and caches missing artifacts under
  `$CARGO_TARGET_DIR/firkin-vminitd/`;
- `runtime-download` compiles without embedded bytes for environments that will
  resolve artifacts later;
- `vendored-vminitd` requires the pinned artifacts under `vendor/vminitd/`.

The build script verifies SHA-256 values from
`build-tools/build-vminitd/pin.toml`.

`build-tools/build-vminitd/fetch.sh` can pre-populate the same cache. The
current pin keeps URL fields empty until real release assets exist, so fresh
clones without local artifacts fail loudly instead of using an unpinned source.
