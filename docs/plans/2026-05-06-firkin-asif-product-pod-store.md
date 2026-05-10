# Firkin ASIF Product Pod Store Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make ASIF a real product pod-store image format for Firkin Apple/VZ pods, with the same shared-rootfs pod behavior already proven on raw ext4 stores.

**Architecture:** ASIF remains a host disk-image format, not a guest filesystem. Firkin will build a raw ext4 pod-store image with the existing `firkin-ext4` writer, convert that image to ASIF with macOS `diskutil image create from --format ASIF`, and then boot the VM with the ASIF image as the predeclared pod-store block device. This avoids fake raw writes into `.asif` files and avoids adding `mkfs.ext4` to `vminitd` for this slice.

**Tech Stack:** Rust `firkin-vmm`, `firkin-runtime`, `firkin-e2b`, Apple Virtualization.framework, macOS `diskutil image create blank/from/attach`, `firkin-ext4`, signed Apple/VZ live tests, jj.

---

## Research Notes

Apple's current `VZDiskImageStorageDeviceAttachment` documentation says Virtualization supports two disk image formats, RAW and ASIF, and shows `diskutil image create blank --fs none --format ASIF --size SIZE IMAGE_PATH` as the creation path before initializing `VZDiskImageStorageDeviceAttachment`. Apple's Virtualization updates also call out ASIF storage utilization for VM images with `VZDiskImageStorageDeviceAttachment`.

The local SDK header is still stale and says only RAW disk images are supported. Runtime proof on macOS 26.3 is stronger than that comment:

- `VZDiskImageStorageDeviceAttachment(url:readOnly:cachingMode:synchronizationMode:)` accepts a blank `.asif` file.
- The same ASIF attachment composes with `VZVirtioBlockDeviceConfiguration`.
- The same ASIF attachment composes with `VZNVMExpressControllerDeviceConfiguration`.
- A signed Firkin live VM smoke booted an ASIF data disk, wrote `firkin-asif-smoke` from the guest, read it back from the same block device, and passed in 14.51s.

That proves ASIF is a valid local VZ disk-image attachment on this host. The remaining product proof is not VZ attach; it is whether the product pod-store route can use the ASIF image generated from Firkin's ext4 writer.

The product route proof also passed after wiring ASIF pod-store conversion:

```text
product pod create image_format=Asif size_bytes=536870912 elapsed_ms=14548
test live_apple_vz_product_pod_route_uses_asif_pod_store ... ok
test result: ok. 1 passed; finished in 15.57s
```

This proves the current conversion path is product-correct. The timing is acceptable for correctness but not yet proof that ASIF should become the default for 5-7 GiB pod stores; that decision still needs the larger autoscale benchmark matrix.

Local command research on this host showed:

```bash
diskutil image create blank --format ASIF --fs None --size 16MiB /tmp/test.asif
diskutil image attach --plist --noMount /tmp/test.asif
diskutil image create from --format ASIF /tmp/source.raw /tmp/converted.asif
```

The important result is `diskutil image create from --format ASIF <raw> <asif>` preserves raw block contents when the ASIF is attached again as a host block device. That gives Firkin a safe host-side formatting path:

```text
firkin-ext4 Writer -> sparse raw ext4 file
diskutil image create from --format ASIF raw.ext4 pod-store.asif
VZDiskImageStorageDeviceAttachment(pod-store.asif)
guest mount /dev/vdX as ext4
```

This is better than adding guest `mkfs.ext4` right now because the synthesized `init.block` does not currently carry `mkfs.ext4`, and `vminitd` already has the right abstraction boundary for mounting, trimming, usage, cleanup, and OCI layer application. Formatting host-created image containers is a host-storage concern.

## Path Ranking As Of 2026-05-06

The implementation should not confuse "best first product path" with "fastest path we could eventually build."

| Path | Product readiness | Expected speed | Risk | Decision |
| --- | --- | --- | --- | --- |
| Raw ext4 file -> `diskutil image create from --format ASIF` | high | good enough to prove; conversion cost must be measured | low | implement now |
| Blank ASIF -> host attach -> direct ext4 writer to `/dev/r<disk>` | medium | plausibly fastest creation path | medium/high: block-device alignment, attach/eject cleanup, host-device safety | benchmark later |
| Blank ASIF -> VZ boot -> guest `mkfs.ext4` -> mount | medium | unknown; VM must boot before formatting | medium: `init.block` lacks `mkfs.ext4` today | later if we need generic blank-volume formatting |
| Raw only | already working | fastest known current path | low | keep as default until ASIF is benchmarked |
| Runtime USB mass-storage ASIF attach | possible | unknown; not expected to beat predeclared block storage | medium/high: hotplug lifecycle, guest discovery, USB storage path | not the pod scaling path |

For the current product ask, the conversion path wins because it:

- uses Apple's documented ASIF image container support;
- uses local `diskutil` functionality that preserves raw block contents;
- keeps ext4 construction in the existing Rust writer;
- avoids host block-device mutation in library code;
- avoids growing `init.block` with e2fsprogs before we know conversion cost is a real bottleneck.

The implementation must add timing evidence for ASIF creation in the signed product smoke. If conversion dominates pod creation for 5-7 GiB stores, the follow-up should be direct ASIF population through an attached host block device with block-aligned writes, not guest `mkfs.ext4` by default.

## Success Definition

This work is complete only when all of these are true:

1. `firkin-vmm` exposes a typed disk-image conversion API, not raw `Command` calls from `firkin-runtime`.
2. `DiskImageFormat::Asif` product pod stores create an ASIF file from an ext4 raw source rather than hard-failing.
3. The runtime removes the intermediate raw ext4 image after successful ASIF conversion.
4. Product runtime capabilities move `pod-store-asif` from unsupported to supported.
5. Existing raw product pod-store behavior remains covered.
6. Existing ASIF unsupported tests are replaced with positive ASIF creation and live product tests.
7. The lower-level signed ASIF VZ smoke remains recorded as passing on macOS 26.3.
8. A signed live product pod route test boots an ASIF pod store, creates a pod, adds a container, removes it, and deletes the pod.
9. The signed ASIF product smoke reports pod-store preparation or route creation timing so we can decide whether direct ASIF population is worth implementing next.
10. A Just target runs the signed ASIF product pod smoke directly.
11. The final closeout lists exact jj commit IDs and verification commands.

## Non-Goals

- Do not add `mkfs.ext4` or `FormatExt4` to `vminitd` in this slice.
- Do not make ASIF the default pod-store format until benchmark evidence says it should be.
- Do not add NVMe support here. This path uses the existing VZ disk-image attachment and virtio-blk device lowering.
- Do not write ext4 bytes directly into a `.asif` file path.
- Do not put new implementation logic in crate-root `lib.rs` files.

## Task 1: Add Typed Disk-Image Conversion In `firkin-vmm`

**Files:**
- Create: `crates/vmm/src/disk_image.rs`
- Modify: `crates/vmm/src/lib.rs`
- Test: `crates/vmm/tests/config.rs`

**Step 1: Move disk-image creation code out of `lib.rs`**

Move these existing types/functions into `crates/vmm/src/disk_image.rs`:

```rust
pub enum DiskImageFormat { Raw, Asif }
pub struct BlankDiskImage { ... }
pub fn create_blank_disk_image(spec: BlankDiskImage) -> Result<()>
```

Keep `lib.rs` as map/glue only:

```rust
mod disk_image;
pub use disk_image::{
    BlankDiskImage, DiskImageConversion, DiskImageFormat, convert_disk_image,
    create_blank_disk_image,
};
```

**Step 2: Add conversion type**

Add:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskImageConversion {
    source: PathBuf,
    destination: PathBuf,
    format: DiskImageFormat,
}
```

Constructors:

```rust
DiskImageConversion::new(source, destination, DiskImageFormat::Asif)
DiskImageConversion::asif(source, destination)
```

Accessors:

```rust
source()
destination()
format()
```

**Step 3: Add conversion implementation**

Implement:

```rust
pub fn convert_disk_image(spec: DiskImageConversion) -> Result<()>
```

For ASIF and RAW, call:

```bash
diskutil image create from --format <RAW|ASIF> <source> <destination>
```

Errors must use existing `firkin_vmm::Error::InvalidConfig` with messages that include source path, destination path, target format, and `diskutil` stderr/stdout summary.

**Step 4: Tests**

Add vmm tests that:

- assert `DiskImageConversion::asif` records source, destination, and format.
- create a tiny sparse raw source file with a sector-aligned marker.
- call `convert_disk_image(... Asif)`.
- attach the ASIF with `diskutil image attach --plist --noMount`, read the first sector from `/dev/r<disk>`, and assert the marker survives.
- always eject the attached disk in cleanup.

Run:

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-asif-target cargo test -q -p firkin-vmm
```

Expected: PASS.

Observed result:

```text
product pod create image_format=Asif size_bytes=536870912 elapsed_ms=14548
test live_apple_vz_product_pod_route_uses_asif_pod_store ... ok
test result: ok. 1 passed; finished in 15.57s
```

**Step 5: Commit**

```bash
jj describe -m "feat: add typed disk image conversion"
jj new
```

## Task 2: Wire ASIF Product Pod Store Preparation

**Files:**
- Modify: `crates/runtime/src/single_node/apple_vz.rs`

**Step 1: Replace ASIF hard-fail**

Change `AppleVzLocalRuntimeDriver::prepare_product_pod_store` so:

```text
Raw:
  staging_dir/pod-store.ext4
  write_empty_pod_store(path, requested_size)

Asif:
  staging_dir/pod-store.raw.ext4
  staging_dir/pod-store.asif
  write_empty_pod_store(raw_path, requested_size)
  convert_disk_image(DiskImageConversion::asif(raw_path, asif_path))
  remove raw_path
  return asif_path
```

Do not silently fall back to raw.

**Step 2: Move capability**

Move `"pod-store-asif"` from `unsupported` to `supported` in `apple_local_runtime_capabilities()`.

**Step 3: Add narrow tests**

Replace `asif_product_pod_store_is_explicitly_unsupported` with positive tests:

- raw uses `.ext4`.
- ASIF uses `.asif`.
- ASIF removes `pod-store.raw.ext4`.
- `sharedRootfs=false` still returns `UnsupportedCapability`.

Run:

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-asif-target cargo test -q -p firkin-runtime --lib product_pod_store
```

Expected: PASS.

**Step 4: Commit**

```bash
jj describe -m "feat: enable asif product pod stores"
jj new
```

## Task 3: Add Product Route ASIF Coverage

**Files:**
- Modify: `crates/runtime/tests/product_pods.rs`
- Modify: `Justfile`

**Step 1: Replace unsupported route test**

Change the current ASIF route test from expecting failure to a focused create-path assertion if it can run without booting. Keep it small and avoid duplicate full pod lifecycle.

**Step 2: Add signed live route**

Add:

```rust
#[tokio::test]
#[ignore = "signed live Apple/VZ ASIF pod route smoke; boots a VM"]
async fn live_apple_vz_product_pod_route_uses_asif_pod_store() { ... }
```

It should:

1. Create a pod with `PodStoreOptions { image_format: Asif, size_bytes: 512 * 1024 * 1024, ..Default::default() }`.
2. Run the same marker-writing container as the raw smoke.
3. Add a sidecar using the same template.
4. Delete the sidecar.
5. Delete the pod.
6. Print or write timing for pod creation, including ASIF pod-store preparation if exposed by the runtime.

Use this as the product-level proof. The lower-level signed ASIF VZ attach proof already passed:

```bash
FIRKIN_ARM64_BUSYBOX_ROOTFS=/tmp/firkin-pod-smoke-rootfs/rootfs.ext4 \
CARGO_TARGET_DIR=/tmp/containerization-firkin-asif-target \
cargo test -q -p firkin-core --features real-vm --test builder \
  live_asif_local_disk_image_is_guest_visible_and_writable --no-run

# after signing the builder test binary with signing/vz.entitlements:
/tmp/containerization-firkin-asif-target/debug/deps/builder-* \
  live_asif_local_disk_image_is_guest_visible_and_writable \
  --exact --nocapture
```

Observed result:

```text
test live_asif_local_disk_image_is_guest_visible_and_writable ... ok
test result: ok. 1 passed; finished in 14.51s
```

**Step 3: Add Just target**

Add:

```just
live-runtime-pod-asif:
    scripts/run-signed-live-runtime-test.sh --test product_pods live_apple_vz_product_pod_route_uses_asif_pod_store
```

**Step 4: Run non-live tests**

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-asif-target cargo test -q -p firkin-runtime --test product_pods
```

Expected: PASS, with live tests ignored.

**Step 5: Commit**

```bash
jj describe -m "test: prove asif product pod route"
jj new
```

## Task 4: Signed Live Verification

**Files:**
- No source edits unless live evidence finds a real bug.

**Step 1: Run signed ASIF pod smoke**

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-asif-target just live-runtime-pod-asif
```

Expected: PASS.

**Step 2: Run regression suite**

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-asif-target cargo test -q -p firkin-vmm
CARGO_TARGET_DIR=/tmp/containerization-firkin-asif-target cargo test -q -p firkin-runtime --lib product_pod_store
CARGO_TARGET_DIR=/tmp/containerization-firkin-asif-target cargo test -q -p firkin-runtime --test product_pods
CARGO_TARGET_DIR=/tmp/containerization-firkin-asif-target cargo test -q -p firkin-runtime --test pod_autoscale
```

Expected: PASS.

**Step 3: Cleanup**

Remove temporary target output only after verification:

```bash
rm -rf /tmp/containerization-firkin-asif-target
```

**Step 4: Final status**

Report:

- exact commit IDs,
- exact verification commands,
- whether ASIF is supported as a product pod-store format,
- any remaining ASIF limitations, especially that ASIF is not yet the default and not benchmarked against raw/NVMe.
