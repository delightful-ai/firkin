# Container surface

> Covers: `ContainerBuilder`, `Container`, `Process`, `ExecConfig`, drop/cancellation semantics at the container level.
>
> Prerequisites: [`README.md`](./README.md) landing narrative. Section 2's examples anchor the types used here.

---

## 1. Overview

The container surface is the heart of the library. A user who never touches the `vmm`, `oci`, or `ext4` crates still touches `Container` — it's the single entry point for "run my thing in a microVM" workflows.

The surface has three visible types:

| Type | Role |
|---|---|
| `ContainerBuilder<Vm, S>` | Pre-spawn configuration. Typestate-parameterized on VM context and rootfs-readiness. Terminals: `output()` / `status()` / `spawn()`. |
| `Container<S>` | A live container handle. Parameterized on stdio shape (`Streams` vs `Pty`; D-025). Lifecycle operations are runtime-checked. |
| `Process<S>` | A process exec'd into a live `Container`, post-spawn. Same `Streams`/`Pty` parameter; narrower surface than `Container`. |

And two configuration types:

| Type | Role |
|---|---|
| `ExecConfig` | Process-level configuration passed to `Container::exec(...)`. |
| `Output` | Returned by `output()` on the builder and `wait_with_output()` on handles — exit status + captured stdout + captured stderr. |

Value types (`Rootfs`, `Mount`, `Stdio`, `PtyConfig`, `User`, `Size`, `LinuxCapabilities`, `LinuxRlimit`, `Signal`, etc.) are defined in [`04-value-types.md`](./04-value-types.md). Error types in [`05-error-model.md`](./05-error-model.md). Cross-cutting drop/cancellation discipline in [`09-cross-cutting.md`](./09-cross-cutting.md).

---

## 2. `ContainerBuilder`

### 2.1 Typestate model

Two axes of typestate, applied orthogonally:

- **Rootfs presence** (`S` parameter): `Init` → `Ready` → `ReadyPty`. Only `Ready` and `ReadyPty` variants expose `.spawn()`. Forgetting to call `.rootfs()` is a compile error.
- **VM context** (`Vm` parameter): `ImplicitVm` (returned by `Container::builder(id)`) or `OnVm<'vm>` (returned by `vm.container(id)`). VM-level configuration methods (`.network()`, `.virtiofs_share()`, `.nested_virtualization()`, `.boot_log()`, `.dns()`, `.hosts()`) exist only on `ImplicitVm` — calling them on an `OnVm<'vm>` builder is a compile error because those choices were already made by the parent VM.

Type-level signatures:

```rust
pub struct ContainerBuilder<Vm: VmContext, S: BuilderState = Init> { /* private */ }

// Sealed trait — we own both implementers; users can't implement.
mod sealed { pub trait Sealed {} }

pub trait VmContext: sealed::Sealed { /* associated items for impl */ }
pub trait BuilderState: sealed::Sealed { /* marker */ }

// VM context markers:
pub struct ImplicitVm;                      // builder owns a single-use VM on spawn
pub struct OnVm<'vm> { /* private reference */ }

// Builder-state markers:
pub struct Init;       // rootfs not set
pub struct Ready;      // rootfs set; default stream-stdio
pub struct ReadyPty;   // rootfs set; pty requested, stderr forbidden

impl VmContext for ImplicitVm {}
impl VmContext for OnVm<'_> {}
impl BuilderState for Init {}
impl BuilderState for Ready {}
impl BuilderState for ReadyPty {}
```

The sealed traits enforce that users can't create custom typestate markers. Only the library's own markers parameterize the builder.

### 2.2 Entry points

```rust
impl Container {
    /// Start a builder with an implicit single-use VM. The builder owns the VM
    /// configuration and boots it inside `.spawn()`. Tear-down on `container.stop()`
    /// destroys the VM along with the container.
    pub fn builder(id: impl Into<ContainerId>) -> ContainerBuilder<ImplicitVm, Init>;
}

// Defined in firkin-core per D-018 — NOT an inherent impl on VirtualMachine<Running>,
// because ContainerBuilder is a `core` type and VirtualMachine lives in `vmm`, which
// cannot import `core` (cycle). The trait is sealed; only `VirtualMachine<Running>`
// implements it. Re-exported from `firkin` so `use firkin::*;` picks it up.
pub trait CoreContainerFactory: sealed::Sealed {
    /// Start a builder that will spawn onto this already-booted VM. Multiple
    /// containers can share a VM this way (Q2/C "pods without a Pod" pattern).
    ///
    /// The builder borrows `&self`, but the resulting `Container` holds its own
    /// shared reference; see `container_shared()` for Arc-based cross-scope sharing.
    fn container<'a>(&'a self, id: impl Into<ContainerId>) -> ContainerBuilder<OnVm<'a>, Init>;

    /// Same as `container()` but returns a builder whose Container holds an Arc<Self>
    /// internally, allowing the resulting Container to outlive the VM variable.
    /// Requires `self: &Arc<VirtualMachine<Running>>`. Details in 02-vm-surface.md.
    fn container_shared(self: &Arc<Self>, id: impl Into<ContainerId>)
        -> ContainerBuilder<OnVmArc, Init>;
}

impl sealed::Sealed for VirtualMachine<Running> {}
impl CoreContainerFactory for VirtualMachine<Running> { /* ... */ }
```

`ContainerId` is a newtype over `String` with validation — see [`04-value-types.md § newtypes`](./04-value-types.md); it lives in `firkin-types` (D-015) and is re-exported from `firkin`. Calling `builder("bad/id")` fails at construction with `Error::Config(ConfigError::InvalidContainerId)`. For compile-time validated literals, use the `container_id!("foo")` macro (D-027).

**v0.1 constraint on `vm.container()` — enforced by types** ([D-019](../DECISIONS.md#d-019--multi-container-vms-require-pre-declared-block-devices-in-v01) + [D-022](../DECISIONS.md#d-022--blockdeviceid-replaces-stringly-paired-block_devicepath--rootfsext4_imagepath) + [D-023](../DECISIONS.md#d-023--rootfs-split-by-vm-context-rootfs-vs-vmrootfs)): on `OnVm`/`OnVmArc` builders, `.rootfs()` accepts only `impl Into<VmRootfs>`. In v0.1 that means either `Rootfs::block_device(id)` where `id: BlockDeviceId` was produced by `VmConfig::builder().block_device(path)`, or `VmRootfs::guest_path(path)` where `path` is an already-mounted rootfs directory inside the running guest. The `Rootfs` enum's `OciBundle`/`Ext4Image`/`RawBlock` variants are not convertible to `VmRootfs` — using them here is a compile error, not a runtime error at spawn time. Users who want elastic pods preboot one pod-store disk, materialize rootfs directories inside it, and start containers from guest paths. Runtime VZ storage attach is not the first pod path.

### 2.3 Configuration methods — grouped by concern

Every method consumes `self` and returns a builder. This makes chains clean and guarantees the builder can't be used after an error-returning setter — which also lets us do more validation in setters without partial-state problems.

#### 2.3.1 Rootfs gate

Two impls, one per VM context, per [D-023](../DECISIONS.md#d-023--rootfs-split-by-vm-context-rootfs-vs-vmrootfs). Both advance typestate from `Init` to `Ready`.

```rust
// ImplicitVm: full Rootfs enum — the VM is about to be constructed, any variant works.
impl ContainerBuilder<ImplicitVm, Init> {
    pub fn rootfs(self, r: impl Into<Rootfs>) -> ContainerBuilder<ImplicitVm, Ready>;
}

// OnVm / OnVmArc: only VmRootfs — the VM is already booted, so the rootfs
// must already exist as a block device or guest path.
impl<'vm> ContainerBuilder<OnVm<'vm>, Init> {
    pub fn rootfs(self, r: impl Into<VmRootfs>) -> ContainerBuilder<OnVm<'vm>, Ready>;
}
impl ContainerBuilder<OnVmArc, Init> {
    pub fn rootfs(self, r: impl Into<VmRootfs>) -> ContainerBuilder<OnVmArc, Ready>;
}
```

See [`04-value-types.md § Rootfs`](./04-value-types.md) for the `Rootfs` enum (`Ext4Image`, `OciBundle`, `RawBlock`) and `VmRootfs` (`BlockDevice` or `GuestPath`).

#### 2.3.2 Process configuration (available on all states)

```rust
impl<Vm: VmContext, S: BuilderState> ContainerBuilder<Vm, S> {
    pub fn command<I, A>(self, args: I) -> Self
    where I: IntoIterator<Item = A>, A: Into<OsString>;

    pub fn env(self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self;

    pub fn envs<I, K, V>(self, vars: I) -> Self
    where I: IntoIterator<Item = (K, V)>, K: Into<OsString>, V: Into<OsString>;

    pub fn working_dir(self, path: impl Into<PathBuf>) -> Self;

    pub fn user(self, user: impl Into<User>) -> Self;

    pub fn hostname(self, hostname: Hostname) -> Self;

    pub fn sysctl(self, key: impl Into<String>, value: impl Into<String>) -> Self;
}
```

`OsString` rather than `String` throughout so non-UTF-8 arguments and environment values work. Linux processes can have non-UTF-8 `argv` / `environ`; swallowing that silently would be a lie ([`type_design.md § uncomfortable truths`](../../../../../../src/personal/beads-rs/docs/philosophy/type_design.md)).

#### 2.3.3 Resources

```rust
impl<Vm: VmContext, S: BuilderState> ContainerBuilder<Vm, S> {
    pub fn cpus(self, n: NonZeroU32) -> Self;             // default NonZeroU32::new(4).unwrap()
    pub fn memory(self, size: Size) -> Self;              // default Size::gib(1); D-026 (no memory_mib)
    pub fn rlimit(self, r: LinuxRlimit) -> Self;          // append; can be called many times
    pub fn rlimits(self, rs: impl IntoIterator<Item = LinuxRlimit>) -> Self;
    pub fn capabilities(self, caps: LinuxCapabilities) -> Self;   // default: LinuxCapabilities::default_oci()
    pub fn no_new_privileges(self, on: bool) -> Self;     // default false
}
```

Validation on `build()`:
- Zero CPUs is a compile error (type is `NonZeroU32`); no runtime variant exists.
- `memory` below VZ minimum (128 MiB) → `Error::Config(ConfigError::MemoryTooSmall)`.

#### 2.3.4 Stdio — streams mode (default)

```rust
impl<Vm: VmContext> ContainerBuilder<Vm, Ready> {
    pub fn stdin(self, s: Stdio) -> Self;                 // default Stdio::null()
    pub fn stdout(self, s: Stdio) -> Self;                // default Stdio::null()
    pub fn stderr(self, s: Stdio) -> Self;                // default Stdio::null()

    /// Transition to pty mode. After this call, `.stderr()` is a compile error.
    ///
    /// Accepts anything that converts to `PtyConfig`, including:
    /// - `(cols, rows)` tuples via `impl From<(u16, u16)> for PtyConfig`
    /// - `PtyConfig::new(cols, rows)` for explicit construction
    /// - `PtyConfig::default()` for 80×24
    pub fn pty(self, config: impl Into<PtyConfig>) -> ContainerBuilder<Vm, ReadyPty>;
}
```

**Defaults rationale**: `Stdio::null()` is the only default that:
- Never deadlocks (unlike `Piped` without drain).
- Never implicitly captures host stdout (unlike `Inherit` for library use).
- Is trivially replaceable by an explicit call for users who want something else.

See [`03-stdio-pty-vsock.md`](./03-stdio-pty-vsock.md) for the full `Stdio` enum and handle story.

#### 2.3.5 Stdio — pty mode

```rust
impl<Vm: VmContext> ContainerBuilder<Vm, ReadyPty> {
    pub fn stdin(self, s: Stdio) -> Self;                 // stdin still usable separately;
                                                          // default Stdio::null()
    // .stdout() is not available — pty provides both directions.
    // .stderr() is not available — pty combines stdout and stderr.
}
```

**Why stdin is still independent**: a pty gives you one combined stdout/stderr duplex, but stdin can still come from a separate source (e.g., `Stdio::null()` to block reads, `Stdio::inherit()` to forward host stdin, `Stdio::piped()` to write programmatically). Most interactive uses want stdin-from-pty automatically — but programmatic "pipe in this script" cases want a separate stdin.

If `.stdin(Stdio::null())` is set along with `.pty(…)`, the container's init sees pty output but can't read input. That's a valid configuration (one-shot capture of pty output for logging, etc.).

#### 2.3.6 Mounts

```rust
impl<Vm: VmContext, S: BuilderState> ContainerBuilder<Vm, S> {
    pub fn mount(self, m: Mount) -> Self;                          // append
    pub fn mounts(self, ms: impl IntoIterator<Item = Mount>) -> Self;

    /// Replace the default mount set (proc, sysfs, devtmpfs, devpts, mqueue, /dev/shm, cgroup2)
    /// with a custom list. If unset, `Mount::defaults()` is used.
    pub fn default_mounts(self, ms: Vec<Mount>) -> Self;

    /// Attach a writable upper layer over the (read-only) rootfs via overlayfs.
    /// Must be a block-device Mount; `Mount::virtiofs` etc. won't work here.
    pub fn writable_layer(self, m: Mount) -> Self;
}
```

See [`04-value-types.md § Mount`](./04-value-types.md) for the full `Mount` enum (`Virtiofs`, `Bind`, `Tmpfs`, `Overlay`, `Proc`, `Sysfs`, `Devtmpfs`, `Devpts`, `Mqueue`, `Cgroup2`, `Custom`).

#### 2.3.7 OCI integration

```rust
impl<Vm: VmContext, S: BuilderState> ContainerBuilder<Vm, S> {
    /// Pull defaults from an OCI `ImageConfig` into this builder. Populates:
    /// - command (from entrypoint + cmd, concatenated as OCI runtime spec dictates)
    /// - env (from config.env)
    /// - working_dir (from config.working_dir, falling back to "/")
    /// - user (from config.user, parsed via User::named or User::numeric)
    ///
    /// Subsequent explicit setters override field-by-field.
    pub fn image_config(self, cfg: &oci::ImageConfig) -> Self;
}
```

This is how `Rootfs::OciBundle(bundle)` + `image_config(bundle.config())` composes cleanly — the rootfs provides the files, the config provides the defaults.

#### 2.3.8 Container-specific knobs

```rust
impl<Vm: VmContext, S: BuilderState> ContainerBuilder<Vm, S> {
    /// Configure seccomp policy. Two shapes supported:
    /// - `Seccomp` struct: structured policy typed field-by-field.
    /// - raw JSON: via `seccomp_profile_json` (escape hatch for policies we don't type yet).
    pub fn seccomp(self, s: Seccomp) -> Self;
    pub fn seccomp_profile_json(self, json: impl Into<String>) -> Self;

    /// SELinux label, passed through to runc. Inert unless the guest kernel has
    /// CONFIG_SECURITY_SELINUX enabled. The bundled kernel does not; users supplying
    /// their own kernel via VmConfig::kernel() may use this.
    pub fn selinux_label(self, label: impl Into<String>) -> Self;

    /// AppArmor profile name, same caveat as selinux_label.
    pub fn apparmor_profile(self, profile: impl Into<String>) -> Self;

    /// Run the container with a minimal init process (bind-mount vminitd at /.cz-init)
    /// that forwards signals and reaps zombies. Useful for processes that are PID 1
    /// and don't implement init-style behavior themselves (e.g., most apps).
    pub fn use_init(self, on: bool) -> Self;

    /// Relay a Unix domain socket between host and guest. See UnixSocketConfig for direction.
    pub fn socket(self, s: UnixSocketConfig) -> Self;
    pub fn sockets(self, ss: impl IntoIterator<Item = UnixSocketConfig>) -> Self;
}
```

#### 2.3.9 VM-level config — only on `ImplicitVm`

These methods configure the implicit single-use VM. They exist on the `ImplicitVm` builder only; calling them on an `OnVm<'vm>` builder is a compile error because the VM is already booted and its config is fixed.

```rust
impl<S: BuilderState> ContainerBuilder<ImplicitVm, S> {
    pub fn network(self, n: Network) -> Self;
    pub fn networks(self, ns: impl IntoIterator<Item = Network>) -> Self;
    pub fn virtiofs_share(self, tag: impl Into<VirtiofsTag>, host: impl Into<PathBuf>) -> Self;
    pub fn rosetta(self, on: bool) -> Self;                // default false; explicit opt-in
    pub fn nested_virtualization(self, on: bool) -> Self;  // default false
    pub fn boot_log(self, bl: BootLog) -> Self;
    pub fn dns(self, d: DnsConfig) -> Self;
    pub fn hosts(self, h: HostsConfig) -> Self;
    pub fn kernel(self, k: KernelImage) -> Self;           // escape hatch; default bundled
    pub fn cmdline_extra(self, s: impl Into<String>) -> Self;
}
```

See [`02-vm-surface.md`](./02-vm-surface.md) for the `VmConfig` equivalents used with explicit VMs, and [`04-value-types.md`](./04-value-types.md) for `Network`, `VirtiofsTag`, `BootLog`, `DnsConfig`, `HostsConfig`, `KernelImage`.

### 2.4 Defaults at a glance

A fully-defaulted container:

```rust
Container::builder("foo")
    .rootfs(Rootfs::ext4_image("/tmp/foo.ext4"))
    .spawn().await?
```

implicitly sets:

| Concern | Default |
|---|---|
| cpus | 4 |
| memory | `Size::gib(1)` |
| command | `[]` (empty; fails at guest-side OCI runtime unless `image_config` provided one) |
| env | `["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"]` |
| working_dir | `/` |
| user | `User::root()` (uid=0, gid=0) |
| capabilities | `LinuxCapabilities::default_oci()` — runc's 14-capability default |
| no_new_privileges | false |
| rlimits | `[]` |
| stdin / stdout / stderr | `Stdio::null()` |
| mounts | `Mount::defaults()` — the Linux-runc standard set |
| writable_layer | none |
| use_init | false |
| seccomp | unset (runc applies its default filter if none specified) |
| selinux_label / apparmor_profile | unset |
| network (ImplicitVm only) | `Network::nat()` |
| virtiofs_shares (ImplicitVm only) | `[]` |
| rosetta (ImplicitVm only) | false; explicit opt-in for amd64-on-arm64 |
| nested_virtualization | false |
| kernel | bundled |

### 2.5 Terminal methods: `spawn` / `output` / `status`

Three terminal methods per [D-021](../DECISIONS.md#d-021--containerbuilderoutput---status-are-the-one-shot-terminals). Pick by intent, not by convention.

```rust
impl<Vm: VmContext> ContainerBuilder<Vm, Ready> {
    /// Spawn and return a live Container<Streams> handle. Use when you want to
    /// interact with the running container — stream stdout, write stdin,
    /// exec additional processes, pause/resume, etc.
    pub async fn spawn(self) -> Result<Container<Streams>, core::Error>;

    /// One-shot: spawn, drain stdout+stderr, wait for exit, return `Output`.
    /// Auto-pipes stdout and stderr if the user hasn't set them. Equivalent to
    /// `self.stdout_pipe_if_unset().stderr_pipe_if_unset().spawn().await?.wait_with_output().await`,
    /// but in one call with no deadlock footgun. Mirrors `tokio::process::Command::output`.
    pub async fn output(self) -> Result<Output, core::Error>;

    /// One-shot: spawn, wait for exit, return `ExitStatus`. Stdio defaults
    /// remain `Stdio::null()` unless the user overrode them. Mirrors
    /// `tokio::process::Command::status`.
    pub async fn status(self) -> Result<ExitStatus, core::Error>;
}

impl<Vm: VmContext> ContainerBuilder<Vm, ReadyPty> {
    /// Spawn and return a live Container<Pty> handle with an infallible pty accessor.
    pub async fn spawn(self) -> Result<Container<Pty>, core::Error>;

    /// One-shot: spawn, drain pty output (combined stdout+stderr on pty),
    /// wait for exit, return `Output` with the combined stream in `stdout`
    /// and an empty `stderr`.
    pub async fn output(self) -> Result<Output, core::Error>;

    /// One-shot: spawn, wait, return `ExitStatus`.
    pub async fn status(self) -> Result<ExitStatus, core::Error>;
}
```

**Which to reach for:**

| You want… | Method |
|---|---|
| Exit code + captured stdout/stderr, one line | `output()` |
| Exit code only, don't care about stdout | `status()` |
| A handle you can read/write/exec/pause on | `spawn()` |

**`spawn` does, in order** (the other terminals delegate to this pipeline then wait):

1. **Validate** the builder. Any validation errors (zero cpus, missing command and no image_config, bad rootfs path, etc.) land here as `Error::Config(…)`.
2. **Assemble** VM config (`ImplicitVm` case) or reuse `vm` (`OnVm` case).
3. **Boot** the VM if `ImplicitVm`; or wait for VM ready if `OnVm` on a just-booted VM.
4. **Ensure** `init.block` is available (D-003 synthesis if not cached).
5. **Mount** the rootfs via vminitd RPCs; apply default-mount set; apply user-provided mounts.
6. **Configure** networking (interfaces + IPs + routes + DNS).
7. **Write** the OCI runtime spec derived from the builder.
8. **CreateProcess** for the init process via vminitd; set up stdio listeners (D-005 inverse-vsock).
9. **StartProcess**.
10. Return `Container<Streams>` / `Container<Pty>`. The init process is running.

Partial failures in steps 3–9 cascade cleanup: stop any launched relays, stop the VM (if ImplicitVm), return the error with context. No half-alive state left behind.

**`output()` + `status()` auto-pipe semantics** (what "drain stdout+stderr" means for `output`):

- `output()` on a `Ready` builder sets `stdout = Stdio::piped()` and `stderr = Stdio::piped()` if the user hadn't overridden them. If the user had explicitly set one to `Stdio::null()` or `Stdio::inherit()`, the override wins and the corresponding `Output` field is empty.
- `output()` on a `ReadyPty` builder drains the pty stream into `Output.stdout`; `Output.stderr` is always empty (pty combines both).
- `status()` never auto-pipes. Defaults stay `Stdio::null()` (no capture) unless the user set them.

This closes the pit-of-failure where a user writes `.stdout(Stdio::piped()).spawn().await?.wait().await?` and the guest deadlocks on a write to an undrained pipe.

---

## 3. `Container<S>` — the live handle

```rust
pub struct Container<S: ContainerStdio = Streams> { /* private */ }

// Sealed markers (users can't implement ContainerStdio).
mod sealed { pub trait Sealed {} }
pub trait ContainerStdio: sealed::Sealed {}

pub struct Streams;     // stdin/stdout/stderr shape
pub struct Pty;         // stdin + combined pty shape

impl sealed::Sealed for Streams {}
impl sealed::Sealed for Pty {}
impl ContainerStdio for Streams {}
impl ContainerStdio for Pty {}
```

Per [D-025](../DECISIONS.md#d-025--containers-typestate-streams-vs-pty), `Container` carries the stdio shape forward from the builder. `ContainerBuilder<_, Ready>::spawn()` returns `Container<Streams>`; `ContainerBuilder<_, ReadyPty>::spawn()` returns `Container<Pty>`. Methods that depend on stdio shape live on the specialized impls; everything else is shared.

`Container<S>` is `Send + Sync` (Arc-internal state; see [`09-cross-cutting.md § Send/Sync`](./09-cross-cutting.md)). Lifecycle operations (`wait`, `stop`, etc.) are runtime-checked against an internal `tokio::sync::Mutex<State>`; `Error::NotRunning { container: ContainerId }` or `Error::ContainerExited { … }` on state mismatch.

### 3.1 Identity and introspection (shared across S)

```rust
impl<S: ContainerStdio> Container<S> {
    pub fn id(&self) -> &ContainerId;

    /// The guest-side init process PID. Returns None between spawn and the first
    /// status report from vminitd (a narrow window; usually available immediately).
    pub fn pid(&self) -> Option<i32>;

    /// Returns the VM this container runs on. Use for introspection; most users
    /// don't need this because container operations cover the common paths.
    pub fn virtual_machine(&self) -> VirtualMachineHandle<'_>;
}
```

`VirtualMachineHandle<'_>` is a borrowing view that exposes VM-level queries (`vm_handle.id()`, `vm_handle.statistics()`, etc.) without consuming or allowing mutating ops on the VM from the Container handle.

### 3.2 Stdio access — per-shape

#### 3.2.1 `Container<Streams>` — stdin / stdout / stderr

```rust
impl Container<Streams> {
    pub fn stdin(&mut self)  -> Option<&mut ChildStdin>;
    pub fn stdout(&mut self) -> Option<&mut ChildStdout>;
    pub fn stderr(&mut self) -> Option<&mut ChildStderr>;

    pub fn take_stdin(&mut self)  -> Option<ChildStdin>;
    pub fn take_stdout(&mut self) -> Option<ChildStdout>;
    pub fn take_stderr(&mut self) -> Option<ChildStderr>;
}
```

`Some` iff the builder set the corresponding `Stdio::Piped`. `None` for `Stdio::Null`, `Stdio::Inherit`, or after `take_*`. Same Option-means-one-of-two-things as `tokio::process::Child` — inherited and documented. No `.pty()` method exists on `Container<Streams>`: calling it is a "method not found" compile error.

#### 3.2.2 `Container<Pty>` — stdin + pty

```rust
impl Container<Pty> {
    pub fn stdin(&mut self)  -> Option<&mut ChildStdin>;
    pub fn take_stdin(&mut self) -> Option<ChildStdin>;

    /// Infallible — the builder typestate (`ReadyPty`) guaranteed a pty exists.
    /// Returns a mutable reference you can read, write, split, or resize.
    pub fn pty(&mut self) -> &mut Pty;
}
```

No `stdout` / `stderr` accessor: a pty combines both into one duplex stream, which `pty()` returns. No `take_pty` in v1: the pty is tied to the live container (lifetime to `&mut Container<Pty>`). Users who want owned halves call `tokio::io::split(c.pty())`.

All handles are `AsyncRead`/`AsyncWrite` as appropriate, `Send + Unpin + 'static`. Full types and semantics in [`03-stdio-pty-vsock.md`](./03-stdio-pty-vsock.md).

### 3.3 Lifecycle operations (shared across S)

```rust
impl<S: ContainerStdio> Container<S> {
    /// Wait for the init process to exit. Does NOT drain stdio; see wait_with_output().
    /// Multiple concurrent calls are allowed; they all resolve with the same ExitStatus.
    pub async fn wait(&mut self) -> Result<ExitStatus, core::Error>;

    /// Wait for exit AND drain stdout + stderr buffers. Consumes `self`.
    /// Only available if stdout and/or stderr are Piped (otherwise stdout/stderr are empty).
    pub async fn wait_with_output(self) -> Result<Output, core::Error>;

    /// Send a signal to the init process inside the guest. Returns as soon as the
    /// kill RPC completes; the process may still be running.
    pub async fn kill(&mut self, signal: Signal) -> Result<(), core::Error>;

    /// Graceful shutdown: SIGTERM → grace period → SIGKILL → unmount rootfs → VM stop
    /// (if ImplicitVm) or cleanup (if OnVm). Consumes `self`.
    ///
    /// Idempotent: calling stop() on an already-stopped container returns the exit status
    /// from the first stop; calling it during shutdown waits for the in-flight shutdown.
    ///
    /// Cancels (via the internal cascading token) any in-flight operations on this
    /// container (exec, copy_in/out, etc.). See 09-cross-cutting.md.
    pub async fn stop(self) -> Result<ExitStatus, core::Error>;

    /// Grace period before stop() escalates SIGTERM to SIGKILL. Default: 10s.
    pub async fn stop_with_grace(self, grace: std::time::Duration)
        -> Result<ExitStatus, core::Error>;
}
```

### 3.4 Pause and resume

```rust
impl<S: ContainerStdio> Container<S> {
    /// Pause the underlying VM. Delegates to VirtualMachine::pause(). If multiple
    /// containers share a VM (OnVm case), this pauses all of them together.
    ///
    /// Errors with Error::Cancelled{VmStopped} if the VM is already stopped.
    pub async fn pause(&mut self) -> Result<(), core::Error>;

    /// Resume a paused VM. Counterpart to pause().
    pub async fn resume(&mut self) -> Result<(), core::Error>;

    pub fn is_paused(&self) -> bool;
}
```

Pause/resume at the container level delegates to the VM because VZ pause is VM-scoped — you can't pause one container while others on the same VM keep running. Documented explicitly: for users in the `OnVm` case, `container.pause()` affects every container on that VM.

### 3.5 Inspection

```rust
impl<S: ContainerStdio> Container<S> {
    /// Request resource statistics for the categories specified.
    /// Omitted categories are None in the returned struct.
    pub async fn statistics(&self, categories: StatCategory)
        -> Result<ContainerStatistics, core::Error>;
}
```

`StatCategory` is a bitflags enum; `StatCategory::all()` for everything. See [`04-value-types.md § Statistics`](./04-value-types.md).

### 3.6 In-container operations

```rust
impl<S: ContainerStdio> Container<S> {
    /// Exec an additional process inside this container. Returns a running Process<E>
    /// where E is the stdio shape of the exec config (Streams or Pty — E is not tied
    /// to S; a Container<Streams> can exec a Process<Pty> and vice versa).
    ///
    /// Unlike Swift's create-then-start split, this library returns a started Process
    /// directly. The ExecConfig builder covers all per-process knobs.
    pub async fn exec<E: ContainerStdio>(
        &mut self,
        id: impl Into<ProcessId>,
        config: ExecConfig<E>,
    ) -> Result<Process<E>, core::Error>;

    /// Copy a host path into the container. For directories, streams tar+gzip over
    /// a dedicated vsock connection. For regular files, raw byte stream.
    ///
    /// Appropriate for one-shot transfers. For live-sync or shared directories, use
    /// Mount::virtiofs instead. See §4 of this file for the full rubric.
    pub async fn copy_in(
        &self,
        host_path: impl AsRef<Path>,
        guest_path: impl AsRef<Path>,
    ) -> Result<(), core::Error>;

    /// Copy a guest path to the host. For directories, guest-side tar+gzip over vsock;
    /// host extracts.
    pub async fn copy_out(
        &self,
        guest_path: impl AsRef<Path>,
        host_path: impl AsRef<Path>,
    ) -> Result<(), core::Error>;

    /// Open an arbitrary vsock channel to a guest listener. Useful for user-defined
    /// guest daemons.
    pub async fn dial_vsock(&self, port: VsockPort) -> Result<VsockStream, core::Error>;
}
```

### 3.7 `ExecConfig<E>` and `Process<E>`

#### 3.7.1 `ExecConfig<E>`

A builder for processes to exec into a running container. Subset of `ContainerBuilder`'s process-level knobs — no rootfs, no resource limits (inherited from container cgroup), no VM-level config. Parameterized on the same `ContainerStdio` markers as `Container<S>` so pty-XOR-stderr is enforced at compile time here too.

```rust
pub struct ExecConfig<E: ContainerStdio> { /* private */ }

impl ExecConfig<Streams> {
    pub fn builder() -> ExecConfigBuilder<MissingCommand>;
}

pub struct ExecConfigBuilder<S> { /* private */ }

pub struct MissingCommand;
pub struct CommandSet;       // resolves to ExecConfig<Streams> on build
pub struct CommandSetPty;    // resolves to ExecConfig<Pty> on build

impl ExecConfigBuilder<MissingCommand> {
    pub fn command<I, A>(self, args: I) -> ExecConfigBuilder<CommandSet>
    where I: IntoIterator<Item = A>, A: Into<OsString>;
}

impl<S> ExecConfigBuilder<S> {
    pub fn env(self, k: impl Into<OsString>, v: impl Into<OsString>) -> Self;
    pub fn envs<I, K, V>(self, vars: I) -> Self where /* ... */ ;
    pub fn working_dir(self, p: impl Into<PathBuf>) -> Self;
    pub fn user(self, u: impl Into<User>) -> Self;
    pub fn no_new_privileges(self, on: bool) -> Self;
    pub fn capabilities(self, caps: LinuxCapabilities) -> Self;
    pub fn rlimit(self, r: LinuxRlimit) -> Self;
}

impl ExecConfigBuilder<CommandSet> {
    pub fn stdin(self, s: Stdio) -> Self;
    pub fn stdout(self, s: Stdio) -> Self;
    pub fn stderr(self, s: Stdio) -> Self;
    pub fn pty(self, cfg: impl Into<PtyConfig>) -> ExecConfigBuilder<CommandSetPty>;
    pub fn build(self) -> ExecConfig<Streams>;
}

impl ExecConfigBuilder<CommandSetPty> {
    pub fn stdin(self, s: Stdio) -> Self;                   // still separately configurable
    // no .stdout / .stderr
    pub fn build(self) -> ExecConfig<Pty>;
}
```

Required field: command. Same typestate gate pattern as `ContainerBuilder.rootfs`.

#### 3.7.2 `Process<E>` — exec'd-process handle

```rust
pub struct Process<E: ContainerStdio = Streams> { /* private */ }

impl<E: ContainerStdio> Process<E> {
    pub fn id(&self) -> &ProcessId;
    pub fn pid(&self) -> Option<i32>;

    // Lifecycle — subset of Container's.
    pub async fn wait(&mut self) -> Result<ExitStatus, core::Error>;
    pub async fn wait_with_output(self) -> Result<Output, core::Error>;
    pub async fn kill(&mut self, signal: Signal) -> Result<(), core::Error>;
}

impl Process<Streams> {
    pub fn stdin(&mut self)  -> Option<&mut ChildStdin>;
    pub fn stdout(&mut self) -> Option<&mut ChildStdout>;
    pub fn stderr(&mut self) -> Option<&mut ChildStderr>;
    pub fn take_stdin(&mut self)  -> Option<ChildStdin>;
    pub fn take_stdout(&mut self) -> Option<ChildStdout>;
    pub fn take_stderr(&mut self) -> Option<ChildStderr>;
}

impl Process<Pty> {
    pub fn stdin(&mut self) -> Option<&mut ChildStdin>;
    pub fn take_stdin(&mut self) -> Option<ChildStdin>;
    /// Infallible — the builder typestate (`CommandSetPty`) guaranteed a pty.
    pub fn pty(&mut self) -> &mut Pty;
}
```

#### 3.7.3 Distinctions from `Container<S>`

A `Process<E>` is narrower than a `Container<S>`:

- **No `exec`** — nested exec is not supported. A user who wants another process runs `container.exec(id2, cfg)` on the parent Container.
- **No `copy_in` / `copy_out`** — file transfer is container-scoped, routes through the Container handle.
- **No `pause` / `resume`** — VM-level only.
- **No `dial_vsock`** — vsock channels are container-scoped.
- **No `stop`** — a Process exits naturally or is killed. If the parent Container stops, every outstanding Process's `wait()` resolves with `Error::ContainerExited { container, status }`.
- **Tied to a live Container** — Process handles don't outlive their parent. Dropping a Process cancels its in-flight operations (drop-future-is-cancel); dropping the parent Container invalidates every outstanding Process handle.

---

## 4. Mounts and file transfer — when to use which

A cross-cutting rubric for users choosing between `Mount::virtiofs`, `Mount::bind`, `copy_in`/`copy_out`, and other options. Worth documenting here because it's the #1 source of confusion in container-system design.

| Use case | Primitive | Rationale |
|---|---|---|
| One-shot config file at spawn time | `copy_in(path, dest)` after spawn | Ephemeral, no need to wire a mount |
| Pull-an-artifact-out-after-exit | `copy_out(dest, path)` | Same, opposite direction |
| Live-edit a source tree from host | `Mount::virtiofs(tag, dest)` + `VmConfig::virtiofs_share(tag, host)` | Changes propagate immediately; zero-copy-ish |
| Shared compiler cache across N containers | Same as above, one VM hosts all N containers (Layout A in §2.5 narrative) | POSIX locking works because all containers share one kernel |
| Shared compiler cache across N VMs | `ContainerBuilder::virtiofs_share` per VM | Works for content-addressable caches only; cross-VM locks are NFS-like |
| Large binary you want in every container | `Mount::bind(host_path, dest)` | Desugars to virtiofs; simple alias |
| Writable scratch area inside container | `Mount::tmpfs(dest)` | Volatile; doesn't persist |
| Persistent writable layer over read-only rootfs | `Mount::overlay(…)` or `ContainerBuilder::writable_layer` | Standard overlay semantics |
| Host Unix socket → guest | `ContainerBuilder::socket(UnixSocketConfig { direction: Into, … })` | Proxies via vsock; works for non-POSIX-compliant sockets |
| Guest Unix socket → host | Same with `direction: OutOf` | |

**Honest caveats:**

- **Naive `cargo build --target-dir=<shared_virtiofs>` with concurrent builders will corrupt** even under Layout A (single kernel, POSIX locks working). Cargo's internal lockfile doesn't scale to 10+ concurrent builders sharing one target. The right fix is `sccache` (content-addressable, atomic-write-based), Bazel's remote cache, or per-builder target dirs with a post-build hardlink pool. The library supports the mount pattern; build-tool concurrency is the user's discipline.
- **ext4 is not a cluster filesystem.** Attaching the same ext4 image as a `RawBlock` rootfs to two VMs concurrently is undefined behavior. If you want shared block storage across VMs, use virtiofs or an explicit cluster-FS.
- **Copy-on-write per-container over a shared virtiofs** requires layering `Mount::overlay(lower=<virtiofs>, upper=<tmpfs or block>, …)` inside the container. The library supports this; declaring it is the user's job.

---

## 5. Drop and cancellation

### 5.1 `Drop` does not `.stop()`

`Container<S>::drop` does **not** issue graceful shutdown. Reasons (applied from [`09-cross-cutting.md § Drop`](./09-cross-cutting.md)):

1. `Drop` is synchronous. `.stop()` is async and has its own failure modes. Blocking on an async future from a sync Drop either deadlocks (same tokio runtime) or silently fails (no runtime, e.g., during shutdown).
2. `Drop` has no way to propagate errors. Silent "stop failed" is worse than no stop.
3. A user who dropped a live container already has a misaligned mental model; adding implicit cleanup hides the misunderstanding rather than surfacing it.

What `Drop` actually does:
- Aborts internal tokio relay tasks (stdio forwarders, socket relays) — sync.
- Closes host-side vsock fds — sync.
- Decrements the internal `Arc<ContainerCore>` — sync. Last ref triggers a best-effort async cleanup task spawned on the current runtime, if any.
- Logs `tracing::warn!` at drop time if the container was still alive:

```
container `<id>` dropped without .stop().await or .wait().await — VM resources may leak until process exit
```

### 5.2 `AbortOnDrop<T>` wrapper

Users who want drop-means-stop semantics (test fixtures, short-lived scripts, teardown handlers):

```rust
pub struct AbortOnDrop<T>(Option<T>);

impl<S: ContainerStdio> AbortOnDrop<Container<S>> {
    pub fn new(c: Container<S>) -> Self;
    pub fn into_inner(self) -> Container<S>;   // extract without running the drop handler
}

impl<S: ContainerStdio> Drop for AbortOnDrop<Container<S>> {
    fn drop(&mut self) {
        if let Some(c) = self.0.take() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Err(e) = c.stop().await {
                        tracing::warn!(?e, "AbortOnDrop<Container>: stop failed");
                    }
                });
            } else {
                tracing::warn!("AbortOnDrop<Container>: no tokio runtime; container leaked");
            }
        }
    }
}
```

Mirrors `tokio::task::AbortOnDrop` wrappers in pattern. Strictly opt-in.

### 5.3 Cancellation — two modes

From [`09-cross-cutting.md § cancellation`](./09-cross-cutting.md), restated at the container level:

**Mode 1 — External cancellation via drop-future.** Users wrap any `async fn` in `tokio::select!` / `tokio::time::timeout` / `tokio_util::sync::CancellationToken::run_until_cancelled`. Dropping the future cancels the operation cleanly because operation state is entirely on the stack frame:

```rust
let result = tokio::time::timeout(Duration::from_secs(30), container.copy_in(src, dst)).await;
match result {
    Ok(Ok(())) => { /* success */ }
    Ok(Err(e))  => { /* copy failed */ }
    Err(_)       => { /* timeout; copy future was dropped cleanly */ }
}
```

Library invariant: every `async fn` owns its internal state in the future's stack frame. No `tokio::spawn` of tasks that outlive the parent future. RAII on port allocations and fd ownership. Dropping the future releases every held resource immediately.

**Mode 2 — Cascading lifecycle cancellation.** When `vm.stop()` or `container.stop()` is called, every in-flight operation on the stopped subtree observes cancellation at its next RPC boundary and returns:

```rust
Error::Cancelled {
    reason: CancelReason::VmStopped,        // or ::ContainerStopped
}
```

This is powered by an internal `CancellationToken` tree:
- `VirtualMachine<Running>` owns a root token.
- Each `Container` has a child token.
- Each in-flight long operation (`copy_in`, `exec`, `dial_vsock`, etc.) gets a per-operation child token.
- `vm.stop()` cancels the VM token → cascades to all containers → cascades to all operations.
- `container.stop()` cancels the container token → cascades to its operations only.

**No public token types in v1.** Users don't construct, clone, or cancel tokens — those are internal. Cancellation is what `stop()` *is*. Users observe the cascade via `Error::Cancelled { reason }` in their error path.

---

## 6. Worked examples — non-obvious flows

### 6.1 Capturing output with a timeout

```rust
use std::time::Duration;
use firkin::{Container, Rootfs};

let result = tokio::time::timeout(
    Duration::from_secs(30),
    Container::builder("pinger")
        .rootfs(Rootfs::ext4_image("/tmp/busybox.ext4"))
        .command(["/bin/ping", "-c", "5", "127.0.0.1"])
        .output(),                                 // D-021: one call, auto-piped, auto-drained
).await;

match result {
    Ok(Ok(output)) => { /* normal */ }
    Ok(Err(e))     => { /* container-level error */ }
    Err(_)         => { /* 30s timeout; future dropped cleanly; container is abandoned */ }
}
```

Note: on timeout, the container is dropped mid-spawn — which does NOT stop it if the VM is already booted. If you want the container killed on timeout, hold the handle explicitly and wrap in `AbortOnDrop`:

```rust
let container = Container::builder("pinger") /* ... */ .spawn().await?;
let c = AbortOnDrop::new(container);
let result = tokio::time::timeout(Duration::from_secs(30), async {
    c.into_inner().wait_with_output().await
}).await;
```

### 6.2 Exec'ing a debug shell into a running container

```rust
let mut c = Container::builder("web-server")
    .rootfs(Rootfs::ext4_image("/srv/web.ext4"))
    .command(["/usr/sbin/nginx", "-g", "daemon off;"])
    .spawn().await?;

// Later, debug a live request pattern...
let mut shell = c.exec(
    "debug-shell",
    ExecConfig::builder()
        .command(["/bin/sh"])
        .pty((120, 40))                       // D-003 fluent form; PtyConfig::new works too
        .build(),
).await?;

let pty = shell.pty();                        // infallible — Process<Pty> guarantees it
// Forward host stdin/stdout to pty, wait for shell exit...
shell.wait().await?;
// c continues running nginx.
```

### 6.3 Graceful shutdown with grace-period override

```rust
// Default stop() gives SIGTERM 10s before escalating to SIGKILL.
// For a DB that needs longer, use stop_with_grace:
let exit = container.stop_with_grace(Duration::from_secs(60)).await?;

// Sends SIGTERM immediately; if not exited after 60s, sends SIGKILL; then unmounts.
```

### 6.4 Observing cascading cancellation

```rust
let (builder, worker_bd) = VmConfig::builder().block_device("/srv/worker.ext4");
let vm = VirtualMachine::new(builder.build()?).boot().await?;
let c = vm.container("worker")
    .rootfs(Rootfs::block_device(worker_bd))
    .command(["/usr/bin/long-job"])
    .spawn().await?;

let copy_task = tokio::spawn({
    let c = c.clone();  // Arc-cloneable via .handle() — only available on containers built
                        // with vm.container_shared() variant; see 02-vm-surface.md
    async move { c.copy_in("/host/big-file", "/guest/data").await }
});

// Meanwhile, user decides to abort:
vm.stop().await?;

// copy_task resolves with Err(Error::Cancelled { reason: CancelReason::VmStopped })
// because the internal token tree cascades the cancellation.
let copy_result = copy_task.await.unwrap();
assert!(matches!(copy_result, Err(core::Error::Cancelled { reason: CancelReason::VmStopped })));
```

---

## 7. Invariants worth locking

1. `ContainerBuilder` typestate: `Init` → `.rootfs()` → `Ready` → `.pty(…)` → `ReadyPty`. Only `Ready` and `ReadyPty` expose `.spawn()`/`.output()`/`.status()` terminals.
2. VM-level methods (`.network()`, `.virtiofs_share()`, `.nested_virtualization()`, `.boot_log()`, `.dns()`, `.hosts()`, `.kernel()`, `.cmdline_extra()`) exist only on `ImplicitVm` builders.
3. Pty builder path excludes `.stdout()` and `.stderr()`; only `.stdin()` still available on `ReadyPty`. `.pty()` takes `impl Into<PtyConfig>` — tuples, the newtype, or `PtyConfig::default()`.
4. `Container<S>` is typestate-parameterized by stdio shape (D-025): `Container<Streams>` has stdin/stdout/stderr; `Container<Pty>` has stdin + infallible `pty()`. Same for `Process<E>`.
5. `ContainerBuilder::output()` / `::status()` are the one-call terminals (D-021); `.spawn()` returns a handle for the interactive case.
6. `wait()` does not drain stdio; `wait_with_output()` drains and consumes self. `output()` on the builder avoids the drain/deadlock footgun by auto-piping + auto-draining in one call.
7. `Drop` never calls `async fn`. `AbortOnDrop<Container<S>>` wrapper for opt-in auto-stop.
8. Cancellation: drop-future-is-cancel for external; internal token tree for cascading from `stop()`.
9. `Process<E>` handles are narrower than `Container<S>`; no nested exec, no copy, no dial_vsock.
10. `copy_in/out` for one-shot; `Mount::virtiofs` for live-sync. Documented side-by-side in §4.
11. `vm.container(…)` / `vm.container_shared(…)` are defined on the sealed `CoreContainerFactory` extension trait in `firkin-core` (D-018), not as inherent methods on `VirtualMachine<Running>` (which lives in `firkin-vmm`). The trait is re-exported from `firkin`.
12. `ImplicitVm` builders take `Rootfs` (enum: `Ext4Image`, `OciBundle`, `RawBlock`); `OnVm`/`OnVmArc` builders take `VmRootfs` (`BlockDevice` or `GuestPath`) (D-023). `Rootfs::OciBundle` on `OnVm` is a compile error, not a runtime error.
13. `BlockDeviceId` is the typed handle returned by `VmConfigBuilder::block_device(path)` (D-022); string-path matching between pre-declaration and rootfs is gone.
14. `ContainerBuilder::memory` takes `Size` (D-026); no `memory_mib`. Users write `Size::mib(…)` or `Size::gib(…)`.

Proceed to [`02-vm-surface.md`](./02-vm-surface.md) for the `VirtualMachine` surface.
