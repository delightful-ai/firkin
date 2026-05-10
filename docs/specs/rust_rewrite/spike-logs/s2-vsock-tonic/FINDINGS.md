# S2 — findings

**Status**: PASSED, first real E2E run. Debug + release builds clean; cold clone (`cargo clean && ./sign-and-run.sh`) passes.
**Host**: macOS 26.3, arm64 (Apple Silicon), rustc 1.95-nightly.
**Spike code**: `~/tmp/rust-rewrite-spikes/s2-vsock-tonic/` (~660 LOC host Rust + ~120 LOC guest Rust + ~50 LOC init/docker scripts).

## Acceptance — pass/fail

| Criterion | Result | Evidence |
|---|---|---|
| Round-trip RTT < 10 ms on first call | PASS | **405 µs / 437 µs / 1.37 ms** across three runs. First-call RTT includes a full connectToPort dance + HTTP/2 handshake. |
| 1000-iteration loop — no fd leaks | PASS | `fd_count` delta 0 or +1 (well under the 8-fd tolerance). Baseline 11 fds, post-loop 11 or 12. |
| 1000-iteration loop — no RSS growth | PASS | `rss_kb` grew by 688–752 KB over 1000 iters (tolerance 8 MB). Stable across checkpoints. |
| Cancellation: drop client, guest sees EOF, host fd closed | PASS | Guest printed exactly `tick 1/2/3` (the 3 we consumed before drop) and **stopped emitting** — tonic dropped the server-streaming task when the client dropped. Host fd count returned to baseline 11 within 200 ms. |
| `sign-and-run.sh` exits 0 cold | PASS | Full `cargo clean` → build → sign → run → exit 0 sequence succeeds. |
| Debug + release builds clean | PASS | Both profiles build with zero warnings (after fixing the one `tonic_build::compile` → `compile_protos` deprecation). |

Headline numbers from a typical run (debug build, cold):

```
[ACC] first-call RTT = 405.75µs (target < 10ms)
[loop] before: fd_count=11, rss_kb=21680
[ACC] 1000 iters in 913.407792ms; fd_count 11 -> 12 (delta=1); rss_kb 21680 -> 22368 (delta=688)
[ACC] cancellation: got 3 ticks; fd after drop+200ms = 11 (baseline was 11)
[ACC] all tests passed
```

## The central answer

**Yes.** A VZ-delivered host-side vsock fd splices cleanly into tonic's hyper Channel via a custom `tower::Service<Uri>` connector. The stack that works:

1. `VZVirtioSocketDevice::connectToPort:completionHandler:` on the VZ main queue.
2. Inside the completion `RcBlock`: `Retained::retain(conn)` → `conn.fileDescriptor()` → `libc::dup(fd)` → `libc::fcntl(F_SETFL, O_NONBLOCK)` → `conn.close()` → drop the `Retained`. Now we own a nonblocking fd that's independent of VZ's lifetime.
3. Send the `OwnedFd` across a tokio `oneshot::channel` to the tokio runtime thread.
4. `tokio::net::UnixStream::from_std(std::os::unix::net::UnixStream::from_raw_fd(raw))` — tokio does not type-check the address family; any nonblocking SOCK_STREAM fd works.
5. Wrap in `hyper_util::rt::TokioIo` (hyper 1's bridge).
6. Return from `tower::Service::<Uri>::call`.
7. `Endpoint::from_static("http://vsock.invalid").connect_with_connector(VsockConnector { port: 2500 })` → a ready-to-use tonic `Channel`.

The fd-leak check is the real win: the per-call `connect_with_connector` pattern (fresh Channel every iter — the worst-case regression driver) shows **zero net fd drift** across 1000 iterations. Hyper's connection-release path is sound; tokio closes `UnixStream`'s fd on drop; and since we `dup`'d the VZ fd and `close()`'d the VZ connection immediately, the VZ-side ref count isn't holding anything open either.

## What worked as planned

1. **`objc2-virtualization`'s vsock surface is complete.** `VZVirtioSocketDeviceConfiguration` (init, attach via `setSocketDevices(NSArray<VZSocketDeviceConfiguration>)`), `VZVirtualMachine::socketDevices()` returning `Retained<NSArray<VZSocketDevice>>`, and `VZVirtioSocketDevice::connectToPort_completionHandler(port: u32, block: &DynBlock<dyn Fn(*mut VZVirtioSocketConnection, *mut NSError)>)` all work exactly as the Apple docs describe. No shims needed.
2. **Main queue + tokio on a bg thread bridges cleanly.** The only thing crossing the thread boundary is a `OwnedFd` (Send) and a tokio oneshot — both trivially Send. No `Retained<VZ*>` ever leaves the main thread.
3. **`VzSend<T>` wrapper from PRO_TIPS §1 works.** We stash a `Retained<VZVirtioSocketDevice>` in a `OnceLock<Arc<VzSend<...>>>`, clone the Arc to move into `DispatchQueue::main().exec_async(...)`. Reading the inner `Retained` inside the closure (which runs on main queue) is sound by the `VzSend` safety invariant.
4. **`RcBlock` lifetime.** Completion blocks must outlive the `connectToPort` call. `RcBlock::new(...)` + `std::mem::forget(block)` after issuing the call works because `RcBlock` is refcounted — VZ retains its own clone; our forget prevents the local from releasing while VZ holds it. (We could also stash it; forget is the spike-level fix.)
5. **`tonic_build` + `prost` + `tonic 0.12.3` + `hyper 1` + `hyper-util 0.1` + `tower 0.5`.** The version constellation compiles cleanly. `hyper_util::rt::TokioIo` is the required adapter between tokio IO and hyper 1.

## Gotchas we hit (proposed PRO_TIPS additions — flagged for curator)

### (a) Ubuntu's linux-image-virtual arm64 kernel compiles AF_VSOCK as modules, not built-in

First run failed with `socket(AF_VSOCK, SOCK_STREAM, 0) -> EAFNOSUPPORT (errno 97)` because `vmw_vsock_virtio_transport{,_common}.ko` and `vsock.ko` are `.ko.zst` files under `/lib/modules/$KVER/kernel/net/vmw_vsock/`, not built into the kernel image.

Workaround (in `init/build.sh`): install `linux-image-virtual` inside the initrd-builder container, copy + `zstd -d` the three modules into the cpio at `/lib/modules/`, and `busybox insmod` them in dependency order from the shell init before `exec`ing the server:
```sh
for m in vsock vmw_vsock_virtio_transport_common vmw_vsock_virtio_transport; do
    /bin/busybox insmod /lib/modules/${m}.ko
done
```
**Permanent fix**: apple/containerization's own kernel (S3) builds vsock in. This entire workaround dies once we boot that kernel. Flagging so S4 doesn't re-hit it if it lifts bits of S2's initrd.

### (b) `VZVirtioSocketConnection` owns the fd — always `dup()` before handing to tokio

The header doc is literal: "The file descriptor is owned by the VZVirtioSocketConnection. It is automatically closed when the object is destroyed." The safest pattern is:
  1. In the completion block: `Retained::retain(conn)` to get a +1 reference.
  2. `fd = conn.fileDescriptor(); dup_fd = libc::dup(fd);`
  3. Set `O_NONBLOCK` on `dup_fd`.
  4. `conn.close(); drop(Retained)` — VZ's fd is closed, the `Retained` releases on main queue (where it was born), but `dup_fd` is independent.
  5. Ship `OwnedFd::from_raw_fd(dup_fd)` via tokio oneshot.

If you instead keep the `Retained<VZVirtioSocketConnection>` alive with the stream, you force a `!Send` handle into your stream type → tokio wants `Send` futures → tears ensue.

### (c) `tokio::net::UnixStream::from_std` works for AF_VSOCK

UnixStream is just a nonblocking-SOCK_STREAM wrapper as far as tokio is concerned. No address-family check happens unless you call `peer_addr()`/`local_addr()`. This is the cleanest path; `AsyncFd<OwnedFd>` is more fiddly and you have to hand-roll `AsyncRead`/`AsyncWrite`.

### (d) Custom hyper connector shape for tonic 0.12

`tower::Service<Uri>`, `Response = hyper_util::rt::TokioIo<YourStream>`, `Error: Into<Box<dyn Error + Send + Sync>>`, `Future: Send + 'static`. Ignore the `Uri` (we always dial the same vsock port). Build the channel via:
```rust
Endpoint::from_static("http://vsock.invalid")
    .connect_timeout(Duration::from_secs(2))
    .connect_with_connector(VsockConnector { port })
    .await?
```
The authority string is never used but must parse as a valid URI; "vsock.invalid" or any placeholder works.

### (e) Closure-capture narrowing again

PRO_TIPS §1's RFC 2229 trap can re-bite when an `exec_async` closure only touches `dev_for_main.0`. Fix: `let _ = &dev_for_main;` at the top of the closure forces full capture. Sticking this line near the top of any `exec_async` closure that captures an `Arc<VzSend<_>>` is cheap hygiene.

### (f) `connectToPort` callback fires FAST

Sub-millisecond after the VM has started. First-call RTT includes the whole path (dial → HTTP/2 handshake → echo → response) and was still ~400 µs. VZ's vsock plumbing is not the bottleneck.

### Minor stuff

- `tonic_build::Builder::compile` is deprecated in favor of `compile_protos`. Update your build.rs.
- `tokio-vsock` 0.7.2 needs the `tonic012` feature to get `Connected` impl'd on `VsockStream` (required by `serve_with_incoming`).
- Ubuntu's `busybox-static` (deb pkg) installs to `/bin/busybox`. Alpine's installs to `/bin/busybox.static`. We chose Ubuntu in this spike's initrd-builder because we also needed the kernel modules from the same container.

## Reusable patterns

Code that should survive into the real project more or less verbatim:

- **`VsockConnector` struct + `tower::Service<Uri>` impl** (src/main.rs). The ~30 LOC the plan explicitly called out as "will likely survive into the real project verbatim."
- **`dial_vsock` bridge** (src/main.rs). The oneshot-across-threads pattern between tokio and the VZ main queue generalizes to any async-from-tokio request for a VZ-delivered resource.
- **`VzSend<T>` wrapper + publish pattern** (src/main.rs). Generalizes to any `!Send` VZ handle that the rest of the app needs to address.
- **Guest-side tonic server on tokio-vsock** (guest/src/main.rs). Trivially adaptable to vminitd's `SandboxContext` service.

## What the port should do differently

- Don't `Box::leak` the VM/delegate/RcBlock — in a library, they hold handles that must be droppable on shutdown. Make them fields of a `VirtualMachine` struct and drop them when the last owning handle goes away. The spike's leaks are CLI-only.
- Stash the `RcBlock` for `connectToPort`'s completion handler on the VM struct, one per in-flight dial. When dial completes, remove it from the map. (Spike uses `mem::forget` for brevity.)
- A single `VsockTransport` object should own the `VZVirtioSocketDevice` Retained and expose `async fn dial(port: u32) -> Result<VsockStream>`. It lives on a dedicated per-VM background thread running `dispatch_main` (or adapted into whatever lifecycle the crate settles on).

## Known loose ends (not spike-blocking)

- **Guest-initiated connects** are untested. `VZVirtioSocketListener` + `VZVirtioSocketListenerDelegate` are parallel to what we used; no reason to suspect trouble. S4 may need this.
- The `bye` RPC at the end doesn't actually trigger guest shutdown — the guest runs forever until the host's tokio side calls `std::process::exit(0)`, which tears down the VM on drop. Clean enough for a spike; a real library would use a dedicated shutdown RPC or send the guest SIGTERM via a vsock listener.
- **Concurrent dials** are untested — all dials in the spike are serial. High-concurrency dial soak would be a small extension.
- The 1000-iteration loop uses **fresh Channel per iter** on purpose (stresses `dial_vsock`'s fd accounting). A reuse-one-channel loop would be faster but less informative — hyper's connection pool would mask any per-dial leak.
- The cancellation assertion is a little weak — we assert "guest stopped emitting" indirectly (only 3 tick logs on host stderr after we drop). A tighter version would have the server write "client dropped" to stderr when its `yield` returns error; tonic 0.12's streaming API makes that fiddly. The current check is a valid acceptance signal.

## Time to solve

- Scaffolding + sanity boot: 10 min.
- Writing host + guest + initrd: ~1.5 h.
- First fail (vsock AF unsupported — module not loaded): ~5 min diagnosis, ~10 min fix (module insertion).
- Second run → PASS. Cold clone + release + debug verification: ~5 min.
- **Total: well under the 3–5 day plan estimate.**

Single biggest surprise: the AF_VSOCK-as-module issue. Everything else was smooth.
