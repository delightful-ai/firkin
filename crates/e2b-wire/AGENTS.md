## firkin-e2b-wire

`firkin-e2b-wire` owns E2B/Cube-compatible request and response DTOs only.

It must not depend on runtime, VM, core, server, evidence, benchmark, or trace
crates. Keep it humble at the edge: preserve SDK JSON shape and tolerate
foreign request evolution where the protocol requires it.
