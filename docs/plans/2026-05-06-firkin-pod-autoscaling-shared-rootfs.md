# Firkin Pod Autoscaling Shared Rootfs Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make Firkin product pods scale many same-template containers inside one Apple/VZ pod VM by adding the missing guest-agent file/reclaim primitives first, then switching pod containers from full rootfs copies to shared template rootfs + per-container writable overlays, and finally proving the result with a signed live 64-container benchmark.

**Architecture:** `vminitd` is the guest-side privileged agent, so cleanup, fstrim, and OCI-layer application belong there instead of being emulated through ad hoc helper containers. Once those RPCs exist, `firkin-core::Pod` should materialize each OCI template once under the pod store, mount per-container overlay rootfses for runtime containers, remove per-container overlay state on scale-down, and expose storage sizing/trim/benchmark knobs through the product pod route. Keep code out of crate-root `lib.rs` files except for existing trait/route glue.

**Tech Stack:** Swift `vminitd` + `SandboxContext.proto`, `make protos`, `make vminitd`, `make init`, Rust `firkin-vminitd-client`, `firkin-core`, `firkin-e2b`, `firkin-runtime`, Apple Virtualization.framework, ext4 pod-store images, ASIF/raw VZ disk image attachments, signed live tests with `signing/vz.entitlements`, jj.

---

## Requirements For Success

### Functional Requirements

1. `vminitd` exposes first-class guest operations for path removal, filesystem trim, filesystem usage, and OCI layer application.
2. Product pod scale-down does not leave stopped container rootfs/upper/work/merged directories in the running pod store.
3. Product pod scale-down can reclaim sparse host disk image space by running guest `fstrim` on the pod-store mount.
4. For same-template containers in one pod VM, OCI layers are applied once per template digest, not once per container.
5. Each container gets an isolated writable overlay:
   - shared lower: `/run/firkin/pod-store/pods/<pod>/templates/<template-key>/rootfs`
   - per-container upper: `/run/firkin/pod-store/pods/<pod>/containers/<container>/upper`
   - per-container work: `/run/firkin/pod-store/pods/<pod>/containers/<container>/work`
   - per-container merged root: `/run/firkin/pod-store/pods/<pod>/containers/<container>/merged`
6. `PodCreateRequest` accepts a pod-store configuration that can request at least:
   - size in bytes, with 7 GiB accepted
   - raw local disk image
   - ASIF local disk image through raw-ext4-to-ASIF conversion
   - trim policy for none/on-stop/on-remove/manual
   - shared-rootfs on/off, default on for product pods
7. A signed live benchmark can create one pod VM and rapidly add/remove 64 same-template containers, producing a JSON artifact with p50/p95 timings and storage usage.

### Non-Functional Requirements

1. No broad compatibility shim. This is a hard cutover for pod internals: product pod OCI rootfses become shared-template overlays by default.
2. No new pod implementation code in `crates/*/src/lib.rs` unless it is existing trait or route dispatch glue.
3. All Rust commands in this checkout use `CARGO_TARGET_DIR=/tmp/containerization-firkin-target`.
4. Commit after each coherent slice with jj.
5. Every behavior change has a failing test first.
6. Live tests that boot VZ are ignored by default and run through ad-hoc signing with `signing/vz.entitlements`.

### Explicit Non-Goals

1. Do not build a production autoscaler/controller in this slice. This implements the primitives, product route knobs, and benchmark evidence needed for autoscaling.
2. Do not implement VZ runtime disk hotplug.
3. Do not expose fake NVMe/cache/sync controls until `firkin-vmm` has actual device/attachment support for them. The product storage profile may name measured current backends only.

---

## Current Repo Facts To Preserve

- `vminitd` server implementation: `vminitd/Sources/vminitd/Server+GRPC.swift`.
- Canonical Swift proto: `Sources/Containerization/SandboxContext/SandboxContext.proto`.
- Rust vendored proto copy: `crates/vminitd-client/proto/SandboxContext.proto`.
- Rust typed client: `crates/vminitd-client/src/lib.rs`.
- Core pod implementation: `crates/core/src/pod.rs`.
- Product pod API: `crates/e2b/src/pods.rs`.
- Apple/VZ product pod runtime: `crates/runtime/src/single_node/apple_vz.rs`.
- Existing live product pod smoke: `crates/runtime/tests/product_pods.rs`.
- Swift proto regeneration command: `make protos`.
- vminitd/init rebuild commands: `make vminitd` and `make init`.
- Signed Rust live test pattern: build the test binary, `codesign --force --sign - --timestamp=none --entitlements signing/vz.entitlements <test-bin>`, then run the ignored exact test.

---

## Task 1: Add vminitd Guest Path/Reclaim RPC Contract

**Files:**
- Modify: `Sources/Containerization/SandboxContext/SandboxContext.proto`
- Modify: `crates/vminitd-client/proto/SandboxContext.proto`
- Test: `crates/vminitd-client/tests/wrappers.rs`

**Step 1: Write failing Rust wrapper tests**

Add tests for typed request builders that do not exist yet:

```rust
#[test]
fn remove_path_request_records_recursive_allow_missing() {
    let request = RemovePath::recursive("/run/firkin/pod-store/pods/p/containers/c")
        .allow_missing(true)
        .into_request();

    assert_eq!(request.path, "/run/firkin/pod-store/pods/p/containers/c");
    assert!(request.recursive);
    assert!(request.allow_missing);
}

#[test]
fn fstrim_request_defaults_to_whole_mount() {
    let request = Fstrim::new("/run/firkin/pod-store").into_request();

    assert_eq!(request.path, "/run/firkin/pod-store");
    assert_eq!(request.minimum_bytes, 0);
}

#[test]
fn apply_oci_layer_request_names_archive_and_destination() {
    let request = ApplyOciLayer::new(
        "/run/firkin/layers/sha256-layer.tar.gz",
        "/run/firkin/pod-store/pods/p/templates/t/rootfs",
    )
    .into_request();

    assert_eq!(request.archive_path, "/run/firkin/layers/sha256-layer.tar.gz");
    assert_eq!(request.destination, "/run/firkin/pod-store/pods/p/templates/t/rootfs");
}

#[test]
fn filesystem_usage_request_records_path() {
    let request = FilesystemUsage::new("/run/firkin/pod-store").into_request();

    assert_eq!(request.path, "/run/firkin/pod-store");
}
```

**Step 2: Run tests to verify they fail**

Run:

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test -q -p firkin-vminitd-client --test wrappers remove_path_request_records_recursive_allow_missing
```

Expected: compile failure because `RemovePath`, `Fstrim`, `ApplyOciLayer`, and `FilesystemUsage` do not exist.

**Step 3: Extend both proto files**

Add RPCs after `Sync`:

```proto
  // Remove a guest path.
  rpc RemovePath(RemovePathRequest) returns (RemovePathResponse);
  // Trim unused blocks for the filesystem mounted at path.
  rpc Fstrim(FstrimRequest) returns (FstrimResponse);
  // Apply an OCI layer archive into an existing guest directory.
  rpc ApplyOciLayer(ApplyOciLayerRequest) returns (ApplyOciLayerResponse);
  // Return filesystem usage for a guest path.
  rpc FilesystemUsage(FilesystemUsageRequest) returns (FilesystemUsageResponse);
```

Add messages:

```proto
message RemovePathRequest {
  string path = 1;
  bool recursive = 2;
  bool allow_missing = 3;
}

message RemovePathResponse {}

message FstrimRequest {
  string path = 1;
  uint64 minimum_bytes = 2;
}

message FstrimResponse {
  uint64 trimmed_bytes = 1;
}

message ApplyOciLayerRequest {
  string archive_path = 1;
  string destination = 2;
}

message ApplyOciLayerResponse {
  uint64 entries_applied = 1;
}

message FilesystemUsageRequest {
  string path = 1;
}

message FilesystemUsageResponse {
  uint64 block_size = 1;
  uint64 total_blocks = 2;
  uint64 free_blocks = 3;
  uint64 available_blocks = 4;
}
```

**Step 4: Regenerate Swift protobufs**

Run:

```bash
make protos
```

Expected: `Sources/Containerization/SandboxContext/SandboxContext.pb.swift` and `Sources/Containerization/SandboxContext/SandboxContext.grpc.swift` update with the new messages and methods.

**Step 5: Add Rust request wrappers**

Add typed builders in `crates/vminitd-client/src/lib.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovePath {
    path: String,
    recursive: bool,
    allow_missing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fstrim {
    path: String,
    minimum_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyOciLayer {
    archive_path: String,
    destination: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilesystemUsage {
    path: String,
}
```

Each type gets a `new(...)` constructor and `into_request(...)` method. `RemovePath` also gets `recursive(...)`, `allow_missing(...)`, and non-recursive constructors.

**Step 6: Run wrapper tests**

Run:

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test -q -p firkin-vminitd-client --test wrappers
```

Expected: pass.

**Step 7: Commit**

Run:

```bash
jj describe -m "feat: add vminitd path reclaim rpc contract"
jj new -m "feat: implement vminitd path reclaim rpc handlers"
```

---

## Task 2: Implement vminitd RemovePath, Fstrim, ApplyOciLayer, FilesystemUsage

**Files:**
- Modify: `vminitd/Package.swift`
- Modify: `vminitd/Sources/LCShim/include/syscall.h`
- Modify: `vminitd/Sources/LCShim/syscall.c`
- Create: `vminitd/Sources/vminitd/PathOperations.swift`
- Modify: `vminitd/Sources/vminitd/Server+GRPC.swift`
- Test: `vminitd/Sources/vminitd/PathOperations.swift` via Swift build and signed live VM tests later

**Step 1: Add LCShim fstrim wrapper**

Add a Linux wrapper around `FITRIM`:

```c
int CZ_fstrim(const char *path, unsigned long long minimum_bytes, unsigned long long *trimmed_bytes);
```

Implementation opens `path` with `O_RDONLY | O_CLOEXEC`, runs `ioctl(fd, FITRIM, &range)`, writes the resulting `range.len` to `trimmed_bytes`, closes the fd, and returns `0` or `-1`.

**Step 2: Implement `PathOperations`**

Create a Swift helper that:

- validates paths are absolute
- rejects `""`, `/`, `/proc`, `/sys`, `/dev`, `/run/container`, and any path containing a NUL byte
- `removePath`: uses `FileManager.default.removeItem(at:)`, with `allowMissing`
- `fstrim`: calls `CZ_fstrim`
- `filesystemUsage`: calls `statvfs`
- `applyOciLayer`: opens `ArchiveReader(file:)` and applies entries to a directory with OCI whiteout semantics

Whiteout semantics:

- `.wh.<name>` removes `<name>` in the same destination directory
- `.wh..wh..opq` removes existing children of the target directory before applying later entries in that layer
- normal files, directories, symlinks, and hardlinks are applied with mode/uid/gid where available
- absolute paths or `..` path escapes fail

**Step 3: Wire RPC handlers**

Add methods to `Initd: ...SimpleServiceProtocol` in `Server+GRPC.swift`:

```swift
func removePath(request: Com_Apple_Containerization_Sandbox_V3_RemovePathRequest, context: GRPCCore.ServerContext) async throws -> Com_Apple_Containerization_Sandbox_V3_RemovePathResponse
func fstrim(request: Com_Apple_Containerization_Sandbox_V3_FstrimRequest, context: GRPCCore.ServerContext) async throws -> Com_Apple_Containerization_Sandbox_V3_FstrimResponse
func applyOciLayer(request: Com_Apple_Containerization_Sandbox_V3_ApplyOciLayerRequest, context: GRPCCore.ServerContext) async throws -> Com_Apple_Containerization_Sandbox_V3_ApplyOciLayerResponse
func filesystemUsage(request: Com_Apple_Containerization_Sandbox_V3_FilesystemUsageRequest, context: GRPCCore.ServerContext) async throws -> Com_Apple_Containerization_Sandbox_V3_FilesystemUsageResponse
```

All failures must return `RPCError(code: .internalError, message: "<operation>")` with the underlying cause.

**Step 4: Build Swift vminitd**

Run:

```bash
make vminitd
```

Expected: `vminitd/bin/vminitd` and `vminitd/bin/vmexec` rebuild.

**Step 5: Rebuild init image**

Run:

```bash
make init
```

Expected: `bin/init.block` is rebuilt from the new `vminitd`.

**Step 6: Commit**

Run:

```bash
jj describe -m "feat: implement vminitd path reclaim rpc handlers"
jj new -m "feat: prove vminitd path reclaim live"
```

---

## Task 3: Prove vminitd RPCs With Signed Live Rust Smokes

**Files:**
- Modify: `crates/core/tests/builder.rs`
- Possibly modify: `scripts/run-signed-live-runtime-test.sh` or add a core equivalent if no reusable helper exists

**Step 1: Add ignored live tests**

Add tests that boot vminitd and call the new RPCs directly:

- `live_vminitd_remove_path_removes_guest_directory`
- `live_vminitd_fstrim_accepts_pod_store_mount`
- `live_vminitd_apply_oci_layer_applies_whiteouts`
- `live_vminitd_filesystem_usage_reports_pod_store`

The layer test must create:

1. base archive with `/kept`, `/removed`, `/dir/old`
2. update archive with `.wh.removed`, `dir/.wh..wh..opq`, `/dir/new`
3. apply both through `ApplyOciLayer`
4. start a tiny container or use copy-out to prove `/removed` and `/dir/old` are gone and `/kept` and `/dir/new` exist

**Step 2: Verify ignored tests compile**

Run:

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test -q -p firkin-core --test builder live_vminitd_ --no-run
```

Expected: compile pass.

**Step 3: Run signed exact live tests**

Build the test binary and sign it:

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test -q -p firkin-core --test builder --no-run
test_bin="$(find /tmp/containerization-firkin-target/debug/deps -maxdepth 1 -type f -perm -111 -name 'builder-*' -exec stat -f '%m %N' {} \; | sort -nr | awk 'NR == 1 {print $2}')"
codesign --force --sign - --timestamp=none --entitlements signing/vz.entitlements "$test_bin"
"$test_bin" live_vminitd_remove_path_removes_guest_directory --ignored --exact --nocapture --test-threads=1
"$test_bin" live_vminitd_fstrim_accepts_pod_store_mount --ignored --exact --nocapture --test-threads=1
"$test_bin" live_vminitd_apply_oci_layer_applies_whiteouts --ignored --exact --nocapture --test-threads=1
"$test_bin" live_vminitd_filesystem_usage_reports_pod_store --ignored --exact --nocapture --test-threads=1
```

Expected: all four pass.

**Step 4: Commit**

Run:

```bash
jj describe -m "feat: prove vminitd path reclaim live"
jj new -m "feat: share pod rootfs templates"
```

---

## Task 4: Switch Core Pods To Shared Template Rootfs + Overlay Containers

**Files:**
- Modify: `crates/core/src/pod.rs`
- Modify: `crates/core/tests/builder.rs`

**Step 1: Write failing core tests**

Add non-live tests for layout and state:

- `pod_template_key_is_stable_for_same_oci_digest`
- `pod_container_layout_uses_template_lower_and_container_overlay`
- `pod_remove_container_requests_overlay_cleanup`
- `pod_materialization_uses_guest_apply_oci_layer_for_whiteouts`

These can assert path construction and request shaping without booting VZ.

**Step 2: Add core types**

Add private layout types in `pod.rs`:

```rust
struct PodTemplate {
    key: String,
    rootfs_path: GuestPath,
    source_digest: String,
}

struct PodContainerLayout {
    base: GuestPath,
    upper: GuestPath,
    work: GuestPath,
    merged: GuestPath,
}
```

Add fields to `Pod`:

```rust
templates: HashMap<String, PodTemplate>,
container_layouts: HashMap<ContainerId, PodContainerLayout>,
```

**Step 3: Materialize OCI templates once**

Change `Pod::add_container` for `PodRootfsSource::OciBundle`:

1. derive template key from bundle digest
2. if template missing:
   - create `/templates/<key>/rootfs`
   - copy each layer archive as a file into `/layers/<digest>`
   - call `ApplyOciLayer` for each layer into the template rootfs
3. create per-container `upper`, `work`, `merged`
4. mount overlay with existing `Mount` RPC:

```text
type=overlay
source=overlay
destination=<merged>
options=[
  "lowerdir=<template-rootfs>",
  "upperdir=<upper>",
  "workdir=<work>",
]
```

5. start the container with `VmRootfs::GuestPath(<merged>)`

**Step 4: Cleanup on container removal**

Change `Pod::remove_container`:

1. stop the container
2. `umount(<merged>, MNT_DETACH)` through existing `Umount`
3. `RemovePath(<container-layout-base>, recursive=true, allow_missing=true)`
4. optionally call `Fstrim(<pod-store>)` according to trim policy

**Step 5: Preserve GuestPath behavior**

For `PodRootfsSource::GuestPath`, keep direct rootfs use unless the caller explicitly requests overlay. The product route should use OCI shared templates.

**Step 6: Run core tests**

Run:

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test -q -p firkin-core --test builder pod_
```

Expected: pass.

**Step 7: Run signed live pod tests**

Run existing live pod add/remove and loopback smokes:

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test -q -p firkin-core --test builder --no-run
test_bin="$(find /tmp/containerization-firkin-target/debug/deps -maxdepth 1 -type f -perm -111 -name 'builder-*' -exec stat -f '%m %N' {} \; | sort -nr | awk 'NR == 1 {print $2}')"
codesign --force --sign - --timestamp=none --entitlements signing/vz.entitlements "$test_bin"
"$test_bin" live_pod_add_remove_smoke --ignored --exact --nocapture --test-threads=1
"$test_bin" live_pod_loopback_smoke --ignored --exact --nocapture --test-threads=1
```

Expected: pass.

**Step 8: Commit**

Run:

```bash
jj describe -m "feat: share pod rootfs templates"
jj new -m "feat: configure product pod storage"
```

---

## Task 5: Add Product Pod Storage, Trim, And Sharing Knobs

**Files:**
- Modify: `crates/e2b/src/pods.rs`
- Modify: `crates/e2b/tests/pods.rs`
- Modify: `crates/runtime/src/single_node/apple_vz.rs`
- Modify: `crates/runtime/tests/product_pods.rs`

**Step 1: Write failing product API tests**

Add tests that serialize/deserialize:

```json
{
  "podID": "pod_1",
  "podStore": {
    "sizeBytes": 7516192768,
    "imageFormat": "raw",
    "trimPolicy": "onRemove",
    "sharedRootfs": true
  },
  "containers": []
}
```

Expected initially: compile or serde failure because `podStore` does not exist.

**Step 2: Add request/response types**

Add:

```rust
pub struct PodStoreOptions {
    pub size_bytes: Option<u64>,
    pub image_format: PodStoreImageFormat,
    pub trim_policy: PodTrimPolicy,
    pub shared_rootfs: bool,
}

pub enum PodStoreImageFormat {
    Raw,
    Asif,
}

pub enum PodTrimPolicy {
    None,
    OnRemove,
    OnStop,
    Manual,
}
```

Defaults:

- `size_bytes`: 7 GiB for product pods
- `image_format`: `Raw` until ASIF wins the larger benchmark matrix; ASIF is now live-proven but not the default
- `trim_policy`: `OnRemove`
- `shared_rootfs`: `true`

**Step 3: Wire runtime storage size**

Change `AppleVzLocalRuntimeDriver::write_empty_pod_store` to accept `Size`, and use the request value instead of hard-coded `192 MiB`.

For `Raw`, keep host-side `firkin_ext4::Writer` creation.

For `Asif`, use the live-proven host-side image-container path:

- write the pod store as a raw ext4 image with the existing `firkin-ext4` writer
- convert that raw ext4 image with `diskutil image create from --format ASIF`
- attach the resulting `.asif` file as the pod-store disk
- remove the intermediate raw ext4 image after successful conversion

Do not add a vminitd formatting RPC for this slice. Runtime proof on macOS 26.3 shows ASIF is accepted by `VZDiskImageStorageDeviceAttachment`, composes with both virtio-blk and NVMe storage configurations, and is guest-visible/writable in a signed Firkin VM smoke. The product proof is therefore the ASIF pod-store route, not blank-image guest formatting.

**Step 4: Wire trim policy**

Pass trim policy into core pod creation and removal. On `OnRemove`, run `Fstrim` after each `remove_container`. On `OnStop`, run it once in `stop_pod` before VM stop.

**Step 5: Run product tests**

Run:

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test -q -p firkin-e2b --test pods
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test -q -p firkin-runtime --test product_pods
```

Expected: pass.

**Step 6: Commit**

Run:

```bash
jj describe -m "feat: configure product pod storage"
jj new -m "feat: add asif pod store formatting"
```

---

## Task 6: Prove ASIF Product Pod Store Route

**Files:**
- Modify: `Sources/Containerization/SandboxContext/SandboxContext.proto`
- Modify: `crates/vminitd-client/proto/SandboxContext.proto`
- Modify: `vminitd/Sources/vminitd/Server+GRPC.swift`
- Modify: `crates/vminitd-client/src/lib.rs`
- Modify: `crates/runtime/src/single_node/apple_vz.rs`
- Test: `crates/runtime/tests/product_pods.rs`

**Step 1: Use the live-proven ASIF path**

Use `firkin_vmm::convert_disk_image(DiskImageConversion::asif(raw, asif))`.

The path is:

```text
firkin-ext4 Writer -> raw ext4 pod-store image
diskutil image create from --format ASIF raw.ext4 pod-store.asif
VZDiskImageStorageDeviceAttachment(pod-store.asif)
guest mount /dev/vdX as ext4
```

Do not fake ASIF by writing raw ext4 bytes to a `.asif` file. Do not add guest `mkfs.ext4` or `FormatExt4` in this slice.

**Step 2: Add live ASIF product pod smoke**

Add ignored test:

```rust
#[tokio::test]
#[ignore = "signed live Apple/VZ ASIF product pod-store smoke; boots a VM"]
async fn live_apple_vz_product_pod_route_uses_asif_pod_store() { ... }
```

It must:

1. create pod with `podStore.imageFormat = "asif"`
2. run one container that writes to emptyDir
3. add one sidecar
4. remove sidecar
5. delete pod

**Step 3: Run signed ASIF smoke**

Run signed exact test with `product_pods-*` test binary. The lower-level signed ASIF VZ smoke already passed on macOS 26.3:

```text
test live_asif_local_disk_image_is_guest_visible_and_writable ... ok
test result: ok. 1 passed; finished in 14.51s
```

Observed: product ASIF route passes with `imageFormat: "asif"` advertised as supported.

```text
product pod create image_format=Asif size_bytes=536870912 elapsed_ms=14548
test live_apple_vz_product_pod_route_uses_asif_pod_store ... ok
test result: ok. 1 passed; finished in 15.57s
```

**Step 4: Commit**

Run:

```bash
jj describe -m "feat: add asif pod store formatting"
jj new -m "feat: benchmark pod autoscaling"
```

---

## Task 7: Add 64-Container Autoscaling Benchmark Artifact

**Files:**
- Create: `crates/runtime/tests/pod_autoscale.rs`
- Modify: `Justfile`
- Maybe modify: `scripts/run-signed-live-runtime-test.sh` to accept a test binary name, or add `scripts/run-signed-live-test.sh`

**Step 1: Add ignored benchmark test**

Create:

```rust
#[tokio::test]
#[ignore = "signed live Apple/VZ pod autoscale benchmark; boots a VM and creates many containers"]
async fn live_apple_vz_product_pod_autoscales_64_shared_template_containers() { ... }
```

Defaults:

- `FIRKIN_POD_AUTOSCALE_CONTAINERS=64`
- `FIRKIN_POD_AUTOSCALE_IMAGE=python:3.12-alpine`
- `FIRKIN_POD_AUTOSCALE_POD_STORE_BYTES=7516192768`
- artifact path: `target/firkin-live-evidence/pod-autoscale-evidence.json`

Benchmark sequence:

1. record host allocated bytes for pod-store image if available
2. create one pod with one initial code-interpreter container
3. add containers `ci-1` through `ci-63`
4. remove containers in reverse order
5. call manual trim if trim policy is not already `OnRemove`
6. delete pod
7. write JSON artifact

Artifact schema:

```json
{
  "containers": 64,
  "image": "python:3.12-alpine",
  "pod_store_bytes": 7516192768,
  "shared_rootfs": true,
  "template_cache_entries": 1,
  "image_format": "raw",
  "create_pod_ms": 16000,
  "add_container_ms": { "p50": 50, "p95": 75, "max": 100 },
  "remove_container_ms": { "p50": 260, "p95": 300, "max": 350 },
  "guest_usage_before_remove_bytes": 80000000,
  "guest_usage_after_remove_bytes": 78000000,
  "host_allocated_before_trim_bytes": 200000000,
  "host_allocated_after_trim_bytes": 199000000,
  "failures": []
}
```

**Step 2: Add Justfile target**

Add:

```make
live-runtime-pod-autoscale:
    mkdir -p target/firkin-live-evidence
    FIRKIN_POD_AUTOSCALE_ARTIFACT="$PWD/target/firkin-live-evidence/pod-autoscale-evidence.json" scripts/run-signed-live-runtime-test.sh --test pod_autoscale live_apple_vz_product_pod_autoscales_64_shared_template_containers
    FIRKIN_POD_AUTOSCALE_ARTIFACT="$PWD/target/firkin-live-evidence/pod-autoscale-evidence.json" cargo test -q -p firkin-runtime --test pod_autoscale pod_autoscale_evidence_artifact_at_env_path_is_valid -- --ignored --exact
```

This uses the generalized signed-test runner so the autoscale integration test
binary can be signed directly.

**Step 3: Run smoke scale first**

Run:

```bash
FIRKIN_POD_AUTOSCALE_CONTAINERS=8 just live-runtime-pod-autoscale
```

Expected: pass and write artifact.

**Step 4: Run 64-container benchmark**

Before running, confirm disk space:

```bash
df -h .
```

Then run:

```bash
FIRKIN_POD_AUTOSCALE_CONTAINERS=64 \
FIRKIN_POD_AUTOSCALE_IMAGE=python:3.12-alpine \
FIRKIN_POD_AUTOSCALE_POD_STORE_BYTES=7516192768 \
just live-runtime-pod-autoscale
```

Expected: pass, artifact written, no leaked pod VM, no leaked staging directory.

**Step 5: Add artifact validator**

Add a small runtime test or CLI validator that rejects:

- any benchmark step failure
- missing p50/p95 data
- container count below requested count
- missing storage usage
- host allocated bytes increasing after remove+trim when the backend reports allocated size

**Step 6: Commit**

Run:

```bash
jj describe -m "feat: benchmark pod autoscaling"
jj new
```

---

## Task 8: Final Verification Sweep

**Files:** no planned source edits.

**Step 1: Run Rust checks**

Run:

```bash
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test -q -p firkin-vminitd-client
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test -q -p firkin-core --test builder pod_
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test -q -p firkin-e2b
CARGO_TARGET_DIR=/tmp/containerization-firkin-target cargo test -q -p firkin-runtime --no-run
scripts/check-firkin-crate-graph.sh
```

Expected: all pass. Existing unrelated warnings may be reported, but no new warnings in changed files.

**Step 2: Run signed live smokes**

Run:

```bash
just live-runtime-pod-autoscale
```

Also rerun the exact product pod route smoke from `crates/runtime/tests/product_pods.rs`.

Expected: both pass.

**Step 3: Inspect jj stack**

Run:

```bash
jj status
jj log -r 'trunk()..@' --no-graph
```

Expected: working copy clean, commits split by the tasks above.

---

## Success Definition

This work is complete only when all of these are true:

1. New vminitd RPCs compile in Swift and Rust.
2. Signed live vminitd RPC smoke proves `RemovePath`, `Fstrim`, `ApplyOciLayer`, and `FilesystemUsage`.
3. Core pod add/remove uses shared template rootfs + per-container overlay for OCI bundles.
4. Removing a running pod container unmounts and deletes its per-container overlay directory while preserving the shared template rootfs.
5. Product pod API accepts a 7 GiB pod-store request.
6. Product pod runtime uses the requested pod-store size.
7. Raw product pod-store path is live-proven.
8. ASIF pod-store path is live-proven through raw-ext4-to-ASIF conversion.
9. A signed live benchmark creates and removes 64 same-template containers in one pod VM and writes a JSON artifact with timings and storage usage.
10. The final response includes exact commit IDs and exact verification commands.
