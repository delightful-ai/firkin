## firkin-single-node

`firkin-single-node` owns one-host Apple/VZ backend composition: local runtime
configuration, scheduling, state persistence, active session records, domain
proxy adapter wiring, and the `SingleNodeBackend` facade.

This is a high-level crate. Lower crates must not depend on it.

Do not put these here:

- low-level VM/container mechanics already owned by `firkin-core` or
  `firkin-vmm`
- generic runtime orchestration that belongs in `firkin-runtime`
- evidence/SLO schemas or benchmark-suite definitions
- product SaaS routing, auth, tenancy, billing, or OpenAPI concerns

If a new operation is useful outside one local host, define the lower runtime
trait or model in `firkin-runtime` or another law-owning crate first, then adapt
it here.
