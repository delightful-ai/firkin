## firkin-core

`firkin-core` owns the user-facing VM-backed container and pod execution
mechanics: builders, rootfs staging, vminitd request lowering, process I/O,
pod store/rootfs materialization, and container snapshots.

Do not put these here:

- production admission or warm-pool policy
- benchmark suite or evidence policy
- single-node durable state
- E2B/Cube HTTP or envd server behavior
- Apple/VZ operator service lifecycle

`firkin-core` may emit trace samples only for phases it directly owns. It must
not validate evidence or decide benchmark completeness.
