# Firkin Storage and Pod Prerequisites Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Land the storage, rootfs, volume, and smoke-test prerequisites that make ASIF-backed pod stores and elastic same-VM pods implementable in Firkin before implementing pods end-to-end.

**Architecture:** Keep five layers separate: host image format, host storage backend, Virtualization.framework device kind, guest filesystem, and container rootfs mode. Prove ASIF with a signed live VZ smoke first. Then add the typed storage model, add a preboot pod-store disk, add `VmRootfs::GuestPath`, materialize container rootfs directories inside that pod store, and build pods e2e on top of guest paths rather than on runtime block-device hotplug.

**Tech Stack:** Rust workspace under `crates/`, `firkin-types`, `firkin-vmm`, `firkin-core`, vminitd RPCs over vsock, Apple Virtualization.framework, `diskutil image create blank`, signed `real-vm` tests, jj.

---

## Status

Design and implementation plan.

Current local result, 2026-05-06:

- ASIF product pod-store support is now live-proven through the host-side
  raw-ext4-to-ASIF conversion path. The current product route smoke creates a
  512 MiB ASIF pod store, creates the pod, adds a sidecar, removes the sidecar,
  and deletes the pod:
  `product pod create image_format=Asif size_bytes=536870912 elapsed_ms=14548`;
  `test live_apple_vz_product_pod_route_uses_asif_pod_store ... ok`;
  1 passed, finished in 15.57s.
- Task 2 ASIF smoke passed with a blank `.asif` local disk image attached as a
  writable virtio-blk data disk.
- The passing smoke used Firkin's product helper and builder path:
  `create_blank_disk_image(BlankDiskImage::new(..., DiskImageFormat::Asif))`
  plus `VmConfig::asif_disk_image(...)`.
  Command:
  `FIRKIN_ARM64_BUSYBOX_ROOTFS=/tmp/firkin-pod-smoke-rootfs/rootfs.ext4 /tmp/containerization-firkin-target/debug/deps/builder-ff21d578071e7e4d live_asif_local_disk_image_is_guest_visible_and_writable --exact --nocapture`.
  Latest result after product API and facade re-export changes: 1 passed,
  0 failed, finished in 4.73s.
- Task 4 pod-store smoke passed with a predeclared ext4 data disk mounted at
  `/run/firkin/pod-store`, bind-mounted into a container, explicitly synced,
  then remounted after VM reboot.
  Smoke: `live_pod_store_ext4_mounts_and_persists_guest_file`.
  Result: 1 passed, 0 failed, finished in 9.41s.
- Task 5 guest-path rootfs smoke passed with a copied busybox ext4 image
  mounted as the pod store and used directly as `VmRootfs::GuestPath`.
  Smoke: `live_on_vm_container_can_start_from_guest_path_rootfs`.
  Result: 1 passed, 0 failed, finished in 4.61s.
- Task 7 loopback setup smoke passed after moving `lo` setup into
  `standard_guest_setup` through vminitd `IpLinkSet`.
  Smoke: `live_two_busybox_containers_share_loopback_without_net_admin`.
  Result: 1 passed, 0 failed, finished in 5.32s.
- Task 6 rootfs materialization now works from an OCI bundle into the mounted
  pod store. The materializer rewrites layers into vminitd-compatible gzip tar
  archives, preserves files/directories/symlinks/mode/uid/gid/mtime, converts
  OCI hardlink entries into relative symlinks, and fails explicitly for
  whiteouts, device nodes, and other unsupported entry types.
  Smoke: `live_pod_store_materializes_busybox_rootfs_and_starts_container`.
  Current result: 1 passed, 0 failed, finished in 5.64s.
- Task 8 first pod e2e now passes over the preboot pod store with no runtime VZ
  storage attach. Smokes:
  `live_pod_two_busybox_containers_share_emptydir`,
  `live_pod_two_busybox_containers_share_loopback`, and
  `live_pod_add_and_remove_container_without_vm_reboot`.
  Current grouped command:
  `CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test -q -p firkin-core --features real-vm --test builder live_pod_ -- --nocapture --test-threads=1`.
  Result: 5 passed, 0 failed, finished in 47.44s. One older env-var-backed
  pod-store persistence smoke printed a skip warning in this grouped run
  because `FIRKIN_ARM64_BUSYBOX_ROOTFS` was not set; the new OCI-backed pod
  smokes ran and passed.

This plan supersedes the runtime-storage pieces of
`docs/plans/2026-05-04-firkin-pod-support-design.md`, specifically:

- PR 2 "runtime block-device attach" through
  `VZVirtualMachine.attachDevice:completionHandler:`.
- PR 3 `BlockDeviceGuard` attach-then-spawn rollback for container rootfses.
- Any statement that a runtime-added pod container must be backed by a new VZ
  block device.

The pod API shape in that document is still useful. The storage substrate under
it changes.

## Hard Corrections

### 1. Storage runtime attach is not `VZVirtualMachine.attachDevice`

Do not implement storage attach by calling
`VZVirtualMachine.attachDevice:completionHandler:`. The local SDK headers show
runtime USB attach through `VZUSBController.attachDevice:completionHandler:`,
and they show `VZUSBMassStorageDevice` as the storage-shaped device that can be
passed to that controller. They do not show a general running-VM storage attach
method on `VZVirtualMachine`.

Implication:

- The primary elastic-pod design must not require one hot-plugged VZ disk per
  added container.
- Runtime USB mass storage can be a later spike, not the pod PR1 substrate.
- Old plan text that says "VZ runtime block attach" must be treated as stale.

### 2. ASIF is a host disk image format, not a guest filesystem

ASIF belongs at the host image-format layer. It is not an ext4 replacement
inside the Linux guest.

Correct layer split:

```rust
enum DiskImageFormat {
    Raw,
    Asif,
}

enum StorageAttachmentBackend {
    LocalDiskImage { path: PathBuf, format: DiskImageFormat },
    NetworkBlockDevice { url: String, timeout: Duration },
    DiskBlock { path: PathBuf },
}

enum BlockDeviceKind {
    VirtioBlk,
    Nvme,
    UsbMassStorage,
}

enum GuestFilesystem {
    Ext4,
    Xfs,
    NoneRawBlock,
}

enum VmRootfs {
    BlockDevice(BlockDeviceId),
    GuestPath(GuestPath),
}
```

This means:

- A `.asif` file can back a VZ storage attachment if VZ accepts it.
- The guest still sees a block device.
- The guest filesystem on that block device is still ext4 or xfs.
- Firkin's current ext4 writer cannot emit ASIF by writing bytes straight to
  the `.asif` file. It writes raw ext4 images.

### 3. NVMe may be a performance backend, not an elasticity backend

`VZNVMExpressControllerDeviceConfiguration` is a preboot storage device kind. It
can be benchmarked against virtio-blk for predeclared disks. It does not solve
runtime container add/remove by itself.

### 4. NBD is a backend, not a hotplug strategy

`VZNetworkBlockDeviceStorageDeviceAttachment` lets VZ connect to an NBD server.
It does not remove the need to decide how the device is presented to the guest.
NBD also brings a server lifecycle, a URL, timeout behavior, and networking
entitlements. Keep it out of the first pods e2e path.

### 5. Elastic pods should scale containers inside one guest filesystem

The primary pod path should be:

```text
boot VM
  init.block as /dev/vda
  pod-store disk as /dev/vdb

guest setup
  mount /dev/vdb at /run/firkin/pod-store
  create /run/firkin/pods/<pod-id>/
  create shared emptyDir mounts once per pod

add container
  materialize rootfs directory under /run/firkin/pods/<pod-id>/rootfs/<container-id>
  write OCI config with root.path = that guest path
  start process with namespaces/cgroups

remove container
  stop process
  unmount bind mounts
  delete rootfs directory or mark for cleanup
  optionally fstrim the pod-store filesystem
```

This avoids a VZ storage device per container. It also makes scale-up/scale-down
mostly guest work, which is the right place for pod elasticity.

## Current Repo Facts

### `firkin-vmm`

Current `BlockDevice` is a path-only local disk image:

```rust
pub struct BlockDevice {
    id: BlockDeviceId,
    path: PathBuf,
}
```

Current `VmConfigBuilder::block_device(path)` allocates a typed slot and stores
the path.

Current Apple VZ implementation builds storage as:

```text
VZDiskImageStorageDeviceAttachment(path, read_only)
  -> VZVirtioBlockDeviceConfiguration
  -> VZVirtualMachineConfiguration.storageDevices
```

Current implementation retains only disk-image attachments:

```rust
Vec<Retained<VZDiskImageStorageDeviceAttachment>>
Vec<Retained<VZVirtioBlockDeviceConfiguration>>
```

There is no storage attachment enum yet. There is no disk image format enum yet.
There is no NVMe or NBD path yet.

### `firkin-core`

Current implicit-VM rootfs type:

```rust
pub enum Rootfs {
    Ext4Image(PathBuf),
    OciBundle(Box<ImageBundle>),
    RawBlock(PathBuf),
}
```

Current running-VM rootfs type:

```rust
pub struct VmRootfs(BlockDeviceId);
```

Current `prepare_on_vm_bundle` always:

1. Connects to vminitd.
2. Runs standard guest setup.
3. Creates the bundle rootfs directory.
4. Maps the `BlockDeviceId` to `/dev/vd*`.
5. Mounts that block device as the container rootfs.

The hard-coded rootfs-device mapping is:

```rust
fn block_device_guest_path(id: BlockDeviceId) -> Result<String> {
    let slot = id.slot().get();
    let index = u8::try_from(slot.saturating_sub(1)).unwrap_or(u8::MAX);
    ...
    Ok(format!("/dev/vd{}", char::from(letter)))
}
```

That is correct for the current predeclared virtio-blk-only path. It is not
enough for guest-path rootfses, NVMe names, USB mass storage, or device
enumeration by stable guest identity.

### `diskutil`

Current host tool help shows:

```text
diskutil image create blank --format <format>
format choices: RAW, ASIF, UDSB
default is RAW unless the image path has .asif or .sparsebundle extension
--fs choices: APFS / ExFAT / MS-DOS / None
```

Use `--fs None` for Firkin pod-store test images. The guest should own the
Linux filesystem.

### Local SDK Header Tension

The local SDK header for `VZDiskImageStorageDeviceAttachment` still says:

```text
Only RAW data disk images are supported.
```

But current macOS tooling exposes ASIF creation, and Apple Virtualization docs
have moved in this area. That smoke gate has now passed on macOS 26.3: ASIF is
accepted by `VZDiskImageStorageDeviceAttachment`, composes with virtio-blk and
NVMe configuration objects, is guest-visible/writable in a signed Firkin VM
smoke, and works as a product pod-store image after converting a raw ext4 image
with `diskutil image create from --format ASIF`.

## Target Sequencing

Use a hard cutover between stages. Do not ship compatibility shims for the
stale runtime-attach plan.

```text
Task 0  Document correction and guardrails
Task 1  VMM storage model, still raw virtio-blk only
Task 2  ASIF live smoke, now passed
Task 3  ASIF local disk-image backend, now implemented through raw-ext4-to-ASIF conversion
Task 4  Pod-store disk model and guest mount lifecycle
Task 5  VmRootfs::GuestPath and OnVm bundle preparation
Task 6  Guest rootfs materializer for pod containers
Task 7  Pod emptyDir and loopback setup
Task 8  First pods e2e over preboot pod store
Task 9  Optional backend/performance spikes: NVMe, NBD, USB mass storage, xfs
```

Definition of ready for pod e2e:

- Firkin can boot a VM with a writable pod-store block device.
- Firkin can mount that pod-store device in the guest.
- Firkin can create two container rootfs directories under the pod store.
- `VmRootfs::GuestPath` can start a container with `root.path` set to an
  already-materialized guest path.
- Two same-VM containers can share `127.0.0.1`.
- Two same-VM containers can mount the same pod `emptyDir`.
- Scale-down can stop one container without killing the whole VM.

## Task 0: Correct The Plan Surface

### Files

- `docs/plans/2026-05-04-firkin-pod-support-design.md`
- `docs/specs/rust_rewrite/DECISIONS.md`
- `docs/specs/rust_rewrite/04-library-surface/01-container-surface.md`
- `docs/specs/rust_rewrite/04-library-surface/02-vm-surface.md`
- `docs/specs/rust_rewrite/04-library-surface/04-value-types.md`
- `docs/specs/rust_rewrite/04-library-surface/10-non-goals.md`

### Changes

Patch the existing pod design with a short supersession note:

```md
> Storage prerequisite correction, 2026-05-06:
> Runtime-added pod containers must not depend on
> `VZVirtualMachine.attachDevice:completionHandler:` for storage. The current
> implementation path is a preboot pod-store disk plus `VmRootfs::GuestPath`.
> See `docs/plans/2026-05-06-firkin-storage-pod-prerequisites.md`.
```

Do not rewrite the whole old pod design in this task. The goal is to stop
future implementation from following stale attach text.

Patch D-019/D-023 text only after Task 5 lands. Until then, leave the existing
v0.1 constraint intact and add a forward pointer that a guest-path mode is the
planned lift.

### Verification

```sh
rg -n "VZVirtualMachine.attachDevice|BlockDeviceGuard|runtime block-device attach" \
  docs/plans/2026-05-04-firkin-pod-support-design.md \
  docs/specs/rust_rewrite
```

Expected:

- The old terms may still appear in historical context.
- Any occurrence in active implementation instructions must point to this
  prerequisite plan or be marked stale.

## Task 1: Add A Typed VMM Storage Model Without Changing Behavior

### Goal

Create the type surface that can represent raw, ASIF, NBD, virtio-blk, NVMe,
and USB mass storage, while preserving current runtime behavior as:

```text
LocalDiskImage { format: Raw } + VirtioBlk
```

### Files

- `crates/vmm/src/lib.rs`
- `crates/vmm/src/vz.rs`
- `crates/vmm/tests/config.rs` if present, otherwise add focused tests under
  the existing vmm test module.

### Public Type Shape

Replace path-only `BlockDevice` with a structured config:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskImageFormat {
    Raw,
    Asif,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageAttachment {
    LocalDiskImage {
        path: PathBuf,
        format: DiskImageFormat,
        read_only: bool,
    },
    NetworkBlockDevice {
        url: String,
        timeout: Duration,
        read_only: bool,
    },
    DiskBlock {
        path: PathBuf,
        read_only: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockDeviceKind {
    VirtioBlk,
    Nvme,
    UsbMassStorage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockDevice {
    id: BlockDeviceId,
    attachment: StorageAttachment,
    device_kind: BlockDeviceKind,
}
```

Keep the ergonomic existing builder:

```rust
pub fn block_device(mut self, path: impl Into<PathBuf>) -> (Self, BlockDeviceId)
```

It must lower to:

```rust
StorageAttachment::LocalDiskImage {
    path,
    format: DiskImageFormat::Raw,
    read_only: false,
}
BlockDeviceKind::VirtioBlk
```

Add explicit new builders:

```rust
pub fn local_disk_image(
    self,
    path: impl Into<PathBuf>,
    format: DiskImageFormat,
    kind: BlockDeviceKind,
) -> (Self, BlockDeviceId);

pub fn readonly_local_disk_image(
    self,
    path: impl Into<PathBuf>,
    format: DiskImageFormat,
    kind: BlockDeviceKind,
) -> (Self, BlockDeviceId);
```

Do not expose NBD or USB builders until a smoke exists. The enum can exist, but
public construction should stay behind crate-internal helpers or explicit
`#[doc(hidden)]` test-only helpers if needed.

### VZ Implementation

In `crates/vmm/src/vz.rs`, replace:

```rust
Vec<Retained<VZDiskImageStorageDeviceAttachment>>
Vec<Retained<VZVirtioBlockDeviceConfiguration>>
```

with an internal retained parts struct that can hold multiple attachment and
device classes:

```rust
struct StorageDeviceParts {
    disk_image_attachments: Vec<Retained<VZDiskImageStorageDeviceAttachment>>,
    // later:
    // nbd_attachments: Vec<Retained<VZNetworkBlockDeviceStorageDeviceAttachment>>,
    // disk_block_attachments: Vec<Retained<VZDiskBlockDeviceStorageDeviceAttachment>>,
    virtio_blk_devices: Vec<Retained<VZVirtioBlockDeviceConfiguration>>,
    // later:
    // nvme_devices: Vec<Retained<VZNVMExpressControllerDeviceConfiguration>>,
    // usb_mass_storage_devices: Vec<Retained<VZUSBMassStorageDeviceConfiguration>>,
}
```

Task 1 only implements:

```text
LocalDiskImage + VirtioBlk
```

Historical gate for a fresh implementation was: before Task 2 passed, return an
explicit unsupported error for `DiskImageFormat::Asif`. That gate is now closed
for this macOS 26.3 runtime.

```rust
Error::InvalidConfig {
    reason: "ASIF disk images require the ASIF live smoke to pass before product use".into(),
}
```

Current working-copy result: Task 2 and Task 3 passed locally, so
`DiskImageFormat::Asif` now lowers through the same
`VZDiskImageStorageDeviceAttachment` path as raw local disk images.

### Tests

Add unit tests for:

- `block_device(path)` still produces slot 1, raw local disk image,
  virtio-blk, writable.
- `readonly_local_disk_image(path, Raw, VirtioBlk)` produces read-only raw
  local disk image.
- `local_disk_image(path, Asif, VirtioBlk)` is representable in config.
- Boot lowering preserves `DiskImageFormat::Asif` for ASIF local disk images.

### Verification

```sh
cargo fmt --all
CARGO_TARGET_DIR=/tmp/containerization-firkin-target \
  cargo test -q -p firkin-vmm --test config
CARGO_TARGET_DIR=/tmp/containerization-firkin-target \
  cargo test -q -p firkin-core --test builder --no-run
```

Expected:

- Existing core builder tests still compile.
- No live VM test is required for Task 1.

## Task 2: Add The ASIF Live Smoke

### Goal

Prove whether VZ accepts an ASIF local disk image attachment on this host. The
smoke must run before any product API claims ASIF support.

### Test Placement

Prefer:

- `crates/core/tests/builder.rs`

Reason: this file already owns signed `real-vm` smokes that boot vminitd,
start containers, and exercise block-device rootfs behavior.

If Task 1 adds enough public vmm plumbing to smoke lower-level devices without
containers, a vmm integration test is also acceptable. Do not create both.

### Smoke Name

```rust
live_asif_local_disk_image_is_guest_visible_and_writable
```

### Host Setup

Use the existing busybox rootfs env var:

```sh
export FIRKIN_ARM64_BUSYBOX_ROOTFS=/tmp/firkin-pod-smoke-rootfs/rootfs.ext4
```

Create the ASIF image inside the test tempdir, not in a fixed global path:

```sh
diskutil image create blank \
  --format ASIF \
  --fs None \
  --size 67108864 \
  "$tmpdir/pod-store.asif"
```

Use byte size to avoid unit parsing drift.

### VM Shape

The test VM should boot with:

```text
/dev/vda = init.block, read-only
/dev/vdb = busybox rootfs, writable enough for current smoke behavior
/dev/vdc = blank ASIF pod-store candidate
```

The container rootfs remains the known raw busybox ext4 image. The ASIF image is
only an extra data disk for the smoke.

### Guest Assertion

Do not require `mkfs.ext4` in the guest. The busybox smoke rootfs may not carry
filesystem tooling.

Assert block visibility and raw read/write:

```sh
test -b /dev/vdc
printf firkin-asif-smoke >/tmp/asif-marker
dd if=/tmp/asif-marker of=/dev/vdc bs=512 count=1 conv=notrunc
dd if=/dev/vdc bs=512 count=1 2>/dev/null | head -c 18
```

Expected stdout contains:

```text
firkin-asif-smoke
```

If `/dev/vdc` enumeration is unstable after Task 1 introduces non-virtio-blk
device kinds, switch to a vminitd helper that enumerates block devices by size
or serial. For Task 2 with virtio-blk only, `/dev/vdc` is acceptable.

### Signing And Run Command

Build:

```sh
CARGO_TARGET_DIR=/tmp/containerization-firkin-target \
  cargo test -q -p firkin-core --features real-vm --test builder --no-run
```

Find the test binary:

```sh
ls -t /tmp/containerization-firkin-target/debug/deps/builder-* | head -n 1
```

Sign:

```sh
codesign --force --sign - \
  --entitlements signing/vz.entitlements \
  /tmp/containerization-firkin-target/debug/deps/builder-<hash>
```

Run:

```sh
FIRKIN_ARM64_BUSYBOX_ROOTFS=/tmp/firkin-pod-smoke-rootfs/rootfs.ext4 \
  /tmp/containerization-firkin-target/debug/deps/builder-<hash> \
  live_asif_local_disk_image_is_guest_visible_and_writable \
  --exact --nocapture
```

Expected:

```text
test live_asif_local_disk_image_is_guest_visible_and_writable ... ok
```

### Failure Handling

If `VZDiskImageStorageDeviceAttachment` rejects `.asif`:

- Do not implement ASIF product support.
- Keep raw local disk images as the pod-store substrate.
- Record the exact NSError text in this document or a follow-up smoke note.

If VZ accepts the ASIF file but the guest write fails:

- Separate attachment support from guest write semantics.
- Inspect read-only flags and VZ attachment mode before changing design.

If ASIF works only as NVMe and not virtio-blk:

- Move ASIF product support behind `BlockDeviceKind::Nvme`.
- Do not make ASIF the default until the NVMe smoke and benchmark pass.

## Task 3: Implement ASIF Local Disk Image Support If Smoke Passes

### Goal

Make ASIF a supported local disk image format for data/pod-store disks, not for
host-written ext4 rootfs images.

### Files

- `crates/vmm/src/lib.rs`
- `crates/vmm/src/vz.rs`
- `crates/core/src/lib.rs` only if core needs a convenience builder.
- `crates/core/tests/builder.rs`

### Product Surface

Add stable builder methods:

```rust
pub fn asif_disk_image(self, path: impl Into<PathBuf>) -> (Self, BlockDeviceId);

pub fn readonly_asif_disk_image(self, path: impl Into<PathBuf>) -> (Self, BlockDeviceId);
```

Lower both to:

```rust
StorageAttachment::LocalDiskImage {
    format: DiskImageFormat::Asif,
    ...
}
BlockDeviceKind::VirtioBlk
```

Only add an NVMe variant if Task 2 proves virtio-blk cannot carry ASIF and a
separate NVMe smoke proves the NVMe path works.

### Creation Helper

Add a host-side helper for blank pod-store images:

```rust
pub struct BlankDiskImage {
    pub path: PathBuf,
    pub size: Size,
    pub format: DiskImageFormat,
}

pub fn create_blank_disk_image(spec: BlankDiskImage) -> Result<()>;
```

For `DiskImageFormat::Raw`, implement in Rust:

```text
File::create(path)
set_len(size.bytes())
```

For `DiskImageFormat::Asif`, shell out to:

```sh
diskutil image create blank --format ASIF --fs None --size <bytes> <path>
```

Wrap tool failure as:

```rust
Error::InvalidConfig {
    reason: format!("failed to create ASIF disk image with diskutil: {stderr}")
}
```

Rationale: ASIF is an Apple host format. Firkin should not invent an ASIF
writer.

### Non-Goals

Do not make `Rootfs::ext4_image(path)` accept `.asif` as an ext4 image.

Do not change the ext4 writer to write ASIF directly.

Do not make ASIF the default rootfs image format.

### Verification

```sh
cargo fmt --all
CARGO_TARGET_DIR=/tmp/containerization-firkin-target \
  cargo test -q -p firkin-vmm
CARGO_TARGET_DIR=/tmp/containerization-firkin-target \
  cargo test -q -p firkin-core --features real-vm --test builder --no-run
codesign --force --sign - --entitlements signing/vz.entitlements \
  /tmp/containerization-firkin-target/debug/deps/builder-<hash>
FIRKIN_ARM64_BUSYBOX_ROOTFS=/tmp/firkin-pod-smoke-rootfs/rootfs.ext4 \
  /tmp/containerization-firkin-target/debug/deps/builder-<hash> \
  live_asif_local_disk_image_is_guest_visible_and_writable \
  --exact --nocapture
```

Expected:

- VMM unit tests pass.
- ASIF smoke passes.

## Task 4: Add Pod-Store Disk Model

### Goal

Add a first-class pod-store disk that is mounted once per VM/pod and used for
elastic container rootfs directories, emptyDir directories, writable overlays,
and future cache volumes.

### Files

- `crates/core/src/lib.rs`
- `crates/vmm/src/lib.rs`
- `crates/core/tests/builder.rs`
- New `crates/core/src/pod_store.rs` if splitting `core/src/lib.rs` becomes
  justified. Keep it in `lib.rs` if the surrounding code has not modularized
  yet.

### Type Shape

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodStoreSpec {
    pub id: PodStoreId,
    pub block_device: BlockDeviceId,
    pub guest_mount: GuestPath,
    pub filesystem: GuestFilesystem,
    pub format_if_blank: bool,
    pub trim_on_stop: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestFilesystem {
    Ext4,
    Xfs,
}
```

Initial support:

- `GuestFilesystem::Ext4` only.
- `format_if_blank = false` unless the guest tooling exists.
- Pod-store image is prepared by host or test setup before boot.

Do not require guest `mkfs.ext4` for the first implementation. Firkin already
knows how to produce raw ext4 images on the host. Product ASIF pod stores use
that raw ext4 source and convert it with `diskutil image create from --format
ASIF`; guest formatting can wait for a separate blank-volume feature.

### Mount Path

Use stable internal paths:

```text
/run/firkin/pod-store
/run/firkin/pods/<pod-id>
/run/firkin/pods/<pod-id>/rootfs/<container-id>
/run/firkin/pods/<pod-id>/emptydir/<volume-name>
/run/firkin/pods/<pod-id>/overlay/<container-id>/{upper,work}
```

Do not place pod-owned runtime data under `/tmp`.

### Guest Setup Flow

Add:

```rust
async fn mount_pod_store(
    client: &mut VminitdClient,
    device: BlockDeviceId,
    spec: &PodStoreSpec,
) -> Result<MountedPodStore>
```

For now, device path can still use `block_device_guest_path` if the pod store
is virtio-blk. The function must be named so Task 9 can replace the mapping
with stable block-device discovery later.

Flow:

```text
mkdir /run/firkin
mkdir /run/firkin/pod-store
mount /dev/vdX -> /run/firkin/pod-store as ext4
mkdir /run/firkin/pods
```

### Verification

Add a live smoke:

```rust
live_pod_store_ext4_mounts_and_persists_guest_file
```

Shape:

1. Create raw ext4 pod-store image on host.
2. Boot VM with busybox rootfs and pod-store block device.
3. Mount pod store in guest.
4. Write `/run/firkin/pod-store/marker`.
5. Stop VM.
6. Reboot same image.
7. Mount pod store.
8. Assert marker still exists.

Expected:

```text
test live_pod_store_ext4_mounts_and_persists_guest_file ... ok
```

This test should pass with raw first. Repeat with ASIF only if Task 3 landed.

## Task 5: Add `VmRootfs::GuestPath`

### Goal

Allow containers in an already-running VM to use an already-mounted guest path
as rootfs, rather than requiring a predeclared block device per container.

### Files

- `crates/core/src/lib.rs`
- `crates/core/tests/builder.rs`
- `docs/specs/rust_rewrite/DECISIONS.md`
- `docs/specs/rust_rewrite/04-library-surface/01-container-surface.md`
- `docs/specs/rust_rewrite/04-library-surface/02-vm-surface.md`
- `docs/specs/rust_rewrite/04-library-surface/04-value-types.md`

### Type Change

Replace:

```rust
pub struct VmRootfs(BlockDeviceId);
```

with:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum VmRootfs {
    BlockDevice(BlockDeviceId),
    GuestPath(GuestPath),
}
```

Keep existing conversions:

```rust
impl From<BlockDeviceId> for VmRootfs {
    fn from(value: BlockDeviceId) -> Self {
        Self::BlockDevice(value)
    }
}
```

Update:

```rust
Rootfs::block_device(id) -> VmRootfs::BlockDevice(id)
```

Add:

```rust
impl VmRootfs {
    pub fn guest_path(path: impl Into<GuestPath>) -> Self;
    pub fn as_block_device(&self) -> Option<BlockDeviceId>;
    pub fn as_guest_path(&self) -> Option<&GuestPath>;
}
```

If `GuestPath` does not exist yet, add a narrowly validated internal newtype:

```rust
pub struct GuestPath(String);
```

Validation:

- Must be absolute.
- Must not contain `..`.
- Must not contain NUL.
- Must not be `/`.

### Bundle Preparation Change

Change:

```rust
async fn prepare_on_vm_bundle(
    vm: &VirtualMachine<Running>,
    builder: &ContainerBuilder<impl VmContext, impl BuilderState>,
    rootfs_device: BlockDeviceId,
    spec: &Spec,
) -> Result<VminitdClient>
```

to:

```rust
async fn prepare_on_vm_bundle(
    vm: &VirtualMachine<Running>,
    builder: &ContainerBuilder<impl VmContext, impl BuilderState>,
    rootfs: &VmRootfs,
    spec: &Spec,
) -> Result<VminitdClient>
```

Then split:

```rust
match rootfs {
    VmRootfs::BlockDevice(device) => {
        mkdir bundle.rootfs_path()
        let source = block_device_guest_path(*device)?;
        mount_container_rootfs(... source ...)
    }
    VmRootfs::GuestPath(path) => {
        ensure_guest_rootfs_path_exists(...)
        write spec.root.path = path
        skip rootfs block-device mount
    }
}
```

Important: vminitd's `Bundle.create` and config writing may still expect the
bundle directory layout. Do not bypass bundle creation blindly. The precise
change should be:

- Keep the bundle directory and `config.json` location under
  `/run/container/<id>/`.
- Set OCI `root.path` in the config to the guest-path rootfs.
- Skip only the `mount /dev/vdX -> bundle.rootfs_path()` step.

If the current `ContainerBundle::write_config_request(spec)` always writes the
bundle's rootfs path into `spec.root.path`, change the spec-building stage so
`Root { path }` is chosen before encoding.

### Tests

Unit tests:

- Existing `BlockDeviceId` rootfs still builds the same mount request.
- `VmRootfs::GuestPath("/run/firkin/pods/p/rootfs/c")` writes that exact
  `root.path` into OCI config.
- Invalid guest paths are rejected.

Live smoke:

```rust
live_on_vm_container_can_start_from_guest_path_rootfs
```

Smoke shape:

1. Boot VM with busybox rootfs block device.
2. Mount that busybox rootfs once.
3. Copy or bind it to a guest path suitable as a rootfs.
4. Spawn an `OnVm` container with `VmRootfs::guest_path(...)`.
5. Run `/bin/echo guest-path-rootfs`.

If copying a rootfs tree is not available yet, defer this live smoke to Task 6
and keep Task 5 unit-only.

### Verification

```sh
cargo fmt --all
CARGO_TARGET_DIR=/tmp/containerization-firkin-target \
  cargo test -q -p firkin-core --test builder --no-run
```

If live smoke is present:

```sh
codesign --force --sign - --entitlements signing/vz.entitlements \
  /tmp/containerization-firkin-target/debug/deps/builder-<hash>
FIRKIN_ARM64_BUSYBOX_ROOTFS=/tmp/firkin-pod-smoke-rootfs/rootfs.ext4 \
  /tmp/containerization-firkin-target/debug/deps/builder-<hash> \
  live_on_vm_container_can_start_from_guest_path_rootfs \
  --exact --nocapture
```

## Task 6: Add Guest Rootfs Materialization

### Goal

Materialize OCI or ext4-derived rootfs contents into a directory inside the
mounted pod store so runtime-added pod containers do not need a new VZ disk.

### First Implementation Choice

Use a tar stream into the guest if possible:

```text
host OCI bundle/layers
  -> host prepares a merged tar stream with whiteouts resolved
  -> vminitd receives/extracts into /run/firkin/pods/<pod>/rootfs/<container>
```

Do not require host mounting of Linux filesystems on macOS.

Do not try to mutate an ext4 image while it is mounted by the guest.

### Files

- `crates/core/src/lib.rs`
- `crates/vminitd-client` crate if present
- vminitd protobuf/client files if extraction RPCs are missing
- `crates/ext4` only if reusing its OCI layer merge logic requires a shared
  trait extraction.

### Required Fidelity

The materializer must preserve:

- Directories.
- Regular files.
- Symlinks.
- Hardlink behavior explicitly. The current implementation rewrites hardlinks
  as relative symlinks before sending the archive to vminitd, because vminitd's
  copy-in extractor applies symlinks but not hardlink entries.
- Executable bits.
- UID/GID.
- Device nodes, or fail explicitly until supported.
- Whiteouts.
- Opaque directories.
- Xattrs, or fail explicitly until supported.

Do not silently flatten unsupported OCI features. A pod rootfs that starts but
has wrong filesystem semantics is worse than a loud unsupported error.

### API Shape

```rust
pub enum PodRootfsSource {
    GuestPath(GuestPath),
    OciBundle(ImageBundle),
    Ext4Image(PathBuf),
}

pub struct MaterializedRootfs {
    pub path: GuestPath,
    pub source_digest: Option<String>,
}

async fn materialize_rootfs_in_pod_store(
    client: &mut VminitdClient,
    pod_store: &MountedPodStore,
    container_id: &ContainerId,
    source: &PodRootfsSource,
) -> Result<MaterializedRootfs>
```

Initial scope can be:

- `GuestPath` pass-through.
- `OciBundle` for simple single-layer busybox/alpine images.
- `Ext4Image` only if there is a guest-side mount-and-copy helper.

### Live Smoke

```rust
live_pod_store_materializes_busybox_rootfs_and_starts_container
```

Shape:

1. Boot VM with pod-store disk.
2. Materialize a busybox rootfs into pod store.
3. Start a container with `VmRootfs::GuestPath`.
4. Assert `/bin/busybox` exists.
5. Run `/bin/echo materialized-rootfs`.

Expected:

```text
materialized-rootfs
```

Current result:

```text
test live_pod_store_materializes_busybox_rootfs_and_starts_container ... ok
1 passed, 0 failed, finished in 5.64s
```

Implementation note:

- OCI layers are not streamed through directly. They are normalized into a
  vminitd-compatible gzip tar archive first.
- Hardlinks are converted to relative symlinks. This keeps BusyBox-style
  applet images compact and executable without relying on vminitd hardlink
  extraction support.
- Whiteouts, device nodes, and other unsupported entry types fail explicitly.

## Task 7: Add Pod `emptyDir` And Loopback Setup

### Goal

Make the two pod features required for meaningful e2e:

- Shared pod-local volume.
- Shared VM loopback.

### EmptyDir V1

Implement `emptyDir` as one pod-owned mount, not one mount per container.

Type:

```rust
pub enum EmptyDirMedium {
    Memory,
    Disk,
}

pub struct EmptyDirVolume {
    pub name: String,
    pub medium: EmptyDirMedium,
    pub size_limit: Option<Size>,
}
```

Initial support:

- `Memory`: mount tmpfs once at
  `/run/firkin/pods/<pod-id>/emptydir/<volume-name>`.
- `Disk`: create a directory inside pod store at
  `/run/firkin/pods/<pod-id>/emptydir/<volume-name>` with no separate mount.

Mount into containers as bind mounts:

```text
source: /run/firkin/pods/<pod-id>/emptydir/<volume-name>
target: <container mount path>
type: bind
options: rbind,rw
```

Tests:

- Two containers write/read the same marker through the same emptyDir.
- Removing one container does not delete the pod emptyDir.
- Stopping the pod removes memory emptyDir.

### Loopback Setup

Current same-VM containers share the VM network namespace because the runtime
does not unshare network namespaces. That is good. The missing piece is making
`lo` reliably up even when no external network is configured.

Add:

```rust
async fn ensure_loopback_up(client: &mut VminitdClient) -> Result<()>
```

Implementation:

```text
IpLinkSet { name: "lo", up: true }
```

Call it from standard guest setup or pod setup. It must not depend on
`VmConfig.networks()` being non-empty.

This should let future smokes drop CAP_NET_ADMIN from container specs when the
only reason for the capability is `ip link set lo up`.

Live smoke:

```rust
live_two_pod_containers_share_loopback_without_net_admin
```

Shape:

1. Start pod VM with no host network attachment if supported.
2. Pod setup calls `ensure_loopback_up`.
3. Start listener container:
   `nc -l -p 18080 -s 127.0.0.1`.
4. Start client container:
   `printf marker | nc 127.0.0.1 18080`.
5. Assert listener receives marker.
6. Container spec does not include CAP_NET_ADMIN.

## Task 8: Implement First Pods E2E Over Pod Store

### Goal

Implement real pod e2e using the substrate from Tasks 4 to 7. Do not use VZ
runtime storage attach.

### Files

- `crates/core/src/lib.rs` or new `crates/core/src/pod.rs`
- `crates/core/tests/pod.rs` or existing `crates/core/tests/builder.rs`
- `docs/plans/2026-05-04-firkin-pod-support-design.md` for API alignment
- `crates/firkin/src/lib.rs` for facade re-exports if needed

### Minimal Pod API

```rust
pub struct Pod {
    id: PodId,
    vm: VirtualMachine<Running>,
    store: MountedPodStore,
    containers: HashMap<ContainerId, Container<Streams>>,
}

pub struct PodBuilder {
    id: PodId,
    vm_config: VmConfig,
    pod_store: PodStoreSpec,
    volumes: Vec<PodVolume>,
    containers: Vec<PodContainerSpec>,
    share_process_namespace: bool,
}

impl PodBuilder {
    pub fn empty_dir(self, volume: EmptyDirVolume) -> Self;
    pub fn container(self, spec: PodContainerSpec) -> Self;
    pub async fn spawn(self) -> Result<Pod>;
}

impl Pod {
    pub async fn add_container(&mut self, spec: PodContainerSpec) -> Result<Container<Streams>>;
    pub async fn remove_container(&mut self, id: &ContainerId) -> Result<()>;
    pub async fn stop(self) -> Result<()>;
}
```

### Initial Restrictions

- One VM is the pod security boundary.
- No runtime VZ disk attach.
- New containers may only use the existing pod store and existing pod volumes.
- New pod volumes cannot be added after pod spawn.
- `share_process_namespace = false` first. Pause-container support can follow.
- Cgroup limits can be represented in types but enforcement can land after the
  first pods e2e if that is explicitly documented.

### E2E Smokes

1. `live_pod_two_busybox_containers_share_emptydir`

   Flow:

   ```text
   pod with emptyDir("work")
   container a writes /work/marker
   container b reads /work/marker
   assert marker
   ```

2. `live_pod_two_busybox_containers_share_loopback`

   Flow:

   ```text
   pod setup brings lo up
   container server listens on 127.0.0.1
   container client connects to 127.0.0.1
   assert marker
   ```

3. `live_pod_add_and_remove_container_without_vm_reboot`

   Flow:

   ```text
   spawn pod with container a
   add container b
   b reads emptyDir created before b existed
   remove b
   a still runs and can exec
   VM remains running
   ```

4. `live_pod_snapshot_restore_preserves_two_container_state`

   Only after the existing single-VM snapshot path is confirmed compatible with
   pod-store mounts.

### Verification

```sh
cargo fmt --all
CARGO_TARGET_DIR=/tmp/containerization-firkin-target \
  cargo test -q -p firkin-core --test builder --no-run
CARGO_TARGET_DIR=/tmp/containerization-firkin-target \
  cargo test -q -p firkin-core --features real-vm --test builder --no-run
codesign --force --sign - --entitlements signing/vz.entitlements \
  /tmp/containerization-firkin-target/debug/deps/builder-<hash>
FIRKIN_ARM64_BUSYBOX_ROOTFS=/tmp/firkin-pod-smoke-rootfs/rootfs.ext4 \
  /tmp/containerization-firkin-target/debug/deps/builder-<hash> \
  live_pod_two_busybox_containers_share_emptydir \
  --exact --nocapture
FIRKIN_ARM64_BUSYBOX_ROOTFS=/tmp/firkin-pod-smoke-rootfs/rootfs.ext4 \
  /tmp/containerization-firkin-target/debug/deps/builder-<hash> \
  live_pod_two_busybox_containers_share_loopback \
  --exact --nocapture
FIRKIN_ARM64_BUSYBOX_ROOTFS=/tmp/firkin-pod-smoke-rootfs/rootfs.ext4 \
  /tmp/containerization-firkin-target/debug/deps/builder-<hash> \
  live_pod_add_and_remove_container_without_vm_reboot \
  --exact --nocapture
```

Expected:

- All live smokes pass.
- No smoke depends on runtime VZ disk attach.
- Loopback smoke does not grant CAP_NET_ADMIN only to bring up `lo`.

Current result:

```text
CARGO_TARGET_DIR=/tmp/containerization-firkin-target \
  cargo test -q -p firkin-core --features real-vm --test builder \
  live_pod_ -- --nocapture --test-threads=1

running 5 tests
warn: FIRKIN_ARM64_BUSYBOX_ROOTFS not set; skipping pod-store live smoke
test result: ok. 5 passed; 0 failed; 0 ignored; finished in 47.44s
```

The warning belongs to the older env-var-backed pod-store persistence smoke.
The OCI-backed materialization, emptyDir, loopback, and add/remove pod e2e
smokes ran and passed in the same command.

## Task 9: Optional Backend And Performance Spikes

These are useful, but not prerequisites for first pods e2e.

### NVMe

Question:

```text
Does `VZNVMExpressControllerDeviceConfiguration` materially improve pod-store
or rootfs performance versus virtio-blk?
```

Add only after Task 1:

```rust
BlockDeviceKind::Nvme
```

Smoke:

```rust
live_nvme_local_disk_image_is_guest_visible_and_writable
```

Guest device name will not be `/dev/vd*`; expect `/dev/nvme*n*`. This requires
stable block-device discovery before product use.

Benchmark:

```text
raw+virtio-blk ext4 pod-store
raw+nvme ext4 pod-store
asif+virtio-blk ext4 pod-store
asif+nvme ext4 pod-store, if ASIF+NVMe works
```

### NBD

Question:

```text
Can Firkin use a local NBD server for expandable or remote pod-store disks
without unacceptable lifecycle and entitlement cost?
```

Prerequisites:

- Local NBD server process.
- `com.apple.security.network.client` entitlement when required.
- Delegate support for connect/error callbacks.
- Shutdown cleanup.

Keep NBD behind an explicit feature or experimental API until this is proven.

### USB Mass Storage Runtime Attach

Question:

```text
Can Firkin hotplug a data disk through `VZUSBController.attachDevice` and make
Linux see it reliably?
```

This is for overflow volumes or debug attach, not pod container rootfs scale-up.

Prerequisites:

- XHCI controller configured at VM boot.
- Objective-C bindings for `VZUSBController`, `VZUSBMassStorageDevice`, and
  `VZUSBMassStorageDeviceConfiguration`.
- Guest uevent/block-device discovery retry.
- Detach behavior test.

Smoke:

```rust
live_usb_mass_storage_hotplug_is_guest_visible_and_detachable
```

Do not put this on the pods critical path.

### XFS

Question:

```text
Is xfs better than ext4 for pod-store metadata-heavy workloads?
```

Prerequisites:

- Guest kernel supports xfs.
- Host or guest can create xfs images.
- vminitd mount RPC can pass filesystem type/options.

Keep ext4 as baseline until this is benchmarked.

## Reclaim And Scale-Down Policy

Pod scale-down has two levels:

1. Container scale-down:
   - Stop process.
   - Unmount bind mounts.
   - Delete or tombstone rootfs directory.
   - Delete per-container overlay dirs.
   - Leave shared pod volumes intact.

2. VM footprint scale-down:
   - Run `fstrim` on pod-store mount after deletions if available.
   - Use memory compaction/ballooning only as best-effort.
   - Recycle idle or high-water VMs when host footprint matters.

Do not promise that deleting files inside the guest immediately shrinks host
allocated bytes. Reclaim is an explicit lifecycle operation.

Future API:

```rust
pub enum TrimPolicy {
    Off,
    OnStop,
    Manual,
    Periodic(Duration),
}

impl Pod {
    pub async fn trim_store(&mut self) -> Result<TrimReport>;
}
```

Initial default:

```text
TrimPolicy::OnStop for pod stores
Manual trim exposed for tests and operators
```

## Risk Register

| Risk | Why it matters | Mitigation |
|---|---|---|
| ASIF rejected by VZ despite host tool support | Local SDK header still says raw-only | Closed on macOS 26.3 by signed VZ and product route smokes |
| ASIF works for attach but not host-written ext4 | ext4 writer emits raw bytes, ASIF is not raw | Convert raw ext4 images with `diskutil image create from --format ASIF`; never write raw bytes directly to `.asif` |
| `/dev/vd*` mapping breaks with NVMe/USB | Current mapping assumes virtio-blk | Add stable guest block discovery before non-virtio product use |
| Rootfs materializer loses whiteouts/xattrs | OCI rootfs becomes semantically wrong | Fail explicitly for unsupported features |
| `emptyDir` cleanup races with live containers | Shared volume outlives one container | Pod owns volume lifecycle, not container |
| Loopback only works with CAP_NET_ADMIN today | Current smoke grants capability for setup | Move `lo` setup to vminitd/pod setup |
| NBD lifecycle leaks server/process | NBD adds host service management | Keep NBD experimental |
| Runtime USB detach races with guest mounts | Hotplug cleanup is hard | Keep USB off pods critical path |
| Host disk bytes keep growing | Sparse images need guest trim | Add explicit trim lifecycle |
| VM memory does not shrink after container stop | macOS/VZ memory reclaim is cooperative | Recycle high-water VMs |

## Not Now

Do not implement these before first pods e2e:

- General runtime VZ storage attach API.
- NBD product backend.
- USB mass storage product backend.
- NVMe default backend.
- XFS default pod-store filesystem.
- ASIF-backed host-written container rootfs images.
- Kubernetes API compatibility.
- Full CRI implementation.
- Cgroup memory enforcement, unless a pod e2e requires it.
- Live disk resize.
- Background memory-reclaim promises.

## Source Pointers

Local repo:

- `crates/vmm/src/lib.rs` for `BlockDevice`, `VmConfig`, and block-device
  builder surface.
- `crates/vmm/src/vz.rs` for
  `VZDiskImageStorageDeviceAttachment -> VZVirtioBlockDeviceConfiguration`.
- `crates/core/src/lib.rs` for `Rootfs`, `VmRootfs`, `prepare_on_vm_bundle`,
  `block_device_guest_path`, and existing live builder smokes.
- `docs/plans/2026-05-04-firkin-pod-support-design.md` for the pod API and
  Swift `LinuxPod` prior art.

Host/SDK:

- `diskutil image create blank --help` for ASIF image creation.
- `VZDiskImageStorageDeviceAttachment.h` for local raw-only header warning.
- `VZUSBMassStorageDevice.h` and `VZUSBController.h` for runtime USB attach
  shape.
- `VZNVMExpressControllerDeviceConfiguration.h` for preboot NVMe storage
  shape.
- `VZNetworkBlockDeviceStorageDeviceAttachment.h` for NBD attachment shape.

## Completion Criteria

The prerequisite phase is complete when all of these are true:

- The old pod plan is marked with a storage supersession note.
- VMM has a typed storage model.
- Raw local disk image behavior remains green.
- ASIF has signed live VZ and product route smoke results recorded.
- ASIF local data disks and ASIF product pod stores are supported as product paths.
- A pod-store disk can be mounted in the guest.
- `VmRootfs::GuestPath` can start an `OnVm` container without a per-container
  VZ disk.
- Rootfs materialization into pod store works for the smoke image.
- `emptyDir` is pod-owned and shared by at least two containers.
- Loopback is configured by the guest setup path, not by per-container
  CAP_NET_ADMIN.
- Pods e2e passes without runtime VZ storage attach.
