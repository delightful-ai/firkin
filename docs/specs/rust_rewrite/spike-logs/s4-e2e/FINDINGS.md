# S4 — findings

Result: **🟢 FULL acceptance.** `echo hello` inside a real container
rootfs round-trips stdout to the host. ~580 LOC Rust (of which ~160
lifted verbatim from S2/S3).

## What worked exactly as expected

- **S2's `dial_vsock` + `VsockConnector`**: copy-pasted, worked first
  try against vminitd on port 1024. First-call RTT ~10ms (matches S2's
  numbers).
- **S3's block-device wiring**: copy-pasted, extended to two disks
  (init.block at /dev/vda; rootfs.ext4 at /dev/vdb) with no surprises.
- **`tonic-build`**: one-liner build.rs, generated clean client stubs.
  The proto's `google.protobuf.Timestamp` satisfied via `prost-types`
  with no config.
- **Watchdog bounding** from S3's `sign-and-run.sh`: no change needed;
  `SPIKE_TIMEOUT_SECS=30` sufficient — container exits ~700 ms after
  start, but vminitd keeps running forever so the watchdog is essential.
- **kata kernel**: the 14 MiB kata 3.17.0 `vmlinux.container` from S3
  worked unchanged. vsock built in → connects from first gRPC attempt.

## Gotchas (**proposed PRO_TIPS additions** at the bottom)

### 1. Stdio is vsock-back, not stream RPC

This was the biggest time sink on reading. The proto has
`CreateProcessRequest { stdin: optional uint32, stdout: optional uint32,
stderr: optional uint32 }` where the uint32 is a **vsock port**. vminitd
then calls `Socket.connect(hostCID:<port>)` from the guest side
(`StandardIO.start()`), meaning **the host must be listening**. This
requires `VZVirtioSocketListener` + delegate (symmetric to `dial_vsock`
but host-as-acceptor).

The delegate returns `bool` — return `true` to accept; the fd on the
`VZVirtioSocketConnection` is again owned by VZ, so `dup()` it and
immediately drop the `Retained<>` (same pattern as S2's connect path).

### 2. vminitd ignores the bundle path you give it

`CreateProcess` takes a container ID; vminitd computes its own bundle
path as `/run/container/<id>` (`ManagedContainer.craftBundlePath`) and
`Bundle.create` mkdir-p's `<bundle>/rootfs`. **This means you have to
mount your container rootfs at `/run/container/<id>/rootfs`**, not
wherever you wanted. WriteFile(config.json) to the "bundle" you
chose is ignored because Bundle.create writes its own config.json
anyway. (We kept the WriteFile call as a smoke test of that RPC.)

### 3. Apple's OCI Codable requires non-optional fields explicitly

`LinuxNamespace.path: String` is declared non-optional (no default
JSON Codable synthesis treating "missing" as ""). So a runc-style
`{"type": "pid"}` decodes to
`DecodingError.keyNotFound: "path"`. Always write namespace entries
as `{"type": "pid", "path": ""}`. Same likely applies to other fields
we haven't touched.

### 4. The rootfs must be writable

`ociAlterations` writes `root.path/etc/hostname` before handing off to
vmexec. Even though my use-case was logically a read-only container,
I had to:
 - `VZDiskImageStorageDeviceAttachment(readOnly: false)`
 - `MountRequest.options: ["rw"]`
 - `spec.root.readonly = false`

(OR provide `spec.hostname = ""` to suppress the write — untested.)

### 5. vmexec's "No such file or directory" is uselessly opaque

vmexec's error path goes through `App.Errno(stage:info:)` →
`ContainerizationError(.internalError, message: "\(info) \(POSIXError)")`.
The **stage** goes into POSIXError's userInfo dict, which
`String(describing:)` doesn't print. So when it bubbles up to the host
as an `RPCError`, all you see is:

```
vmexec error: internalError: " Error Domain=NSPOSIXErrorDomain Code=2 "No such file or directory""
```

with no hint which syscall failed. It's one of:
 - `reOpenDevNull` → `open("/dev/null", O_RDWR)` — ENOENT if /dev
   doesn't have devtmpfs contents
 - `configureConsole` → `remove("<rootfs>/dev/ptmx")` — ENOENT if
   /dev/ptmx isn't a pre-existing symlink
 - `execvpe` → ENOENT if `process.args[0]` isn't in the rootfs

The fix that worked: bind-mount vminitd's own `/dev` (guest kernel
devtmpfs) into the container. That gives the container everything
devtmpfs provides (null, zero, ptmx, tty, random, urandom, ...) with
zero ceremony. Real library will probably enumerate individual dev
nodes instead (and dump more info in the vmexec error path — PR?).

### 6. Bind-mounting individual /dev nodes over a fresh tmpfs is the "runc way" — but brittle

The OCI-common pattern is `/dev` as tmpfs + individual bind mounts for
`/dev/null`, `/dev/zero`, etc. `ContainerizationOS.Mount.mount(root:)`
does auto-create bind-mount target files when source is non-dir
(line 331-337 of Mount.swift), so bind-mounting individual device nodes
onto a fresh /dev tmpfs **should** work. But I couldn't get it to:
maybe `configureConsole`'s `remove(rootfs/dev/ptmx)` races with the
tmpfs mount. Didn't debug further — `rbind /dev` is simpler and works.

## Reusable patterns for the real library

### `StdioListenerDelegate` (~60 LOC)

```rust
define_class!(
    #[unsafe(super(NSObject))]
    struct StdioListenerDelegate {
        tag: std::sync::atomic::AtomicU32,  // which port this delegate handles
    }

    unsafe impl NSObjectProtocol for StdioListenerDelegate {}

    unsafe impl VZVirtioSocketListenerDelegate for StdioListenerDelegate {
        #[unsafe(method(listener:shouldAcceptNewConnection:fromSocketDevice:))]
        fn accept(&self, _listener: &VZVirtioSocketListener,
                  conn: &VZVirtioSocketConnection,
                  _dev: &VZVirtioSocketDevice) -> bool {
            let raw = unsafe { conn.fileDescriptor() };
            let dup_fd = unsafe { libc::dup(raw) };
            // set nonblocking, publish fd to a shared slot, wake pollers...
            true
        }
    }
);
```

Paired with `virtio_dev.setSocketListener_forPort(&listener, port)`.
Mirror image of S2's `dial_vsock`. Accept path is simpler because we
don't need a dispatch_main trampoline — the delegate fires on whatever
queue VZ chose (main queue in our case).

### The OCI runtime spec we used

```jsonc
{
  "ociVersion": "1.0.2",
  "hostname": "container-0",
  "process": {
    "terminal": false,
    "user": { "uid": 0, "gid": 0 },
    "args": ["/bin/echo", "hello"],
    "cwd": "/",
    "env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
    "capabilities": { "bounding": ["CAP_AUDIT_WRITE","CAP_KILL","CAP_NET_BIND_SERVICE"],
                      "effective": ["CAP_AUDIT_WRITE","CAP_KILL"],
                      "permitted": ["CAP_AUDIT_WRITE","CAP_KILL"] },
    "noNewPrivileges": true
  },
  "root": { "path": "/run/container/container-0/rootfs", "readonly": false },
  "mounts": [
    { "destination": "/proc", "type": "proc", "source": "proc" },
    { "destination": "/dev",  "type": "bind", "source": "/dev",
      "options": ["bind", "rw", "rbind"] },
    { "destination": "/sys",  "type": "sysfs", "source": "sysfs",
      "options": ["nosuid", "noexec", "nodev", "ro"] }
  ],
  "linux": {
    "namespaces": [
      { "type": "pid",   "path": "" },
      { "type": "mount", "path": "" },
      { "type": "ipc",   "path": "" },
      { "type": "uts",   "path": "" }
    ]
  }
}
```

This is ~30% of a real runc spec but enough to exec echo. Real library
will want to fill out resource limits, cgroup settings, more dev
bind-mounts (so containers don't see all the host's devices), etc.

### rootfs.ext4 build recipe

`docker run --platform linux/arm64 alpine:3.20` with `mkfs.ext4 -d
<dir> out.ext4`. Builds e2fsck-clean 64 MiB ext4 in ~5 s. No loop
mounts needed. Portable to CI. Don't need S5's Rust ext4 writer for
this kind of ad-hoc build.

## Known loose ends

- **stdin**: untested. The inverse pattern (host dials guest, pushes
  bytes) should just work — but we should confirm before Phase 1
  starts.
- **Concurrent containers**: untested. vminitd's cgroup layout is
  per-container, so it should Just Work. But the listener pattern
  needs port allocation logic.
- **Copy RPC**: we didn't exercise `CopyIn`/`CopyOut`. It's unusual: the
  RPC returns metadata, and **the data flows over a separate vsock
  port** the host specifies. Same listener pattern works.
- **Container teardown**: we exit the host process before calling
  `DeleteProcess` or unmounting. No resource leaks visible per-run,
  but repeated invocations on the same VM weren't tested.
- **Kernel panic on host-side drop**: we didn't test what happens if
  we drop the `Retained<VZVirtualMachine>` before the container exits.
  Spike kills everything via exit(0).

## Time to solve

~1h 45min of focused work. Breakdown:
- ~20 min reading source (proto, vminitd Swift, vmexec Swift, OCI).
- ~15 min wiring vsock/disk/kernel harness (copy from S2/S3).
- ~5 min on first-compile-green.
- ~10 min on stretch minimum (Sync RPC).
- ~15 min on rootfs.ext4 build script + docker debugging.
- ~30 min on the CreateProcess/StartProcess debugging loop (gotchas
  #2, #3, #4, #5, #6 each needed a round-trip).
- ~10 min on stdio listener delegate + collector tasks.
- ~10 min on documentation.

## Proposed PRO_TIPS additions

(For the curator to fold into `PRO_TIPS.md`. Each is in "here's the
trap, here's the fix" shape.)

### Proposed §20 — Host-side vsock listener (the "other direction")

Mirror image of §13's connect path.

```rust
define_class!(
    #[unsafe(super(NSObject))]
    struct ListenerDelegate {
        slot: /* a way to publish the fd to your reactor */,
    }
    unsafe impl NSObjectProtocol for ListenerDelegate {}
    unsafe impl VZVirtioSocketListenerDelegate for ListenerDelegate {
        #[unsafe(method(listener:shouldAcceptNewConnection:fromSocketDevice:))]
        fn accept(&self, _l: &VZVirtioSocketListener,
                  conn: &VZVirtioSocketConnection,
                  _d: &VZVirtioSocketDevice) -> bool {
            let raw = unsafe { conn.fileDescriptor() };
            let fd  = unsafe { libc::dup(raw) };           // VZ owns the original
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFL);
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
            // publish to your reactor; VZ closes its copy when `conn` is
            // released after we return (we don't retain it).
            true
        }
    }
);
// Create and register:
let listener = unsafe { VZVirtioSocketListener::init(VZVirtioSocketListener::alloc()) };
unsafe { listener.setDelegate(Some(proto_obj_ref)); }
unsafe { virtio_dev.setSocketListener_forPort(&listener, port); }
// IMPORTANT: setDelegate is WEAK. Keep the delegate alive.
// Box::leak(Box::new(delegate)); Box::leak(Box::new(listener));
```

Note: unlike the connect path, we DON'T take `Retained::retain(conn)`
— the delegate just looks at its fd, dups it, and returns. VZ releases
the connection object once our delegate returns (plus whatever async
bookkeeping it does). Empirically no fd leaks over the single
accept-and-return cycle the spike exercised.

### Proposed §21 — vminitd SandboxContext quirks

When driving vminitd over gRPC:

1. **Bundle path is implicit**: vminitd uses
   `/run/container/<containerID>` — NOT anything you pass. Mount your
   container rootfs at `/run/container/<containerID>/rootfs`.
2. **Spec decoder is strict Codable**: `LinuxNamespace` wants
   `{type, path}` both present; use `"path": ""` for unshare semantics.
3. **Rootfs must be writable** unless you suppress hostname:
   `ociAlterations` writes `<root>/etc/hostname` before exec.
4. **Stdio is vsock ports**, not stream RPC (see §20). Host must have
   a `VZVirtioSocketListener` per stream on the ports you pass in
   `CreateProcessRequest.{stdin,stdout,stderr}`.
5. **vmexec error messages are opaque**: `NSPOSIXErrorDomain Code=2`
   could be any of several ENOENT failures. Start your debugging by
   verifying `/dev` has real device nodes visible to the container —
   `rbind`-mounting vminitd's own `/dev` is the simplest fix.

### Proposed §22 — rootfs.ext4 build recipe for spikes

```bash
docker run --rm --platform linux/arm64 \
    -v $OUT:/out alpine:3.20 sh -c '
  apk add --no-cache e2fsprogs busybox-static
  mkdir -p /build/{bin,etc,dev,proc,sys,root,tmp}
  cp /bin/busybox.static /build/bin/busybox
  cd /build
  for a in echo sh ls cat sleep; do ln -sf /bin/busybox bin/$a; done
  echo "root:x:0:0:root:/root:/bin/sh" > etc/passwd
  echo "root:x:0:" > etc/group
  dd if=/dev/zero of=/out/rootfs.ext4 bs=1M count=64
  mkfs.ext4 -F -d /build /out/rootfs.ext4
  e2fsck -fy /out/rootfs.ext4 || true
'
```

Produces a 64 MiB e2fsck-clean ext4 with busybox in ~5s. No loop mounts
(macOS can't), no EXT4 writer (S5 is in parallel). Enough for
"echo-inside-container" smoke tests.
