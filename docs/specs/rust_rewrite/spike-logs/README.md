# Spike logs

Durable notes + handoff state for each spike in `../02-spike-plan.md`.

Layout:

```
spike-logs/
├── README.md       — this file
└── s<N>-<topic>/
    ├── JOURNAL.md  — running log, events, decisions
    ├── STATUS.md   — current state, repro steps, handoff to other claudes
    └── FINDINGS.md — what we learned, gotchas, reusable patterns
```

Spike **code** lives outside this repo at `~/tmp/rust-rewrite-spikes/s<N>-*/`
per the plan; the notes here survive so future claudes (or future us) can
pick up without re-deriving everything.

Before starting any spike: read [`../SPIKE_RUNBOOK.md`](../SPIKE_RUNBOOK.md)
for the dev loop and [`../PRO_TIPS.md`](../PRO_TIPS.md) for technical
gotchas. Scaffold from [`../spike-template/`](../spike-template/).

## Index

| Spike | Status | Notes |
|---|---|---|
| S1 — Boot empty Linux VM from Rust | ✅ Passed | [s1-boot/](./s1-boot/) |
| S2 — Vsock ↔ tonic transport | ✅ Passed | [s2-vsock-tonic/](./s2-vsock-tonic/) — ~400 µs RTT, VsockConnector pattern |
| S3 — Cross-build vminitd | ✅ Passed | [s3-vminitd-build/](./s3-vminitd-build/) — vminitd on vsock 1024 |
| S4 — End-to-end (pull/rootfs/exec) | ✅ Passed (full) | [s4-e2e/](./s4-e2e/) — busybox echo hello round-trips via inverse-vsock stdio |
| S5 — EXT4 writer | ✅ Passed (T1+T2+partial T3) | [s5-ext4/](./s5-ext4/) — 2709 LOC; e2fsck clean; VM mount validated |
| S6 — Entitlements & codesigning | ✅ Passed | [s6-vmnet-entitlements/](./s6-vmnet-entitlements/) — vmnet shared-mode ad-hoc on macOS 26+; bridged defers to Phase 3 |
| S7 — Rosetta | ✅ Passed | [s7-rosetta/](./s7-rosetta/) — amd64 uname → `x86_64` in arm64 guest |
| S8 — vminitd bundling numbers | ✅ Passed | [s8-bundling-bench/](./s8-bundling-bench/) — embed vminitd ELF (131 MiB), not init.block |
| S9 — vmnet end-to-end reachability | ✅ Passed (full) | [s9-vmnet-reachability/](./s9-vmnet-reachability/) — container reaches `8.8.8.8`; host→container 0.2 ms via vmnet |

## Shared idioms (from the passed spikes)

- **`Box::leak(Box::new(retained))`** before `dispatch_main()` for objects
  that must outlive all callbacks. Acceptable in CLI spikes; not in the
  real library.
- **`dispatch2::dispatch_main()`** is the simplest main-thread pump for a
  VZ CLI. Everything on main queue = no `Send` gymnastics around
  `Retained<VZ*>`.
- **`VzSend<T>`** (`unsafe impl<T> Send for VzSend<T> {}`) is the escape
  hatch when you actually need to cross queue boundaries. Watch out for
  RFC 2229 closure-capture narrowing — force a full capture with
  `let _ = &wrapper;` inside any `move` closure that only touches a field.
- **Ad-hoc codesign + `entitlements.plist` (com.apple.security.virtualization)**
  is enough to run VZ in dev — no paid Apple Developer Program needed for
  NAT networking.
- **`dup()` the `VZVirtioSocketConnection` fd** before handing to tokio;
  VZ's connection owns and auto-closes the original on release.
- **Long-running guests**: set `SPIKE_TIMEOUT_SECS=N` when invoking
  `sign-and-run.sh`; the template's watchdog treats SIGTERM/SIGKILL at
  timeout as success.
- **Guest-initiated vsock (stdio)**: host runs a `VZVirtioSocketListener`
  with a delegate that accepts and dups the fd — mirror image of §13's
  connect path. vminitd's `CreateProcess` stdio ports work this way.
- **ext4 + vsock in one kernel**: use the kata 3.17.0 `vmlinux.container`
  S3 fetched. Ubuntu's `linux-image-virtual` has both as modules and
  needs insmod dances.
- **Idiomatic Rust spikes**: read `beads-rs/docs/philosophy/` first.
  Newtypes, domain-named error variants, no traits unless two
  implementations exist. See S5's findings for a worked example.
