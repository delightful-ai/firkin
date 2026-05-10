## firkin-trace

`firkin-trace` is the measurement leaf. It owns `BenchmarkSample`,
`BenchmarkMetricKind`, `BenchmarkUnit`, `Recorder`, spans, samplers,
checkpoints, tags, drain envelopes, and recorder stats.

It must not depend on any Firkin workspace crate.

Do not put these here:

- runtime, VM, OCI, pod, or E2B domain types
- SLO gates or evidence artifact schemas
- benchmark suite policy or workload runners
- filesystem/network/guest metrics collection that requires higher-domain
  clients

Keep hot-path APIs allocation-conscious and bounded. Shared tags belong on the
drain envelope unless a sample-specific tag is genuinely needed.
