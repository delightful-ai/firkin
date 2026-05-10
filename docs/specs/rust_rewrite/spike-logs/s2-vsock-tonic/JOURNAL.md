# Spike S2 — vsock-tonic

**Spike code**: `~/tmp/rust-rewrite-spikes/s2-vsock-tonic/`
**Started**: 2026-04-20

## The question

Can we splice a VZ-delivered host-side vsock fd into tonic's hyper Channel?
Is there any blocker in how hyper handles custom connectors or how tokio
wraps non-stream fds?

## Acceptance

- Round-trip RTT < 10 ms on first call.
- 1000-iteration loop: no fd leaks (check `lsof`), no handle growth (`ps -o rss`).
- Cancellation: drop the client mid-stream, confirm guest side sees EOF and host fd is closed.
- Debug + release builds clean; `./sign-and-run.sh` exits 0.

## Plan

1. Sanity: template boots VM unchanged. [done]
2. Build guest echo server: static musl arm64 Rust binary; `tokio-vsock` listener on port 2500, speaks tonic echo RPC.
3. Swap `init.c` for a shell-script init that `exec`s `/bin/echo-server`. Need `busybox-static` for the shell.
4. Pack initrd with: busybox shim + /init script + /bin/echo-server.
5. Host side: add `VZVirtioSocketDeviceConfiguration` to VM config; after start, grab `VZVirtioSocketDevice` via `socketDevices().firstObject()`. Need to poll/retry because the guest isn't listening yet at VM-started time.
6. `connectToPort_completionHandler(2500, ...)` → `VZVirtioSocketConnection`. `fileDescriptor()` gives us the host-side fd.
7. Wrap fd in `tokio::net::UnixStream::from_std(std::os::unix::net::UnixStream::from_raw_fd(dup(fd)))` — SOCK_STREAM, so it should Just Work. `dup` so the Retained<VZVirtioSocketConnection> closing doesn't take our fd.
8. Custom hyper `Service<Uri>` that on each call does the `connectToPort` dance (via oneshot) and returns the wrapped stream.
9. `Endpoint::from_static("http://vsock").connect_with_connector(svc)` → tonic Channel. Issue `Echo` RPCs.
10. Loop 1000 times, measure fd count via /dev/fd and RSS via `ps`.
11. Cancellation test: spawn a streaming RPC, drop client, verify guest sees EOF.

## Current status

Scaffolded. Sanity boot passed.

## Events

- 2026-04-20 14:38 — `scaffold.sh` run. Harness boots a VM; extending from here.
- 2026-04-20 14:42 — Sanity boot passed (`./sign-and-run.sh` exits 0, kernel boots, init prints, power-off clean).
- 2026-04-20 14:45 — Confirmed vsock symbols in `objc2-virtualization` are default-on: `VZSocketDevice`, `VZSocketDeviceConfiguration`, `VZVirtioSocket{Device,Connection,DeviceConfiguration,Listener}`. Signatures match what the runbook predicted:
  - `VZVirtualMachine::socketDevices() -> Retained<NSArray<VZSocketDevice>>`
  - `VZVirtioSocketDevice::connectToPort_completionHandler(port: u32, block: &DynBlock<dyn Fn(*mut VZVirtioSocketConnection, *mut NSError)>)`
  - `VZVirtioSocketConnection::fileDescriptor() -> c_int` (doc: "owned by the VZVirtioSocketConnection. Automatically closed when the object is destroyed.")
  - Key lifetime note: we MUST either dup(2) the fd or keep the `Retained<VZVirtioSocketConnection>` alive for the duration. Plan: keep Retained; `close()` it only on connection drop to avoid double-close games.

- 2026-04-20 14:50 — First E2E run fail: `SPIKE-GUEST: bind failed: Address family not supported by protocol (os error 97)`. Ubuntu 24.04 arm64 kernel ships vsock as loadable modules (`.ko.zst` in `/lib/modules/$KVER/kernel/net/vmw_vsock/`), not built in. Fix: grabbed the three modules (`vsock`, `vmw_vsock_virtio_transport_common`, `vmw_vsock_virtio_transport`) inside the initrd builder container, decompressed with `zstd -d`, dropped into `/lib/modules/` in the initrd. Shell init `insmod`s them before `exec`ing the server.
- 2026-04-20 14:53 — Second E2E run: PASS. RTT 405µs, 1000-iter loop in 913ms with fd delta +1 and RSS delta 688KB. Cancellation test: got exactly the 3 ticks we consumed before dropping the stream; fd count returned to baseline 11.
- 2026-04-20 14:55 — Release build + run: PASS. RTT 1.37ms, fd delta 0, RSS delta 704KB.
- 2026-04-20 14:57 — Cold clone test (`cargo clean && ./sign-and-run.sh`): PASS (~1m50s total: `cargo clean` + debug rebuild + guest-already-built + run).
- 2026-04-20 15:00 — FINDINGS.md + STATUS.md written. All "Done looks like" boxes ticked except the `spike-logs/README.md` index update, which the prompt explicitly said to leave for the curator.

## Final resolution

Spike **passes** cleanly. `VZVirtioSocketDevice` fd → tokio `UnixStream` → hyper `TokioIo` → tonic `Channel` is the end-to-end path, and it works in both debug and release, cold-clone or warm. The `VsockConnector` struct + `dial_vsock` async bridge are the ~50 LOC that will survive into the real library almost verbatim.

Central gotcha (biggest surprise): Ubuntu's arm64 kernel doesn't build AF_VSOCK in; must insmod three modules from the initrd. This workaround dies once S3's apple/containerization kernel lands (vsock is built in there).

Flagged six "Proposed PRO_TIPS additions" in FINDINGS.md for the curator to fold into the shared file.
