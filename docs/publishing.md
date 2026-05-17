# Publishing Runbook

This is the manual release runbook for `0.0.1` and later unstable releases.

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
6. Create the release tag only after dry-runs pass.
7. Let the release workflow create the GitHub release and attach assets.

## 0.0.2 Status

`0.0.2` was published to crates.io on 2026-05-17 after:

```bash
cargo metadata --format-version 1
cargo fmt --check
scripts/check-firkin-crate-graph.sh
git diff --check
CARGO_TARGET_DIR=/tmp/firkin-0.0.2-target cargo check --workspace --all-targets
CARGO_TARGET_DIR=/tmp/firkin-0.0.2-clippy-target cargo clippy --workspace --all-targets -- -D warnings
CARGO_TARGET_DIR=/tmp/firkin-0.0.2-test-target cargo test --workspace
```

`0.0.2` contains the public-release fixes for runtime vmnet symbol resolution
and deterministic `.localhost` E2B proxy preflight.

External consumer proof:

```bash
rm -rf /tmp/firkin-consumer-smoke-002
cargo new /tmp/firkin-consumer-smoke-002 --bin
cd /tmp/firkin-consumer-smoke-002
cargo add firkin@0.0.2
cargo add tokio@1 --features macros,rt-multi-thread
cargo check
cargo run
```

This proof compiled and ran without `path`, `patch.crates-io`, or git
dependencies.

## 0.0.1 Status

`0.0.1` was published to crates.io on 2026-05-17 after:

```bash
cargo metadata --format-version 1
cargo fmt --check
CARGO_TARGET_DIR=/tmp/firkin-standalone-target cargo check --workspace --all-targets
CARGO_TARGET_DIR=/tmp/firkin-standalone-target cargo clippy --workspace --all-targets -- -D warnings
CARGO_TARGET_DIR=/tmp/firkin-standalone-target cargo test --workspace
scripts/check-firkin-crate-graph.sh
git diff --check
cargo publish --dry-run -p firkin-types
cargo publish --dry-run -p firkin-artifacts
```

Published crates:

```text
firkin-types
firkin-trace
firkin-artifacts
firkin-envd
firkin-vminitd-bytes
firkin-vsock
firkin-ext4
firkin-e2b-wire
firkin-vmm
firkin-sandbox
firkin-admission
firkin-hygiene
firkin-template
firkin-evidence
firkin-oci
firkin-vminitd-client
firkin-e2b-contract
firkin-e2b-server
firkin-core
firkin-benchmark
firkin-runtime
firkin-single-node
firkin
```

External consumer proof:

```bash
rm -rf /tmp/firkin-consumer-smoke
cargo new /tmp/firkin-consumer-smoke --bin
cd /tmp/firkin-consumer-smoke
cargo add firkin@0.0.2
cargo add tokio@1 --features macros,rt-multi-thread
cargo check
cargo run
```

This proof compiled and ran without `path`, `patch.crates-io`, or git
dependencies.
