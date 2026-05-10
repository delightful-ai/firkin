# Spike S4 — e2e

**Spike code**: `~/tmp/rust-rewrite-spikes/s4-e2e/`
**Started**: 2026-04-20

## The question

"With vminitd working, can we drive a real `CreateProcess`/`WaitProcess`
cycle and get stdout back?" Full lifecycle:
`Mount → WriteFile(config.json) → CreateProcess → StartProcess →
stream stdout → WaitProcess → exit 0`, and get `echo hello` to return "hello\n".

## Acceptance

- vminitd reports process exit code 0.
- Host sees `hello\n` streamed back on stdout.
- `sign-and-run.sh` exits 0.

Bar hierarchy per handoff:
1. Stretch minimum — one RPC round-trips (vsock→vminitd plumbing works).
2. Target — Mount + WriteFile + CreateProcess succeed.
3. Full — `echo hello` round-trips stdout.

## Plan

1. Reuse S3 assets (kata kernel + init.block).
2. Lift S2's `VsockConnector`/`dial_vsock` and S3's block-device wiring
   into a new harness that attaches `init.block` + eventually a container
   rootfs.ext4.
3. `build.rs` via `tonic-build` from the vendored `SandboxContext.proto`.
4. Once vsock→vminitd is green (stretch), add container rootfs and the
   RPC sequence.

## Key proto surprise

`SandboxContext.proto` stdio is NOT a streaming RPC — `CreateProcessRequest`
carries `stdin/stdout/stderr` as **vsock ports** (uint32). The guest dials
**back** to the host on `VsockType.hostCID` (= 2). So full-acceptance
requires a host-side VZVirtioSocketListener on chosen ports, not just
host-initiated connects. That's strictly more work than S2's one-way
connect. Capture the time bar early; stretch+target don't need this.

## Events

- 2026-04-20 15:20 — `scaffold.sh` run. Harness boots a VM.
- 2026-04-20 15:20 — Read SandboxContext.proto + Server+GRPC.swift.
  Discovered stdio is vsock-port-based (guest dials host), not stream RPC.
  Target: swap kernel to kata (vsock built in), add init.block disk,
  dial vminitd port 1024, call a lightweight RPC (Sync) as smoke test
  first. Then escalate.
- 2026-04-20 15:23 — Staged S3 assets: init.block (384 MiB), vmlinux
  (kata 3.17.0, 14 MiB, vsock built in) → `assets/`.
- 2026-04-20 15:24 — Added `tonic-build` / `prost` build.rs; dropped the proto
  verbatim into `proto/SandboxContext.proto`. Package is
  `com.apple.containerization.sandbox.v3` → rust mod
  `com::apple::containerization::sandbox::v3`.
- 2026-04-20 15:25 — First compile green.
- 2026-04-20 15:27 — Stretch minimum GREEN: `Sync` RPC round-tripped in
  ~10 ms from VM boot → first gRPC response. Also verified `Getenv(PATH)`
  (returned `None`, which is correct — vminitd doesn't inherit env from
  the kernel boot path) and `ContainerStatistics` (empty list).
- 2026-04-20 15:28 — Built container rootfs.ext4 (64 MiB, e2fsck clean)
  via `docker run --platform linux/arm64 alpine:3.20` with `mkfs.ext4 -d`.
  Tree: busybox-static at /bin/busybox + symlinks for echo/sh/ls/cat/sleep,
  /etc/passwd, /etc/group.
- 2026-04-20 15:30 — First `CreateProcess` call: OCI spec decode failed on
  `Key 'path' not found at linux.namespaces[0]`. apple/containerization's
  `LinuxNamespace` Codable requires both `type` and `path` (non-optional).
  Added `"path": ""` to each namespace entry.
- 2026-04-20 15:31 — CreateProcess now: "The volume is read only"
  (NSCocoaErrorDomain 642). Cause: `ociAlterations` writes
  `/etc/hostname` into rootfs. Fixed by switching rootfs disk attach from
  readonly=true → false AND mount option "ro" → "rw" AND
  `root.readonly=false` in spec.
- 2026-04-20 15:33 — Caught second gotcha: vminitd's `ManagedContainer`
  uses `craftBundlePath(id) → /run/container/<id>` — NOT the bundle
  path I pass. `Bundle.create` mkdir-p's `<bundle>/rootfs`. So the right
  mount target is `/run/container/container-0/rootfs`, not my arbitrary
  `/run/container-0/rootfs`.
- 2026-04-20 15:35 — CreateProcess OK, StartProcess fails with
  "vmexec error: internalError: \" Error Domain=NSPOSIXErrorDomain Code=2
  "No such file or directory"\"". The stage field in POSIXError.userInfo
  isn't in `String(describing:)`, so the error is ~useless for
  localization. Guesses: `/dev/null` (reOpenDevNull), `/dev/ptmx`
  (configureConsole), or `execvpe(/bin/echo)` in pivoted rootfs.
- 2026-04-20 15:37 — Tried tmpfs+devpts+bind-mounts for individual /dev
  nodes. No change. Realized `configureConsole` targets
  `rootfs + "/dev/ptmx"` as an absolute path — after tmpfs overlays /dev,
  that path is inside the tmpfs, which is empty. Switched approach: single
  bind-mount of vminitd's `/dev` (devtmpfs provided by guest kernel) onto
  the container `/dev`. Now everything the container needs (null, zero,
  ptmx, random, urandom, tty) is present as real char devices.
- 2026-04-20 15:38 — **TARGET + FULL-EXCEPT-STDOUT GREEN.** Full pipeline
  works: Mkdir, Mount /dev/vdb, WriteFile, CreateProcess, StartProcess
  (pid=80), WaitProcess returned exit=0. echo hello ran and exited 0.
  Without stdio listeners we can't assert on the bytes yet.
- 2026-04-20 15:42 — Added VZVirtioSocketListener + delegate plumbing
  to accept guest-initiated vsock connections on ports 10000 (stdout) and
  10001 (stderr). `StandardIO.start()` on the guest dials hostCID:<port>
  per fd. Delegate dup()s the fd and publishes it via
  `Mutex<Option<OwnedFd>>` + a `STDIO_WAKER` Mutex<Vec<Waker>>. Tokio side
  polls.
- 2026-04-20 15:45 — **FULL ACCEPTANCE GREEN.** stdout read 6 bytes =
  `"hello\n"`. Exit 0. Both debug and release build + run end-to-end.
  Time from `cargo clean` → first RPC: ~11s build + ~5s VM boot (mostly
  kernel waiting for "random: crng init done") + ~10ms first gRPC.

## Resolution

The vertical plumbing works end-to-end. Five RPCs round-trip cleanly,
a container boots inside vminitd's ManagedProcess/vmexec flow, and
stdout flows back via VZVirtioSocketListener. Scariest-question
answered: **yes**, the stack composes.

Code at `~/tmp/rust-rewrite-spikes/s4-e2e/`, ~580 LOC Rust including
the listener delegate. Lifted ~120 LOC from S2 (dial_vsock +
VsockConnector) and ~40 LOC from S3 (block device wiring) verbatim.
The spike-specific code is ~420 LOC, mostly OCI spec JSON + RPC
sequence.
