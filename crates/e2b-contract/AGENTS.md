## firkin-e2b-contract

`firkin-e2b-contract` owns the runtime-facing contracts needed by the local E2B
compatibility layer: capability reporting, envd process/filesystem adapter
traits, port targets, runtime adapter traits, and template/runtime request
contracts.

Traits here are law surfaces for compatibility. Do not add a method because one
implementation wants a shortcut. Split a capability or move the trait to the
consumer if the law changes.

Do not import `firkin-runtime`, `firkin-single-node`, `firkin-core`,
`firkin-vmm`, `firkin-e2b-server`, evidence, or benchmark crates.
