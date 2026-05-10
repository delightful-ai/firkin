# Phase 1 execution plan — shipping v0.1.0

The spike phase answered every scary question. This plan is how we turn nine throwaway spike binaries into a library we can publish.

**Pole star**: a user can `cargo add firkin`, write ~15 lines of Rust, run it on a macOS 26+ box, and get a container exec'd with reachable networking.

If at any point during Phase 1 we're doing something that doesn't obviously move us closer to that outcome, we're off track.

---

## What v0.1.0 is (and isn't)

### Is

- A **Rust library**: `firkin` (the facade; re-exports everything), `firkin-core` (orchestrator), plus publishable sibling crates (`firkin-types`, `firkin-ext4`, `firkin-oci`, `firkin-vsock`, `firkin-vmm`, `firkin-vminitd-client`, `firkin-vminitd-bytes`). See [`04-library-surface/`](./04-library-surface/) for the full public-API design.
- Supports macOS 26+, Apple Silicon (arm64 primary; x86_64 best-effort).
- Full feature set proven by S1–S9: boot a VM, pull an OCI image, build a container rootfs via the Rust ext4 writer, boot vminitd from an embedded ELF, run a container process with reachable vmnet IP-per-container networking, stream stdio via inverse-vsock, handle Rosetta for amd64 guests.
- Ad-hoc codesigning with one entitlement. No Apple Developer Program required to build from source.
- A thin `cli` crate that exercises the library — dev-facing, not a Docker replacement.

### Isn't

- **Not a `docker` / `podman` / `apple/container` alternative.** Those are CLIs built on libraries. v0.1.0 is the library. A full CLI can land on top as v0.2+ or a separate project.
- **Not feature-complete.** See "Explicit deferrals" below. Deeper ext4 extent
  trees, htree directories, metadata checksums, journal support, bridged
  networking, Windows containers, and nested containers are deferred or scoped
  out. Multi-group ext4 and OCI whiteout merge semantics landed later in the
  Rust crate.
- **Not API-stable.** Pre-1.0. Breaking changes expected through v0.x.

### Ship criteria

v0.1.0 tag happens when **all** of these hold:

- [ ] Walking skeleton (§ "The walking skeleton" below) runs green in CI on a macos-14 runner.
- [ ] All nine spike proofs reproduce as library integration tests (gated behind `--features real-vm` — CI runs this on an arm64 runner at least nightly).
- [ ] `cargo fmt`, `cargo clippy -D warnings`, `cargo test --workspace` all green.
- [ ] Feature-matrix check is green: default feature set, `--features runtime-download`, `--features vendored-vminitd` each compile. D-017 makes these mutually exclusive with the default fetch-at-build path, so `--all-features` is not a valid invocation.
- [ ] Each crate has a README + rustdoc for public items.
- [ ] `examples/run-busybox.rs` works cold from a fresh clone.
- [ ] `DECISIONS.md` entries are reflected in the code (newtype-per-number, domain-error-per-capability, `BlockDeviceId` handles not string paths, `Rootfs`/`VmRootfs` split, etc.).

Nothing more. No premature docs site, no performance benchmarks beyond what the library needs to prove it works, no Docker-compat CLI.

---

## The walking skeleton

This is the first integration milestone. Until this passes, nothing is working yet. After it passes, everything else is feature + polish.

```rust
// examples/walking-skeleton.rs
use firkin::{Container, Rootfs};
use firkin::oci::{Client, Reference};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bundle = Client::default()
        .pull(&Reference::parse("docker.io/library/busybox:latest")?)
        .await?;

    let output = Container::builder("hello")
        .image_config(bundle.config())
        .rootfs(Rootfs::oci_bundle(bundle))
        .command(["/bin/echo", "hello from a real rust-rewrite container"])
        .output().await?;                         // D-021: one-call terminal

    assert!(output.status.success());
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}
```

That prints `hello from a real rust-rewrite container` after pulling busybox, writing an ext4 rootfs, booting a VM with vminitd, running `/bin/echo`, and streaming stdout via inverse-vsock. End-to-end. The exact surface this uses (`Container::builder`, `Rootfs::oci_bundle`, `.output()`, `oci::Client::default().pull`, `ImageBundle` via `ext4::OciLayerSource`) is pinned in [`04-library-surface/`](./04-library-surface/).

**Every crate is built in service of this being callable.** If a crate's feature doesn't show up here or in the "reproduce the spike proofs" integration tests, it's not in v0.1.0.

---

## Repo shape

Follow `03-project-layout.md`'s workspace layout. The Phase 1 delta is: we know exactly what goes in each crate now.

```
firkin/                            # D-014 (lib crate name = firkin; bin name = fk)
├── Cargo.toml                     # workspace manifest; [workspace.dependencies] has every pinned version
├── rust-toolchain.toml            # pin stable; no nightly in v0.1
├── .cargo/config.toml
├── build-tools/
│   └── build-vminitd/             # CI-side Swift toolchain + make recipe (from S3)
│       └── pin.toml               # apple/containerization SHA + Swift 6.3 + SDK SHA + ELF asset URL (D-017)
├── crates/
│   ├── types/                     # shared value types (D-015); no workspace deps
│   ├── ext4/                      # from S5; depends on `types`
│   ├── vsock/                     # from S2 + S4; depends on `types`; portable (D-016)
│   ├── vminitd-bytes/             # ~20 LOC: `include_bytes!` of the pinned ELF; build.rs downloads (D-017)
│   ├── vmm/                       # from S1 + S3 + S4 + S6 + S7 harnesses; depends on `vsock` + `types`
│   ├── vminitd-client/            # from S4 + S7 + S9 RPC usage; depends on `vsock` + `types`
│   ├── oci/                       # from S4 rootfs build path; depends on `ext4` + `types`
│   ├── core/                      # orchestrator; depends on all of the above
│   └── cli/                       # dev-facing, thin
├── examples/
│   ├── walking-skeleton.rs
│   └── s*-replay/                 # one example per spike, lifted
├── tests/
│   └── integration/               # cross-crate tests, gated behind --features real-vm
├── docs/                          # minimal
└── .github/workflows/
    ├── ci.yml
    ├── build-vminitd.yml
    └── release.yml
```

Entitlements.plist + a codesign wrapper script live at repo root (keep the ceremony visible to new contributors).

---

## Crate build sequence

Order matters for dependency reasons, but it's also how we stage the walking skeleton. Each section names its lift sources.

### 0. `types` (week 1, first)

**Lift from**: nothing; pure new code per D-015.

This crate is a leaf. Every other crate imports from it, so it has to land first. Contents:

- `ContainerId`, `ProcessId`, `VsockPort`, `VirtiofsTag`, `VmId`, `Size`, `Hostname`.
- `Platform`, `Os`, `Arch` (with `Platform::current()` et al).
- `NamespaceKind` (Linux namespace kinds for vminitd's RPCs).
- The corresponding `InvalidX` error types (`InvalidContainerId`, `InvalidHostname`, etc.).
- `#[forbid(unsafe_code)]`.
- Deps: `thiserror`, `uuid`.

**Ships**: ~300 LOC of pure value types with validated constructors. Every other crate and the facade re-export from here.

### 1. `ext4` (week 1)

**Lift from**: `~/tmp/rust-rewrite-spikes/s5-ext4/` + whatever codex's S5-Tier-4 pass produces.

**What changes going library-grade**:
- Public API split: `lib.rs` exposes `FileSystemBuilder`, `Image`, `Ext4Error`. `bin/` examples move to `examples/`.
- No `println!` — use `tracing`.
- Remove CLI preset shortcuts that don't belong in a library.
- Add a **contract test suite** per `beads-rs/docs/philosophy/test_design.md`: if we later add a "stream directly into a tokio AsyncWrite" impl, it runs the same laws.
- Keep the newtype + domain-error shape S5 established (D-007).

**Tier-4 status handling**: Tiers 1–4 passed — depth-1 extent trees, whiteouts (char-device markers), opaque-dir markers (`.wh..wh..opq`), guest-mount probes. The remaining Swift-reference features that did *not* land in the spike get an explicit `crates/ext4/DEFERRED.md` entry with the Phase 2 unlock condition: extent trees deeper than depth-1, htree directories, metadata_csum, inline_data, resize_inode. Multi-group images landed later in the Rust crate.

**Ships**: a publishable ext4 writer crate. No dependency on `vmm`, `vsock`, or anything macOS. Works on Linux CI too.

### 2. `vminitd-bytes` (week 1, parallel with ext4)

**Lift from**: S3's build recipe, S8's decision (Strategy A embedding the ELF), [D-017](./DECISIONS.md#d-017--vminitd-elf-distributed-via-pinned-download-not-checked-in) for the fetch model.

Content is essentially:

```rust
// Pinned at VMINITD_REV; regenerated by CI on pin.toml bump.
pub const VMINITD_AARCH64: &[u8] = include_bytes!(
    env!("VMINITD_AARCH64_PATH")
);
pub const VMINITD_SHA256: &str = env!("VMINITD_SHA256");
```

`build.rs` resolves `VMINITD_AARCH64_PATH` via D-017:

1. `FIRKIN_VMINITD_PATH` env var if set (offline / air-gapped override) — sha256-verified against `pin.toml`.
2. `$CARGO_TARGET_DIR/firkin-vminitd/<sha256>/vminitd-aarch64-unknown-linux-musl` if present in the per-workspace cache.
3. Otherwise: download from the pinned GitHub release asset URL in `pin.toml` using `ureq`, verify sha256, write to the cache path above. First build on a fresh machine; cached thereafter.
4. `--features vendored-vminitd` replaces the download path with `$CARGO_MANIFEST_DIR/../../vendor/vminitd/<target>/vminitd` (user supplies via git-LFS or local mirror; `vendor/vminitd/**` is `.gitignore`d).
5. `--features runtime-download` shifts fetch from build time to runtime and leaves the const empty — not used with the default.

Features `vendored-vminitd` and `runtime-download` are mutually exclusive with each other and with the default fetch-at-build path. Build fails loudly with a readable error if both are set.

Ships as the sole owner of the 131 MiB embed (per PRO_TIPS §30 — keep the blob in a leaf so nobody else eats the link tax).

### 3. `vsock` (week 2, parallel with early `vmm`)

**Lift from**: S2 (`VsockConnector`, `dial_vsock` FD wrapping) + S4 (listener-delegate FD production pattern) — note the split with `vmm` per [D-016](./DECISIONS.md#d-016--firkin-vsock-owns-streamlistener-types-vmm-depends-on-vsock).

**What this crate owns** (portable; no `objc2` deps):
- `VsockStream`: `AsyncRead + AsyncWrite + Send + Unpin` over an `OwnedFd` via `tokio::io::unix::AsyncFd`.
- `VsockListener`: async stream of `VsockStream`s from `OwnedFd`s fed in by an internal channel.
- `VsockPeer`: `(cid, VsockPort)` tuple.
- `VsockConnector`: `tower::Service<Uri>` for tonic/hyper integration (from S2).
- `TonicChannel` helper: takes a dialer + port, returns a `tonic::Channel`.
- `#[forbid(unsafe_code)]` at crate level; `#[allow(unsafe_code)]` only on the internal `AsyncFd` wrapping module if needed.

**FD sources**: this crate does *not* know about VZ. `OwnedFd`s arrive via `VsockStream::from_owned_fd(fd, port)` / `VsockListener::from_channel(rx)` constructors. `vmm` produces them from VZ delegates; tests produce them from `tokio-vsock` loopback.

**Tests**: loopback `tokio-vsock` listener → connector → one-shot echo (no VM needed for unit tests). This is the key reason the crate is portable.

### 4. `vmm` (week 2–3)

**Lift from**: S1's boot harness + S4's block+vsock wiring + S4's listener delegate + S6's vmnet attachment + S7's Rosetta share.

**What changes going library-grade**:
- No `Box::leak`. The VM is a struct with proper `Drop` that tears down on release.
- No `dispatch_main()` — the library hosts its own dedicated serial `DispatchQueue` per VM; calls proxy through async channels.
- State transitions exposed as `tokio::sync::watch<VmState>` streams.
- Errors as `VmError` (thiserror enum, domain variants per D-007): `Start { source }`, `Validate { source }`, `AlreadyStopped`, etc.
- `#[forbid(unsafe_code)]` at crate level, `#[allow(unsafe_code)]` with justification on the specific modules that reach into objc2 (`define_class!` blocks, the `VzSend<T>` wrapper, `VmnetNetworkStruct`'s `RefEncode`).

Exposes (per [`04-library-surface/02-vm-surface.md`](./04-library-surface/02-vm-surface.md) — that doc is canonical, this is a summary):
- `VirtualMachine<NotBooted>` / `VirtualMachine<Running>` typestate — `VirtualMachine::new(cfg) -> VirtualMachine<NotBooted>` is cheap; `.boot().await -> Result<VirtualMachine<Running>, vmm::Error>` is the one-way door.
- `VmConfig` + `VmConfigBuilder` — `cpus: NonZeroU32`, `memory: Size`, `network`, `virtiofs_share`, `rosetta`, `nested_virtualization`, `boot_log`, `kernel: KernelImage`, `cmdline_extra`, `block_device(path)` (for the D-019 multi-container-per-VM path).
- `VirtualMachine<Running>::dial(port)` / `::listen(port)` — vsock dial + inverse-vsock listen (D-005). Returns `vsock`'s `VsockStream` / `VsockListener` (D-016).
- `VirtualMachine<Running>::pause` / `::resume` / `::statistics` / `::stop` / `::stop_with_grace`.
- Re-exports: `Network`, `BootLog`, `KernelImage`, `VmPhase`, `VmStatistics` (owned here); `VirtiofsTag`, `VsockPort`, `VmId` (re-exported from `types`); `VsockStream`, `VsockListener` (re-exported from `vsock`).
- `preflight() -> Preflight` capability probe.
- `vmm::Error` capability enum (per `05-error-model.md §3`), including `UnclassifiedVZ` tombstone.
- `AbortOnDrop<VirtualMachine<Running>>` opt-in wrapper (the `StoppableAsync` seal is owned by `firkin-core`; this type is re-exported through `firkin`).
- `VZ*` / `Retained<_>` / `DispatchQueue` / `block2::*` never leak into any public signature (§12 one-way door in `08-vmm-crate.md`).
- **Not here**: `VirtualMachine<Running>::container()` / `::container_shared()` — those live on the `CoreContainerFactory` extension trait in `firkin-core` (D-018), because they return `ContainerBuilder` which is a `core` type.

`.boot()` is VZ-only. Kernel lookup + init.block synthesis happen in `firkin-core` before `.boot()` is called — `vmm` has no `ext4` or `vminitd-bytes` dependency (cross-crate detail rewritten after planning-pass feedback).

### 5. `vminitd-client` (week 4)

**Lift from**: S4 (`Mount` / `WriteFile` / `CreateProcess` / `StartProcess` / `WaitProcess`) + S7 (`SetupEmulator`) + S9 (`IpLinkSet` / `IpAddrAdd` / `IpRouteAddDefault` / `ConfigureDns`).

**What the crate owns**:
- `tonic-build` integration. `proto/` dir with `SandboxContext.proto` copied from apple/containerization at `VMINITD_REV`.
- Typed wrappers that **know vminitd's quirks** (PRO_TIPS §21):
  - `LinuxNamespace::unshare(NamespaceKind)` — always emits `{type, path: ""}`, never bare `{type}`. `NamespaceKind` comes from `firkin-types` (D-015).
  - `ContainerBundle::for_id(id)` — returns `/run/container/<id>` given a `ContainerId` from `types`; caller doesn't choose.
  - `NetworkConfig::apply_to(client, interface, config)` — wraps the five-RPC sequence from S9. Hides the CIDR-string/bare-IP distinction.
- Errors as `VminitdError` with variants for each RPC family.
- Deps: `tonic`, `prost`, `tower`, `vsock`, `types`.
- `#[forbid(unsafe_code)]`.

This is the crate whose public API probably changes most if vminitd's proto drifts. Keep it small. Note the RPC shape is **process-centric**: `create_process` / `start_process` / `wait_process` / `kill_process` / `delete_process` / `resize_process`. There is no `create_container` / `close_container` / `stat` RPC — containers are created implicitly by the first process on them, and file transfer goes through the streaming `Copy` plane (not a stat RPC).

### 6. `oci` (week 5–6)

**Lift from**: S4 (image pull via `oci-client`, layer extraction + `cctl rootfs create --ext4` as interim).

**What changes**:
- Replace `cctl` shell-out with a Rust pipeline driving `ext4`. Call sites convert `MediaType` → `ext4::LayerCompression` when invoking `Writer::write_oci_layers(...)`.
- Whiteout + opaque-dir handling: if codex's S5-Tier-4 delivered them, use them. Otherwise explicit `UnsupportedFeature::Whiteout` error at rootfs-assembly time for multi-layer images with whiteouts. Document in `DEFERRED.md`.
- OCI runtime-spec builder: typed `RuntimeSpec` struct, knows about vminitd's Codable-strictness (every `LinuxNamespace` has explicit `path: ""`, every `process.capabilities.*` populated, etc.).
- Public `ImageBundle` type (not `Bundle` — see [D-020](./DECISIONS.md#d-020--ocibundle-renamed-to-ociimagebundle)) represents the pulled artifact on disk.

### 7. `core` (week 6–7)

**Lift from**: the sum of the other crates + the orchestration patterns in S4/S9.

The facade. Contains (full spec in [`04-library-surface/`](./04-library-surface/); this is a summary of what lands in `firkin-core`):
- `Container<S>`, `ContainerBuilder<Vm, S>`, `Process<E>`, `ExecConfig<E>`, `Output` — the user-facing container surface. `S`/`E` are the `Streams`/`Pty` stdio markers (D-025).
- `Rootfs` (enum: `Ext4Image`, `OciBundle`, `RawBlock`) and `VmRootfs` (newtype over `BlockDeviceId`) — the D-023 split. `Rootfs::block_device(id) -> VmRootfs` is the bridge.
- Builder terminals: `.spawn() -> Container<S>`, `.output() -> Output`, `.status() -> ExitStatus` (D-021). The one-call terminals auto-pipe + auto-drain.
- Per-container state machine inside `.spawn()`: validate → pull/stage rootfs → boot VM → await vsock → apply netlink → create process → stream stdio → wait → teardown.
- The init-block cache (keyed by `VMINITD_SHA256`, stored under `$XDG_CACHE_HOME`).
- The vmnet network registry — one `vmnet_network_ref` shared across multiple VMs per D-012.
- **`StoppableAsync` + `AbortOnDrop<T>`** (per `09-cross-cutting.md §2.2`): sealed trait, impls for `Container<S>` + `VirtualMachine<Running>`.
- **`CoreContainerFactory`** (per D-018): sealed extension trait that adds `vm.container(id) -> ContainerBuilder<OnVm<'_>, Init>` and `vm.container_shared(id) -> ContainerBuilder<OnVmArc, Init>` to `VirtualMachine<Running>` (which lives in `vmm` but cannot declare these methods itself without importing `core`).
- **No `Runtime` / `RunRequest` / `RunResponse`** — the builder-shaped surface replaces the Swift-style "runtime + request" pattern.

### 8. `cli` (week 7)

Thin clap-based binary. Subcommands: `run`, `pull`, `exec`, `debug boot`. Its value is letting humans exercise `core` without writing Rust.

### 9. CI, docs, release (week 7–8)

- `ci.yml`: fmt + clippy + check + test on every PR. macos-14.
- `build-vminitd.yml`: manual + path-triggered on `pin.toml` changes. Runs `make -C vminitd` (bypassing `linux-build LIBC=musl` per PRO_TIPS §16), uploads as release asset. Self-hosted runner or GH's macos-14-arm64.
- `release.yml`: tag-triggered. Publishes the publishable crates (`ext4`, `vsock` maybe, `oci-spec` if we expose one). Builds the CLI. Ad-hoc signs. Uploads to GitHub release.
- Each crate's README + rustdoc on public items. Nothing elaborate.

---

## The "CLI → library" rewrite checklist

Every crate goes through this as part of its lift. A spike binary → library crate transformation has a consistent shape:

| Spike CLI pattern | Library replacement |
|---|---|
| `Box::leak(Box::new(x))` | Owning struct field + proper `Drop`; or an `Arc<_>` if shared. |
| `std::process::exit(N)` in a delegate callback | Send `Err(CapabilityError::...)` / `Ok(state)` through a `tokio::sync::oneshot` the caller awaits. |
| `dispatch_main()` blocking main thread | Per-VM dedicated serial queue; orchestrator uses tokio; VZ queue proxies to tokio via channels. |
| `unwrap!` / `expect!` on external fallible calls | `?` + a properly-typed variant on the crate's error enum. |
| `println!` / `eprintln!` | `tracing::{info,warn,error,debug}!`. |
| Hardcoded paths (`assets/vmlinux`) | Config struct with sensible defaults; cache-dir resolver using `etcetera` or `dirs`. |
| `SPIKE_*` env vars | Typed config + `RuntimeConfig::builder()` methods; env-var overrides only at the very top (CLI / examples). |
| `std::mem::forget(rc_block)` to extend lifetime | `RcBlock` stashed as a struct field (per-call) — the struct's lifetime is the block's lifetime. |
| Single binary, single test run | Workspace with law tests per crate + integration tests gated behind `real-vm` feature. |

If you find yourself copying a spike pattern without applying this checklist, stop.

---

## Testing strategy

Per `beads-rs/docs/philosophy/test_design.md` (D-007): law, example, scenario, regression. No "I wrote this test for coverage" tests.

### Per-crate unit tests

- `ext4`: the full S5 suite + whatever codex adds in T4. Each test kills a concrete family of wrong writers (e.g., "any writer that puts the superblock at offset 0 of block 0 fails `superblock_at_canonical_offset`").
- `vmm`: mock-based contract tests (no real VM). Validate config-builder invariants. Integration tests gated.
- `vsock`: loopback against tokio-vsock spawned listeners. No VM.
- `vminitd-client`: serde round-trips on the generated structs; verify our quirks-wrappers emit the exact bytes the Swift side decodes.
- `oci`: against a local OCI registry (zot or `oci-client`'s test fixtures). No network. Includes a law test for the `ext4::OciLayerSource` impl on `ImageBundle` (D-024): for any multi-layer bundle, iterating via the trait yields the same `(path, compression)` sequence as iterating `.layers()` + `.compression()` directly.
- `core`: mocked vmm/vminitd-client/oci; focuses on state-machine correctness. Unit tests cover `.output()` auto-pipe + auto-drain (D-021) and the `Container<Streams>` vs `Container<Pty>` return-type split (D-025).

### Integration tests (`tests/integration/`, gated behind `--features real-vm`)

Reproduce each spike's acceptance criterion as a `#[test]`:

```rust
#[cfg(feature = "real-vm")]
#[tokio::test]
async fn s4_replay_busybox_echo_hello() { /* ... */ }

#[cfg(feature = "real-vm")]
#[tokio::test]
async fn s9_replay_vmnet_reachability() { /* ... */ }
```

These run on the self-hosted macos-arm64 runner nightly. PR CI runs only the non-gated tests.

---

## Risks — what could still go wrong

(Cross-reference `03-project-layout.md`'s risk register.)

Phase 1 introduces a handful of **rewrite risks** beyond the technical risks the spikes retired:

1. **Spike-to-library translation bugs.** The CLI patterns (`Box::leak`, `exit(N)` in callbacks) hide lifecycle subtleties. Mitigation: the explicit rewrite checklist above. Code review each crate's first PR with that checklist visible.

2. **Test coverage holes.** Spikes proved "happy path works"; library-grade means the error paths are tested too. Mitigation: for each domain-error variant, write at least one test that reaches it. Property tests (`proptest`) on `ext4` structural invariants.

3. **Proto drift.** `SandboxContext.proto` evolves upstream. Mitigation: D-008 vendored proto + pinned `VMINITD_REV`. Regenerating stubs is a deliberate act, not a silent upgrade.

4. **objc2 major-version bumps.** If objc2 0.7 drops some pattern we rely on, everything stalls. Mitigation: pinned in `[workspace.dependencies]`. Rust-ecosystem breakage is real — watch release notes.

5. **"Just one more feature" scope creep.** Deeper ext4 extent trees, htree
   directories, metadata checksums, journal support, bridged networking, and
   concurrent container stress remain tempting. Mitigation: `DEFERRED.md` per
   crate, visible during review.

6. **CI runner availability.** vminitd rebuild + integration tests want an arm64 macos runner. GH's macos-14 is fine but pricier than Linux CI. Budget for self-hosted if bill pressure emerges.

---

## Explicit deferrals

Phase 1 **does not** include:

- **Multi-group EXT4** (>128 MiB single image). Landed later in the Rust crate
  with writer and e2fsck coverage.
- **Deep-extent-tree ext4** files (>~512 MiB per file). Phase 2. Same reasoning.
- **htree directories.** Linear dirs are fine for e2fsck + mount. Phase 2.
- **metadata_csum.** The kernel accepts images without it. Phase 2.
- **OCI whiteout / opaque-dir merge semantics** unless codex's S5-T4 lands them. Phase 2 with an explicit `UnsupportedFeature::Whiteout` error if we encounter them and don't support them.
- **Bridged networking** (`VZBridgedNetworkDeviceAttachment`). Requires paid dev program + provisioning profile. Phase 3, as a separately-released feature crate.
- **Pre-macOS-26 hosts.** D-001. Don't build compatibility scaffolding.
- **Two-containers-same-VM.** D-012 — we go one VM per container.
- **Docker-compatible CLI.** The v0.1 CLI is dev-facing, not a Docker replacement. A compat CLI is a separate project layer on `core`.
- **Nested containers / Kata-style container-in-container.** Way out of scope.
- **Windows / Linux host support.** This is a macOS library.

Keeping these explicit means when the first "but what about…" lands in code review, we have a doc entry to point at instead of a fresh argument.

---

## Timeline sketch

Not calendar commitments. Dependency-ordered rough sizing assuming one focused engineer or ~2 with some parallelism.

| Week | Focus | Shipped by end of week |
|---|---|---|
| 1 | `types` + `ext4` + `vminitd-bytes` + `vsock` | `types` leaf ships. Publishable ext4 crate (works on Linux CI). vminitd-bytes leaf crate (first build downloads ELF). vsock portable, loopback tests green. |
| 2–3 | `vmm` | Library can boot a VM and open a vsock connection (reusing `vsock` types). Integration test replays S1 + S2. |
| 4 | `vminitd-client` | Library can issue all 10+ RPCs we use. Integration test replays S4's RPC sequence against S3's init.block. |
| 5–6 | `oci` | Library can pull an image (exposes `ImageBundle`) + assemble a container rootfs via `ext4`. S4's integration test replays without `cctl`. |
| 6–7 | `core` + walking skeleton | Container surface + `CoreContainerFactory` trait. `examples/walking-skeleton.rs` runs. Integration tests for S4, S7, S9 replays all green. |
| 7 | `cli` + `examples/*` | Humans can run containers without writing Rust. |
| 7–8 | CI + release machinery + docs + v0.1.0 tag | `gh release` with signed binaries + crates.io publishes. |

**Single-dev budget: 7–8 weeks of focused work for v0.1.0.** With parallel work on week-1 crates (`types` → `ext4` / `vminitd-bytes` / `vsock` in a topological fan-out) and on `vmm` + `vminitd-client` (independent once `vsock` is green) the critical path is closer to 5–6 weeks.

Exits: "couldn't do the rewrite quickly" isn't a risk — the spikes proved the pieces work. The risk is scope expansion. Ship early; iterate.

---

## First PRs — a concrete opening sequence

1. **PR #1 — workspace scaffold**. `Cargo.toml` (empty workspace), `rust-toolchain.toml`, `.cargo/config.toml`, `entitlements.plist`, `scripts/sign.sh`, empty `crates/` dirs with `AGENTS.md` or `README.md` one-liners, `.github/workflows/ci.yml` with fmt+clippy+check (test suite not required yet — no code). Add `clap`, `assert_cmd`, `humantime`, `toml`, `ureq` to `[workspace.dependencies]`. CI green on an empty build.

2. **PR #2 — `crates/types`** (D-015). Pure value types: `ContainerId`, `ProcessId`, `VsockPort`, `VirtiofsTag`, `VmId`, `Size`, `Hostname`, `Platform`/`Os`/`Arch`, `NamespaceKind`. All constructors + `InvalidX` errors. Unit tests cover each validation path. No dep on any other workspace crate.

3. **PR #3 — `crates/ext4`**. Lift S5's `src/` verbatim; apply the rewrite checklist; add the contract test suite; move CLI presets to `examples/`. Define `ext4::LayerCompression` enum; `Writer::write_oci_layers` takes `IntoIterator<Item = (impl AsRef<Path>, LayerCompression)>`. CI runs its unit tests. Contract tests pass. No integration into the rest of the workspace yet.

4. **PR #4 — `crates/vminitd-bytes` + `build-tools/build-vminitd/`** (D-017). `pin.toml` (apple/containerization SHA, Swift rev, ELF sha256, GitHub release asset URL). `build.rs` download+verify logic with `ureq`. `include_bytes!` const. `vendored-vminitd` feature. Doc: "how to regenerate vminitd" with the S3 recipe. `.gitignore` `vendor/vminitd/**`.

5. **PR #5 — `crates/vsock`** (D-016 owner side). Define `VsockStream` / `VsockListener` / `VsockPeer` from `OwnedFd`. `VsockConnector` for tonic. Loopback tests against `tokio-vsock`. No VZ dependency.

6. **PR #6 — `crates/vmm`**. The biggest extraction. Lift S1's boot config + S4's device wiring + S6's vmnet attachment + S7's Rosetta share. Depends on `vsock` (D-016): VZ connect + listener delegates produce `OwnedFd`s that vsock wraps into `VsockStream` / `VsockListener`. `VmConfigBuilder::block_device(path)` for D-019. Apply the full rewrite checklist. The first real "did we get the library shape right?" review.

7. **PR #7 — `crates/vminitd-client`**. Proto vendored + tonic-build + quirks-aware wrappers. Uses `vsock` for transport; `types` for `ContainerId`/`NamespaceKind`. Unit tests against a mock server.

8. **PR #8 — integration test: replay S4**. First `--features real-vm` test that composes vmm + vsock + vminitd-client. It should pass; if it doesn't, something went wrong in PR #5–#7. Iterate.

9. **PR #9 — `crates/oci`** (D-020). `ImageBundle` (not `Bundle`). Rootfs assembly via `ext4` + `LayerCompression`. Replaces `cctl` path.

10. **PR #10 — `crates/core`**. Orchestrator. Container/ContainerBuilder/Process/ExecConfig. `StoppableAsync` + `AbortOnDrop<T>`. `CoreContainerFactory` extension trait (D-018) with impls for `VirtualMachine<Running>`. Walking skeleton runs.

11. **PR #11 — `crates/cli`**. Thin subcommands (run/pull/exec/debug). Uses `clap`, `humantime`.

12. **PR #12 — CI polish + release.yml + v0.1.0 tag prep**. All integration tests (S4, S7, S9 replays) green.

---

## Appendix: status as of Phase 1 kickoff

- Spikes: S1 ✅ S2 ✅ S3 ✅ S4 ✅ S5 ✅ (Tiers 1–4 passed) S6 ✅ S7 ✅ S8 ✅ S9 ✅
- jj bookmark: `rust-rewrite-spikes` on apple/containerization clone, commit `8220c723` (or later).
- S5-T4 landed: fold into `ext4`'s lift (PR #2).

Start with PR #1.
