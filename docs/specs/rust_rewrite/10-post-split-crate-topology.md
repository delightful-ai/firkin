# Firkin Post-Split Crate Topology

Status: current topology reference, 2026-05-08.

This document is the crate-boundary anchor for Rust-side Firkin work after the
workspace split. The older split plan in
`docs/plans/2026-05-06-firkin-workspace-crate-split-spec.md` is historical
implementation context; this file is the surface future changes should cite.

## Current Crate Shape

The old coarse crates are intentionally gone:

- There is no `firkin-substrate` crate. Its responsibilities now live in
  `firkin-admission`, `firkin-artifacts`, `firkin-hygiene`,
  `firkin-evidence`, `firkin-template`, and `firkin-benchmark`.
- There is no monolithic `firkin-e2b` crate. Wire DTOs, runtime contracts, and
  local servers are split across `firkin-e2b-wire`, `firkin-e2b-contract`, and
  `firkin-e2b-server`.
- Single-node Apple/VZ backend code lives in `firkin-single-node`, not inside
  `firkin-runtime::single_node`.
- Low-level measurement primitives live in `firkin-trace`; evidence validation
  and benchmark execution are separate crates.

## Dependency Direction

Cargo dependencies should follow this direction:

```text
cli / facade / product servers
  -> benchmark and single-node composition
  -> runtime orchestration
  -> core container and pod mechanics
  -> VMM, vminitd-client, OCI, ext4, vsock
  -> trace and validated types
```

Runtime calls may pass through traits, but crate dependencies stay acyclic and
must pass `scripts/check-firkin-crate-graph.sh`.

## Measurement Ownership

- `firkin-trace`: raw event names, event traces, sample primitives, lifecycle
  and workload labels.
- `firkin-evidence`: metric catalog, derivation laws, scorecards, trust labels,
  promotion blockers, and validation.
- `firkin-benchmark`: benchmark math, suite definitions, and artifact writers.
- `firkin-runtime`, `firkin-single-node`, and `firkin-core`: emit live samples
  at the runtime seams that own the observed work.
- `firkin-cli`: operator commands, reports, baseline save/compare, and
  validation command surfaces.

Do not add generic crates or modules named `common`, `shared`, `utils`,
`helpers`, or `models`. Name the fact or law being owned.

## Product Path Boundaries

The agent-computer product path is `browser + database + CLI`. Shell-only
latency remains useful telemetry, but it cannot promote product readiness,
product density, or autoscale claims.

Product density and prestarted-slot density are separate laws:

- `density.max_agent_computers_before_ready_p95_doubles` includes product-pod
  container add/start and must use `ready_signal=agent_computer_ready_after_container_add`.
- `density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles`
  excludes container add/start and must use
  `ready_signal=request_fifo_acceptance`.

Promotion predicates live in `firkin-evidence`, not in shell scripts or docs.
