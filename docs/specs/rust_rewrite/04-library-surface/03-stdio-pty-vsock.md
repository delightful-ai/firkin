# Streaming surfaces — Stdio, Pty, Vsock

> Covers: the `Stdio` configuration enum, `ChildStdin`/`ChildStdout`/`ChildStderr` handles, `Pty` duplex type, `VsockStream` / `VsockListener` / `VsockPort`, reserved port ranges, internal mechanics.
>
> Prerequisites: [`01-container-surface.md`](./01-container-surface.md) for the builder and handle context.

---

## 1. Overview

Three related streaming surfaces share one internal mechanism (vsock fds wrapped as `tokio::io::AsyncRead` / `AsyncWrite`) but present three distinct user-facing shapes:

1. **Stdio** — container/process standard streams. Configured via `Stdio` enum at builder time; handles returned post-spawn via `take_stdin/stdout/stderr`.
2. **Pty** — pseudo-terminal. A duplex stream that combines stdin + stdout + stderr, with resize semantics.
3. **Vsock** — user-accessible arbitrary vsock channels to/from the guest. For custom guest daemons and just-a-microVM uses.

All three produce types that are `AsyncRead` / `AsyncWrite` as appropriate, `Send + Unpin + 'static`, and work with every standard tokio idiom.

---

## 2. `Stdio` — configuration enum

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stdio {
    /// The guest-side end is connected to /dev/null. The host side does not allocate
    /// a listener. No handle is returned from take_* after spawn.
    Null,

    /// The guest-side end is connected to a vsock fd on a library-allocated port.
    /// After spawn, take_* returns Some(handle) wrapping an AsyncRead/AsyncWrite over
    /// that fd. If the handle is dropped without being read, the guest's writes
    /// eventually block when the kernel socket buffer fills (~64 KiB typical).
    Piped,

    /// Library-allocated relay task forwards bytes between the vsock fd and the host
    /// process's tokio::io::stdout() / stderr() / stdin() as appropriate. No handle
    /// is returned from take_*; the library owns the forwarding.
    Inherit,
}

impl Stdio {
    pub fn null() -> Self;
    pub fn piped() -> Self;
    pub fn inherit() -> Self;
}
```

**Defaults for `stdin` / `stdout` / `stderr`: `Stdio::Null`** in both `ContainerBuilder` and `ExecConfig`.

### 2.1 Default choice — rationale

The three candidates and why Null wins by default:

| Default | Problem |
|---|---|
| `Piped` | Deadlock footgun — if the user doesn't drain a handle, the guest blocks on write when the kernel buffer fills. `find /` hangs after a few MB. |
| `Inherit` | Chatty side effect — every container implicitly writes to the host process's stdout. Acceptable for a CLI tool; surprising for a library. Also costs 2-3 relay tasks per container. |
| `Null` | Never deadlocks. Never steals the host's stdio. Trivially replaced via explicit `.stdout(Stdio::inherit())` or `.stdout(Stdio::piped())` when the user wants something else. |

A user who wants to see container output writes:

```rust
Container::builder(id)
    .rootfs(...)
    .stdout(Stdio::inherit())
    .stderr(Stdio::inherit())
    .spawn().await?
```

Two lines. Discoverable. Explicit.

### 2.2 When to use which

| Goal | `stdin` | `stdout` / `stderr` |
|---|---|---|
| Silent container (log to files via OCI config) | `Null` | `Null` |
| See output live in the terminal (dev loop) | `Null` (or `Inherit`) | `Inherit` |
| Capture output into memory for testing | `Null` | `Piped` + `wait_with_output()` |
| Pipe data in, pipe result out | `Piped` | `Piped` |
| Forward host terminal into the container | `Inherit` | `Inherit` (but consider `.pty()` instead for full terminal) |
| Interactive shell | (not used; pty carries stdin) | (not used; pty replaces stdout) — see §4 |

---

## 3. Piped handles — `ChildStdin`, `ChildStdout`, `ChildStderr`

After spawn, the `take_*` methods on `Container` and `Process` return `Option<ChildStd*>`:

```rust
pub struct ChildStdin  { /* private */ }
pub struct ChildStdout { /* private */ }
pub struct ChildStderr { /* private */ }

// Traits:
impl tokio::io::AsyncWrite for ChildStdin  {}
impl tokio::io::AsyncRead  for ChildStdout {}
impl tokio::io::AsyncRead  for ChildStderr {}

// Send + Unpin + 'static — all three.
```

`Some` iff the builder set the corresponding slot to `Stdio::Piped`. `None` otherwise (`Stdio::Null` or `Stdio::Inherit`).

### 3.1 Drop = EOF

Dropping `ChildStdin` closes the write side on the host; manifests as EOF in the guest. There is no explicit `close_stdin()` method — drop the handle or set it to `None`.

```rust
let mut stdin = container.take_stdin().unwrap();
stdin.write_all(b"hello\n").await?;
drop(stdin);           // guest sees EOF on the next read
```

### 3.2 Back-pressure

`AsyncRead` / `AsyncWrite` semantics propagate through to the kernel socket buffer. If the user doesn't read `ChildStdout`, the vsock buffer fills, and the guest's write blocks. No library-level buffering.

### 3.3 Interleaving stdout and stderr

Two separate handles, separate streams. If the user wants one merged stream in output order (stderr interleaved with stdout), they compose manually:

```rust
use tokio::io::AsyncReadExt;
let mut stdout = container.take_stdout().unwrap();
let mut stderr = container.take_stderr().unwrap();

// Interleave with tokio::select! pulling from whichever is ready first.
let mut merged = Vec::new();
loop {
    tokio::select! {
        chunk = read_chunk(&mut stdout) => match chunk {
            Ok(Some(bytes)) => merged.extend_from_slice(&bytes),
            _ => break,
        },
        chunk = read_chunk(&mut stderr) => match chunk {
            Ok(Some(bytes)) => merged.extend_from_slice(&bytes),
            _ => break,
        }
    }
}
```

This is standard tokio idiom; the library doesn't bake it in. For the common "I just want captured output" case, `wait_with_output()` is the one-liner.

### 3.4 `wait_with_output` — captured Output

The equivalent of Swift's `ioTracker` that blocked `wait()` on stdio drain. In our API it's an **explicit** method so the semantics are visible:

```rust
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,         // empty if stdout was Null or Inherit
    pub stderr: Vec<u8>,         // empty if stderr was Null or Inherit
}

impl Container {
    pub async fn wait_with_output(self) -> Result<Output, core::Error>;
}

impl Process {
    pub async fn wait_with_output(self) -> Result<Output, core::Error>;
}
```

Internal implementation: `tokio::try_join!(child.wait(), read_to_end(stdout), read_to_end(stderr))`. Consumes `self` so the handles are owned; `Drop` wouldn't observe them otherwise.

---

## 4. `Pty` — the mutually-exclusive-with-stderr duplex

```rust
pub struct Pty { /* private */ }

impl tokio::io::AsyncRead  for Pty {}
impl tokio::io::AsyncWrite for Pty {}
// Pty: Send + Unpin + 'static

impl Pty {
    /// Apply new rows/cols to the pty. Sends a TIOCSWINSZ-equivalent RPC to
    /// vminitd; propagates SIGWINCH to the guest process automatically.
    pub async fn resize(&mut self, size: PtyConfig) -> Result<(), core::Error>;

    /// Most recently applied size. Does not query the guest — returns the last
    /// size this side sent or the initial builder config.
    pub fn size(&self) -> PtyConfig;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtyConfig { pub cols: u16, pub rows: u16 }
```

### 4.1 Typestate invariant (restated)

`.pty(config)` on a `ContainerBuilder<_, Ready>` transitions to `ContainerBuilder<_, ReadyPty>`. On `ReadyPty`, `.stderr(…)` is a compile error. Same for `ExecConfigBuilder<CommandSet>` → `.pty(config)` → `ExecConfigBuilder<CommandSetPty>`.

A container built with `.pty(…)` returns:
- `None` from `stdout()` and `stderr()` — the pty combines them.
- `Some(Pty)` from `pty()` — the single duplex handle.
- `Some(ChildStdin)` from `stdin()` — stdin is still separately configurable (default `Stdio::null()`; can be set to `Stdio::inherit()` for "forward host stdin through the pty" or `Stdio::piped()` for programmatic writes).

### 4.2 Why duplex, not split halves

A Unix pty is inherently one bidirectional master-side fd. Swift's API exposes it that way via `FileHandle`. Splitting into `(PtyReader, PtyWriter)` would:
- Force the library to implement split halves with a shared fd and internal coordination.
- Forfeit the standard `tokio::io::split(pty)` idiom users already know.

So `Pty: AsyncRead + AsyncWrite`. Users who want split halves call `tokio::io::split(pty)` themselves.

### 4.3 Resize semantics

`pty.resize(new_size).await?` sends the resize RPC. Completion means vminitd applied it; SIGWINCH propagation to the guest process happens inside the guest kernel.

No hooks, no callbacks — if the user wants to know when SIGWINCH has been delivered to a specific process, they arrange that inside the container. The library's contract ends at "we told the pty to resize, the guest kernel was told."

---

## 5. Vsock — user-accessible arbitrary channels

Three types: `VsockStream`, `VsockListener`, `VsockPort`. Per [D-016](../DECISIONS.md#d-016--firkin-vsock-owns-streamlistener-types-vmm-depends-on-vsock), `VsockStream` / `VsockListener` / `VsockPeer` live in **`firkin-vsock`** (a portable leaf with no `objc2` deps); `VsockPort` lives in **`firkin-types`** (D-015). `firkin-vmm` depends on `firkin-vsock` and hands it `OwnedFd`s produced by VZ's connect / listener-delegate machinery. All three types are re-exported from `firkin-vmm` (for the just-a-microVM use case) and from `firkin` (the facade) so user code doesn't need to import `firkin-vsock` directly.

### 5.1 `VsockPort` — typed port number

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct VsockPort(u32);

impl VsockPort {
    pub const fn new(port: u32) -> Self;
    pub const fn get(self) -> u32;
}

impl From<u32> for VsockPort { /* ... */ }
```

A newtype prevents "which port number did I mean" bugs (`scatter.md § translation`). All dial / listen methods take `VsockPort`, not raw `u32`.

### 5.2 `VsockStream` — AsyncRead + AsyncWrite

```rust
pub struct VsockStream { /* private */ }

impl tokio::io::AsyncRead  for VsockStream {}
impl tokio::io::AsyncWrite for VsockStream {}
// VsockStream: Send + Unpin + 'static
```

Returned by:
- `VirtualMachine<Running>::dial(port)` — host dials guest.
- `Container::dial_vsock(port)` — container-scoped; dials a port that the guest side of this container's VM is listening on.
- `VsockListener::accept()` — listener accepts a guest-dialed connection.

### 5.3 `VsockListener` — async stream of `VsockStream`

```rust
pub struct VsockListener { /* private */ }

impl VsockListener {
    /// Block until a guest dials in. Returns the connection + peer info.
    pub async fn accept(&self) -> Result<(VsockStream, VsockPeer), vmm::Error>;

    /// As a Stream for consumption via for-await or StreamExt::next.
    pub fn incoming(&self) -> impl futures::Stream<Item = Result<VsockStream, vmm::Error>> + '_;

    /// Graceful close — stops accepting new connections, existing streams keep working.
    pub fn finish(self);
}

pub struct VsockPeer {
    pub cid: u32,       // VZ convention: CID 3 for guest, CID 2 for host
    pub port: VsockPort,
}
```

Only obtained via `VirtualMachine<Running>::listen(port)`. Container-scoped listen is NOT exposed — container stdio is the only inverse-vsock path and the library owns that (D-005).

### 5.4 Access-point matrix

| Where obtained | Method | Semantics |
|---|---|---|
| `VirtualMachine<Running>` | `async fn dial(port) -> VsockStream` | Host dials guest; guest is listening |
| `VirtualMachine<Running>` | `fn listen(port) -> VsockListener` | Host listens; guest is dialer |
| `Container` | `async fn dial_vsock(port) -> VsockStream` | Same as VM dial but scoped — errors if container is not running |

---

## 6. Reserved port ranges

Some vsock ports are reserved for library use and will error on user `dial` or `listen`:

| Range | Use |
|---|---|
| `1024` | vminitd gRPC service (guest listens; host dials) |
| `0x1000_0000` – `0x2000_0000` | Library-allocated for stdio listeners and socket relays |
| All others | Free for user code |

Calling `vm.dial(VsockPort::new(1024))` or `vm.listen(VsockPort::new(0x1000_0001))` returns:

```rust
Err(vmm::Error::ReservedPort {
    port: VsockPort(0x1000_0001),
    reason: "0x1000_0000-0x2000_0000 is reserved for library stdio/relay allocation",
})
```

The range `0x1000_0000` – `0x2000_0000` is preserved from Swift `apple/containerization`'s convention. Keeping this matching makes future interop (e.g., sharing a vminitd configuration with Swift consumers) simpler.

---

## 7. Internal mechanics (summary, not user-visible)

All three of §3 (stdio), §4 (pty), and §5 (vsock) share the same under-the-hood plumbing:

1. **Host-delivered fd.** Either via `VZVirtioSocketDevice.connect(toPort:)` returning a `VZVirtioSocketConnection` (outbound; PRO_TIPS §13), or via `VZVirtioSocketListener` delegate firing `shouldAcceptNewConnection:` (inbound; PRO_TIPS §20). This step lives in `firkin-vmm`.
2. **`dup` + `O_NONBLOCK`.** `firkin-vmm` duplicates the fd so VZ can release its ownership cleanly; sets non-blocking mode. The resulting `OwnedFd` is handed to `firkin-vsock`.
3. **`tokio::io::unix::AsyncFd` wrapping.** `firkin-vsock` (portable; no `objc2` dependency — D-016) registers the fd with the tokio reactor; reads/writes become `AsyncRead`/`AsyncWrite` via the standard pattern.
4. **Type-specific wrappers.** `ChildStdin` adds write-only discipline. `ChildStdout` / `ChildStderr` adds read-only. `Pty` carries the resize-RPC side channel. `VsockStream` is the plain duplex.
5. **For `Stdio::Inherit`.** A per-stream `tokio::spawn` relay task runs `tokio::io::copy_bidirectional` or per-direction `tokio::io::copy` against `tokio::io::stdin() / stdout() / stderr()`.

Users don't see any of this. Cross-boundary type is always `AsyncRead` / `AsyncWrite`; no `AsyncFd<_>` or `RawFd` leaks. The split between `vmm` (FD production) and `vsock` (FD consumption + wrapping) is also the seam that lets `firkin-vsock` be unit-tested against `tokio-vsock` loopback listeners with no VM required.

---

## 8. Cancellation and back-pressure — reminder

Consistent with [`09-cross-cutting.md § cancellation`](./09-cross-cutting.md):

- Any `.await` on stdio, pty, or vsock operations is drop-future-is-cancel safe.
- Back-pressure is inherited from kernel socket buffers — no library-level buffering.
- Timeouts compose via `tokio::time::timeout(dur, handle.read(…))`.
- Dropping a handle closes the host end of the fd; guest sees EOF / connection-reset as appropriate.

---

## 9. Invariants worth locking

1. `Stdio` has three variants: `Null`, `Piped`, `Inherit`. Default for stdin/stdout/stderr is `Null`.
2. `ChildStdin: AsyncWrite`, `ChildStdout: AsyncRead`, `ChildStderr: AsyncRead`. All Send + Unpin + 'static.
3. `Pty: AsyncRead + AsyncWrite`. Duplex, not split halves. `resize(size)` via async method; `size()` returns cached last-set.
4. Terminal XOR stderr enforced at compile time via builder typestate ([§2.3.5 of 01-container-surface.md](./01-container-surface.md)).
5. `VsockPort` is a newtype defined in `firkin-types` (D-015). Reserved ranges (`1024`, `0x1000_0000`–`0x2000_0000`) error on user dial/listen.
6. `VsockStream` / `VsockListener` / `VsockPeer` live in `firkin-vsock` (D-016) — portable; no `objc2` dependency. `firkin-vmm` depends on `firkin-vsock` and re-exports the types; `firkin` re-exports them again for user ergonomics.
7. Container-scoped `listen()` is not exposed. Only VM-level. Container stdio uses inverse-vsock internally (D-005).
8. `Stdio::Inherit` spawns library-owned relay tasks; cost is 1 task per active direction per container.
9. Drop = EOF for `ChildStdin`; no explicit `close_stdin()` method.
10. `wait_with_output(self) -> Output` is the explicit drain-and-consume alternative to `wait() -> ExitStatus`.

Proceed to [`04-value-types.md`](./04-value-types.md) for the full value-type catalog.
