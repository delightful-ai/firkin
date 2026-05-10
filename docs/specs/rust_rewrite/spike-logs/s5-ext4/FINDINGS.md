# S5 — findings

Tier 1 cleared in ~3 focused hours; Tier 2 added in the next ~1.5. The rest
of the spike went into closing Tier 3 for real (depth-1 extent trees +
structural diff) and adding Tier 4's OCI overlay markers plus guest-side
listing probes. The "~1 week" budget in `02-spike-plan.md` was generous: the
calendar risk was still front-loaded into reading Swift + kernel headers, and
the implementation itself stayed mechanical once the on-disk shapes were
understood.

## What worked as planned

### `#[repr(C)]` + `bytemuck::Pod` is still the right tool

Swift's `withUnsafeLittleEndianBytes(of: superblock)` pattern still maps cleanly
to Rust as `bytemuck::bytes_of(&sb)` / `pod_read_unaligned::<T>()`. The
`min_const_generics` feature remains load-bearing for `[u8; 60]`, `[u8; 64]`,
and friends.

Static size asserts on the on-disk structs continue to pay for themselves:
they catch layout drift before the first `e2fsck` run.

### Newtypes stayed useful once device nodes showed up

`BlockNumber`, `InodeNumber`, and `BlockSize` were already doing real work.
Extending `FileMode` with `IFCHR` for whiteouts kept the new overlay support in
the same pattern instead of sneaking raw octal masks through the builder.

### Error design held up

No new catch-all error was needed. The new behavior fit inside the existing
domain errors:

- invalid whiteout targets still surface as `InvalidFilename` /
  `PathConflict`
- missing extent-leaf allocation remains an `Internal` invariant break
- unsupported deeper fanout continues to use `ExtentDepthExceeded`

That is the right shape for the real crate too: callers should see domain
stories, not `io::Error` enums leaking out of the core.

### Test shape discipline kept the extension honest

The suite now has 18 tests plus 2 doc tests:

- 9 law/example tests in `tests/roundtrip.rs`
- 9 fixture/debugfs/e2fsck tests in `tests/fixture.rs`

The important new tests each kill a concrete family of bad implementations:

- `oversized_file_uses_a_depth_one_extent_tree`
  - kills writers that let file extents overflow the inode's four inline slots
    but still claim `eh_depth = 0`, or forget the external leaf block
- `depth_one_extent_fixture_passes_e2fsck_and_debugfs_reports_index_root`
  - kills writers whose bytes look locally plausible but fail actual ext4
    readers once the tree leaves the inode body
- `whiteout_fixture_is_a_character_device_and_passes_e2fsck`
  - kills writers that fake whiteouts as regular files or symlinks
- `opaque_dir_fixture_is_an_empty_regular_file_marker`
  - kills writers that encode opacity as xattrs, directories, or non-empty
    marker payloads
- `determinism_same_input_same_bytes`
  - now covers whiteout + opaque markers too, so overlay semantics do not
    regress determinism

## Design decisions that mattered

### Stay single-group; cap file extents at 4096 blocks

There were two plausible ways to force a depth-1 tree inside the spike:

1. broaden the spike to multi-group allocation
2. stay single-group and deliberately cap contiguous file extents so the inode's
   four inline slots overflow within the 128 MiB budget

I chose option 2 because it is narrower and more consistent with the spike's
existing shape. The serializer now caps contiguous file extents at
**4096 blocks (16 MiB)**. That gives a real depth-1 tree for an `80 MiB + 1 B`
file without dragging group iteration, per-group bitmaps, or backup-superblock
rules into S5.

This is a spike decision, not a claim that the real crate should fragment files
this way forever. The real crate can revisit coalescing once multi-group support
lands.

### Whiteout/opaque support belongs in the builder API

The user-facing surface is now:

- `add_whiteout("/upper/gone")` → `/upper/.wh.gone` as a `0:0`
  character-device inode
- `add_opaque_dir("/opaque")` → `/opaque/.wh..wh..opq` as an empty
  regular file

This is better than asking callers to hand-assemble special filenames. The API
accepts the logical overlay intent and emits the actual on-disk marker names.

## Gotchas that cost real time

### Every directory still needs a data block

This remained the first real `e2fsck` footgun. Empty directories still need
space for `.` and `..`; skipping their data block yields misleading reference
count errors.

### Inline xattr `e_value_offs` is still relative to `buf + 4`

Nothing changed here: the parser truth still lives in
`lib/ext2fs/ext_attr.c::read_xattrs_from_buffer`, and getting the base wrong
still yields the useless `allocation collision` message.

### Initramfs needs both `/dev` and `/proc`

`/dev` was already required so `/dev/vda` exists. The new Tier 4 guest probes
added a second initramfs gotcha:

- `/proc/cmdline` is unreadable until procfs is mounted
- after mounting procfs, the parser must trim trailing ASCII whitespace and
  treat all ASCII whitespace as token separators, not just `' '`

Without that, the host passes the right kernel command line and the guest still
reports either "no probes requested" or `ENOENT` on the requested path.

## Structural diff vs `mkfs.ext4`

### Fixture and normalization

Comparison fixture:

- one file, `/big`
- payload = `80 MiB + 1 byte`
- image size = `128 MiB`

`mkfs.ext4` was invoked with the same intended feature mask:

- enabled: `ext_attr,sparse_super2,filetype,extent,flex_bg,large_file,huge_file,extra_isize`
- disabled: `64bit,metadata_csum,dir_index,dir_nlink,has_journal,orphan_file,resize_inode,sparse_super`

Before comparing, I normalized both images by zeroing:

- UUID
- superblock timestamps
- mount-count fields
- inode timestamps across the inode table

### What matches

After normalization, the load-bearing geometry matches:

- feature mask
- block count = `32768`
- inode count = `8192`
- block size = `4096`
- inode size = `256`
- first inode = `11`
- blocks-per-group = `32768`
- inodes-per-group = `8192`
- both images pass `e2fsck -nf`

### What still diverges

The normalized images are still far from byte-identical (`cmp` still reports
~4.24M differing bytes). The meaningful reasons are:

1. **Extent packing**
   - ours: depth-1 root + one external leaf block; six extents
     (`4096,4096,4096,4096,4096,1`)
   - `mkfs.ext4`: depth-0 inline tree with three extents
   - why: the spike now deliberately caps contiguous file extents at 4096
     blocks to exercise depth-1 plumbing; `mkfs.ext4` coalesces the long tail

2. **Superblock policy knobs**
   - ours: `Default mount options: (none)`, `s_log_groups_per_flex = 31`
   - `mkfs.ext4`: `Default mount options: user_xattr acl`,
     `Filesystem flags: signed_directory_hash`, random directory-hash seed,
     `Flex block group size: 16`
   - why: the spike leaves policy-ish knobs zero unless the kernel/e2fsck needs
     them; mkfs writes its normal defaults even with `dir_index` disabled

3. **Ownership**
   - ours: inode 12 owner/group = `0:0`
   - `mkfs.ext4`: inode 12 owner/group = the host source file's owner
     (`501:0` in this run)
   - why: `mkfs.ext4 -d` preserves host metadata from the source tree; the
     spike currently zeroes uid/gid unless the caller says otherwise

4. **Free/used block accounting**
   - ours: `11768` free blocks
   - `mkfs.ext4`: `11766` free blocks, `Overhead clusters: 516`
   - why: mkfs reserves/places a few more metadata blocks while populating from
     a host directory; the spike keeps the single-pass layout tighter

### Honest conclusion

This spike is no longer hand-wavy about the `mkfs.ext4` comparison:

- the filesystem geometry and required feature bits match
- the remaining byte differences are explained, not mysterious
- the biggest intentional divergence is the new 16 MiB extent cap that keeps
  depth-1 support testable inside the single-group budget

That is enough for the spike. It is not yet a claim of byte identity with
either `mkfs.ext4` or `cctl rootfs create --ext4`.

## Reusable patterns

### "Reserve at the start, finalize at the end"

The single-pass `Finalizer` pattern still holds:

1. allocate inode numbers
2. reserve block 0 + GDT
3. write data blocks
4. write any external extent-leaf blocks
5. write inode table
6. write bitmaps + GDT
7. write superblock

The important extension is that external extent-leaf blocks fit naturally into
the same high-water-mark allocator. They did not require a second pass.

### Guest probes via kernel cmdline are cheap and effective

The VM harness now supports:

- `SPIKE_CAT_PATH=/path`
- `SPIKE_LS_PATHS=/a,/b`

That is enough to validate "the guest can read this file" and "the guest sees
these overlay marker names and inode types" without building a larger RPC or
agent surface into the initrd.

## Known loose ends

1. **Single block group.** Still the main real limitation before container-sized
   images.
2. **Extent tree depth > 1.** Depth-1 is in; deeper fanout is still out.
3. **htree directories.** Linear directories only.
4. **Metadata checksums.** `metadata_csum` remains off.
5. **Byte-for-byte parity against `cctl rootfs create --ext4`.** Still not
   attempted; that flow needs a different fixture and more runtime baggage.

## Proposed PRO_TIPS additions

Still worth folding upstream:

1. Inline xattr `e_value_offs` base differs from block-xattr `e_value_offs`.
2. Every directory needs a data block.
3. Kata's kernel has ext4 + virtio_blk + devtmpfs built in.
4. `bytemuck` needs `min_const_generics` for ext4-shaped arrays.
5. `pod_read_unaligned` is the right test-side decoder for on-disk structs.
6. Guest probes that read `/proc/cmdline` need procfs mounted first.

## Time

- Reading philosophy + spec + Swift reference: ~60 min
- `types.rs`, `error.rs`, `layout.rs`: ~30 min
- First version of `builder.rs`: ~60 min
- First e2fsck-clean `/hello`: +15 min
- Fixing xattr offs-base + link-count bugs: ~45 min
- Tier 3 depth-1 close-out + Tier 4 overlay semantics: ~60 min
- Guest probe generalization + procfs/cmdline fix: ~30 min
- CLI presets, verification passes, and durable notes: ~50 min

**Total: ~6.5 hours.**
