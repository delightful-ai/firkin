# Rust ecosystem — cloned + verified (corrected)

All repos cloned to `~/vendor/github.com/<owner>/<repo>`.
Verified with `cargo check` **and** exit-code inspection (initial pass was bugged — `cargo ... | tail` reports tail's exit code, not cargo's; don't repeat that mistake).

## Host-side VMM driver

### ✅ `objc2-virtualization` — works after submodule init

- **Gotcha**: `generated/` in the `objc2` workspace is a git submodule (`objc2-generated.git`). A plain `git clone` leaves it empty and `cargo check` fails with missing symbols. Fix: `git submodule update --init --depth 1`. Do this in the vendoring script.
- After submodule init: `cargo check -p objc2-virtualization --all-features` → **EXIT=0**.
- Coverage audit against `Cargo.toml` default features (line refs are in `~/vendor/github.com/madsmtm/objc2/framework-crates/objc2-virtualization/Cargo.toml`):

| Need | Feature / Type | Confirmed |
|---|---|---|
| VM lifecycle | `VZVirtualMachine`, `VZVirtualMachineConfiguration`, `VZVirtualMachineDelegate` | ✅ |
| Linux boot | `VZLinuxBootLoader` | ✅ |
| EFI boot | `VZEFIBootLoader` | ✅ |
| Block devices | `VZVirtioBlockDeviceConfiguration`, `VZDiskImageStorageDeviceAttachment`, `VZDiskBlockDeviceStorageDeviceAttachment` | ✅ |
| Networking (NAT) | `VZNATNetworkDeviceAttachment` | ✅ |
| Networking (vmnet) | `VZVmnetNetworkDeviceAttachment` | ✅ |
| Networking (raw) | `VZFileHandleNetworkDeviceAttachment`, `VZBridgedNetworkDeviceAttachment` | ✅ |
| Vsock host side | `VZVirtioSocketDevice`, `VZVirtioSocketConnection`, `VZVirtioSocketListener` | ✅ |
| Rosetta | `VZLinuxRosettaDirectoryShare` | ✅ |
| Virtiofs | `VZVirtioFileSystemDeviceConfiguration`, `VZMultipleDirectoryShare` | ✅ |
| Entropy | `VZVirtioEntropyDeviceConfiguration` | ✅ |

**The scary one — vsock connect with completion handler — is real.** In `generated/Virtualization/VZVirtioSocketDevice.rs:77-84`:

```rust
#[unsafe(method(connectToPort:completionHandler:))]
pub unsafe fn connectToPort_completionHandler(
    &self,
    port: u32,
    completion_handler: &block2::DynBlock<
        dyn Fn(*mut VZVirtioSocketConnection, *mut NSError),
    >,
);
```

And `VZVirtioSocketConnection.rs:48-50` exposes the raw fd:

```rust
#[unsafe(method(fileDescriptor))]
pub unsafe fn fileDescriptor(&self) -> c_int;
```

**→ vsock↔tonic path is mechanically confirmed.** Call `connectToPort_completionHandler` via objc2+block2, receive a `VZVirtioSocketConnection` in the callback, pull `fileDescriptor()`, wrap as tokio `AsyncRead+AsyncWrite`, feed to hyper via a custom `Connector`. ~200 LOC of glue.

## OCI image pipeline

### ✅ `rust-oci-client` (package: `oci-client`) — ideal surface

- `cargo check` → **EXIT=0**.
- `src/client.rs:1239` exposes `pull_blob<T: AsyncWrite>` — streams blob bytes directly into any `AsyncWrite`, with **built-in digest verification** (both header digest and layer digest computed on-the-fly). Perfect for piping layers straight into an EXT4 image or tar extractor without buffering.
- Also exposes: `pull`, `pull_manifest`, `pull_manifest_and_config`, `pull_blob_stream`, `pull_blob_stream_partial` (resumable), `pull_referrers`, `fetch_manifest_digest`. Full client.

### ✅ `oci-spec-rs` (package: `oci-spec`)

- `cargo check` → **EXIT=0**. Covers image-spec and runtime-spec types. Standard reuse.

## Prior art / reference

### ✅ `krunkit` — good macOS-side VMM reference

- `cargo check` → **EXIT=0**.
- Different hypervisor substrate (Hypervisor.framework, not VZ), but the **shape** of the project (Rust VMM + `krunkit.entitlements` plist + `build.rs` + edk2 for EFI boot + Makefile-driven release) is the closest Rust prior-art to what we'd build. Worth reading before picking a project layout.

### ✅ Alternatives we don't plan to use but verified anyway

- `virtualization-rs` (suzusuzu) — `cargo check` passes (132 warnings). Older hand-rolled VZ bindings. Redundant alongside `objc2-virtualization`.
- `applevisor` (Impalabs) — `cargo check` passes. Hypervisor.framework — wrong altitude for us.
- `xhypervisor` (RWTH-OS) — `cargo check` passes. Same.

### ⏭ Skipped — not plain cargo

- `libkrun` — Makefile-orchestrated with feature flags (`EFI=1` for macOS). Not a pure cargo build. Reference only.
- `youki` — huge Linux-only workspace. Only relevant if we ever swap runc → youki inside vminitd (out of scope for v1).
- `bollard` — Docker API client, not on our path.

## vminitd reality check

Looked at `vminitd/Package.swift` and `vminitd/Makefile`:

- vminitd is **not self-contained**. Depends on:
  - Parent Swift package `containerization` (sibling: `path: "../"`) — pulls `Containerization`, `ContainerizationArchive`, `ContainerizationNetlink`, `ContainerizationIO`, `ContainerizationOS`.
  - grpc-swift-2 + grpc-swift-nio-transport + grpc-swift-protobuf
  - swift-log, swift-system, swift-argument-parser, swift-protobuf
  - Local C targets: `CVersion`, `LCShim`, `Cgroup`
- Built via `swift build --swift-sdk {aarch64,x86_64}-swift-linux-musl` — cross-compiles from macOS to a **static Linux musl ELF**. Requires: Swift 6.3 + the static-SDK artifact bundle (checksum pinned in `vminitd/Makefile`).
- Output: `vminitd/bin/vminitd` (+ `vmexec`). The top-level `make init` target bakes these into `bin/init.block` — an EXT4 image that's the init rootfs.

**Implication for a Rust port**: we do **not** need to rewrite vminitd or demand users install Swift. Strategy: build vminitd/vmexec once in CI using this repo, vendor the binaries (or `init.block`) as release artifacts of our Rust crate, load them at VM-boot time. Swift toolchain is a **build-time** dep of the binary blob, not a **use-time** dep of the Rust crate.

Later option (out of scope for v1): port vminitd to Rust using `tonic` as the gRPC server and `youki` or direct `runc` invocation for OCI runtime. Eliminates the Swift build-time dep entirely.

## Corrected EXT4 port scope

Earlier I handwaved "4,439 LOC of EXT4 + also the siblings." Actual siblings imported by EXT4 code:

| Module | Used by EXT4 | Role | Replace with |
|---|---|---|---|
| `ContainerizationArchive` (1,385 LOC) | Formatter+Unpack, EXT4+Formatter, EXT4Reader+Export | libarchive wrapper for reading tar/tar.gz | **`tar-rs` + `flate2`**, or `libarchive-rs` for parity |
| `ContainerizationOS` (5,968 LOC) | EXT4.swift | file ops, sysctl, terminal, user/group | **`std::fs`, `nix`, `rustix`** — tiny subset actually needed |
| `ContainerizationExtras` (2,714 LOC) | Formatter+Unpack | utility helpers | **ad-hoc** — only the functions EXT4 touches |
| `SystemPackage` | many | swift-system, cross-platform posix | **`rustix` or `nix`** |
| `Foundation` | all | stdlib | **`std`** |
| `CoreFoundation` | UnsafeLittleEndianBytes.swift | byte ops | **`std::mem`, `bytemuck`, or `zerocopy`** |

**Real port target: ~4,500 LOC of actual EXT4 logic.** The sibling modules don't need verbatim porting — just function-level replacement as their call sites come up during the port.

## What's still unverified (deferred)

- Full `cargo build` (not just check) of objc2-virtualization. Check covers the type graph; a real build would shake out link errors. Low risk — the crate is actively maintained.
- vminitd cross-compile on this machine. Requires swiftly + the static SDK artifact bundle. Not doing it until we actually need the binary.
- Whether a tonic client can be pointed at an AF_VSOCK-backed fd without hyper freaking out about unknown URI schemes. Very low risk — hyper supports arbitrary `Connector`s. But worth a spike.
