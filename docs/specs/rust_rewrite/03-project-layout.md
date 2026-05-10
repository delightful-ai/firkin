# Project layout, build machinery, release

Decisions the user has already made:
- **Library-first.** CLI is a top-level crate under the same workspace, used to exercise lib features during dev — not a first-class product on day one.
- **New repo.** Rust workspace, `crates/` layout.
- **vminitd bundling**: ✅ **settled by S8** — embed the 131 MiB vminitd ELF via `include_bytes!`. Runtime download is a fallback behind `--features runtime-download`. See `spike-logs/s8-bundling-bench/` for numbers. The `ext4` crate synthesizes `init.block` on-host from the vminitd ELF at first-use — no shipping the 384 MiB block image.
- **Platform floor**: macOS 26+ only (matches apple/container; unlocks vmnet shared ad-hoc). No pre-26 compatibility scaffolding.
- **Signing**: ad-hoc only; `entitlements.plist` contains just `com.apple.security.virtualization`. No Apple Developer Program required for the v1 feature set. See `DECISIONS.md` D-002.

Everything below follows from those. Spike-validated elements are linked inline.

---

## Repo shape

```
<project-name>/                  # new repo
├── Cargo.toml                   # workspace manifest
├── rust-toolchain.toml          # pin a stable release
├── .cargo/config.toml           # macOS target, rustflags, linker
├── crates/
│   ├── types/                   # firkin-types: shared value types (ContainerId, VsockPort, Platform, ...) — D-015
│   ├── ext4/                    # EXT4 writer (port of ContainerizationEXT4)
│   ├── vsock/                   # async wrappers over vsock OwnedFds (AsyncRead/Write); loopback-testable — D-016
│   ├── vmm/                     # VZ driver (the objc2-virtualization wrapper); produces OwnedFds for vsock
│   ├── vminitd-client/          # tonic-generated SandboxContext client + helpers
│   ├── vminitd-bytes/           # leaf holding include_bytes!(vminitd ELF); ELF resolved via pinned download (D-017)
│   ├── oci/                     # image pull + layer extraction + rootfs assembly
│   ├── substrate/               # production substrate models: capacity, snapshots, warm pools, template builds
│   ├── template/                # template build executor: clone, checkout, setup, cache warm
│   ├── runtime/                 # production composition: core + template + substrate + API adapters
│   ├── core/                    # the facade: pull → boot → run → stream → wait
│   └── cli/                     # dev-facing CLI (clap). Top-level, consumes `core`. Binary name: `fk` (D-014).
├── vendor/
│   └── vminitd/                 # .gitignored; optionally populated for the `vendored-vminitd` feature
├── build-tools/
│   └── build-vminitd/           # scripts to cross-build vminitd; CI uploads ELF as a release asset (D-017)
├── docs/
├── examples/
├── tests/                       # integration tests (macOS-gated)
└── .github/
    └── workflows/
        ├── ci.yml               # cargo check/clippy/fmt on every PR
        ├── build-vminitd.yml    # rebuilds vminitd, uploads as release artifact
        └── release.yml          # tag-triggered: build lib + CLI + pin vminitd
```

**Project name**: deferred. Candidates to workshop: `vzcontainer`, `pivz`, `appcontainer-rs`, `rusticate`. Pick before we publish.

---

## Crate responsibilities & dependency graph

```
cli ──► core ──┬──► vmm ──────► vsock ──► types
               │       │         (no objc2; portable; tokio-vsock loopback testable — D-016)
               │       └──────► objc2-virtualization (external)
               ├──► vsock
               ├──► vminitd-client ──► vsock
               │                   └──► types
               ├──► oci ──► ext4
               │      └──► types
               ├──► ext4 ──► types
               ├──► types                 (leaf; no workspace deps; D-015)
               └──► vminitd-bytes         (leaf; include_bytes!(vminitd ELF); ELF resolved via download + sha256 — D-017)

substrate ──► types
template ──► substrate
runtime ──► core + template + substrate + e2b
```

- `types`, `ext4`, `vsock`, `oci`, and `substrate` are Linux-portable (no `objc2-*` deps); they build and test on Linux CI.
- `vmm`, `vminitd-client`, `core`, `cli` are macOS-only (transitively depend on `objc2-virtualization` via `vmm`).
- `vminitd-bytes` is the sole owner of the ~131 MiB ELF blob. Every other crate that needs the bytes depends on it (currently just `core`); `ld` dead-strips the const for consumers that don't instantiate a VM.
- `vsock` owns the user-visible `VsockStream` / `VsockListener` / `VsockPeer` types; `vmm` depends on `vsock` and hands it `OwnedFd`s produced by VZ's connect + listener-delegate plumbing (D-016).

| Crate | What it owns | External deps | Shape | Spike evidence |
|---|---|---|---|---|
| **types** | Shared value types: `ContainerId`, `ProcessId`, `VsockPort`, `VirtiofsTag`, `VmId`, `Size`, `Hostname`, `Platform`/`Os`/`Arch`, `NamespaceKind`. Plus corresponding `InvalidX` error types. Per D-015. | `thiserror`, `uuid` | Pure leaf. No workspace deps. Deliberately small; types without validated construction don't belong here. | — (design decision) |
| **vmm** | VZ configuration builders, `VZVirtualMachine` lifecycle, delegate, device attachment helpers. **Plus** `VZVirtioSocketListener` delegates for guest-dialed-back connections (container stdio). Hands `OwnedFd`s produced by VZ to `vsock` for user-visible stream types. | `objc2`, `objc2-foundation`, `objc2-virtualization`, `block2`, `dispatch2`, `vsock`, `types` | Safe Rust over the Obj-C surface. Everything on main queue + `dispatch_main()`; use `VzSend<T>` wrapper only when crossing queue boundaries. State transitions exposed as tokio channels. | S1 (boot), S3 (block device), S4 (stdio listener), S6 (vmnet), S7 (Rosetta share) |
| **vsock** | Public `VsockStream` / `VsockListener` / `VsockPeer` — async wrappers around `OwnedFd` via `tokio::io::unix::AsyncFd`. Plus `VsockConnector` (tower::Service) for tonic/hyper. **Portable**: no `objc2-*` deps. Per D-016. | `tokio`, `hyper`, `tower`, `types` | Standalone; testable with a loopback fd pair (`tokio-vsock` in tests) — no VM required. | S2 (dial), S4 (accept) |
| **vminitd-client** | `tonic`-generated stubs from `SandboxContext.proto`, plus ergonomic async wrappers. Also knows vminitd's quirks: bundle path implicit at `/run/container/<id>`, strict Codable on the runtime-spec decoder, stdio-vsock ports. | `tonic`, `prost`, `tower`, `vsock`, `types` | Proto vendored from apple/containerization at a pinned rev. | S4 (full RPC sequence) |
| **ext4** | EXT4 image writer. Port of `ContainerizationEXT4`. **Also** the on-host synthesizer for `init.block` (consumed by `core` at first VM boot, once per vminitd-ELF-hash). | `nix`/`rustix`, `bytemuck`, `types` | Pure byte manipulation, `#[repr(C)]` + bytemuck for on-disk structs. Newtypes (`BlockNumber`, `InodeNumber`, `BlockSize`). Domain-named `thiserror` variants. Zero macOS coupling. Testable on any platform. | S5 (writer), S8 (convergence: don't ship init.block; synthesize) |
| **oci** | Registry pull (`oci-client`), manifest/config types (`oci-spec`), layer extraction (`tar` + `flate2` + `zstd`), whiteout handling, container rootfs assembly (drives `ext4`). Exposes `ImageBundle` (D-020). | `oci-client`, `oci-spec`, `tar`, `flate2`, `zstd`, `ext4`, `types` | Mostly orchestration. | S4 (image pull + rootfs build via `cctl` as interim) |
| **substrate** | Production substrate control models: `CapacityLedger`, `ResourceBudget`, snapshot artifact manifests, warm-pool keys/entries/ledger, and template build jobs. Hides single-node scheduling, snapshot, warm-pool, and template-build semantics without pulling in VM or OCI runtime machinery. | `types`, `thiserror` | Pure model crate. No `core`, `vmm`, `oci`, `vminitd-client`, or E2B dependency. | Production-substrate goal |
| **template** | Template build execution: clone repository, checkout ref, run setup commands, run cache-warming commands, and return the base snapshot manifest described by `substrate`. | `substrate`, `thiserror` | Filesystem/process executor. No `core`, `vmm`, `oci`, `vminitd-client`, or E2B dependency. | Production-substrate goal |
| **runtime** | Production composition root for Apple/VZ-backed substrate behavior: wires `core` VM/container mechanics, `template` build/freshness execution, `substrate` capacity/snapshot/warm-pool policy, and optional E2B/Cube adapters. Owns VZ-backed implementations of consumer-owned traits such as `TemplateSnapshotSink`. | `core`, `template`, `substrate`, optional `e2b` | Top-level orchestrator crate. Lower-level crates must not depend on it. This is where API semantics meet VM mechanics without creating cycles. | Production-substrate goal |
| **vminitd-bytes** | Leaf crate holding `include_bytes!` of the pinned vminitd ELF. Exposes `pub const VMINITD_AARCH64: &[u8]` (and optional `_X86_64`) plus its SHA-256. | none; `build.rs` resolves ELF by downloading from a pinned GitHub release asset (D-017), sha256-verified, cached per-workspace. `FIRKIN_VMINITD_PATH` env var and `vendored-vminitd` feature are escape hatches. | ~20 LOC. Isolates the ~131 MiB blob so it doesn't rebuild on unrelated edits; `ld` dead-strips for consumers that don't reference it. | S8 (bundling decision) |
| **core** | Orchestrator state machine. Holds kernel path + vminitd ELF (via `vminitd-bytes`) + `ext4` synthesizer. Drives vmm + vminitd-client + oci into a container lifecycle. Defines the `CoreContainerFactory` extension trait (D-018) for `vm.container()` / `vm.container_shared()`. | `vmm`, `vsock`, `vminitd-client`, `oci`, `ext4`, `types`, `vminitd-bytes` | The VM/container orchestration crate users import through the `firkin` facade. It does not own production scheduling or template-service policy. | S8 (bundling numbers + dead-strip behavior) |
| **cli** | clap-based `run`, `pull`, `exec`, `debug` subcommands for dev. | `core`, `clap`, `tracing-subscriber` | Thin. Its value is exercising `core`. | — |

Each crate is `#![forbid(unsafe_code)]` except `vmm` (and `vsock`/`ext4` at module boundaries for raw fd wrapping / byte layout, respectively). `types` forbids unsafe.

`scripts/check-firkin-crate-graph.sh` enforces the Firkin production-substrate
crate graph in CI. The important rule for the production substrate is that
runtime composition is allowed to depend on `core`, `template`, `substrate`, and
API adapter crates; those crates must not depend back on the composition layer
or on each other's higher-level policy.

**New architectural element from S4**: the `vmm` crate exposes a listener-delegate pattern for guest-dialed-back connections. vminitd's `CreateProcessRequest.{stdin,stdout,stderr}` take vsock *port numbers* that the guest connects back to — the library has to be listening. Same fd-dup + O_NONBLOCK pattern as the connector path; see PRO_TIPS §20. The `OwnedFd`s produced by this pattern are handed to `vsock`'s constructors (D-016) so the user-visible types are the same whether the FD came from a VZ-backed VM or a loopback test.

---

## vminitd bundling strategy

✅ **Settled by S8** (see `spike-logs/s8-bundling-bench/FINDINGS.md` for raw numbers). Refined by [D-017](./DECISIONS.md#d-017--vminitd-elf-distributed-via-pinned-download-not-checked-in): the ELF is never committed to git. Three paths, one default:

**A. Build-time download (default, per D-017)**
- `firkin-vminitd-bytes/build.rs` resolves the ELF in this order:
  1. `FIRKIN_VMINITD_PATH` env var if set — used as-is, sha256-verified against `pin.toml`.
  2. `$CARGO_TARGET_DIR/firkin-vminitd/<sha256>/vminitd-<target-triple>` if cached from a prior build.
  3. Otherwise: download from the pinned GitHub release asset URL in `pin.toml`, verify sha256, cache at the path above.
- Embeds the bytes via `include_bytes!` into a const **inside the leaf crate**. Isolating the blob keeps the 131 MiB link-tax off day-to-day edits in other crates.
- At first-use on a given host: synthesize `init.block` via the `ext4` crate, keyed by the vminitd-ELF SHA-256; cache in `$XDG_CACHE_HOME/<project>/init-blocks/<sha256>.ext4`. Subsequent VM boots on the same host reuse the cached image in O(stat) time.
- First-build download is typically ~11 s on a 100 Mbps link (S8 measured); cached thereafter.

**B. Runtime download** (feature-gated)
- `--features runtime-download` (and implicitly `--no-default-features` on crate builds that exclude the blob).
- Fetches the pinned ELF from a GitHub release on first `core::run` call, not at build time. Caches in `$XDG_CACHE_HOME`. Progress hook so callers can show a spinner.
- For consumers who can't tolerate a 133 MB release binary (e.g. vendored into another product with strict binary-size budgets). **Mutually exclusive with A.**

**C. Vendored ELF** (feature-gated, for air-gapped / offline environments)
- `--features vendored-vminitd` expects the ELF at `vendor/vminitd/<target-triple>/vminitd`. `vendor/vminitd/**` is in `.gitignore` — contributors who need this path set up git-LFS or a local mirror themselves. `vendor/vminitd/README.md` documents how to populate it.
- Same sha256 verification against `pin.toml`.
- **Mutually exclusive with A.**

**Why A is the new default**: GitHub rejects pushes of single files >100 MiB; the ELF is ~131 MiB. A checked-in default would force git-LFS on every contributor — friction for the 95% who never touch vminitd to help the 5% who do. Download-once-per-machine flips that tax.

**Measured numbers** (M-series, macOS 26.3, rustc 1.95-nightly):

| | Cold build | Warm-touch-lib | Warm-touch-main | Release binary | First-run latency |
|---|---|---|---|---|---|
| A (embedded ELF, 131 MiB, per-D-017 first-build adds ~11 s @ 100 Mbps download once) | **4.85 s + 11 s first** | **4.51 s** | **0.49 s** | 133 MB | n/a |
| B (runtime-download) | 5.19 s | 0.26 s | 0.15 s | **1.0 MB** | +0.6 s loopback / ~11 s @ 100 Mbps |
| C (vendored ELF, if set up) | **4.85 s** | **4.51 s** | **0.49 s** | 133 MB | n/a |

All three hit the plan's tolerances (cold < 60 s including first-build download, warm < 5 s, first-run < 3 s) when you pick the right strategy. **Do not embed `init.block`** (384 MiB): warm-rebuild balloons to 20–40 s and peak RSS hits 7 GB. The `ext4`-crate on-host synthesis was specifically motivated by this constraint.

**Rationale for having B as a feature, not a fallback-when-A-is-slow:**
- `ld` dead-strips unreferenced `include_bytes!` consts (measured: a consumer of `ext4` without `core` pays ~422 KB, not 131 MB). So A is already cheap for consumers who don't actually instantiate a VM.
- `.rlib` on disk still carries the blob (~2.0 GB `target/` growth), which matters only for target-dir-sensitive CI workflows. B is the option for those.

**Rationale for making A the default over C:** see D-017. The 100 MiB GitHub file-size limit rules out a naive checked-in artifact; git-LFS would tax every contributor. Download-once-per-machine (cached in `$CARGO_TARGET_DIR/firkin-vminitd/...`) is lower friction.

### vminitd version pinning

- A single `VMINITD_REV` constant in `build-tools/build-vminitd/pin.toml` holds:
  - apple/containerization git SHA.
  - Swift toolchain version (pinned: 6.3.0 via swiftly; see PRO_TIPS §15).
  - Static-linux SDK artifact URL + SHA (pinned: `swift-6.3-RELEASE_static-linux-0.1.0`, SHA `d2078b69bdeb5c31202c10e9d8a11d6f66f82938b51a4b75f032ccb35c4c286c`).
  - SHA-256 of the resulting ELF per target triple.
  - GitHub release asset URL per target triple (populated by the `build-vminitd` workflow after each successful rebuild; consumed by `firkin-vminitd-bytes/build.rs` for the default download path — see D-017).
- CI rebuilds vminitd only when `pin.toml` changes.
- Users who clone the repo fresh get the same bytes as CI: `build.rs` downloads via the pinned URL, verifies against the pinned sha256. Determinism matters here.

---

## Build machinery

### Workspace-level

- `rust-toolchain.toml` pins to a specific stable (e.g. 1.84 at time of writing).
- `.cargo/config.toml`:
  - `[target.aarch64-apple-darwin]` and `[target.x86_64-apple-darwin]`: `rustflags = ["-C", "link-arg=-sectcreate,__TEXT,__info_plist,..."]` if we need embedded Info.plist for entitlements.
- Workspace `Cargo.toml` keeps shared dependency versions in `[workspace.dependencies]`.

### vminitd build

`build-tools/build-vminitd/build.sh`:
```
git clone https://github.com/apple/containerization.git /tmp/containerization
cd /tmp/containerization
git checkout $VMINITD_REV
make cross-prep
make linux-build LIBC=musl
cp vminitd/bin/vminitd $OUT_DIR/vminitd-<target-triple>
cp vminitd/bin/vmexec $OUT_DIR/vmexec-<target-triple>
```

Runs in CI on macOS. Output is uploaded as a GitHub release asset keyed by the tuple `(VMINITD_REV, target-triple)`.

### Codesigning

A dev-signed binary for running VZ from Rust:
```
codesign --force \
  --entitlements build-tools/entitlements.plist \
  --sign - \
  target/debug/cli
```

`entitlements.plist` starts with `com.apple.security.virtualization`. If S6 proves we need more for vmnet, add there. Cribbed from `krunkit.entitlements`.

---

## Testing strategy

Three layers:

1. **Unit tests** per crate. `ext4` gets hammered — every feature in S5 becomes a unit test with a golden image + e2fsck validation.
2. **Integration tests** in `tests/` — boot a VM, run a container, assert stdout. Gated behind `--features real-vm` so `cargo test` doesn't require a codesigned binary by default.
3. **Golden-image differential**: `ext4` crate ships a harness that, given an OCI image, produces an ext4 via both (a) our Rust writer and (b) apple/containerization's Swift writer (via `cctl rootfs create`). Diff byte-for-byte. Tolerate timestamp/UUID. **This is the correctness backstop for the port.**

CI runs unit tests + clippy on every PR. Integration tests run on tag / nightly on a self-hosted or GitHub-hosted M-series runner.

---

## CI shape

`.github/workflows/ci.yml`:
- `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo check --all-features`, `cargo test --workspace` (unit only).
- Runs on `macos-14` (Apple Silicon).

`.github/workflows/build-vminitd.yml`:
- Manual + path-triggered on `build-tools/build-vminitd/pin.toml` changes.
- Builds vminitd for both `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`.
- Uploads as release asset.

`.github/workflows/release.yml`:
- Tag-triggered.
- Builds lib + CLI for `aarch64-apple-darwin` + `x86_64-apple-darwin`.
- Codesigns with Apple Developer certs (if available) or ad-hoc.
- Publishes crates.io (non-CLI ones) + GitHub release (CLI binaries).

---

## Risk register — updated post-spike

| Risk | Prior likelihood / impact | Status after spikes | Mitigation |
|---|---|---|---|
| EXT4 writer has subtle correctness bugs that pass e2fsck but break containers | medium / high | **reduced** — S5 writer e2fsck-clean across 7 feature flags, mounts + reads in VM, Tier 4 overlay semantics (whiteouts + opaque dirs) validated via guest-mount probes | Structural-parity diff vs `mkfs.ext4` on Tier 3 documented; law tests (from `test_design.md`) pin invariants; golden fixtures in `tests/fixtures/`. |
| vminitd gRPC protocol drifts upstream | low-medium / medium | **unchanged** | Pin `VMINITD_REV`. Regen `vminitd-client` stubs on bump. Watch upstream releases. |
| ~~vmnet requires paid Apple Developer Program~~ | ~~medium / medium~~ | **retired (S6)** | Shared-mode vmnet works ad-hoc on macOS 26+. Bridged still gated but deferred to Phase 3. |
| `objc2-virtualization` deprecates APIs | low / low | **unchanged** — 0.3.2 works; `VZVmnetNetworkDeviceAttachment::init` is marked unavailable but reachable via `msg_send!` (PRO_TIPS §29). | Pin major version. objc2 is actively maintained. |
| ~~VZ behavior changes across macOS versions~~ | ~~low-medium / medium~~ | **reduced** — floor is macOS 26+; we test a single version | macOS 26+ only; test on newest minor. |
| Swift toolchain churn breaks vminitd cross-build | medium / low | **unchanged** — recipe known good as of 2026-04-20 | `pin.toml` freezes Swift version (6.3.0) + static-linux SDK SHA. CI runner image pinned. |
| Rosetta license-acceptance flow is awkward for CLI/library users | low / low | **reduced (S7)** — programmatic install works when system-wide EULA already accepted; fresh Macs still see a one-time GUI prompt | Document the one-shot install; `installRosettaWithCompletionHandler:` is in `VZLinuxRosettaDirectoryShare`. Phase 3 scope. |
| Tonic/hyper upgrade changes Connector API | low / low | **unchanged** — tonic 0.12 + hyper 1 + hyper-util 0.1 works as of 2026-04 | Pin in `[workspace.dependencies]`. |
| **NEW: vmnet end-to-end reachability unverified** | — | **S9 dispatched 2026-04-20** | When S9 returns, risk resolves one way or the other. |
| **NEW: container stdio is inverse-vsock, not stream RPC** | — | **known (S4)** — had to invent a listener-delegate path | Pattern documented in PRO_TIPS §20; lifts to `vmm` + `vminitd-client`. |
| **NEW: Swift Codable strictness in vminitd's runtime-spec decoder** | — | **known (S4)** — `LinuxNamespace.path` required; no Optional defaults | `vminitd-client` wrappers always emit `path: ""` for unshare-style namespaces. Add similar discipline to other spec fields as discovered. |

---

## Open decisions

1. **Project name.** Pick one before first commit.
2. **MSRV policy**: track stable? Hold to N-2? *Recommendation: track stable, bump freely until v1.*
3. **License**: Apache-2.0 to match upstream Swift. MIT optional dual-license for broader reuse of non-EXT4 crates.
4. **Whether `ext4` becomes its own independently-publishable crate** with no `vminitd` or VZ coupling. *Recommendation: yes — it's generally useful and keeps the port honest.*
5. **vminitd eventual Rust rewrite**: deferred indefinitely. Note it as a "someday" in the README's non-goals.

---

## What this is *not*

- Not a Docker replacement. This is a library that lets Rust programs run OCI containers in microVMs on macOS. A Docker-compat CLI can be built on top later.
- Not a Linux port. This targets macOS-on-Apple-Silicon first. Intel macOS is probably trivial via VZ but not a focus. Linux/KVM is out of scope — there are better choices there.
- Not a vminitd reimplementation. The Swift binary stays; we bundle it.
