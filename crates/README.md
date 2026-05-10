# Firkin Rust Workspace

The Rust rewrite lives under `crates/` and follows
`docs/specs/rust_rewrite/03-project-layout.md`.

The workspace is built in topological order. `types` is the first crate because
every later crate depends on its validated value types. `substrate` owns the
production control models for capacity, snapshots, warm pools, and template
builds without depending on VM/runtime crates. `template` owns local template
build execution on top of those substrate contracts. `e2b` contains the SDK-shaped
control-plane wire contract that the future local backend will serve.
