# Firkin Docs

Start here when evaluating the standalone Firkin repository.

- [`specs/rust_rewrite/10-post-split-crate-topology.md`](specs/rust_rewrite/10-post-split-crate-topology.md)
  is the current crate-boundary contract.
- [`plans/2026-05-07-firkin-sandbox-library-surface-spec.md`](plans/2026-05-07-firkin-sandbox-library-surface-spec.md)
  captures the public sandbox surface direction.
- [`specs/rust_rewrite/04-library-surface/README.md`](specs/rust_rewrite/04-library-surface/README.md)
  records the Rust-first API design notes.
- [`artifacts/`](artifacts/) keeps durable benchmark/proof summaries.
- [`publishing.md`](publishing.md) is the manual crates.io/GitHub release
  runbook.
- [`.agents/skills/firkin-release`](../.agents/skills/firkin-release/SKILL.md)
  captures the release workflow for future agents.
- [`.agents/skills/firkin-external-consumer`](../.agents/skills/firkin-external-consumer/SKILL.md)
  captures the crates.io-only consumer smoke.
- [`.agents/skills/firkin-profiling`](../.agents/skills/firkin-profiling/SKILL.md)
  captures the benchmark/profiling loop.

Some historical documents still reference Apple's Swift Containerization
sources because they are provenance and prior-art notes. The standalone repo
does not contain those Swift sources.
