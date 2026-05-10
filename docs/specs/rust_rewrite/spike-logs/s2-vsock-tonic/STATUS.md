# S2 — status snapshot

Last updated: 2026-04-20

## State
- 🟢 **Passed.** All acceptance criteria met (see FINDINGS.md).

## Repro

```bash
cd ~/tmp/rust-rewrite-spikes/s2-vsock-tonic
./sign-and-run.sh              # debug; builds, signs, runs, exits 0
PROFILE=release ./sign-and-run.sh

# From a cold clone:
cargo clean && ./sign-and-run.sh
```

Expected output tail:
```
[host] VM started
SPIKE-INIT: handing off to echo-server
SPIKE-GUEST: echo-server starting
SPIKE-GUEST: bound vsock port 2500
[tokio] channel up
[ACC] first-call RTT = ~400µs (target < 10ms)
[loop] before: fd_count=11, rss_kb=~21000
[ACC] 1000 iters in ~1s; fd_count 11 -> 11/12; rss_kb delta <1MB
[ACC] cancellation: got 3 ticks; fd after drop+200ms = 11 (baseline was 11)
[ACC] all tests passed
```

## Assets
- `~/tmp/rust-rewrite-spikes/s2-vsock-tonic/assets/vmlinux` — symlink to s1-boot's kernel.
- `~/tmp/rust-rewrite-spikes/s2-vsock-tonic/assets/initrd.cpio` — ~3.1 MB; contains busybox-static shell init, three vsock kernel modules (.ko), and the guest `echo-server` binary.
- `~/tmp/rust-rewrite-spikes/s2-vsock-tonic/assets/echo-server` — 1.1 MB static arm64 Rust binary (tonic echo on AF_VSOCK port 2500).
- `entitlements.plist` — same as S1 (`com.apple.security.virtualization`).

## Key code paths (will survive into the library)

- `src/main.rs::VsockConnector` — the `tower::Service<Uri>` hyper connector. Thin wrapper calling `dial_vsock`, wrapping in `hyper_util::rt::TokioIo`.
- `src/main.rs::dial_vsock` — the tokio-to-main-queue bridge. oneshot-based.
- `src/main.rs::VzSend<T>` — the !Send -> Send adapter used to stash `Retained<VZVirtioSocketDevice>` in a static.
- `guest/src/main.rs` — the tokio-vsock + tonic echo server. Template for vminitd's `SandboxContext` server.
- `init/build.sh` — initrd builder; specifically the kernel-module injection (needed only until S3 kernel is available).

## Done checklist

- [x] Acceptance criteria met (quoted in FINDINGS.md)
- [x] `sign-and-run.sh` exits 0 cold
- [x] Debug + release builds clean
- [x] JOURNAL.md has a final resolution entry
- [x] FINDINGS.md written
- [x] State line reads "🟢 Passed"
- [ ] spike-logs/README.md index updated (out of scope per prompt; curator merges)
- [x] Any PRO_TIPS.md additions flagged (in FINDINGS.md "Proposed PRO_TIPS additions" section)

## Handoff to other claudes

### S3 (cross-build vminitd + boot it)
- Unaffected by S2. Can proceed in parallel (was already in flight per task prompt).

### S4 (end-to-end: pull image, hand-build rootfs, exec container process)
- **Lift directly**: `VsockConnector` and `dial_vsock` from `src/main.rs`. These are the nearly-real-project shape of the vsock transport.
- **Lift concept**: the `VzSend<T>` wrapper + `OnceLock` publish pattern for making !Send VZ handles reachable from tokio.
- **Skip**: the initrd kernel-module injection hack — S3's kernel has vsock built in, so init just `exec`s vminitd directly.
- **Gotcha (carry forward)**: always `dup()` the fd out of `VZVirtioSocketConnection` before handing to tokio; drop the Retained on main queue immediately after. Documented in FINDINGS.md §(b).

### S6 (entitlements)
- No change from S1. Ad-hoc sign with `com.apple.security.virtualization` works; adding a vsock device does not require additional entitlements.

### Proposed PRO_TIPS.md merges (for curator)
Six items, all in FINDINGS.md under "Gotchas we hit". Priority order:
1. `VZVirtioSocketConnection` fd ownership + `dup()` pattern (§(b)) — this is the single most easily-gotten-wrong thing about VZ vsock.
2. Custom hyper connector shape for tonic 0.12 (§(d)) — copy-pasteable.
3. `tokio::net::UnixStream::from_std` for AF_VSOCK fds (§(c)) — non-obvious.
4. RFC 2229 reminder specific to exec_async (§(e)) — reinforces existing PRO_TIPS §1.
5. `tonic_build::compile` deprecation + `tokio-vsock` `tonic012` feature (minor) — quick one-liners.
6. AF_VSOCK-as-module in Ubuntu kernel (§(a)) — goes away once S3 kernel lands; probably worth a footnote rather than full entry.

## Open questions NOT answered by S2
- Guest-initiated vsock connects (`VZVirtioSocketListener` + delegate). S4 may need this; not scary, just untested.
- Concurrent dials (we only tested serial).
- Behavior under long-lived Channels with many streams (we tested many short channels). Hyper's connection pooling should make long-lived channels easier, not harder.
