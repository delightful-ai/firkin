## firkin-benchmark

`firkin-benchmark` owns named benchmark suites, benchmark run orchestration, and
conversion of suite output into evidence artifacts.

This crate is high in the graph and may depend on runtime/single-node surfaces
to run product-like checks. No lower runtime/library crate may depend on it.

Do not put low-level sample primitives here. Use `firkin-trace`. Do not put SLO
schema law here. Use `firkin-evidence`.
