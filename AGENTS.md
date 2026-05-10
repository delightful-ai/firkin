# Firkin Agent Instructions

## Communication

Be direct and concrete. State the observable behavior or artifact that will
prove the work is done before making non-trivial changes.

## Repository Boundary

This is the standalone `delightful-ai/firkin` Rust repository. Do not edit
`/Users/darin/vendor/github.com/apple/containerization` while working here.
That checkout is source material and prior art only.

The repo intentionally excludes Apple's Swift `Containerization` package and
service implementation. If a task appears to need Swift source, first ask
whether the requirement belongs in Firkin or in the upstream reference checkout.

## Version Control

Use `jj` when it is initialized in this checkout. Commit frequently with rich
messages that explain:

- why this change was made;
- what obvious alternative was rejected and why;
- what remains alpha/scaffolded;
- which files, checks, or issue IDs anchor the decision.

History rewrites are allowed for this standalone repo when the user asks for a
clean publishable history. Never rewrite or mutate the Apple Containerization
checkout.

## Rust Topology

Keep crate roots as maps: module declarations, public re-exports, and facade
glue only. Put implementation in named modules.

Do not introduce crates or modules named `common`, `shared`, `utils`,
`helpers`, or generic `models`. Name the concept that owns the knowledge.

Default private. Widen visibility only when a real crate boundary requires it.

Current topology reference:
`docs/specs/rust_rewrite/10-post-split-crate-topology.md`.

Important boundaries:

- `firkin-types` and `firkin-trace` stay leaf-like.
- `firkin-runtime` must not depend on `firkin-single-node`,
  `firkin-evidence`, or `firkin-benchmark`.
- `firkin-single-node` owns one-host Apple/VZ backend composition.
- `firkin-e2b-wire`, `firkin-e2b-contract`, and `firkin-e2b-server` stay split.
- `firkin-core` owns VM-backed container and pod mechanics, not benchmark,
  admission, evidence, or E2B server behavior.

Run `scripts/check-firkin-crate-graph.sh` after dependency changes.

## Verification

Prefer the smallest useful check first, then broaden:

```bash
cargo metadata --format-version 1
cargo fmt --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/check-firkin-crate-graph.sh
git diff --check
```

Live Apple/VZ tests are not proof unless the binaries were signed and the
runtime artifact inputs are known. Use `Justfile` targets for live proofs.

## Publishing

`0.0.1-alpha` is unstable. Do not run `cargo publish`, create public releases,
or make the GitHub repo public without explicit user approval.

Publishing work should update `docs/publishing.md` with exact dry-run status
and remaining manual steps.
