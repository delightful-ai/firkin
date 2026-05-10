# vendored vminitd artifacts

This directory is intentionally ignored except for this README.

Use it only with `firkin-vminitd-bytes/vendored-vminitd` when an offline or
air-gapped build needs local runtime artifacts. Place the pinned files at:

```text
vendor/vminitd/aarch64-unknown-linux-musl/vminitd
vendor/vminitd/aarch64-unknown-linux-musl/vmexec
```

The build script still verifies both files against
`build-tools/build-vminitd/pin.toml`.
