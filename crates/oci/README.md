# firkin-oci

`firkin-oci` owns OCI image references, registry pulls, cached image bundles,
platform selection, and runtime spec construction.

The crate turns an image reference plus platform into an `ImageBundle` that
`firkin-core` can stage into an ext4 rootfs. It keeps registry and OCI JSON
details out of the VM and container orchestration crates.
