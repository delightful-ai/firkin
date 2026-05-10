# Value types

> Covers: the building blocks. Every newtype, enum, and value-struct referenced elsewhere in this design has its full definition here.
>
> Prerequisites: [`README.md`](./README.md) for principles; [`01-container-surface.md`](./01-container-surface.md) and [`02-vm-surface.md`](./02-vm-surface.md) for usage context.

---

## 1. Overview

This file is a **catalog**, not an argument. Each type pins its exact definition so implementers don't have to reconstruct it from usage. Ordering is loosely by cohesion — newtypes first, then config enums, then process-level types, then statistics.

### 1.1 Owning crate per type (D-015, D-016)

Types are distributed across the workspace like this; all are re-exported from `firkin` (the facade), so user code writes `firkin::ContainerId` regardless:

| Crate | Types |
|---|---|
| `firkin-types` (leaf — D-015) | `ContainerId`, `ProcessId`, `VsockPort`, `VirtiofsTag`, `VmId`, `Size`, `Hostname`, `Platform`, `Os`, `Arch`, `NamespaceKind`, `BlockDeviceId` (D-022), and their `InvalidX` error types; the `container_id!`, `virtiofs_tag!`, `hostname!` macros (D-027) |
| `firkin-vsock` (D-016) | `VsockStream`, `VsockListener`, `VsockPeer` |
| `firkin-vmm` | `Network`, `BootLog`, `KernelImage`, `VmPhase`, `VmStatistics` |
| `firkin-ext4` | `BlockNumber`, `InodeNumber`, `BlockSize`, `Features` (internal to ext4; not user-facing beyond the crate's own API), `LayerCompression` (per the §2.3 `write_oci_layers` signature), the `OciLayerSource` sealed trait (D-024) |
| `firkin-oci` | `Reference`, `ImageBundle` (D-020 — renamed from `Bundle`), `Layer`, `Auth`, `Credentials`, `AuthProvider`, `TlsConfig`; re-exports `Manifest`, `ImageConfig`, `Descriptor`, `MediaType`, `Digest` from `oci-spec` |
| `firkin-core` (facade) | `Rootfs`, `VmRootfs` (D-023), `Streams` / `Pty` + sealed `ContainerStdio` (D-025), `Mount`, `MountOptions`, `DevptsOptions`, `MountFlags`, `User`, `ExitStatus`, `KilledReason`, `Output`, `LinuxCapabilities`, `Capability`, `InvalidCapability`, `LinuxRlimit`, `RlimitKind`, `Seccomp` (+ helpers), `DnsConfig`, `HostsConfig`, `HostsEntry`, `UnixSocketConfig`, `SocketDirection`, `FileMount`, `PtyConfig`, `CancelReason`, `StatCategory`, `ContainerStatistics`, `CpuStats`, `MemoryStats`, `IoStats`, `IoDeviceStats`, `NetworkStats`, `NetworkInterfaceStats`; re-exports `Signal` from `nix`; the `CoreContainerFactory` extension trait (D-018) |

When a type in one crate references a type from another, it does so via the explicit dependency. `firkin-oci`'s `Auth` may reference `firkin-types`' newtypes, but not the reverse (`firkin-types` is the leaf).

### 1.2 Derive baseline

Everything here derives:
- `Debug` always.
- `Clone` unless the type owns non-cloneable resources (e.g., it holds `Box<dyn Trait>`).
- `PartialEq, Eq, Hash` where value-equality makes sense (IDs, enums, small structs).
- `Copy` where cheap (newtypes over scalars, small enums, `PtyConfig`).
- `serde::Serialize, serde::Deserialize` behind a `serde` Cargo feature (off by default; used for tests and optional user workflows).

No type derives `Default` unless the default has a real meaning and documentation explaining it.

---

## 2. Newtypes — the anti-translation discipline

Every identity, quantity, or tag is a newtype. Rule: if a value has a *meaning* beyond its underlying representation, it gets a newtype. ([`scatter.md § translation`](../../../../../../src/personal/beads-rs/docs/philosophy/scatter.md) + [`type_design.md § information holds its shape`](../../../../../../src/personal/beads-rs/docs/philosophy/type_design.md).)

### 2.1 `ContainerId`

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContainerId(String);

impl ContainerId {
    /// Construct with validation. Rules:
    /// - non-empty
    /// - ≤ 64 characters
    /// - matches [a-zA-Z0-9_.-]+
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidContainerId>;

    pub fn as_str(&self) -> &str;
}

impl std::str::FromStr for ContainerId { /* calls new() */ }
impl std::fmt::Display for ContainerId { /* writes inner */ }

#[derive(Debug, thiserror::Error)]
pub enum InvalidContainerId {
    #[error("container id is empty")]
    Empty,
    #[error("container id `{0}` is too long (max 64 chars)")]
    TooLong(String),
    #[error("container id `{0}` contains forbidden characters")]
    ForbiddenChars(String),
}

/// Compile-time validated construction from a string literal (D-027).
/// Expands to a const-fn validation call plus the internal unchecked constructor.
/// Invalid literals fail at compile time.
///
/// ```
/// use firkin::container_id;
/// let id = container_id!("web");             // OK
/// // let bad = container_id!("bad/id");       // compile error
/// ```
#[macro_export]
macro_rules! container_id { ($s:literal) => { /* ... */ }; }
```

Used as: DNS hostname (guest-side), filesystem path segment (`/run/container/<id>/…`), cgroup path (`/container/<id>`). Strings that fail these constraints leak into the guest and cause opaque failures; validating at construction is cheap, honest, and catches real bugs.

### 2.2 `ProcessId`

Same rules as `ContainerId` — reuses `InvalidContainerId` validation constraints but as a distinct type:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProcessId(String);

impl ProcessId {
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidProcessId>;
    pub fn as_str(&self) -> &str;
}
```

Separate type from `ContainerId` because the two don't mix: passing a `ContainerId` where a `ProcessId` is expected is a conceptual error, even if the underlying character sets are identical.

### 2.3 `VmId`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VmId(uuid::Uuid);

impl VmId {
    /// Generate a fresh VM ID. The library uses this internally; users rarely construct them.
    pub fn new() -> Self;
    pub fn as_uuid(&self) -> &uuid::Uuid;
}

impl std::fmt::Display for VmId { /* hyphenated uuid string */ }
```

UUID v4, library-generated. Users almost never construct these; VMs are ephemeral and unnamed from a user perspective.

### 2.3a `NamespaceKind`

Owned by `firkin-types` (D-015); consumed by `firkin-vminitd-client` for the `unshare`-namespace wrappers that emit the Codable-strict `{type, path: ""}` shape vminitd expects.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NamespaceKind {
    Pid,
    Mount,
    Network,
    Ipc,
    Uts,
    User,
    Cgroup,
    Time,
}

impl NamespaceKind {
    /// The lowercase `type` field vminitd's Codable decoder accepts
    /// (`"pid"`, `"mount"`, `"network"`, ...).
    pub fn as_spec_str(&self) -> &'static str;
}
```

`#[non_exhaustive]` — the Linux kernel may add more namespace kinds.

### 2.3b `BlockDeviceId`

Owned by `firkin-types` (D-022). Returned by `VmConfigBuilder::block_device(path)` alongside the updated builder; consumed by `Rootfs::block_device(id)` on `OnVm`/`OnVmArc` container builders.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockDeviceId(NonZeroU32);

impl BlockDeviceId {
    /// Construction is internal to `firkin-vmm`'s VmConfigBuilder. Users get
    /// instances from the builder, never construct them directly.
    pub(crate) fn from_slot(slot: NonZeroU32) -> Self;

    /// Opaque numeric form for debugging and Display.
    pub fn slot(self) -> NonZeroU32;
}

impl std::fmt::Display for BlockDeviceId { /* e.g. "bd#1" */ }
```

**No `new()`:** users cannot forge a `BlockDeviceId`. They come from a validated `VmConfigBuilder::block_device(path)` call only. Cross-VM misuse (passing one VM's id to another VM's `vm.container(...).rootfs(...)`) is caught at `spawn()` time by a runtime check; compile-time safety would require a phantom-lifetime parameterization that cost more reader surface than it's worth.

### 2.4 `VsockPort`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct VsockPort(u32);

impl VsockPort {
    pub const fn new(port: u32) -> Self;
    pub const fn get(self) -> u32;
}

impl From<u32> for VsockPort { /* new() */ }
```

See [`03-stdio-pty-vsock.md § reserved port ranges`](./03-stdio-pty-vsock.md). Runtime enforcement of reserved ranges lives at the dial/listen sites; the newtype itself doesn't validate at construction (users have legitimate reasons to name reserved ports in configuration contexts).

### 2.5 `VirtiofsTag`

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VirtiofsTag(String);

impl VirtiofsTag {
    /// Rules:
    /// - non-empty
    /// - printable ASCII only
    /// - ≤ 36 bytes (VZ cap; longer tags silently truncate otherwise)
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidVirtiofsTag>;
    pub fn as_str(&self) -> &str;
}

/// Compile-time validated literal construction (D-027). Invalid literals fail
/// at compile time via a const-fn assertion.
///
/// ```
/// use firkin::virtiofs_tag;
/// let t = virtiofs_tag!("cargo-cache");       // OK
/// ```
#[macro_export]
macro_rules! virtiofs_tag { ($s:literal) => { /* ... */ }; }
```

Validated at construction to catch tag-too-long bugs before they silently truncate at the VZ boundary and produce mysterious "can't find the share" errors inside the guest.

### 2.6 `Hostname`

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hostname(String);

impl Hostname {
    /// Parse + validate against RFC 1123 §2.1 (with the RFC 952 single-segment
    /// relaxation). Rules:
    /// - 1-253 total bytes
    /// - each label: 1-63 bytes, `[a-zA-Z0-9]` with interior `-` allowed
    /// - labels separated by `.`; no leading/trailing `.`
    /// - not a bare numeric (prevents `123` being mistaken for an IP)
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidHostname>;
    pub fn as_str(&self) -> &str;
}

impl std::str::FromStr for Hostname { /* calls new() */ }
impl std::fmt::Display for Hostname { /* writes inner */ }

#[derive(Debug, thiserror::Error)]
pub enum InvalidHostname {
    #[error("hostname is empty")]
    Empty,
    #[error("hostname `{0}` exceeds 253 bytes")]
    TooLong(String),
    #[error("hostname label `{label}` is empty or >63 bytes")]
    BadLabel { label: String },
    #[error("hostname `{0}` contains a label with forbidden characters")]
    ForbiddenChars(String),
    #[error("hostname `{0}` is purely numeric (use an IP type instead)")]
    NumericOnly(String),
}

/// Compile-time validated literal construction (D-027).
/// ```
/// use firkin::hostname;
/// let h = hostname!("web.local");
/// ```
#[macro_export]
macro_rules! hostname { ($s:literal) => { /* ... */ }; }
```

Validated at construction so invalid hostnames don't leak into the guest and fail opaquely at `sethostname(2)` time. Same pattern as `ContainerId` — the scatter.md §true discipline: make the claim true at the boundary where it's cheap, not seven function calls later where it's confusing.

### 2.7 `Size`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct Size(u64);     // bytes

impl Size {
    pub const fn bytes(n: u64) -> Self;
    pub const fn kib(n: u64) -> Self;   // n * 1024
    pub const fn mib(n: u64) -> Self;   // n * 1024 * 1024
    pub const fn gib(n: u64) -> Self;   // n * 1024^3
    pub const fn tib(n: u64) -> Self;   // n * 1024^4

    pub const fn as_bytes(self) -> u64;
    pub const fn as_kib(self) -> u64;   // truncating division
    pub const fn as_mib(self) -> u64;
    pub const fn as_gib(self) -> u64;
}

impl std::ops::Add for Size { /* bytes + bytes */ }
impl std::ops::Sub for Size { /* saturating; doesn't panic */ }

impl std::fmt::Display for Size {
    // Humanized: 1024 -> "1 KiB", 1536 -> "1.5 KiB", etc.
    // Powers of 1024, not 1000.
}
```

Arithmetic only between `Size`-and-`Size`. No `u64 * Size` or `Size / u64` — if users want scaling, they convert explicitly via `.as_bytes()`.

### 2.8 `Signal` — re-export from `nix`

```rust
pub use nix::sys::signal::Signal;
```

Rationale: `nix` is already a transitive dep (used inside `ext4` for stat/mode constants, used inside `vmm` for process-signalling syscalls in host-side helpers). Re-exporting saves reinventing a well-maintained enum that matches Linux signals exactly.

Cost: `nix` appears in the public API surface for anyone who types out `Signal`. Accepted; `nix` is a stable, ubiquitous crate.

---

## 3. `Rootfs` and `VmRootfs` — source of a container's root filesystem

Per [D-023](../DECISIONS.md#d-023--rootfs-split-by-vm-context-rootfs-vs-vmrootfs): two distinct types, one per VM context. `Rootfs` is for the `ImplicitVm` path (single-use VM constructed at spawn time); `VmRootfs` is for the `OnVm`/`OnVmArc` path (pre-booted VM where the rootfs must already exist as either a block device or a guest path). Each `ContainerBuilder::rootfs` impl takes only the shape valid for its VM context — mixing them is a compile error, not a runtime one.

### 3.1 `Rootfs` — ImplicitVm variants

```rust
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Rootfs {
    /// A pre-built .ext4 file on the host. Attached as virtio-block.
    Ext4Image(PathBuf),

    /// Layers + config from an OCI pull. Library assembles the ext4 at spawn time
    /// via the `ext4` crate.
    OciBundle(oci::ImageBundle),     // D-020: renamed from oci::Bundle

    /// Arbitrary block-device image. User promises it's mountable. Library attaches
    /// as virtio-block without transformation.
    RawBlock(PathBuf),
}

impl Rootfs {
    pub fn ext4_image(p: impl Into<PathBuf>) -> Self;
    pub fn oci_bundle(b: oci::ImageBundle) -> Self;
    pub fn raw_block(p: impl Into<PathBuf>) -> Self;

    /// Constructs a `VmRootfs` from a pre-declared block device. Exists on
    /// `Rootfs` purely as a naming convenience so users write
    /// `Rootfs::block_device(id)` regardless of context — the return type
    /// is `VmRootfs`, so it only type-checks on `OnVm`/`OnVmArc` builders.
    pub fn block_device(id: BlockDeviceId) -> VmRootfs;
}
```

`#[non_exhaustive]` lets future additions (`Rootfs::Squashfs(PathBuf)`, `Rootfs::Directory(PathBuf)`) land without breaking the enum match.

### 3.2 `VmRootfs` — OnVm / OnVmArc variants

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum VmRootfs {
    BlockDevice(BlockDeviceId),
    GuestPath(GuestPath),
}

impl VmRootfs {
    pub fn guest_path(path: GuestPath) -> Self;
    pub fn as_block_device(&self) -> Option<BlockDeviceId>;
    pub fn as_guest_path(&self) -> Option<&GuestPath>;
}

impl From<BlockDeviceId> for VmRootfs {
    fn from(id: BlockDeviceId) -> Self { Self::BlockDevice(id) }
}
```

`GuestPath` is an absolute, normalized guest path. It rejects relative paths, `/`, NUL, empty components, and `..`. The guest-path variant is the pod-store substrate: boot one writable pod-store disk, mount it in the guest, materialize rootfs directories below it, and start containers with `VmRootfs::GuestPath`.

### 3.3 Usage recap

```rust
// ImplicitVm path: takes Rootfs
Container::builder("x")
    .rootfs(Rootfs::ext4_image("/tmp/x.ext4"))       // OK
    .rootfs(Rootfs::oci_bundle(bundle))              // OK
    .rootfs(Rootfs::raw_block("/dev/loop5"))         // OK

// OnVm path: takes VmRootfs (via Into)
let (builder, bd) = VmConfig::builder().block_device("/srv/x.ext4");
let vm = VirtualMachine::new(builder.build()?).boot().await?;
vm.container("x")
    .rootfs(Rootfs::block_device(bd))                // OK — returns VmRootfs
    .rootfs(bd)                                      // also OK — From<BlockDeviceId>
    .rootfs(VmRootfs::guest_path(GuestPath::new("/run/firkin/pods/p/rootfs/x")?))
    // .rootfs(Rootfs::ext4_image(...))              // compile error: Rootfs ≠ VmRootfs
```

---

## 4. `Mount` and friends

### 4.1 `Mount` — structured per-fstype variants

```rust
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Mount {
    Virtiofs { tag: VirtiofsTag, destination: PathBuf, options: MountOptions },
    Bind     { source: PathBuf, destination: PathBuf, options: MountOptions },
    Tmpfs    { destination: PathBuf, size: Option<Size>, options: MountOptions },
    Overlay  { lower: Vec<PathBuf>, upper: PathBuf, work: PathBuf, destination: PathBuf },
    Proc     { destination: PathBuf },
    Sysfs    { destination: PathBuf, options: MountOptions },
    Devtmpfs { destination: PathBuf, options: MountOptions },
    Devpts   { destination: PathBuf, options: DevptsOptions },
    Mqueue   { destination: PathBuf, options: MountOptions },
    Cgroup2  { destination: PathBuf, options: MountOptions },

    /// Escape hatch for fstypes the library doesn't know about. Pass-through to vminitd.
    Custom {
        fstype: String,
        source: String,
        destination: PathBuf,
        options: MountOptions,
    },
}

impl Mount {
    pub fn virtiofs(tag: impl Into<VirtiofsTag>, dest: impl Into<PathBuf>) -> Self;
    pub fn bind(source: impl Into<PathBuf>, dest: impl Into<PathBuf>) -> Self;
    pub fn tmpfs(dest: impl Into<PathBuf>) -> Self;
    pub fn overlay(
        lower: impl IntoIterator<Item: Into<PathBuf>>,
        upper: impl Into<PathBuf>,
        work: impl Into<PathBuf>,
        dest: impl Into<PathBuf>,
    ) -> Self;
    pub fn proc(dest: impl Into<PathBuf>) -> Self;
    pub fn sysfs(dest: impl Into<PathBuf>) -> Self;
    pub fn devtmpfs(dest: impl Into<PathBuf>) -> Self;
    pub fn devpts(dest: impl Into<PathBuf>) -> Self;
    pub fn mqueue(dest: impl Into<PathBuf>) -> Self;
    pub fn cgroup2(dest: impl Into<PathBuf>) -> Self;
    pub fn custom(
        fstype: impl Into<String>,
        source: impl Into<String>,
        dest: impl Into<PathBuf>,
    ) -> Self;

    /// The Linux-standard default mount set runc/vminitd expects:
    /// /proc, /sys, /dev, /dev/pts, /dev/mqueue, /dev/shm, /sys/fs/cgroup
    pub fn defaults() -> Vec<Mount>;

    /// OCI-runtime-style default set (slight layout difference from `defaults`;
    /// mirrors Swift's LinuxContainer.defaultOCIMounts()).
    pub fn oci_defaults() -> Vec<Mount>;
}
```

Option overrides default via consuming-self builder methods on the variants' constructors:

```rust
impl Mount {
    // Example: Mount::bind("/host", "/guest").read_only()
    //
    // These methods are on Mount rather than MountOptions to hide the variant-internal
    // option field. Each returns Self with the flag set; chained calls build up options.
    pub fn read_only(mut self) -> Self;
    pub fn no_suid(mut self) -> Self;
    pub fn no_exec(mut self) -> Self;
    pub fn no_dev(mut self) -> Self;
    pub fn relatime(mut self) -> Self;
    pub fn noatime(mut self) -> Self;
    pub fn extra_option(mut self, opt: impl Into<String>) -> Self;
}
```

### 4.2 `MountOptions`

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MountOptions {
    pub flags: MountFlags,
    pub extra: Vec<String>,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct MountFlags: u32 {
        const READ_ONLY  = 1 << 0;
        const NO_SUID    = 1 << 1;
        const NO_EXEC    = 1 << 2;
        const NO_DEV     = 1 << 3;
        const RELATIME   = 1 << 4;
        const NOATIME    = 1 << 5;
    }
}

impl MountOptions {
    pub const fn new() -> Self;
    pub fn read_only(self) -> Self;
    pub fn no_suid(self) -> Self;
    pub fn no_exec(self) -> Self;
    pub fn no_dev(self) -> Self;
    pub fn relatime(self) -> Self;
    pub fn noatime(self) -> Self;
    pub fn with_flags(self, flags: MountFlags) -> Self;
    pub fn extra(self, opt: impl Into<String>) -> Self;   // e.g. "mode=755", "uid=1000"
}
```

`extra` carries options the library doesn't model structurally (`mode=`, `uid=`, `gid=`, `size=` for tmpfs, etc.). Kept as strings because they're fstype-specific and typing them all would double the surface.

### 4.3 `DevptsOptions`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DevptsOptions {
    pub new_instance: bool,
    pub gid: u32,
    pub mode: u32,
    pub ptmxmode: u32,
}

impl Default for DevptsOptions {
    /// new_instance: true, gid: 5 (the tty group), mode: 0o620, ptmxmode: 0o666.
    /// Matches runc's default.
    fn default() -> Self { /* ... */ }
}
```

Devpts is special-cased because its options are structured enough to justify a typed struct rather than extra strings.

---

## 5. `User` — process identity

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum User {
    /// A name resolved inside the guest via /etc/passwd at process start.
    Named(String),

    /// Numeric uid + gid with optional supplementary groups.
    Numeric { uid: u32, gid: u32, extra_groups: Vec<u32> },
}

impl User {
    pub fn root() -> Self;                      // Numeric { uid: 0, gid: 0, extra_groups: vec![] }
    pub fn named(name: impl Into<String>) -> Self;
    pub fn numeric(uid: u32, gid: u32) -> Self;
    pub fn with_extra_groups(mut self, groups: Vec<u32>) -> Self;
}

impl Default for User {
    fn default() -> Self { Self::root() }
}

impl From<u32> for User {
    // Convenience: 1000 -> Numeric { uid: 1000, gid: 1000, extra_groups: vec![] }
    fn from(uid: u32) -> Self { /* ... */ }
}
```

`Named` vs `Numeric` is a real distinction: named users require a `/etc/passwd` lookup in the guest (which may fail if the rootfs has no `/etc/passwd` — busybox cases). Numeric users always work.

---

## 6. Process outcomes

### 6.1 `ExitStatus`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitStatus {
    code: Option<i32>,
    signal: Option<Signal>,
    killed_reason: Option<KilledReason>,
}

impl ExitStatus {
    /// True iff code == Some(0) and signal is None.
    pub fn success(&self) -> bool;

    /// Exit code if the process exited normally.
    pub fn code(&self) -> Option<i32>;

    /// Signal that terminated the process, if any.
    pub fn signal(&self) -> Option<Signal>;

    /// Additional context on what killed the process (OOM, host signal, guest signal).
    /// None if the process exited normally.
    pub fn killed_reason(&self) -> Option<KilledReason>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KilledReason {
    /// Guest kernel OOM-killed the process (cgroup memory limit hit).
    OomKill,
    /// A signal from the host (e.g., our stop() sent SIGTERM).
    SignalFromHost { signal: Signal },
    /// A signal from inside the guest (e.g., another process in the container, or init).
    SignalFromGuest { signal: Signal },
}
```

`KilledReason` disambiguates what `signal: Some(SIGKILL)` means in practice — was this an OOM kill, our graceful stop escalation, or something inside the container killed itself? (Audit A.5 addition.)

### 6.2 `Output`

```rust
#[derive(Clone, Debug)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,    // empty if stdout was not Piped
    pub stderr: Vec<u8>,    // empty if stderr was not Piped
}
```

Returned by `wait_with_output()` on `Container` and `Process`. Mirrors `std::process::Output` shape.

---

## 7. `LinuxCapabilities` and the `Capability` enum

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinuxCapabilities {
    pub bounding: Vec<Capability>,
    pub effective: Vec<Capability>,
    pub inheritable: Vec<Capability>,
    pub permitted: Vec<Capability>,
    pub ambient: Vec<Capability>,
}

impl LinuxCapabilities {
    /// runc's 14-capability default set: CHOWN, DAC_OVERRIDE, FSETID, FOWNER, MKNOD,
    /// NET_RAW, SETGID, SETUID, SETFCAP, SETPCAP, NET_BIND_SERVICE, SYS_CHROOT, KILL,
    /// AUDIT_WRITE.
    pub fn default_oci() -> Self;

    /// Every Capability variant in every set. Most permissive.
    pub fn all() -> Self;

    /// Empty sets. Most restrictive; caller must opt in to each capability needed.
    pub fn empty() -> Self;

    /// Shorthand: the given set is used for bounding, effective, and permitted;
    /// inheritable and ambient are empty. Matches a common containerd/runc pattern.
    pub fn single_set(caps: Vec<Capability>) -> Self;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Capability {
    Chown,
    DacOverride,
    DacReadSearch,
    Fowner,
    Fsetid,
    Kill,
    Setgid,
    Setuid,
    Setpcap,
    LinuxImmutable,
    NetBindService,
    NetBroadcast,
    NetAdmin,
    NetRaw,
    IpcLock,
    IpcOwner,
    SysModule,
    SysRawio,
    SysChroot,
    SysPtrace,
    SysPacct,
    SysAdmin,
    SysBoot,
    SysNice,
    SysResource,
    SysTime,
    SysTtyConfig,
    Mknod,
    Lease,
    AuditWrite,
    AuditControl,
    Setfcap,
    MacOverride,
    MacAdmin,
    Syslog,
    WakeAlarm,
    BlockSuspend,
    AuditRead,
    Perfmon,
    Bpf,
    CheckpointRestore,
}

impl Capability {
    /// The list every Capability variant. Used for LinuxCapabilities::all() construction.
    pub const ALL: &'static [Capability] = &[ /* every variant */ ];

    /// OCI-spec-compatible uppercase string representation, e.g. "CAP_CHOWN".
    /// Used for the runc spec handoff and for parsing from user-supplied config.
    pub fn as_cap_str(&self) -> &'static str;

    /// Parse "CAP_CHOWN" or "chown" (case-insensitive, with/without CAP_ prefix).
    pub fn parse(s: &str) -> Result<Self, InvalidCapability>;
}

#[derive(Debug, thiserror::Error)]
#[error("unknown capability `{0}`")]
pub struct InvalidCapability(String);
```

`#[non_exhaustive]` because Linux adds new capabilities over time (`CAP_BPF` in 5.8, `CAP_CHECKPOINT_RESTORE` in 5.9). Adding variants to the enum is a non-breaking change.

---

## 8. `LinuxRlimit` and `RlimitKind`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LinuxRlimit {
    pub kind: RlimitKind,
    pub hard: u64,
    pub soft: u64,
}

impl LinuxRlimit {
    pub fn new(kind: RlimitKind, hard: u64, soft: u64) -> Self;
    /// Shorthand when hard == soft.
    pub fn symmetric(kind: RlimitKind, limit: u64) -> Self;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RlimitKind {
    AddressSpace,      // RLIMIT_AS
    CoreFileSize,      // RLIMIT_CORE
    CpuTime,           // RLIMIT_CPU
    DataSize,          // RLIMIT_DATA
    FileSize,          // RLIMIT_FSIZE
    Locks,             // RLIMIT_LOCKS
    LockedMemory,      // RLIMIT_MEMLOCK
    MessageQueue,      // RLIMIT_MSGQUEUE
    Nice,              // RLIMIT_NICE
    OpenFiles,         // RLIMIT_NOFILE
    NumberOfProcesses, // RLIMIT_NPROC
    ResidentSetSize,   // RLIMIT_RSS
    RealtimePriority,  // RLIMIT_RTPRIO
    RealtimeTimeout,   // RLIMIT_RTTIME
    SignalsPending,    // RLIMIT_SIGPENDING
    StackSize,         // RLIMIT_STACK
}

impl RlimitKind {
    pub fn as_rlimit_str(&self) -> &'static str;              // "RLIMIT_AS" etc.
    pub fn parse(s: &str) -> Result<Self, InvalidRlimit>;
}
```

Enum over `String` because rlimits have a closed set. Stringly-typed "RLIMIT_NOFILE" would allow typos.

---

## 9. `Seccomp`

Audit A.4 addition. Two shapes ship: structured type for typed policies, pass-through JSON escape hatch.

### 9.1 Structured path

```rust
#[derive(Clone, Debug)]
pub struct Seccomp {
    pub default_action: SeccompAction,
    pub architectures: Vec<SeccompArch>,
    pub syscalls: Vec<SeccompSyscallRule>,
    pub default_errno: Option<i32>,    // used when default_action == Errno
}

impl Seccomp {
    pub fn builder() -> SeccompBuilder;

    /// runc's default seccomp profile — allows the common syscalls, denies the rest.
    pub fn default_runc_profile() -> Self;

    /// Most restrictive: deny everything except a small subset specified by the caller.
    pub fn strict(allowed: Vec<SeccompSyscallRule>) -> Self;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeccompAction {
    Kill,              // kill process
    KillThread,
    KillProcess,
    Errno,             // fail with errno (specified via default_errno)
    Trace,
    Allow,
    Log,
    Trap,
    Notify,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SeccompArch {
    X86,
    X86_64,
    X32,
    Arm,
    Aarch64,
    // ... a closed set matching runc's accepted architectures
}

#[derive(Clone, Debug)]
pub struct SeccompSyscallRule {
    pub names: Vec<String>,     // syscall names to match (e.g. "read", "write")
    pub action: SeccompAction,
    pub args: Vec<SeccompArgRule>,  // optional arg-match filters
    pub errno_ret: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeccompArgRule {
    pub index: u32,                 // syscall arg index 0-5
    pub value: u64,
    pub value_two: Option<u64>,     // second value for range/bitwise ops
    pub op: SeccompOp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeccompOp {
    NotEqual,
    LessThan,
    LessThanOrEqual,
    EqualTo,
    GreaterThanOrEqual,
    GreaterThan,
    MaskedEqual,
}
```

### 9.2 Pass-through JSON

For complex policies we don't model structurally (rare), caller supplies the runc-compatible JSON directly:

```rust
impl ContainerBuilder<_, _> {
    pub fn seccomp_profile_json(self, json: impl Into<String>) -> Self;
}
```

The library doesn't parse or validate this; it's handed to vminitd → runc as-is.

---

## 10. `Network` — NIC attachment

```rust
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Network {
    /// VZ's built-in NAT. All VMs share a hidden private subnet; no inbound reachability
    /// from outside the host. Works ad-hoc-signed with no extra entitlements.
    Nat,

    /// vmnet shared mode. Each VM gets its own IP from a vmnet-managed subnet.
    /// Containers can reach each other by IP; external hosts cannot initiate.
    /// Works ad-hoc-signed on macOS 26+ (D-002).
    VmnetShared { subnet: Option<Ipv4Network> },

    // Bridged variant deferred (D-002); extension point:
    // Bridged { nic: NicId },
}

impl Network {
    pub fn nat() -> Self;
    pub fn vmnet_shared() -> Self;                              // default subnet from vmnet
    pub fn vmnet_shared_with_subnet(subnet: Ipv4Network) -> Self;
}

/// Subnet type — simple wrapper; could be ipnetwork::Ipv4Network re-export.
pub struct Ipv4Network { /* base addr + prefix length */ }
```

Not a trait because there aren't two implementations — `Nat` and `VmnetShared` are two variants of one enum. Adding `Bridged` later is a `#[non_exhaustive]` variant add, not a trait impl.

---

## 11. `DnsConfig`, `HostsConfig`, `UnixSocketConfig`, `FileMount`

### 11.1 `DnsConfig`

```rust
#[derive(Clone, Debug, Default)]
pub struct DnsConfig {
    pub nameservers: Vec<std::net::IpAddr>,
    pub search: Vec<String>,
    pub options: Vec<String>,    // e.g. "ndots:2", "timeout:1"
}
```

### 11.2 `HostsConfig` and `HostsEntry`

```rust
#[derive(Clone, Debug, Default)]
pub struct HostsConfig {
    pub entries: Vec<HostsEntry>,
}

#[derive(Clone, Debug)]
pub struct HostsEntry {
    pub ip: std::net::IpAddr,
    pub hostname: String,
    pub aliases: Vec<String>,
}
```

### 11.3 `UnixSocketConfig` and `SocketDirection`

```rust
#[derive(Clone, Debug)]
pub struct UnixSocketConfig {
    pub id: String,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub direction: SocketDirection,
    pub permissions: Option<u32>,   // mode bits
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketDirection {
    /// Host socket → guest. Host owns the listener; guest connects.
    Into,

    /// Guest socket → host. Guest owns the listener; host connects.
    OutOf,
}
```

### 11.4 `FileMount`

```rust
#[derive(Clone, Debug)]
pub struct FileMount {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub read_only: bool,
}
```

Small-file mount support. The library transforms file mounts into directory shares internally (matching Swift `FileMountContext` behavior), because VZ virtiofs shares directories, not individual files.

---

## 12. `PtyConfig`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtyConfig {
    pub cols: u16,
    pub rows: u16,
}

impl PtyConfig {
    pub const fn new(cols: u16, rows: u16) -> Self;
}

impl Default for PtyConfig {
    fn default() -> Self { Self { cols: 80, rows: 24 } }
}

impl From<(u16, u16)> for PtyConfig {
    fn from((cols, rows): (u16, u16)) -> Self { Self { cols, rows } }
}
```

Small value type. Used both at builder time (`ContainerBuilder::pty(cfg)` / `ExecConfigBuilder::pty(cfg)` — both take `impl Into<PtyConfig>`) and at resize time (`Pty::resize(cfg)`). The `From<(u16, u16)>` impl lets `.pty((120, 40))` read cleanly alongside `.pty(PtyConfig::default())` or `.pty(PtyConfig::new(120, 40))`.

---

## 12a. `ContainerStdio` markers — `Streams` and `Pty`

Per [D-025](../DECISIONS.md#d-025--containers-typestate-streams-vs-pty), `Container<S>` and `Process<E>` are parameterized on a stdio-shape marker:

```rust
mod sealed { pub trait Sealed {} }
pub trait ContainerStdio: sealed::Sealed {}

#[derive(Debug)]
pub struct Streams;        // stdin/stdout/stderr shape (default)

#[derive(Debug)]
pub struct Pty;            // stdin + combined pty duplex

impl sealed::Sealed for Streams {}
impl sealed::Sealed for Pty {}
impl ContainerStdio for Streams {}
impl ContainerStdio for Pty {}
```

Not constructable (no `::new`); these exist only in type position. `ContainerBuilder<_, Ready>::spawn()` returns `Container<Streams>`; `ContainerBuilder<_, ReadyPty>::spawn()` returns `Container<Pty>`. The infallible `Container<Pty>::pty() -> &mut Pty` replaces the Option-returning shape that lied about what the typestate guaranteed.

Why sealed: users can't add a third stdio shape without coordination. Keeps `impl<S: ContainerStdio> Container<S>` bounded by exactly two types.

---

## 13. `BootLog`

```rust
pub enum BootLog {
    /// Serial console writes to this file. Library opens it for append.
    File(PathBuf),

    /// Serial console writes go to this async sink.
    Writer(Box<dyn tokio::io::AsyncWrite + Send + Unpin>),
}
```

Not `Clone` (the `Writer` variant holds a trait object that's not cloneable in general).

---

## 14. `KernelImage`

```rust
pub struct KernelImage { /* private */ }

impl KernelImage {
    /// The kernel this library ships with. Resolved at boot via the `core` crate's
    /// embedded-or-runtime-download strategy (D-003).
    pub fn bundled() -> Self;

    /// A custom kernel image on disk. Caller responsibility to ensure format
    /// compatibility (uncompressed Image format on arm64; bzImage on x86_64).
    pub fn from_file(path: impl Into<PathBuf>) -> Self;
}
```

Opaque by design — users don't manipulate kernel images, they point at them. Internal representation carries the path + metadata (arch, format) resolved at `boot()` time.

---

## 15. `CancelReason`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelReason {
    /// Cascaded from VirtualMachine::stop().
    VmStopped,

    /// Cascaded from Container::stop().
    ContainerStopped,
    //
    // No External variant — drop-future cancellation just unwinds; no error path runs.
}
```

Used in `core::Error::Cancelled { reason: CancelReason }`. See [`05-error-model.md`](./05-error-model.md).

---

## 16. Statistics types

### 16.1 `StatCategory` — requested-category bitflags

```rust
bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct StatCategory: u32 {
        const CPU     = 1 << 0;
        const MEMORY  = 1 << 1;
        const IO      = 1 << 2;
        const NETWORK = 1 << 3;
    }
}

impl StatCategory {
    pub fn all() -> Self;
}
```

### 16.2 `ContainerStatistics`, `VmStatistics`

```rust
#[derive(Clone, Debug)]
pub struct ContainerStatistics {
    pub container_id: ContainerId,
    pub cpu:     Option<CpuStats>,       // None if StatCategory::CPU not requested
    pub memory:  Option<MemoryStats>,
    pub io:      Option<IoStats>,
    pub network: Option<NetworkStats>,
}

#[derive(Clone, Debug)]
pub struct VmStatistics {
    pub vm_id:   VmId,
    pub cpu:     Option<CpuStats>,
    pub memory:  Option<MemoryStats>,
    pub io:      Option<IoStats>,
    pub network: Option<NetworkStats>,
}
```

`Option<T>` per subsystem: `None` reliably means "the caller didn't request this category" rather than "everything was zero." (`type_design.md § information holds its shape`.)

### 16.3 Subsystem stats

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuStats {
    pub usage_ns: u64,
    pub throttle_ns: u64,
    pub nr_periods: u64,
    pub nr_throttled: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryStats {
    pub usage: Size,
    pub max_usage: Size,
    pub limit: Size,
    pub rss: Size,
    pub cache: Size,
}

#[derive(Clone, Debug)]
pub struct IoStats {
    pub per_device: Vec<IoDeviceStats>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoDeviceStats {
    pub device: String,           // e.g. "vda"
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub ops_read: u64,
    pub ops_written: u64,
}

#[derive(Clone, Debug)]
pub struct NetworkStats {
    pub per_interface: Vec<NetworkInterfaceStats>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkInterfaceStats {
    pub name: String,             // e.g. "eth0"
    pub bytes_rx: u64,
    pub bytes_tx: u64,
    pub packets_rx: u64,
    pub packets_tx: u64,
    pub errors_rx: u64,
    pub errors_tx: u64,
}
```

---

## 17. `Platform`, `Os`, `Arch`

For OCI multi-arch manifest-list resolution. See [`07-oci-crate.md § Platform`](./07-oci-crate.md) for the pull-time selection behavior.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
    pub variant: Option<String>,       // e.g. "v8" for arm64/v8
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Os {
    Linux,
    // Darwin, Windows — OCI supports them; this library targets Linux containers only.
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Arch {
    Amd64,
    Arm64,
    Arm,
    Riscv64,
    Ppc64le,
    S390x,
}

impl Platform {
    /// The host's platform, OS-hardcoded to Linux (since we run Linux containers).
    /// On M-series Mac: Linux/arm64/v8. On Intel Mac: Linux/amd64.
    pub fn current() -> Self;

    pub fn linux_amd64() -> Self;
    pub fn linux_arm64() -> Self;
    pub fn linux_arm64_v8() -> Self;
}
```

`#[non_exhaustive]` on `Os` and `Arch` so OCI spec additions land without breaking.

---

## 18. Types NOT in this catalog

These types exist in the library but are *internal* (not exposed to users):

- `VzSend<T>` — `unsafe impl Send` wrapper for `!Send` Obj-C retaineds. Internal to `vmm`.
- `CancellationToken` trees — internal to cascading-cancellation plumbing.
- `Retained<VZ*>` — never exposed. See [`08-vmm-crate.md`](./08-vmm-crate.md).
- `DispatchQueue` — never exposed.
- `VsockConnector` (`tower::Service<Uri>`) — internal to `vminitd-client`.
- All `oci-client` / `oci-spec` types that aren't re-exported from `oci` — see [`07-oci-crate.md § re-exports`](./07-oci-crate.md).

---

## 19. Invariants worth locking

1. Every value-with-meaning gets a newtype. Raw `u32`, `u64`, `String` never appear in a public API where a typed alternative exists.
2. Enums over stringly-typed configuration wherever the set is closed (Capability, RlimitKind, SeccompAction, Os, Arch, MountFlags, NamespaceKind).
3. `#[non_exhaustive]` on enums where Linux / OCI / the future might add variants.
4. `Option<T>` per-subsystem on statistics structs means "not requested," not "zero."
5. `KilledReason` disambiguates OOM, host-signal, and guest-signal kills.
6. `Signal` is a re-export from `nix`. Don't reinvent.
7. `Rootfs`, `Mount`, `Network`, `BootLog`, `User` are enums (sum over variants), not tagged structs.
8. Sealed traits exist only where they encode capability (`ContainerStdio`, D-025) or orphan-rule-respecting extensions (`CoreContainerFactory`, D-018; `OciLayerSource`, D-024). Per `trait_design.md § most traits shouldn't exist`, nothing else in this catalog opens a polymorphism seam.
9. `Rootfs::OciBundle` wraps `oci::ImageBundle`, not `oci::Bundle` (D-020 rename).
10. `Rootfs` (ImplicitVm) and `VmRootfs` (OnVm) are distinct types (D-023); `Rootfs::OciBundle` on an `OnVm` builder is a compile error.
11. `BlockDeviceId` is the only handle that crosses from `VmConfigBuilder::block_device(path)` to `VmRootfs` (D-022); users never construct one manually.
12. `Streams` / `Pty` are non-constructable zero-sized markers (D-025); only used in type position.
13. Compile-time validated `container_id!`, `virtiofs_tag!`, `hostname!` macros exist alongside runtime fallible `::new(s)` constructors (D-027); dynamic strings still go through `::new()?`.
14. Shared value-type ownership lives in `firkin-types` (D-015); `VsockStream`/`VsockListener`/`VsockPeer` live in `firkin-vsock` (D-016); everything is re-exported from `firkin` (the facade).

Proceed to [`05-error-model.md`](./05-error-model.md) for the error hierarchy.
