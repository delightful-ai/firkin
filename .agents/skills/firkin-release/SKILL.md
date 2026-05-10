---
name: firkin-release
description: Use when preparing, publishing, verifying, or diagnosing Firkin crates.io/GitHub releases from the standalone Firkin repository.
---

# Firkin Release

Use this skill inside the standalone Firkin repository when the task involves
CI, crate packaging, crates.io publication, GitHub release drafts, or release
verification.

## Hard Rules

- Do not edit `/Users/darin/vendor/github.com/apple/containerization`; it is
  source evidence only.
- Do not run `cargo publish` unless the user explicitly approves publishing in
  the current thread.
- Keep the GitHub repository private unless the user explicitly asks to make it
  public.
- Use a hard cutover. Do not add compatibility shims for removed Apple Swift
  package paths.

## Local Release Gates

Run these from the repo root before publishing or pushing a release commit:

```bash
cargo metadata --format-version 1
cargo fmt --check
CARGO_TARGET_DIR=/tmp/firkin-standalone-target cargo check --workspace --all-targets
CARGO_TARGET_DIR=/tmp/firkin-standalone-target cargo clippy --workspace --all-targets -- -D warnings
CARGO_TARGET_DIR=/tmp/firkin-standalone-target cargo test --workspace
scripts/check-firkin-crate-graph.sh
git diff --check
```

`firkin-vminitd-bytes` defaults to `runtime-download`, so normal checks and
consumer installs do not require local vminitd/vmexec binaries. Live VM proof
remains a separate, signed macOS flow.

## Publish Order

Publish in dependency order. Crates.io rate-limits new crate creation; if a 429
response gives a retry timestamp, wait and resume from the failed crate.

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
firkin-runtime
firkin-single-node
firkin-benchmark
firkin
```

Use:

```bash
cargo publish -p <crate>
```

If dependent crates fail with `no matching package named firkin-* found`, the
upstream crate has not propagated through the index yet. Wait and retry the
same crate; do not change dependency versions.

## GitHub Release Setup

The repository should contain:

- `.github/workflows/ci.yml` for macOS Rust checks.
- `.github/workflows/package.yml` for package dry-runs and tarball checks.
- `.github/workflows/publish.yml` for manual crates.io publication with the
  `crates-io` environment and `CRATES_IO_TOKEN` secret.
- `.github/workflows/release.yml` for draft prerelease artifacts on
  `v0.0.1-alpha*` tags.

After push, confirm:

```bash
gh repo view delightful-ai/firkin --json nameWithOwner,visibility,defaultBranchRef,url
gh workflow list --repo delightful-ai/firkin
```

Visibility must remain `PRIVATE`.

## External Verification

After crates.io publication completes, switch to the `firkin-external-consumer`
skill and verify from a fresh temp project using crates.io dependencies only.
