# Publishing Runbook

This is a manual runbook until crate ownership, vminitd release assets, and
release credentials are confirmed.

1. Verify the repository is clean and on `main`.
2. Run local gates:

   ```bash
   cargo metadata --format-version 1
   cargo fmt --check
   cargo check --workspace --all-targets
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   scripts/check-firkin-crate-graph.sh
   git diff --check
   ```

3. Run `cargo package` for publishable crates. During a first-ever bootstrap,
   dependent crates can only fully verify after their lower-level `firkin-*`
   dependencies have reached the crates.io index. If verification reports a
   missing just-published `firkin-*` crate, wait for index propagation and retry
   the same crate.

   ```bash
   cargo package -p firkin-types
   cargo package -p firkin-trace
   cargo package -p firkin-artifacts
   cargo package -p firkin-envd
   cargo package -p firkin-vminitd-bytes

   cargo package -p firkin-core
   ```
4. Confirm crate ownership on crates.io.
5. For later releases, configure the GitHub environment `crates-io` with a
   `CRATES_IO_TOKEN` secret, then run the `Publish crates` workflow manually.
6. Create a `v0.1.0.N` tag only after dry-runs pass.
7. Let the release workflow create a draft release. Publish the draft manually.

Do not run `cargo publish`, make the GitHub repo public, or publish a release
until those actions are explicitly approved.
