## Firkin Crate Topology

This directory is governed by the post-split crate graph in
`../docs/specs/rust_rewrite/10-post-split-crate-topology.md`.

Do not resurrect deleted compatibility crates:

- no `crates/substrate`
- no `crates/e2b`
- no `runtime/src/single_node`

## Boundary Rules

- `firkin-types` and `firkin-trace` are leaf-like. They must not depend on any
  Firkin workspace crate.
- `firkin-benchmark` is high in the graph. Runtime/library crates must never
  depend on it.
- `firkin-evidence` validates claims and artifacts. It does not run benchmark
  suites or emit lifecycle spans.
- `firkin-runtime` composes runtime operations. It must not import
  `firkin-single-node`, `firkin-evidence`, or `firkin-benchmark`.
- `firkin-single-node` owns one-host Apple/VZ backend composition. Lower crates
  must not depend on it.
- `firkin-e2b-wire` is DTO-only. `firkin-e2b-contract` owns runtime-facing
  protocol traits and compatibility request contracts. `firkin-e2b-server` owns
  local Hyper/envd/domain-proxy servers. Do not collapse them.
- `firkin-core` owns VM-backed container and pod mechanics. It must not own
  admission, benchmark policy, evidence, single-node state, or E2B server
  behavior.

Run `scripts/check-firkin-crate-graph.sh` after dependency changes.

## Public API Discipline

Every crate root should be a map. New implementation belongs in named modules.
Expose only entry points and decision products. Prefer `pub(crate)` until a
downstream crate proves the boundary needs a public item.
