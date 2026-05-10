# Deferred EXT4 Features

This crate intentionally implements the EXT4 subset needed by the current
Apple/VZ local runtime path. The writer mirrors the Swift formatter's practical
shape: 32-bit ext4, multi-block-group layout, extents, file type directory
entries, flex_bg, sparse_super2, xattrs, large files, huge files, and extra
inode size.

Do not infer production-template compatibility from this file. These are hard
gaps until they are implemented and covered by both filesystem-level tests and
guest-mount evidence.

## Deferred

| Feature | Current behavior | Unlock condition |
|---|---|---|
| Deeper extent trees | Files can spill from inline extent leaves into a depth-1 extent leaf block only. | Add depth > 1 extent indexes/leaves, roundtrip tests, and guest reads for large sparse and non-sparse files. |
| HTree directory indexing | Directories are written as linear directory blocks. | Add `dir_index` support, hash seed/version handling, split tests, and guest directory traversal proof. |
| Metadata checksums | `metadata_csum` is off. | Add checksum generation/validation across superblock, group descriptors, bitmaps, inode tables, dirs, and xattrs. |
| Journal writing | No journal is emitted. | Add jbd2 journal structures only if a writable rootfs path requires it; read-only images do not need this. |
| `resize_inode` / online resize metadata | Not emitted. | Implement only if the runtime supports growing existing rootfs images in place. |
| `inline_data` | Not emitted. | Implement only if space or compatibility tests prove it is needed. |

## Implemented And Proved

| Feature | Evidence |
|---|---|
| Multi-block-group image layout | `crates/ext4/tests/writer_api.rs` serializes a 1 KiB-block image crossing the first group; `crates/ext4/tests/fixture.rs` verifies a multi-group fixture with `e2fsck -nf`. |

## Current acceptance bar

Any feature claimed as supported needs:

1. Unit or integration coverage in `crates/ext4/tests`.
2. `e2fsck -nf` clean output for the generated image.
3. Linux guest mount/read evidence through the real VM replay path when the
   feature affects kernel-visible layout.
4. An updated compatibility note in
   `docs/specs/rust_rewrite/05-e2b-cube-local-backend.md` if it changes E2B or
   production-template readiness.
