# `ext4` crate

> Covers: public API of the `ext4` crate — `Writer`, `Features`, `BlockNumber` / `InodeNumber`, the `init_block` module (D-003 synthesis), golden-diff correctness backstop, Linux-portable testing.
>
> Builds on: S5 spike findings ([`spike-logs/s5-ext4/`](../spike-logs/s5-ext4/)), [D-003](../DECISIONS.md#d-003--embed-vminitd-elf-not-initblock), [D-004](../DECISIONS.md#d-004--ext4-crate-is-the-source-of-truth-for-both-initblock-and-container-rootfs).

---

## 1. Scope

`ext4` is the **EXT4 image writer**. One mission, two consumers:

1. **Container rootfs assembly** — `core::Container::builder(...).rootfs(Rootfs::OciBundle(…))` routes here. Takes OCI layer tarballs, produces a single mountable `.ext4` file, handling whiteouts, hardlinks, xattrs, opaque-directory markers.
2. **`init.block` synthesis** (D-003) — produces the initial-rootfs block image from a vminitd ELF at first-use on a host. Cached by SHA-256 for subsequent runs.

**Out of scope** (named explicitly to prevent re-litigation):
- **Reading** ext4 images. No paired Reader type in v1. Guest kernel reads natively; golden-image tests compare by byte-hash against `mkfs.ext4` output without needing a logical reader.
- **Resize / grow / shrink** existing images. Immutable-after-finalize.
- **Journal (`jbd2`) writing.** ext4 without a journal is valid; vminitd's `init.block` and container rootfses are written-once, mounted-read-only (or overlay-upper) — journal is pure cost.
- **Encryption (`fscrypt`) / casefolding / bigalloc.** Explicit non-goals.
- **Any macOS coupling.** D-004. The crate builds and tests on Linux CI so unit feedback stays fast.

---

## 2. `Writer` — the main type

### 2.1 Construction

```rust
pub struct Writer { /* private */ }

impl Writer {
    /// Open `path` for writing; pre-size to `size`. If the file exists, it is
    /// truncated. Caller owns the file on drop of an un-finalized Writer.
    pub fn new(path: impl Into<PathBuf>, size: Size) -> Result<Self, Error>;

    /// In-memory sink. Useful for tests and for init.block synthesis
    /// where we want bytes, not a file.
    pub fn in_memory(size: Size) -> Result<Self, Error>;
}
```

### 2.2 Configuration (builder-style, consuming self)

```rust
impl Writer {
    pub fn features(self, f: Features) -> Self;
    pub fn block_size(self, size: BlockSize) -> Self;
    pub fn inode_count(self, n: u32) -> Self;
    pub fn uuid(self, uuid: uuid::Uuid) -> Self;
    pub fn label(self, label: impl Into<String>) -> Self;
    pub fn reserved_blocks_pct(self, pct: u8) -> Self;   // default 5 (matches mkfs.ext4)
}
```

Defaults:

| Field | Default |
|---|---|
| `features` | `Features::default_set()` — see §3 |
| `block_size` | `BlockSize::Size4K` |
| `inode_count` | 1 per 16 KiB of size (matches `mkfs.ext4 -T default`) |
| `uuid` | randomly generated per `Writer::new`; override for deterministic images |
| `label` | empty |
| `reserved_blocks_pct` | 5 |

### 2.3 Content methods (consuming self)

```rust
/// Per-layer compression hint. `ext4` does not depend on `oci-spec` (would
/// fatten the dependency graph for consumers who want ext4 without OCI);
/// `firkin-oci`'s `Layer::compression()` is where `oci::MediaType` → this
/// enum happens, on the oci side of the boundary. Deterministic dispatch —
/// no content sniffing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayerCompression {
    None,   // tar
    Gzip,   // tar + gzip
    Zstd,   // tar + zstd
}

/// Sealed trait (D-024) implemented by anything that can describe itself as
/// a sequence of OCI-layer sources. `firkin-oci` provides `impl OciLayerSource
/// for ImageBundle`; tests provide ad-hoc impls. `ext4` stays `oci`-free.
mod sealed { pub trait Sealed {} }
pub trait OciLayerSource: sealed::Sealed {
    /// Ordered (path, compression) pairs. Paths point at the raw compressed
    /// layer files on disk (typically `oci`'s content-addressable cache);
    /// compression tells the writer how to decode each stream.
    fn layers(&self) -> impl Iterator<Item = (&Path, LayerCompression)> + '_;
}

impl Writer {
    /// High-level: write every layer from any OCI layer source (e.g. a pulled
    /// `oci::ImageBundle`). Decompresses + extracts + handles whiteouts.
    pub fn write_oci_layers(self, src: &impl OciLayerSource) -> Result<Self, Error>;

    /// Low-level: hand-rolled (path, compression) pairs. Useful for tests
    /// fabricating layers without going through `oci`, and for users with
    /// an unusual layer source shape.
    pub fn write_layers_raw<I, P>(self, layers: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (P, LayerCompression)>,
        P: AsRef<Path>;

    pub fn write_directory(
        self,
        guest_path: impl AsRef<Path>,
        host_source: impl AsRef<Path>,
    ) -> Result<Self, Error>;

    pub fn write_file(
        self,
        guest_path: impl AsRef<Path>,
        content: &[u8],
        mode: u32,
    ) -> Result<Self, Error>;

    pub fn write_symlink(
        self,
        guest_path: impl AsRef<Path>,
        target: impl AsRef<Path>,
    ) -> Result<Self, Error>;

    pub fn write_chardev(
        self,
        guest_path: impl AsRef<Path>,
        major: u32,
        minor: u32,
        mode: u32,
    ) -> Result<Self, Error>;

    pub fn write_blockdev(
        self,
        guest_path: impl AsRef<Path>,
        major: u32,
        minor: u32,
        mode: u32,
    ) -> Result<Self, Error>;

    pub fn write_xattr(
        self,
        guest_path: impl AsRef<Path>,
        name: &str,
        value: &[u8],
    ) -> Result<Self, Error>;
}
```

### 2.4 Finalization

```rust
impl Writer {
    /// Flush superblock + bitmaps + group descriptor tables, close the backing file,
    /// return its path.
    pub fn finalize(self) -> Result<PathBuf, Error>;

    /// For in-memory Writers: consume self, return the bytes.
    pub fn into_bytes(self) -> Result<Vec<u8>, Error>;
}
```

### 2.5 Why consuming-self builder style

Every content method takes `self` by value, returns `Result<Self, Error>`. Two reasons:

1. **Composable method chains** read cleanly: `Writer::new(…).features(…).write_oci_layers(layers)?.finalize()?`.
2. **No half-used Writer after an error.** If `write_directory` fails midway, the partially-written Writer can't be used again — `?` already consumed it. Contrast with `&mut self` methods: an error might leave the Writer in a partial state the caller could keep using, surfacing problems later.

The cost is slightly more verbose ownership flow, but it eliminates an entire class of correctness bugs.

### 2.6 Send / Sync

- `Writer: Send` — owns a `File` or `Vec<u8>` + simple state.
- `Writer: !Sync` — content methods take `self` (or `&mut self` internally); no concurrent write pattern.

### 2.7 Drop

Drops the backing file descriptor. The file on disk is left as-is (partial). Caller's responsibility to `fs::remove` on error if they don't want the partial artifact lingering.

---

## 3. `Features` — what ext4 features the image uses

The `Features` bitflags enumerates every ext4 feature the writer *can* express on disk. That's separate from what the *current* library version has an implementation for — an unimplemented flag set in `Features` returns `Error::UnsupportedFeature { feature }` from `finalize()`. The enum is honest about the full target; the constructors are honest about what ships when.

```rust
bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Features: u64 {
        // ─── Phase 1 (v0.1 ships these) ─────────────────────
        const EXT_ATTR      = 1 << 0;    // extended attributes (xattr)
        const SPARSE_SUPER2 = 1 << 1;    // superblock backups on sparse blocks only
        const FILETYPE      = 1 << 2;    // directory entries carry file type
        const EXTENT        = 1 << 3;    // extent-based file allocation (depth 0 + depth 1)
        const FLEX_BG       = 1 << 4;    // flexible block groups
        const LARGE_FILE    = 1 << 5;    // >2 GiB files
        const HUGE_FILE     = 1 << 6;    // ≥4 TiB files
        const EXTRA_ISIZE   = 1 << 7;    // extra isize bytes in inodes

        // ─── Phase 2 target (enum exists; writer returns UnsupportedFeature until impl lands) ─
        const DIR_INDEX     = 1 << 8;    // HTree hash-indexed directories
        const META_BG       = 1 << 9;    // meta block groups for very large GDT layouts
        const BIT_64        = 1 << 10;   // 64-bit block addressing (>16 TiB)
        const METADATA_CSUM = 1 << 11;   // checksums on SB/group-desc/inode/dir-blocks
        const DEEP_EXTENTS  = 1 << 12;   // extent trees deeper than depth 1
        const DIR_NLINK     = 1 << 13;   // directory hardlink count > 65000
        const INLINE_DATA   = 1 << 14;   // inline tiny files in the inode itself
    }
}

impl Features {
    /// The v0.1 ship set — every feature this library version actually implements.
    /// This is what `Writer::new(..)` uses if no `.features(..)` call overrides it.
    /// Grows per-release as Phase 2 features land; `cargo semver-checks` will flag
    /// any release that *removes* a flag, which is a breaking change.
    ///
    /// v0.1: `EXT_ATTR | SPARSE_SUPER2 | FILETYPE | EXTENT | FLEX_BG | LARGE_FILE
    ///        | HUGE_FILE | EXTRA_ISIZE`
    pub fn default_set() -> Self;

    /// S5's spike-validated subset. Identical to v0.1 `default_set()` but pinned —
    /// its value does not change across releases. Used by the golden-diff harness
    /// to guarantee the Phase-1 fixtures keep passing even as `default_set()` grows.
    pub fn spike_set() -> Self;

    /// The mkfs.ext4 parity target — what the writer *aspires* to support. Union of
    /// every flag above. Requesting this on a pre-Phase-2 version is a way to get
    /// a clean `UnsupportedFeature` error for any flag not yet implemented, rather
    /// than silently downgrading.
    pub fn mkfs_parity_target() -> Self;
}
```

The four states for any given flag are:

1. **In `default_set()` + implemented**: used by default, golden-diff-tested.
2. **In `mkfs_parity_target()` but not `default_set()`**: enum variant exists; `Writer::features(..)` with this flag set returns `Error::UnsupportedFeature` from `finalize()`. Phase 2 work item.
3. **Not in either**: not a shippable feature yet; no enum variant (would be a breaking change to add).
4. **Explicit non-goal**: flagged in `§10` of this file — `journal`, `fscrypt`, `casefold`, `bigalloc`.

`spike_set()` is frozen; `default_set()` grows over releases; `mkfs_parity_target()` is the ceiling.

### 3.1 Per-feature rationale + phase

| Flag | Phase | Why it matters |
|---|---|---|
| `EXT_ATTR` | **1** | xattrs needed for OCI label passthrough + runc defaults |
| `SPARSE_SUPER2` | **1** | Reduced superblock redundancy; ~1% space savings on large images |
| `FILETYPE` | **1** | O(1) file-type lookup in dir listings |
| `EXTENT` | **1** | Modern extent allocation; linear-pointer blocks are legacy. Depth-0 + depth-1 trees. |
| `FLEX_BG` | **1** | Block-group allocator gets better layouts |
| `LARGE_FILE` + `HUGE_FILE` | **1** | >2 GiB single-file support (ML model weights, DB files) |
| `EXTRA_ISIZE` | **1** | Nanosecond-resolution mtime |
| `DIR_INDEX` (HTree) | **2** | O(log n) lookups in large directories; multi-second speedup on `apt install` / `cargo build`. Unlock: real debian/ubuntu rootfses with >1k files in `/usr/bin`. |
| Multi-block-group layout | **1** | Implemented without the `META_BG` feature bit; writer and e2fsck tests cover images that cross the first group. |
| `META_BG` | **2** | Meta block groups for very large group-descriptor layouts. Unlock: images whose descriptor tables need meta-bg scaling. |
| `BIT_64` | **2** | Enables filesystems > 16 TiB. Unlock: bulk-data / dataset images. |
| `METADATA_CSUM` | **2** | Checksums on metadata blocks; mount-without-csums emits kernel warnings on modern kernels. Unlock: mount-clean kernel log. |
| `DEEP_EXTENTS` | **2** | Extent trees with interior nodes (depth > 1); required for files >~512 MiB inside a single layer. Unlock: large single-file layers. |
| `DIR_NLINK` | **2** | Directories with > ~32k subdirectories; e.g. `/var/cache/apt/archives` on debian, deep Python namespace trees. Unlock: dev images with package managers. |
| `INLINE_DATA` | **2** | Tiny files (symlinks, tiny configs) stored in inode without block allocation; saves both space and lookup hops. Unlock: space/perf polish. |

Phase 1 = landed in `default_set()` at v0.1 ship. Phase 2 = enum-reachable but `UnsupportedFeature` until impl lands; promoted into `default_set()` when it does.

---

## 4. Newtypes

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockNumber(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InodeNumber(pub u32);

/// Block size is constrained; not free-form u32.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockSize { Size1K, Size2K, Size4K }

impl BlockSize {
    pub const fn as_bytes(self) -> u32;   // 1024 / 2048 / 4096
}

impl Default for BlockSize { /* Size4K */ }
```

S5 findings validated that these newtypes catch real bugs at compile time (the `encode_inline_extent` story). They derive meaningful arithmetic sparingly: `BlockNumber + u64 -> BlockNumber`, but NOT `BlockNumber + BlockNumber`, to prevent nonsensical math (`scatter.md § translation` applied).

---

## 5. `init_block` module — D-003 synthesis

```rust
pub mod init_block {
    use super::{Error, Writer, Size};
    use std::path::{Path, PathBuf};

    /// Synthesize init.block from a vminitd ELF. Result is cached in $XDG_CACHE_HOME
    /// keyed by SHA-256 of the ELF bytes; subsequent calls with the same input
    /// return the cached path in O(stat) time.
    pub fn synthesize(elf: &[u8]) -> Result<PathBuf, Error>;

    /// Same as synthesize but writes to a caller-chosen path, no caching.
    pub fn synthesize_to(elf: &[u8], dest: impl Into<PathBuf>) -> Result<(), Error>;

    /// The cache path that synthesize would produce for `elf`. Useful for
    /// warming, invalidation, or debugging stale caches.
    pub fn cache_path(elf: &[u8]) -> PathBuf;

    /// XDG cache dir used for init.block artifacts. Override via env var
    /// `FIRKIN_CACHE`.
    pub fn cache_dir() -> PathBuf;
}
```

### 5.1 What `synthesize` writes

An ext4 image containing:
- `/sbin/vminitd` — the ELF, mode 0755.
- `/bin`, `/sbin`, `/proc`, `/sys`, `/run`, `/dev`, `/etc`, `/tmp` — empty directories.
- `/etc/passwd` with a minimal root entry.
- `/etc/hosts` with a minimal localhost entry.
- `/etc/resolv.conf` — empty (configured at container spawn per-container).
- Symlinks vminitd expects (e.g. `/sbin/init` → `/sbin/vminitd`).

Deterministic: same ELF input → same ext4 bytes out. UUID is derived from the ELF's SHA-256 hash (not random per synthesis call). Same timestamps (epoch-zero) for all entries.

### 5.2 Cache key

```
$XDG_CACHE_HOME/firkin/init-blocks/<sha256_hex>.ext4
```

The SHA-256 is of the ELF bytes exactly as passed to `synthesize()`. Cache entries never expire; worst case is disk fills up with old vminitd versions, mitigated by the fact that `pin.toml` only changes rarely.

### 5.3 Callers

- **`core/build.rs`** — during `cargo build`, ensures the cache is warm for the pinned vminitd ELF. Idempotent; no-op if cached.
- **`core` at `VirtualMachine::boot`** — synthesizes-or-reads-cache when a VM needs `init.block` attached.

Convergence: D-003 (embed ELF, not init.block) + D-004 (one ext4 writer for rootfs + init.block) meet here.

---

## 6. Correctness — the golden-diff testing contract

### 6.1 The law

> For every fixture F and every `Features` subset S ⊆ `default_set()` *for the current library version*:
>
> `Writer::new(tmp, size).features(S).write_directory("/", F).finalize()` produces bytes byte-identical to `mkfs.ext4 -O <S_features> tmp && mount tmp /mnt && cp -a F/* /mnt/ && umount tmp`, modulo `(UUID, timestamps)`.

In `test_design.md § 2` terms, this is a **Law test** — "for all valid F and S, this property holds." A failing assertion means either our Writer has a bug or `mkfs.ext4`'s behavior changed (worth investigating, not reverting).

Because `default_set()` grows over releases, the law's domain grows with it — every new Phase 2 feature promotes from "enum-reachable + `UnsupportedFeature`" to "enum-reachable + golden-diff-tested" in one PR.

### 6.2 Fixture layout

```
ext4/tests/
├── fixtures/               # input trees
│   ├── busybox/            # a minimal container rootfs
│   ├── debian-slim/        # a larger one; exercises DIR_INDEX + DIR_NLINK
│   ├── with-xattrs/        # exercises EXT_ATTR
│   ├── deep-tree/          # exercises DEEP_EXTENTS
│   └── tiny-files/         # exercises INLINE_DATA
├── golden/                 # pre-built images from mkfs.ext4
│   ├── busybox-default.ext4
│   ├── busybox-spike_set.ext4
│   ├── ...
└── golden_diff_test.rs     # the Law test harness
```

### 6.3 Running

```bash
# Fast unit tests (no golden diff):
cargo test -p ext4

# Full golden-diff suite (requires mkfs.ext4 + mount privileges; Linux-only):
cargo test -p ext4 --features golden-diff
```

Golden-diff needs mount privileges (Linux CI runs as root; macOS CI can't run it because macOS doesn't support loop-mounting ext4 natively). That's why macOS CI runs only unit tests for `ext4`; Linux CI runs golden-diff.

### 6.4 How `default_set()` growth works in practice

At any given release, `default_set()` = what the writer actually implements. The golden-diff harness drives `mkfs.ext4` with the same flags, so "what we ship" and "what we test" stay locked together.

When a Phase 2 feature lands (say, `DIR_INDEX`):

1. Implementation lands in the writer — `finalize()` now emits hash-indexed directory blocks when the flag is set.
2. A new golden fixture goes in `ext4/tests/fixtures/` exercising the feature.
3. `default_set()` in `impl Features` gains `| DIR_INDEX`.
4. Release notes cite: "`DIR_INDEX` promoted from Phase 2 to default."

Consumers who pinned `= "0.3.x"` see no change. Consumers on `"0.x"` pick up the new default on their next `cargo update`; existing fixtures keep passing because `spike_set()` is frozen.

The three-constructor setup (`spike_set()` / `default_set()` / `mkfs_parity_target()`) is what lets this happen without churn: `spike_set()` pins regression coverage, `default_set()` tracks reality, `mkfs_parity_target()` advertises the ceiling.

---

## 7. Crate boundaries — what `ext4` does NOT depend on

Per D-004, the `ext4` crate's `Cargo.toml` contains no macOS-specific dependencies:

- **No `objc2-*`**
- **No `core-foundation-*`**
- **No `cocoa-*`**

Allowed deps: `bytemuck`, `thiserror`, `uuid`, `bitflags`, `flate2` (for gzip OCI layer decompression), `zstd` (for zstd layer decompression — audit A.3 addition), `tar` (layer entry parsing), `nix` (for stat/chmod constants on Linux), `sha2` (for init-block cache keys).

No `serde` / `serde_json` at runtime (the crate doesn't serialize anything user-facing); only as a `dev-dependencies` for test fixture handling.

This means `cargo check --target x86_64-unknown-linux-gnu` on the `ext4` crate alone works on any platform — enabling fast cross-platform CI.

---

## 8. Error surface

Defined in full in [`05-error-model.md § ext4::Error`](./05-error-model.md). Summary:

- `Io(#[source] std::io::Error)` — filesystem I/O.
- `DiskFull { block }` — writer ran out of space mid-write.
- `InvalidLayer { layer, reason }` — OCI layer contains an entry we can't handle.
- `UnsupportedFeature { feature }` — user asked for a feature we don't implement.
- `ImageTooSmall { requested, needed }` — Writer::new size is too small for the content.
- `OrphanWhiteout { path }` — whiteout entry with no matching lower entry.
- `XattrTooLarge { name }` — xattr value exceeds inline + block-store capacity.
- `DirNLinkRequired { path }` — dir needs > 65k hardlinks but DIR_NLINK feature isn't set.

No classifier helpers (no `is_transient` etc.); every variant requires an input / config fix. A blanket "always retryable = false" is uninformative.

---

## 9. Worked examples

### 9.1 Rootfs from OCI layers

```rust
use firkin::ext4::{Writer, Features, Size};

let rootfs_path = Writer::new("/tmp/my-rootfs.ext4", Size::mib(128))
    .features(Features::default_set())
    .label("my-rootfs")
    .write_oci_layers(&bundle)?                   // trait-dispatched (D-024)
    .finalize()?;

// rootfs_path is "/tmp/my-rootfs.ext4"; hand it to Rootfs::ext4_image(...)
```

`&bundle` satisfies `OciLayerSource` via `firkin-oci`'s impl — no explicit `.layers_for_ext4()` call, no method name mentioning `ext4` on the oci side. `ext4` still doesn't depend on `oci-spec`; the conversion from `oci::MediaType` to `LayerCompression` lives on `oci::Layer::compression()`.

### 9.2 Minimal hand-built rootfs (spike-style testing)

```rust
let rootfs_path = Writer::new("/tmp/hello.ext4", Size::mib(8))
    .features(Features::spike_set())
    .write_directory("/", "/dev/null")?       // create /
    .write_file("/hello", b"hi\n", 0o644)?
    .write_symlink("/greeting", "/hello")?
    .finalize()?;
```

### 9.3 In-memory for a test

```rust
let bytes = Writer::in_memory(Size::mib(8))
    .write_file("/marker", b"test", 0o644)?
    .into_bytes()?;

assert_eq!(&bytes[0x400..0x402], &[0x53, 0xef]);  // ext4 magic at superblock offset
```

### 9.4 Synthesizing init.block

```rust
let vminitd_elf: &[u8] = include_bytes!("../vendor/vminitd");

// First call: ~200ms to write the ~384 MiB init.block.
let path_1 = firkin::ext4::init_block::synthesize(vminitd_elf)?;

// Second call: O(stat). Same path.
let path_2 = firkin::ext4::init_block::synthesize(vminitd_elf)?;
assert_eq!(path_1, path_2);
```

---

## 10. Cross-crate integration

### 10.1 How `core` uses `ext4`

- At `VirtualMachine::boot`: calls `init_block::synthesize(VMINITD_ELF)` to get a path, attaches it as a virtio-block device. Synthesis is memoized by SHA-256 so repeat boots are O(stat).
- At `Container::spawn` with `Rootfs::OciBundle(bundle)`: builds a `Writer`, writes layers via `write_oci_layers`, finalizes to a temp path, attaches to the VM as the container's rootfs.

### 10.2 How `oci` interacts with `ext4`

`firkin-oci` implements the sealed `ext4::OciLayerSource` trait for `ImageBundle` (D-024). The trait method yields `(&Path, ext4::LayerCompression)` pairs — path points at the compressed layer file in oci's content-addressable cache, and the compression (produced by `oci::Layer::compression()`) tells ext4 how to decode it. `ext4::Writer::write_oci_layers(&bundle)` dispatches through the trait and handles decompression + tar extraction + whiteout processing internally.

**No type flows directly between `oci` and `ext4`** beyond `LayerCompression` and the `OciLayerSource` trait, both owned by `ext4`. `oci` depends on `ext4` (to impl the trait); `ext4` does not depend on `oci` / `oci-spec`. (`scatter.md § local`: interfaces at clean boundaries; no scattered knowledge.)

---

## 11. Invariants worth locking

1. `Writer` is a consuming-self builder. Methods take `self`, return `Result<Self, Error>`.
2. `Features::default_set()` matches `mkfs.ext4` defaults — the recommended production set.
3. `Features::spike_set()` preserves S5's minimum for regression-testing the narrow subset.
4. `BlockNumber` and `InodeNumber` are newtypes; arithmetic is constrained.
5. `init_block::synthesize(elf)` is the single D-003 entrypoint; cached by SHA-256; deterministic.
6. No `Reader` in v1. Byte-for-byte diff against `mkfs.ext4` is the correctness backstop.
7. No macOS coupling; crate compiles and tests fully on Linux CI.
8. `write_oci_layers` takes `&impl OciLayerSource` (D-024, sealed trait in `ext4`); `write_layers_raw` takes hand-rolled `(P, LayerCompression)` pairs for tests. `LayerCompression` is owned by `ext4` so the crate stays free of `oci-spec` deps (audit A.3).
9. `ext4` depends on `firkin-types` for shared newtypes (D-015); does not depend on `oci` / `oci-spec` / `vmm` / `vsock`. `firkin-oci` depends on `ext4` (to implement `OciLayerSource` for `ImageBundle`), not the reverse.

Proceed to [`07-oci-crate.md`](./07-oci-crate.md) for the OCI registry client.
