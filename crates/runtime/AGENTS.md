## firkin-runtime

`firkin-runtime` is the reusable runtime orchestration crate. It composes
`core`, `template`, `admission`, `artifacts`, `hygiene`, `trace`, and E2B
compatibility contracts into snapshot restore, template build, warm-pool,
preflight, session, continuation, and adapter operations.

Do not put these here:

- `firkin-single-node` state stores, schedulers, or Apple/VZ backend services
- benchmark suite runners or evidence validation
- E2B wire DTO definitions
- Hyper route/server ownership that belongs in `firkin-e2b-server`

Runtime code may depend on compatibility contracts where it implements a
runtime-facing trait. It must not become the product control plane.
