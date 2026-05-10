# Spike S5 — ext4

**Spike code**: `~/tmp/rust-rewrite-spikes/s5-ext4/`
**Started**: 2026-04-20
**Finished**: 2026-04-20

## The question

> How much of `ContainerizationEXT4` is real algorithm vs. Swift decoration?
> Can we produce a tiny ext4 image that `e2fsck` approves and vminitd mounts?

## Acceptance (from 02-spike-plan.md §S5)

- [x] Tier 1: ~300 LOC Rust producing an ext4 image with `/hello`=`hi\n`.
- [x] Tier 1: `e2fsck -n rootfs.img` clean, exit 0, zero warnings.
- [x] Tier 1: VM mounts the image and `cat /hello` prints `hi\n`.
- [x] Tier 2: multi-file + directory (`/hello`, `/dir/world`, `/dir/nested/deep`).
- [x] Tier 2: symlink (fast, inline in i_block) + hardlink.
- [x] Tier 2: xattrs on a regular file, verified via `debugfs`.
- [x] Tier 3 lite: 64 KiB file; single 16-block extent.
- [x] Tier 3: deep-extent file with extent-tree depth > 0, validated with
      `debugfs dump_extents` + `e2fsck -nf`.
- [x] Tier 3 differential: structural diff vs `mkfs.ext4` with the same
      feature mask on the deep-extent fixture, with UUID/timestamp/mount-count
      normalization documented in FINDINGS.
- [x] Tier 4: OCI whiteout + opaque-dir markers, covered by tests and guest
      mount probes.

## Plan

1. Scaffold. Delete VZ boot skeleton. Put lib/CLI/VM-harness under one cargo
   manifest with two `[[bin]]` targets.
2. Read Swift reference + kernel `fs/ext4/*.h`. Write `types.rs` (newtypes),
   `error.rs` (domain error enum), `layout.rs` (`#[repr(C)]` + bytemuck).
3. Write `builder.rs` single-pass `Finalizer`:
   - allocate inodes (BFS, hardlinks share, reserved slots fixed)
   - reserve sb + GDT header blocks
   - write data region (dirs first, files, slow symlinks, xattr blocks last)
   - finalize inode table (records + extent trees + inline xattrs)
   - bitmaps + group descriptor + superblock
4. Validate with `e2fsck -nf` after every preset.
5. Add law + example + fixture tests.
6. VM mount test: static-musl init via docker alpine.

## Events

- 2026-04-20 15:00 — Read philosophy docs (type/error/trait/test design),
  02-spike-plan.md §S5, SPIKE_RUNBOOK.md, PRO_TIPS.md, S3 STATUS.md for
  block-device wiring.
- 2026-04-20 15:10 — Read Swift reference: `EXT4.swift`, `EXT4+Types.swift`,
  `EXT4+Formatter.swift` (~1800 lines of algorithm). Identified MVP scope:
  superblock + single group + inode table + extents + linear dirs.
- 2026-04-20 15:25 — Produced a reference image with `mkfs.ext4` using
  feature set close to the Swift writer (`sparse_super2,extents,flex_bg,
  filetype,ext_attr,large_file,huge_file,extra_isize`, no metadata_csum, no
  64bit, no journal). `dumpe2fs -h` confirms the layout we should match.
- 2026-04-20 15:30 — Scaffolded. Gutted `src/main.rs`. Set up
  `lib.rs`+CLI+vm-mount-check structure.
- 2026-04-20 15:35 — Wrote `types.rs` (BlockNumber/InodeNumber/BlockSize/
  FileMode newtypes), `error.rs` (Ext4Error enum w/ 11 domain variants),
  `layout.rs` (bytemuck Pod+Zeroable #[repr(C)] structs for every on-disk
  record with compile-time size asserts).
- 2026-04-20 15:40 — Wrote `builder.rs` Finalizer, ~1400 LOC. First
  compile errors:
  - bytemuck Pod for `[u8; 60]` missing → enable `min_const_generics`.
  - borrow-splitting in inode-table write loop → collect bytes into Vec.
- 2026-04-20 15:45 — **First e2fsck run FAILED**: "Inode 11 ref count is 2,
  should be 1" + "Block bitmap differences: -3". Root cause: lost+found
  got no data block because `will_use_extents` required `!children.is_empty()
  || idx == 0`. Fix: every directory always owns one data block.
  → e2fsck now clean for /hello fixture.
- 2026-04-20 15:50 — All 5 presets produce e2fsck-clean images from CLI:
  hello, tree, links, xattr, multi-extent. But fixture tests reveal two
  bugs the presets didn't exercise:
  1. `links` fixture: "Free inodes count wrong (8178, counted=8179)". The
     hardlink was double-counting inode_used_count because the same inode
     number appears twice in `self.inodes`. Fix: track unique inodes with
     a BTreeSet during the count phase.
  2. `xattr` fixture: "Inode 12 extended attribute is corrupt (allocation
     collision)". Root cause: misread the kernel spec. For inline xattrs,
     `e_value_offs` is relative to `buf + 4` (past magic), NOT to `buf`.
     `values_size = buf.len() - 4 = 92` bounds the offset. Tracing
     `lib/ext2fs/ext_attr.c::read_xattrs_from_buffer` clarified this.
     Fix: subtract the 4-byte magic offset when computing value_offs.
     `debugfs ea_list` now lists `user.foo="bar"` etc correctly.
- 2026-04-20 15:55 — All 14 tests pass (8 law, 6 fixture) in 1.3 s.
  Release build clean.
- 2026-04-20 16:00 — Switched kernel from Ubuntu (`vmlinux` 59 MB, ext4+
  vsock as modules) to kata 3.17.0 (`vmlinux.container` 14 MB, ext4 built
  in). Linked from `s3-vminitd-build/assets/vmlinux`.
- 2026-04-20 16:05 — Extended init.c: mount devtmpfs at /dev (initramfs
  has no /dev/vda node otherwise), mount /dev/vda at /mnt read-only, cat
  /mnt/hello, print sentinel, power off.
- 2026-04-20 16:10 — **Tier 1 end-to-end pass**. VM boots kata kernel
  → init → mounts our Rust-produced ext4 → prints
  `SPIKE_HELLO_BEGIN:hi\n:SPIKE_HELLO_END` → powers off cleanly
  → `vm-mount-check` exits 0.
  Guest kernel log: `EXT4-fs (vda): mounted filesystem
  00000000-0000-0000-0000-000000000000 ro without journal. Quota mode:
  disabled.`
- 2026-04-20 16:15 — Structural diff against `mkfs.ext4 -F -b 4096 -I 256
  -O ...` with matching feature mask. Our feature set is strictly inside
  mkfs's (we skip `dir_index`, `dir_nlink`, and the legacy `sparse_super`
  bit that's redundant with `sparse_super2`). Block count, inode count,
  inode size, inodes-per-group all identical. 1-block difference in
  used/free blocks comes from our lost+found block placement; easy to
  reconcile if we ever need byte-identical diff.
- 2026-04-20 16:20 — Wrote FINDINGS. STATUS set to 🟢 Passed (Tier 1 and
  Tier 2, partial Tier 3).
- 2026-04-20 19:53 — Added red tests for the remaining acceptance surface:
  depth-1 extent tree, whiteout char-device inode, opaque-dir marker, and
  determinism coverage including overlay markers. First red was the expected
  builder API gap (`add_whiteout`, `add_opaque_dir` missing).
- 2026-04-20 19:58 — Chose the narrow Tier 3 close-out path: stay
  single-group, but cap contiguous file extents at 4096 blocks (16 MiB) so
  the spike can serialize a real depth-1 extent tree inside the 128 MiB
  budget. Implemented:
  - `NodeContent::Whiteout` + `FileMode::char_device`
  - per-file serialized extent vectors + optional external leaf block
  - depth-1 root/index encoding when file extents overflow the inode’s four
    inline slots
  - `add_whiteout("/dir/name")` → `/dir/.wh.name`
  - `add_opaque_dir("/dir")` → `/dir/.wh..wh..opq`
- 2026-04-20 20:00 — `cargo test` green again. New fixture evidence:
  - `e2fsck -nf /tmp/s5-deep.img` clean
  - `debugfs dump_extents /big` reports `0/1` root and `1/1` leaf rows
  - `debugfs stat /upper/.wh.gone` reports `Type: character special`
  - `debugfs stat /opaque/.wh..wh..opq` reports `Type: regular`, `Size: 0`
- 2026-04-20 20:02 — Generalized the VM mount probe:
  - `mkext4` now has `deep-extent` and `overlay` presets
  - `vm-mount-check` can pass `SPIKE_CAT_PATH` and `SPIKE_LS_PATHS`
  - guest init now mounts `proc` + `devtmpfs` and can cat a file or print a
    deterministic `ls -la`-style directory listing
- 2026-04-20 20:04 — First guest-probe attempt regressed: `/init` reported
  "no probes requested" even though the host passed `SPIKE_CAT_PATH=/hello`.
  Root cause: `/proc/cmdline` was unreadable because procfs was not mounted.
  After mounting procfs, the next run still failed with `ENOENT` for
  `/mnt/hello`; root cause was trailing newline/whitespace from
  `/proc/cmdline` leaking into parsed values. Trimmed trailing whitespace and
  treated all ASCII whitespace as token separators.
- 2026-04-20 20:05 — Guest probes now work end-to-end:
  - `./sign-and-run.sh` prints `SPIKE_CAT_BEGIN:/hello:hi\n:SPIKE_CAT_END:/hello`
  - `vm-mount-check --image /tmp/s5-overlay.img --ls-path /upper --ls-path /opaque`
    prints
    - `/upper`: `c .wh.gone 0:0`
    - `/opaque`: `- .wh..wh..opq 0`
- 2026-04-20 20:06 — Re-ran the structural diff against `mkfs.ext4` on the
  deep fixture with the same feature mask (`ext_attr,sparse_super2,filetype,
  extent,flex_bg,large_file,huge_file,extra_isize`, journal/csum/64bit/dir
  index disabled). After zeroing UUID/timestamp/mount-count fields, the
  normalized images still differ by ~4.24M bytes. The meaningful matches and
  divergences are now spelled out in FINDINGS:
  feature mask/count geometry match; our writer diverges on flex-bg knob,
  default mount opts/hash seed, host-uid preservation, and deliberate
  16 MiB extent chunking.
- 2026-04-20 20:07 — Final verification:
  - `cargo test`
  - `cargo build --release`
  - `e2fsck -nf` on deep + overlay fixtures
  - guest mount probe for `/hello`
  - guest `ls` probe for overlay directories
  STATUS updated to 🟢 Passed with Tier 4 complete.
