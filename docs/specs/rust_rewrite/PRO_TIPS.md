# PRO_TIPS — things that'll bite if you don't know

Hard-won from S1 (`spike-logs/s1-boot/`). Read this *before* you start
writing Rust against `objc2-virtualization`. Each tip is here because it
cost us time; don't re-pay that cost.

When you hit something that isn't in this file, **add it**. Future-you / the
other claudes are reading this.

---

## 1. Threading & queues

### Default to "everything on main queue + `dispatch_main()`"

VZ requires a single serial dispatch queue per VM. The cheapest such queue
is the **main queue**. `dispatch2::dispatch_main()` blocks the main thread
and pumps it forever. That's exactly what a CLI spike wants.

```rust
use dispatch2::dispatch_main;

fn main() {
    // ...build config, delegate, VM, start it...
    unsafe { vm.startWithCompletionHandler(&start_block) };

    // Leak owning Retained<_>s before dispatch_main (it diverges; OS reaps).
    Box::leak(Box::new(vm));
    Box::leak(Box::new(delegate));
    std::mem::forget(start_block);

    dispatch_main();  // -> !
}
```

Delegate / completion-handler callbacks fire on the main queue. `exit(0)`
from the delegate when you're done.

### Don't reach for a custom `DispatchQueue` unless you actually need parallel VMs

If you do: everything in `objc2-virtualization` is **!Send** (no
`unsafe impl Send` anywhere in `generated/Virtualization/*.rs`). That
trips `DispatchQueue::exec_async`'s `F: Send + 'static` bound.

Escape hatch:

```rust
/// Wrap a !Send Obj-C handle so it can cross queue boundaries. Safe iff
/// the inner value is only touched on the VZ serial dispatch queue.
struct VzSend<T>(T);
unsafe impl<T> Send for VzSend<T> {}
// Add `Sync` too only if you genuinely share &T across threads.
```

### The RFC 2229 closure-capture trap

Rust's disjoint-closure-captures rule will bite you when you wrap a
!Send thing in `VzSend` and then destructure inside a `move` closure:

```rust
let wrapped = VzSend(config.clone());    // Send (wrapper)
queue.exec_async(move || {
    let cfg = wrapped.0;                  // ← closure narrows capture to
                                          //   wrapped.0, which is !Send
    use_cfg(cfg);
});
// compile error: closure not Send
```

Fixes (pick one):

```rust
// (a) Force full capture with a meaningless reference use
queue.exec_async(move || {
    let _ = &wrapped;                     // full-capture force
    let cfg = wrapped.0;
    use_cfg(cfg);
});

// (b) Access via field, keep wrapper in scope
queue.exec_async(move || {
    let cfg: &Retained<_> = &wrapped.0;   // borrow doesn't narrow in the same way
    use_cfg(cfg);
});

// (c) Wrap in Arc<VzSend<_>>; Arc<T: Send + Sync> is Send
let shared: Arc<VzSend<Retained<_>>> = Arc::new(VzSend(config.clone()));
// (requires unsafe impl<T> Sync for VzSend<T> {} — only if safe in your flow)
```

**Strong preference**: avoid the whole mess by staying on main queue.

### `DispatchQueue` API notes

- `DispatchQueue::new(Some(&CStr), None)` — label is `Option<&CStr>`, not `&str`. Use `CStr::from_bytes_with_nul(b"my.label\0").unwrap()`.
- `DispatchQueue::current()` is **deprecated** (returns `DispatchRetained`, not `Option`). Capture the queue explicitly via a clone if you need it inside a closure.
- `DispatchQueue::main()` returns `&'static Self` — always available.
- `exec_sync<F: Send + FnOnce()>` **returns unit** (no return value). If you need to shuttle data out of the queue, use a channel or a shared `Arc<Mutex<Option<_>>>`.
- `dispatch2::dispatch_main()` is re-exported from the crate root: `use dispatch2::dispatch_main;`.

---

## 2. `define_class!` — new-ish syntax

The old `#[ivars = T]` attribute was removed. Ivars are now inline struct
fields; getter methods are auto-generated per field.

```rust
use objc2::{define_class, msg_send, AnyThread, Ivars};
use objc2_foundation::{NSObject, NSObjectProtocol};

define_class!(
    // SAFETY notes must cite:
    //   - superclass subclassing constraints
    //   - thread affinity (MainThreadOnly? or any-thread?)
    //   - whether Drop is implemented (and what it must not do)
    #[unsafe(super(NSObject))]
    struct MyDelegate {
        fired: std::sync::atomic::AtomicBool,
        tx: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
    }

    unsafe impl NSObjectProtocol for MyDelegate {}

    // Conform to whatever protocol:
    unsafe impl SomeDelegate for MyDelegate {
        #[unsafe(method(someMethod:))]
        fn some_method(&self, arg: &NSObject) {
            // Access ivars via auto-generated getters:
            self.fired().store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(tx) = self.tx().lock().unwrap().take() { let _ = tx.send(()); }
        }
    }
);

impl MyDelegate {
    fn new() -> objc2::rc::Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars::<Self> {
            fired: std::sync::atomic::AtomicBool::new(false),
            tx: std::sync::Mutex::new(None),
        });
        unsafe { msg_send![super(this), init] }
    }
}
```

Notes:

- **`Ivars::<Self>`** is the macro-generated constructor type. Field names must match the struct body.
- **Auto-generated getters**: one method per ivar, same name, returns `&Field`. `self.fired()` yields `&AtomicBool`.
- **Send/Sync propagate from ivars**: if every ivar is Send+Sync, the class is Send+Sync. `Mutex<T>` and `AtomicBool` are Send+Sync, which is usually what you want for a delegate.
- **No thread marker** means any-thread. Add `#[thread_kind = MainThreadOnly]` only if the protocol requires main-thread affinity (VZVirtualMachineDelegate does **not**).

---

## 3. Obj-C object lifetimes

### Delegates are weak

`VZVirtualMachine::setDelegate` does **not** retain the delegate. Keep a
`Retained<MyDelegate>` alive on the Rust side for the lifetime the VM runs.
In a CLI spike, `Box::leak(Box::new(delegate))` before `dispatch_main()`.
In a library, hold it as a field of the owning struct.

### Completion-handler blocks

Use `block2::RcBlock` — heap-allocated, refcounted, lives as long as anyone
holds a clone. `StackBlock` only survives the current scope; VZ completion
handlers fire **after** the call returns, so `StackBlock` will dangle.

```rust
let block = block2::RcBlock::new(move |err: *mut NSError| {
    if err.is_null() { /* ok */ } else { /* fail */ }
});
unsafe { vm.startWithCompletionHandler(&block) };
std::mem::forget(block);  // if nothing else holds it and it must survive
```

In a library, hold RcBlocks as fields of the owning struct rather than
forgetting them.

### Superclass coercion

Methods on parent classes often want `&Parent`. Given `&Retained<Child>`,
you can't just coerce — you need `objc2::ClassType::as_super()`.

```rust
use objc2::ClassType;

let child: Retained<VZVirtioConsoleDeviceSerialPortConfiguration> = /* ... */;
let parent: &VZSerialPortConfiguration = (&*child).as_super();
config.setSerialPorts(&NSArray::from_slice(&[parent]));
```

### `NSArray` construction

```rust
use objc2_foundation::NSArray;

// From borrowed slice:
let arr: Retained<NSArray<T>> = NSArray::from_slice(&[&obj1, &obj2]);

// From owned Retained slice:
let arr: Retained<NSArray<T>> = NSArray::from_retained_slice(&[r1, r2]);
```

---

## 4. NSFileHandle / NSURL / NSString glue

```rust
use objc2_foundation::{NSFileHandle, NSString, NSURL};

// NSString from Rust:
let s = NSString::from_str("hello");

// file:// URL from a path. `fileURLWithPath:` accepts relative paths; we
// canonicalize first so behavior doesn't depend on cwd.
let abs = std::fs::canonicalize(path)?.to_string_lossy().into_owned();
let url = NSURL::fileURLWithPath(&NSString::from_str(&abs));

// Wrap an existing fd as NSFileHandle (no closeOnDealloc here -> fd not closed on drop):
use std::os::fd::AsRawFd;
let fh = NSFileHandle::initWithFileDescriptor(NSFileHandle::alloc(),
                                              std::io::stdout().as_raw_fd());
```

`NSFileHandle` is `unsafe impl Send` (check `generated/Foundation/NSFileHandle.rs:16`).

---

## 5. NSError → String

```rust
fn nserror_desc(err: &NSError) -> String {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2_foundation::NSString;
    let code: isize = unsafe { msg_send![err, code] };
    let desc: Retained<NSString> = unsafe { msg_send![err, localizedDescription] };
    format!("NSError code={code} desc={}", desc.to_string())
}
```

Validation errors are where codesigning / entitlement issues surface most
often — always log the `localizedDescription`, not just the code.

---

## 6. Codesigning + entitlements

The minimum to run VZ from Rust in dev:

```xml
<!-- entitlements.plist -->
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.virtualization</key>
    <true/>
</dict>
</plist>
```

```bash
# Ad-hoc sign (- means ad-hoc). No paid dev program needed.
codesign --force --sign - --entitlements entitlements.plist path/to/binary

# Verify:
codesign -d --entitlements :- path/to/binary
```

- **No provisioning profile, no Apple Developer Program** is needed for NAT-backed VMs.
- `vmnet` (`com.apple.vm.networking`) is a separate question — S6 proper.
- **Re-sign after every `cargo build`**: cargo overwrites the binary, which strips the signature. Bake this into your `sign-and-run.sh`.

---

## 7. Asset acquisition via docker

No zig / musl cross compiler on macOS? No problem — OrbStack ships docker.

### Arm64 Linux kernel (vmlinux / Image format, uncompressed)

```bash
docker run --rm --platform linux/arm64 -v /tmp/out:/out ubuntu:24.04 bash -c '
  apt-get update -qq && apt-get install -y --no-install-recommends linux-image-virtual >/dev/null
  cp /boot/vmlinuz-*-generic /out/vmlinuz.gz.raw
'
gunzip -c /tmp/out/vmlinuz.gz.raw > ./assets/vmlinux
# Verify: "Linux kernel ARM64 boot executable Image, little-endian, 4K pages"
file ./assets/vmlinux
```

~56 MB uncompressed. VZ accepts it directly (ARMd magic + MZ/PE stub).

### Static arm64 musl init + cpio initrd

```bash
# In your spike dir with init/init.c:
docker run --rm --platform linux/arm64 -v $PWD/init:/src -v $PWD/assets:/out \
  alpine:3.20 sh -c '
    apk add --no-cache musl-dev gcc make cpio linux-headers >/dev/null
    cd /src
    gcc -static -Os -s -o /tmp/init init.c
    mkdir -p /tmp/initroot && cp /tmp/init /tmp/initroot/init
    cd /tmp/initroot && find . | cpio -o -H newc > /out/initrd.cpio
  '
```

The kernel auto-runs `/init` from the initramfs when booted with an initrd.
No `init=` cmdline override needed for that path.

### Static arm64 guest-side tonic binary (for S2)

Same recipe, swap to a docker image with Rust + musl target:

```bash
docker run --rm --platform linux/arm64 -v $PWD/guest:/src -v $PWD/assets:/out \
  rust:1-alpine sh -c '
    apk add --no-cache musl-dev >/dev/null
    rustup target add aarch64-unknown-linux-musl 2>/dev/null || true
    cd /src && cargo build --release --target aarch64-unknown-linux-musl
    cp target/aarch64-unknown-linux-musl/release/<bin> /out/<bin>
  '
```

(You're already on arm64, so `--platform` matches the host; you probably
don't need it but it doesn't hurt.)

### Sharing assets between spikes

The kernel (`assets/vmlinux`) is identical across spikes. Cheap to
symlink from subsequent spikes:

```bash
ln -s ../s1-boot/assets/vmlinux ./assets/vmlinux
```

---

## 8. `objc2-virtualization` API discovery

When you need a symbol, grep the generated files directly. They're the
authoritative API.

```bash
# Find the file for a type:
ls ~/vendor/github.com/madsmtm/objc2/generated/Virtualization/ | grep -i <term>

# Find a method across the crate:
grep -rn "method(connectToPort" ~/vendor/github.com/madsmtm/objc2/generated/Virtualization/
```

The `framework-crates/objc2-virtualization/Cargo.toml` default features are
already on for the common stuff (VZLinuxBootLoader, VZVirtioSocket*, etc.).
If you need `block2` or `dispatch2` interop, **enable those features
explicitly** in your spike's Cargo.toml:

```toml
objc2-virtualization = {
  path = "../../../vendor/github.com/madsmtm/objc2/framework-crates/objc2-virtualization",
  features = ["dispatch2", "block2"],
}
```

**Gotcha**: The `generated/` dir is a git submodule (`objc2-generated.git`).
If cargo-check fails with missing symbols on a fresh vendored clone, run:

```bash
cd ~/vendor/github.com/madsmtm/objc2
git submodule update --init --depth 1
```

---

## 9. Send/Sync cheat sheet for VZ types

- **None of `VZ*` are Send or Sync by default.** Don't grep for them;
  we already looked — zero matches under `generated/Virtualization/`.
- `Retained<T>: Send` requires `T: Send + Sync`. So `Retained<VZ*>` is
  !Send without help.
- `NSFileHandle` **is** Send + Sync (see `generated/Foundation/NSFileHandle.rs`).
  Safe to cross threads.
- `DispatchQueue` / `DispatchRetained<DispatchQueue>` are Send + Sync.
- When in doubt: grep the generated file for `unsafe impl (Send|Sync)`. If
  nothing matches, assume !Send and !Sync.

---

## 10. Versions we know work

| Crate | Version | Source |
|---|---|---|
| `objc2` | 0.6.4 | `~/vendor/github.com/madsmtm/objc2/crates/objc2` |
| `objc2-foundation` | 0.3.2 | `~/vendor/.../framework-crates/objc2-foundation` |
| `objc2-virtualization` | 0.3.2 | `~/vendor/.../framework-crates/objc2-virtualization` |
| `block2` | 0.6.2 | `~/vendor/.../crates/block2` |
| `dispatch2` | 0.3.1 | `~/vendor/.../crates/dispatch2` |

Always path-deps these into your spike's `Cargo.toml` — avoids a crates.io
version drift surprise.

```toml
[dependencies]
objc2 = { path = "../../../vendor/github.com/madsmtm/objc2/crates/objc2" }
objc2-foundation = { path = "../../../vendor/github.com/madsmtm/objc2/framework-crates/objc2-foundation", features = ["NSFileHandle", "NSURL", "NSString", "NSError", "NSArray"] }
objc2-virtualization = { path = "../../../vendor/github.com/madsmtm/objc2/framework-crates/objc2-virtualization", features = ["dispatch2", "block2"] }
block2 = { path = "../../../vendor/github.com/madsmtm/objc2/crates/block2" }
dispatch2 = { path = "../../../vendor/github.com/madsmtm/objc2/crates/dispatch2", features = ["alloc", "objc2"] }
```

Relative path `../../../vendor/...` works from `~/tmp/rust-rewrite-spikes/s<N>-*/`.

---

## 11. When you hit a Rust compile error you don't understand

Most errors against `objc2-virtualization` have a shape:

| Error | Cause |
|---|---|
| `*const UnsafeCell<()> cannot be sent between threads` | Retained<VZ\*> crossing a thread boundary. Stay on main queue or use VzSend. |
| `no method named as_super` | Missing `use objc2::ClassType;`. |
| `expected Option<&CStr>, found &str` | `DispatchQueue::new` wants a CStr label. |
| `the syntax for specifying instance variables has changed` | Old `#[ivars = T]`. Move to inline struct fields. |
| `mismatched types: expected ..., found ()` on `exec_sync` | `exec_sync` doesn't return values; use a channel. |
| `method not found` on a deprecated `current_queue` | It's `DispatchQueue::current()`, deprecated, returns non-Option. Avoid; capture queue explicitly. |

Grep this file for the error phrase — if it's here, the fix is a line away.
If it's not, **add it when you solve it**.

---

## 12. When in doubt: read the passed spikes

Reference implementations to lift from:

- **S1** (`~/tmp/rust-rewrite-spikes/s1-boot/src/main.rs`): boot harness, StopDelegate, ns_url_file / nserror_desc, main-queue + dispatch_main pattern.
- **S2** (`~/tmp/rust-rewrite-spikes/s2-vsock-tonic/src/main.rs`): `VsockConnector` (tower::Service<Uri>), `dial_vsock` (oneshot bridge VZ queue ↔ tokio), vsock fd-ownership pattern.
- **S3** (`~/tmp/rust-rewrite-spikes/s3-vminitd-build/src/main.rs`): block-device wiring (VZDiskImageStorageDeviceAttachment + VZVirtioBlockDeviceConfiguration), watchdog-bounded sign-and-run.

---

## 13. Vsock — the least obvious bits (from S2)

### `VZVirtioSocketConnection` owns its fd — always `dup()` before handing to tokio

The header doc is literal: "The file descriptor is owned by the
`VZVirtioSocketConnection`. It is automatically closed when the object
is destroyed."

The safe pattern — inside the `connectToPort_completionHandler` `RcBlock`:

```rust
// We're on the VZ main queue here. `conn` is the raw pointer VZ handed us.
let conn = unsafe { objc2::rc::Retained::retain(conn) }.expect("conn nil");
let raw = unsafe { conn.fileDescriptor() };
let dup_fd = unsafe { libc::dup(raw) };
// Set nonblocking for tokio:
let flags = unsafe { libc::fcntl(dup_fd, libc::F_GETFL) };
unsafe { libc::fcntl(dup_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
// Close VZ's reference — our dup is independent now.
unsafe { conn.close() };
drop(conn);  // on main queue; safe
// Ship OwnedFd to tokio via oneshot:
tx.send(Ok(unsafe { OwnedFd::from_raw_fd(dup_fd) })).ok();
```

Keeping the `Retained<VZVirtioSocketConnection>` alive with the stream
forces a `!Send` handle into your stream type, and tokio wants `Send`
futures → tears.

### Host-side vsock fd → tonic Channel

Wrap `OwnedFd` as `tokio::net::UnixStream::from_std(std::os::unix::net::UnixStream::from_raw_fd(raw))`. Tokio doesn't type-check address family unless you call `peer_addr` / `local_addr`. Then `hyper_util::rt::TokioIo` + a custom `tower::Service<Uri>` connector:

```rust
impl tower::Service<Uri> for VsockConnector {
    type Response = TokioIo<UnixStream>;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    // ... ignore the Uri; always dial the same vsock port ...
}

let chan = Endpoint::from_static("http://vsock.invalid")
    .connect_timeout(Duration::from_secs(2))
    .connect_with_connector(VsockConnector { port })
    .await?;
```

The authority string is never used but must parse; "vsock.invalid" works.

### Guest kernel may not have vsock built in — Ubuntu's doesn't

Ubuntu 24.04's arm64 `linux-image-virtual` compiles AF_VSOCK as modules:

```
/lib/modules/<kver>/kernel/net/vmw_vsock/
├── vsock.ko.zst
├── vmw_vsock_virtio_transport.ko.zst
└── vmw_vsock_virtio_transport_common.ko.zst
```

First `socket(AF_VSOCK, SOCK_STREAM, 0)` dies with `EAFNOSUPPORT (errno 97)`.

Fix in your initrd build: copy + `zstd -d` the three modules into `/lib/modules/` in the cpio, and `busybox insmod` them in dependency order from the init script:

```sh
for m in vsock vmw_vsock_virtio_transport_common vmw_vsock_virtio_transport; do
    /bin/busybox insmod /lib/modules/${m}.ko
done
```

**Permanent fix**: use S3's kernel (kata 3.17.0 `vmlinux.container` or apple/containerization's own kernel when built). Both compile vsock in.

### `tokio-vsock` pinning for guest servers

`tokio-vsock = "0.7"` with the **`tonic012`** feature enables the `Connected` impl on `VsockStream` that `tonic::transport::Server::serve_with_incoming` requires.

### RFC 2229 closure narrowing — it re-bites

§1's trap shows up any time an `exec_async` closure touches `wrapper.0` and ignores the rest of `wrapper`. The fix is always:

```rust
queue.exec_async(move || {
    let _ = &wrapper;     // force full capture, kills narrowing
    // ...now use wrapper.0 freely
});
```

Stick this one-liner near the top of any `exec_async` closure capturing an `Arc<VzSend<_>>` or similar.

### `connectToPort` is FAST

Sub-millisecond after start. First-call RTT in S2 was ~400 µs including the HTTP/2 handshake. VZ vsock plumbing is not the bottleneck.

---

## 14. Watchdog pattern for long-running guests (from S3)

vminitd, echo servers, anything that keeps running — the guest won't
power off on its own. The template's `sign-and-run.sh` (as of S3) supports:

```bash
SPIKE_TIMEOUT_SECS=10 ./sign-and-run.sh
```

Inside, it:
1. Spawns the binary.
2. Sleeps `N` seconds.
3. Sends SIGTERM, waits, SIGKILL if needed.
4. Treats exit code `143` (SIGTERM) or `137` (SIGKILL) **as success**.

Use this when "reached a stable running state" is the acceptance
criterion (e.g. "vminitd logs `serving on vsock`"). For spikes with a
natural end-of-life (S1, S2's test routine), leave `SPIKE_TIMEOUT_SECS`
unset — the binary exits 0 when it's done and we report that directly.

---

## 15. Swift toolchain setup for `make linux-build` (from S3)

`/usr/bin/swift` on macOS 26.3 is 6.2.x — **too old** for
apple/containerization's pinned 6.3.0 (`.swift-version`). Also `cctl`,
which is a host-side Swift binary, needs 6.3 to build.

Use swiftly everywhere:

```bash
# Install swiftly (one-off; Apple installer pkg):
curl -O https://download.swift.org/swiftly/darwin/swiftly.pkg
# ... or let `make -C vminitd cross-prep` do it + fetch the SDK.

# Make Swift 6.3 the default:
swiftly install 6.3.0
# Subsequent commands: use ~/.swiftly/bin/swift, not /usr/bin/swift.

~/.swiftly/bin/swift --version        # => 6.3.0
~/.swiftly/bin/swift build --product cctl -c debug --disable-automatic-resolution
```

Static-linux SDK (auto-installed by `cross-prep`):

- `swift-6.3-RELEASE_static-linux-0.1.0`
- URL: `https://download.swift.org/swift-6.3-release/static-sdk/swift-6.3-RELEASE/swift-6.3-RELEASE_static-linux-0.1.0.artifactbundle.tar.gz`
- SHA-256: `d2078b69bdeb5c31202c10e9d8a11d6f66f82938b51a4b75f032ccb35c4c286c`

Pinned by the repo's own `vminitd/Makefile`. Don't drift without reason.

---

## 16. Building vminitd artifacts (from S3)

The top-level `Makefile` has three traps. Know them.

### `make linux-build LIBC=musl` needs apple/container CLI on Darwin

It invokes a `linux_run` helper that launches a Linux dev container via the apple/container CLI. If `container` isn't installed, you get a one-line error.

**Bypass**: `make -C vminitd` does the equivalent cross-compile using swiftly + the static-linux SDK directly. No container required. Critical for CI on macOS-arm64 runners that don't have apple/container installed.

### `make init` doesn't produce `bin/init.block`

Top-level `make init` `rm -f`s init.block up front, then runs cctl *without* `--ext4`. That only writes a tar + an OCI record; ext4 is lazily materialized at VM-boot time by `InitImage.initBlock(at:for:)` in the Swift library.

**Get a real init.block** with `cctl rootfs create --ext4`:

```bash
./bin/cctl rootfs create \
    --vminitd vminitd/bin/vminitd \
    --vmexec  vminitd/bin/vmexec  \
    --ext4    bin/init.block      \
    --label   org.opencontainers.image.source=https://github.com/apple/containerization \
    bin/init.rootfs.tar.gz
```

`--ext4` triggers `EXT4Unpacker.unpack(...)` and writes the image deterministically. Required for CI-reproducible artifact builds.

### `make fetch-default-kernel` chokes on URL-shaped vendored paths

The vendored repo path contains a colon (`.../https:/github.com/...`). `Protobuf.Makefile:24` parses `$(ROOT_DIR)` as a target list and dies with `*** target pattern contains no '%'`. Run the fetch manually:

```bash
mkdir -p .local bin
curl -SsL -o .local/kata.tar.gz \
    https://github.com/kata-containers/kata-containers/releases/download/3.17.0/kata-static-3.17.0-arm64.tar.xz
tar -xf .local/kata.tar.gz -C .local/ --strip-components=1
cp -L .local/opt/kata/share/kata-containers/vmlinux.container bin/vmlinux
```

kata 3.17.0's `vmlinux.container` is arm64 Image format, has vsock built in, ~14 MiB. Drop-in replacement for apple's own kernel until S3-alternate-path materialises it.

---

## 17. Debug vminitd double-execs — that's normal

DEBUG builds of vminitd self-exec with `FOREGROUND=1` set so the outer PID 1 stays alive to collect startup errors before a kernel panic. Release builds skip this. Two "DEBUG mode active" log lines from a single vminitd run is **expected**, not a bug.

---

## 18. Block devices (from S3)

Adding a readonly ext4 rootfs to the VM config:

```rust
use objc2_virtualization::{
    VZDiskImageStorageDeviceAttachment, VZStorageDeviceConfiguration,
    VZVirtioBlockDeviceConfiguration,
};

let disk_url = ns_url_file("assets/init.block");
let attach: Retained<VZDiskImageStorageDeviceAttachment> = unsafe {
    VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_error(
        VZDiskImageStorageDeviceAttachment::alloc(),
        &disk_url,
        true,  // readOnly
    )?
};
let dev: Retained<VZVirtioBlockDeviceConfiguration> = unsafe {
    VZVirtioBlockDeviceConfiguration::initWithAttachment(
        VZVirtioBlockDeviceConfiguration::alloc(),
        attach.as_super(),
    )
};
let dev_parent: &VZStorageDeviceConfiguration = (&*dev).as_super();
unsafe {
    config.setStorageDevices(
        &NSArray::<VZStorageDeviceConfiguration>::from_slice(&[dev_parent]),
    );
};
```

Multiple disks: push more `VZVirtioBlockDeviceConfiguration`s into the
`from_slice` array. They show up inside the guest as `/dev/vda`, `/dev/vdb`,
... in the order you supplied them.

Cmdline for booting from the first block device:
```
console=hvc0 root=/dev/vda rootfstype=ext4 ro init=/sbin/vminitd panic=-1
```

`rootfstype=ext4` isn't strictly required (kernel auto-detects) but is
explicit and avoids probe surprises.

---

## 19. vminitd facts — for anything talking to it (from S3)

- **Vsock port**: `1024`, hardcoded at `vminitd/Sources/vminitd/AgentCommand.swift:44`.
- **Default subcommand**: `agent`. `init=/sbin/vminitd` with no args is correct.
- **init.block mount mode**: readonly. vminitd mounts `/run` (tmpfs), `/sys`, `/sys/fs/cgroup`, and `binfmt_misc` itself from the guest side. Don't try to write to the rootfs.
- **Writable container rootfs**: provide a second `VZVirtioBlockDeviceConfiguration` — it lands as `/dev/vdb`.
- **Errors don't bring the kernel down**. `VZVirtualMachineDelegate::virtualMachine:didStopWithError:` only fires on kernel-level panic. For health checks, dial vsock port 1024 with a tonic probe.

---

## 20. Host-side vsock listener — the "other direction" (from S4)

Mirror image of §13's connect path. Required when you need guest-initiated
connections *into* the host — specifically, vminitd's stdio ports
(`CreateProcessRequest.{stdin,stdout,stderr}` are `optional uint32` vsock
ports the **guest connects back to**).

```rust
define_class!(
    #[unsafe(super(NSObject))]
    struct ListenerDelegate {
        slot: /* publish fd to your reactor */,
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

let listener = unsafe { VZVirtioSocketListener::init(VZVirtioSocketListener::alloc()) };
unsafe { listener.setDelegate(Some(proto_obj_ref)); }
unsafe { virtio_dev.setSocketListener_forPort(&listener, port); }
// setDelegate is WEAK — keep the delegate alive (Box::leak for spikes).
```

Unlike the connect path (§13), we do **not** take
`Retained::retain(conn)` — the delegate just dups the fd and returns.
VZ releases the connection object after we return. Empirically no fd
leaks over single accept-and-return cycles.

---

## 21. vminitd SandboxContext quirks (from S4)

When driving vminitd over gRPC:

1. **Bundle path is implicit**: vminitd uses `/run/container/<containerID>` — NOT anything you pass. Mount your container rootfs at `/run/container/<containerID>/rootfs`. `WriteFile(config.json)` to your own bundle path is ignored — `Bundle.create` writes its own `config.json`.
2. **Spec decoder is strict Codable**: `LinuxNamespace` wants `{type, path}` both present. Use `"path": ""` for unshare semantics — a bare `{"type": "pid"}` hits `DecodingError.keyNotFound: "path"`. Likely same trap on other "optional in runc, required in Swift" fields.
3. **Rootfs must be writable** unless you suppress hostname: `ociAlterations` writes `<root>/etc/hostname` before exec. Either `VZDiskImageStorageDeviceAttachment(readOnly: false)` + `MountRequest.options: ["rw"]` + `spec.root.readonly = false`, **or** set `spec.hostname = ""` (untested).
4. **Stdio is vsock-back, not stream RPC** — see §20. Host must listen on the ports you pass in `CreateProcessRequest.{stdin,stdout,stderr}`.
5. **vmexec error messages are opaque**: `NSPOSIXErrorDomain Code=2` could be ANY of several ENOENT failures (`open("/dev/null")`, `remove("<rootfs>/dev/ptmx")`, `execvpe(args[0])`). `App.Errno(stage:info:)` stashes the stage in `POSIXError.userInfo` which `String(describing:)` drops. Start debugging by verifying `/dev` has real devtmpfs contents; `rbind`-mounting vminitd's own `/dev` is the cheapest fix.

---

## 22. rootfs.ext4 build recipe for spikes (from S4)

Until S5's Rust ext4 writer lands, the portable path to an ext4 container rootfs for spikes:

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

Produces a 64 MiB e2fsck-clean ext4 with busybox in ~5 s. No loop mounts (macOS can't do ext4 loopback). Ready for `echo-inside-container` smoke tests.

---

## 23. EXT4 xattr `e_value_offs` base differs between inline and block (from S5)

The e2fsprogs docs don't spell this out; trace `lib/ext2fs/ext_attr.c::read_xattrs_from_buffer` to see it. Get it wrong and `e2fsck` reports the uselessly-opaque `allocation collision` with no further diagnostics.

- **Inline xattrs**: `e_value_offs` is relative to the **entries region** start (i.e. `inode_inline_xattrs_ptr + sizeof(__u32)`, past the 4-byte magic). The bound is `storage_size = INODE_SIZE - GOOD_OLD - extra_isize - 4 = 92` for 256-byte inodes with extra_isize=32.
- **Block xattrs**: `e_value_offs` is **absolute within the block** (value_start = block buffer base). Bound is `blocksize`.

Use the wrong base → "allocation collision" from e2fsck because the kernel thinks the value spills past the end of the inline region.

---

## 24. Every ext4 directory needs a data block — even lost+found, even empty (from S5)

Empty directories still have `.` and `..` entries. If you skip the "allocate a block" step as an optimization, you get an inode with the right link count but no extents, `i_size = 0`, `i_blocks = 0`. `e2fsck` reports `Inode 11 ref count should be 1`, which means "this thing looks like a regular file with no data, but its link count is 2" — misleading if you're hunting a link-count bug.

Rule: **every directory inode gets at least one block, even if it only holds `.` and `..`**.

---

## 25. Kernel capability quick-ref (extended from S5)

Which bits are compiled in vs. built as modules, by kernel source:

| Capability | `s1-boot` (Ubuntu `linux-image-virtual`) | `s3-vminitd-build` (kata 3.17.0) |
|---|---|---|
| AF_VSOCK | module (`.ko.zst`; see §13 for insmod) | built-in |
| ext4 fs | module | built-in |
| virtio_blk | module | built-in |
| devtmpfs + auto-mount | module | built-in (+ auto) |

**Use the kata kernel** (from S3 or the direct kata release tarball — see §16) for any spike that mounts ext4 or uses vsock. Ubuntu's kernel requires the module-loading dance in the initrd. Cross-spike sharing:

```bash
ln -s ~/tmp/rust-rewrite-spikes/s3-vminitd-build/assets/vmlinux ./assets/vmlinux
```

---

## 26. `bytemuck` feature flags for ext4-shaped types (from S5)

`Superblock`, `Inode256`, and other on-disk structs contain `[u8; 60]`, `[u8; 64]`, and other arrays larger than 32 elements. `#[derive(Pod, Zeroable)]` expands to a trait impl per field, and the default bytemuck impls for `[T; N]` only cover `N ≤ 32`.

```toml
bytemuck = { version = "1.x", features = ["derive", "min_const_generics"] }
```

Without `min_const_generics`: `the trait Pod is not satisfied for [u8; 60]`, which is confusing because the build error doesn't hint at the missing feature.

---

## 27. `bytemuck::pod_read_unaligned` for reading on-disk structs in tests (from S5)

`Superblock` has 8-byte alignment. If you read 1024 bytes from a file into a `[u8; 1024]`, the buffer is 1-byte aligned and `bytemuck::from_bytes::<Superblock>(&buf)` **panics at runtime** with `TargetAlignmentGreaterAndInputNotAligned`.

Use `bytemuck::pod_read_unaligned::<Superblock>(&buf[..1024])` for any test assertion that loads on-disk bytes back into their struct shape.

---

## 28. Rosetta wiring recipe (from S7)

Four steps to run amd64 binaries inside an aarch64 guest via apple/containerization's path. All APIs lifted from `Sources/Containerization/Vminitd+Rosetta.swift` + `ContainerizationOS/Linux/Binfmt.swift`.

**Host side** (before `startWithCompletionHandler`):
```rust
// Probe availability. If NotInstalled, call installRosettaWithCompletionHandler:
// programmatically — on a system where the EULA is already accepted it's
// non-interactive; on a fresh Mac it may show a GUI prompt.
let avail = unsafe { VZLinuxRosettaDirectoryShare::availability() };
// ... then, unconditionally after Installed:
let share = unsafe {
    VZLinuxRosettaDirectoryShare::init(VZLinuxRosettaDirectoryShare::alloc())?
};
let dir_cfg = unsafe {
    VZVirtioFileSystemDeviceConfiguration::initWithTag(
        VZVirtioFileSystemDeviceConfiguration::alloc(),
        &NSString::from_str("rosetta"),
    )
};
unsafe { dir_cfg.setShare(Some(share.as_super())) };
unsafe { config.setDirectorySharingDevices(&NSArray::from_slice(&[dir_cfg.as_super()])) };
```

**Guest side** (over vminitd's SandboxContext gRPC, after VM start):
1. `Mkdir("/run/rosetta")` (plain RPC).
2. `Mount { fstype: "virtiofs", source: "rosetta", destination: "/run/rosetta", options: [] }`.
3. `SetupEmulator { binary_path: "/run/rosetta/rosetta", name: "x86_64", type: "M", offset: "", magic: <amd64 ELF magic>, mask: <amd64 ELF mask>, flags: "CF" }`.

`magic`/`mask` values: lift verbatim from the Swift `Binfmt.Entry.amd64()` static. The **`F` flag (fix-binary)** matters — it keeps the rosetta fd alive across the container's mount-namespace switch, so no per-container bind-mount into the container's rootfs is needed. Just set the process spec's `args[0]` to an amd64 binary and it Just Works.

Acceptance smoke test: `/bin/uname -m` (amd64 busybox) → `x86_64\n` stdout streamed via the inverse-vsock path from §20.

---

## 29. vmnet networking from Rust on macOS 26+ (from S6)

apple/container's networking model (IP-per-container via `VZVmnetNetworkDeviceAttachment` shared mode) is usable from Rust with **ad-hoc codesigning and no extra entitlements**. This is macOS 26's relaxation — don't port old lore from 13/14.

### Entitlements matrix — what actually works

| Attachment | macOS 26+ | Ad-hoc sign | `com.apple.security.virtualization` | `com.apple.vm.networking` | Works? |
|---|---|---|---|---|---|
| `VZNATNetworkDeviceAttachment` | ✓ | ✓ | required | — | ✓ |
| `VZVmnetNetworkDeviceAttachment` (shared) | ✓ | ✓ | required | — | ✓ |
| `VZVmnetNetworkDeviceAttachment` (shared) | ✓ | ✓ | required | + | ❌ AMFI: signature invalid |
| `VZVmnetNetworkDeviceAttachment` (shared) | ✓ | dev cert | required | + | ❌ amfid: no matching profile |
| `VZBridgedNetworkDeviceAttachment` | ✓ | needs profile | required | required | paid dev program + profile |

**Counter-intuitive**: adding `com.apple.vm.networking` to get vmnet is *wrong*. AMFI refuses any binary that claims a restricted entitlement without a matching provisioning profile. For shared-mode vmnet, leave the extra key out entirely.

### The `objc2-virtualization` 0.3.2 papercut

`VZVmnetNetworkDeviceAttachment` has `init` / `new` marked unavailable in the generated bindings. The real initializer is `-initWithNetwork:` taking a `VZVmnetNetwork *`. You reach it via `msg_send!`, and the `VZVmnetNetwork` type has to encode its ivar layout as `^{vmnet_network=}`. That needs a handful of custom bindings:

```rust
// Stub type for vmnet_network (the private ivar struct).
#[repr(C)] struct VmnetNetworkStruct { _p: [u8; 0] }
unsafe impl objc2::encode::RefEncode for VmnetNetworkStruct {
    const ENCODING_REF: objc2::encode::Encoding =
        objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Struct("vmnet_network", &[]));
}

// Then VZVmnetNetwork extends NSObject. extern_class! + bind:
//   fn networkWithMode:...(...)   // builder
//   fn macAddress() -> VZMACAddress
// See s6-vmnet-entitlements/src/main.rs for the ~40 LOC wrapper.
```

A shared-mode vmnet network is constructed via `+[VZVmnetNetwork sharedNetwork]` (singleton) or by creating an isolated one with its own subnet. The attachment wraps that network.

### What Phase 1's library must still solve

S6 proved the *host-side attachment* works. Two things still need doing in `core` / `vmm`:

1. **Guest DHCP** — vmnet runs DHCP on its side, but the container still has to pick up the lease. vminitd handles this via `ContainerizationNetlink`; the library passes network config through the SandboxContext.
2. **Multiple containers on the same network** — `VZVmnetNetworkDeviceAttachment` binds one VM to one vmnet network. For IP-per-container, either (a) one VM per container with a shared `VZVmnetNetwork`, or (b) multiple virtio-net devices inside one VM with vminitd mapping each to a container's netns. apple/container does (a) — keep it simple.

---

## 30. `include_bytes!` cost, quantified (from S8)

Observed on M-series macOS 26.3 / rustc 1.95-nightly with 131 MiB and 384 MiB blobs. If you're embedding anything into a Rust binary at compile time, budget accordingly:

- **Cold-build time scales ~0.1 s per MiB of embedded blob**. 131 MiB adds ~5 s; 384 MiB adds ~12 s.
- **Warm-rebuild time scales ~0.2 s per MiB** when the blob's *enclosing crate* is touched — because rustc re-links. Touching a leaf downstream doesn't pay this.
- **Peak RSS during codegen ≈ 18× blob size.** 384 MiB embed → 7.2 GB peak; 131 MiB → ~2.5 GB. On a 16 GB laptop with an IDE running, the 384 MiB case swaps.
- **`ld` dead-strips unreferenced `include_bytes!` consts** in the linked binary. A downstream crate that pulls the library but never touches the blob pays ~422 KB, not 131 MB. `.rlib` on disk still carries the full bytes though — target-dir bloat is real.
- **`cargo check` is immune** (no codegen / link). Day-to-day feedback loop stays fast; only release builds pay.

### Tactical conclusion

Put the embed in its own tiny leaf crate that nobody is likely to touch. That way only whoever actually *uses* the embedded blob pays the link tax, and a change to any other crate in the workspace doesn't trigger a slow relink.

For this project specifically: embed the ~131 MiB `vminitd` ELF, not the ~384 MiB `init.block`. The `ext4` crate (S5) synthesizes `init.block` on-host from the vminitd ELF. 2.4× cold-build speedup and 2.9× final-binary-size cut, for free.

---

## 31. vmnet attachments require a MAC address (from S9)

`VZVirtualMachineConfiguration::validateWithError()` silently fails for a `VZVirtioNetworkDeviceConfiguration` with a `VZVmnetNetworkDeviceAttachment` if you don't set a MAC. S6 didn't hit this because its guest never actually brought eth0 up — configuration validation was deferred.

```rust
let mac = unsafe { VZMACAddress::randomLocallyAdministeredAddress() };
net_cfg.setMACAddress(&mac);
```

`randomLocallyAdministeredAddress` gives you a 48-bit address with the "locally administered" bit set — RFC-correct for ephemeral/synthetic devices. Don't try to invent one by hand; the class handles the bit-setting and uniqueness.

---

## 32. vminitd's network RPC sequence + the CIDR-string gotcha (from S9)

After `Mount` + `CreateProcess` but before `StartProcess`, issue these five RPCs to give the container a reachable IP. Order matters — bringing `eth0` up before the address is assigned works but is a race against the container's first outbound packet.

```
IpLinkSet       { interface: "lo",   up: true }
IpAddrAdd       { interface: "eth0", ipv4Address: "192.168.70.2/24" }
IpLinkSet       { interface: "eth0", up: true }
IpRouteAddDefault { interface: "eth0", ipv4Gateway: "192.168.70.1" }
ConfigureDns    { location: "<rootfs path>",
                  nameservers: ["192.168.70.1", "8.8.8.8", "1.1.1.1"] }
```

**The trap**: `IpAddrAdd.ipv4Address` is **CIDR-formatted** (`"<ip>/<prefix>"`), not a bare IP. vminitd's Swift side parses it with `CIDRv4(string:)` which requires the prefix. Passing `"192.168.70.2"` alone silently rejects with a confusing deserialize error.

All five complete in <100 ms total over gRPC. Derived from `Sources/Containerization/LinuxContainer.swift:594-617` + `Vminitd.swift`.

**First-ping warmup**: the host's first `ping <container-ip>` may return "No route to host" — vmnet's kernel-side routing takes ~500 ms to settle after the attachment comes up. One retry after a brief sleep always succeeds. Budget for this in any reachability check.

**Container's `ping` needs `CAP_NET_RAW`** in the OCI spec's `process.capabilities.bounding/permitted/effective`, because busybox's `ping` uses a raw socket. Easy to forget.

---

## 33. vmnet subnet discovery from Rust (from S9)

Before issuing the RPCs in §32, you need to know the subnet + gateway + container IP to hand to vminitd. vmnet picks the subnet — query it:

```rust
extern "C" {
    fn vmnet_network_get_ipv4_subnet(
        network: *const c_void,
        subnet: *mut vmnet_ipv4_subnet,
    ) -> c_int;
}

#[repr(C)]
struct vmnet_ipv4_subnet {
    addr: [u8; 4],         // network address, e.g. 192.168.70.0
    netmask: [u8; 4],      // e.g. 255.255.255.0
}
```

From there: gateway is typically `subnet_addr | 0x01` (vmnet convention); container IP is the next unused address in the subnet (library tracks allocations).

The objc2-virtualization side of `VZVmnetNetwork` gives you the `vmnet_network_ref` pointer via `-[VZVmnetNetwork _network]` (private ivar, reached via `msg_send!`). ~30 LOC helper in `s9-vmnet-reachability/src/main.rs` productizes this into a `vmnet_subnet()` function returning `(Ipv4Addr, u8)` (network + prefix). Lifts verbatim.
