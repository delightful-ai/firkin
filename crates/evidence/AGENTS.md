## firkin-evidence

`firkin-evidence` owns benchmark summaries, lifecycle and overhead evidence
artifacts, SLO gates, and soak proof schemas.

It consumes samples from `firkin-trace`. It does not emit lifecycle spans, run
benchmarks, operate VMs, or own runtime policy.

Do not depend on `firkin-runtime`, `firkin-single-node`, `firkin-template`,
`firkin-admission`, `firkin-artifacts`, `firkin-hygiene`, or E2B crates. If an
artifact needs a new fact, lower it to a trace sample or pass it in through an
evidence DTO.
