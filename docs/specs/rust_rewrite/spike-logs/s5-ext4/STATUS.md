# S5 — status snapshot

Last updated: 2026-04-20

## State
- 🟢 **Passed** — Tier 1 through Tier 4 acceptance is met inside the spike
  scope.
- Fresh verification on 2026-04-20:
  - `cargo test` passes (`tests/fixture.rs`: 9, `tests/roundtrip.rs`: 9,
    plus 2 doc tests).
  - `cargo build --release` passes.
  - `e2fsck -nf` is clean on the `hello`, `tree`, `links`, `xattr`,
    `multi-extent`, `deep-extent`, and `overlay` fixtures.
  - Guest mount succeeds on both flows:
    - default `/hello` cat via `./sign-and-run.sh`
    - overlay directory listing via `vm-mount-check --ls-path /upper --ls-path /opaque`

## Cleared tiers

- **Tier 1** ✅ `mkext4` emits a 128 MiB ext4 image with `/hello = hi\n`.
  `e2fsck -nf` is clean. The guest mounts the image read-only and prints
  `SPIKE_CAT_BEGIN:/hello:hi\n:SPIKE_CAT_END:/hello`.
- **Tier 2** ✅ Multiple files + nested directories, fast symlinks, hardlinks,
  and xattrs all survive `e2fsck -nf`. `debugfs` confirms inline xattrs.
- **Tier 3** ✅ A large file now overflows the inode’s four inline extent
  slots and serializes as a real depth-1 extent tree:
  - fixture: `/big` = `80 MiB + 1 byte`
  - `debugfs dump_extents /big` shows a `0/1` root and `1/1` leaf rows
  - `e2fsck -nf` stays clean
  - structural diff against `mkfs.ext4` with the same feature mask is
    documented in FINDINGS.md, including the deliberate extent-packing
    divergence
- **Tier 4** ✅ OCI overlay semantics are covered:
  - `add_whiteout("/upper/gone")` emits `/upper/.wh.gone` as a `0:0`
    character-device inode
  - `add_opaque_dir("/opaque")` emits `/opaque/.wh..wh..opq` as an empty
    regular file
  - the determinism law test includes both features
  - the guest `ls` probe shows:
    - `/upper`: `c .wh.gone 0:0`
    - `/opaque`: `- .wh..wh..opq 0`

## Repro

```bash
cd ~/tmp/rust-rewrite-spikes/s5-ext4

# Core verification.
cargo test
cargo build --release

# Rebuild the guest initrd after editing init/init.c.
./init/build.sh

# Tier 1 VM flow (default cat /hello).
SPIKE_TIMEOUT_SECS=60 ./sign-and-run.sh

# Tier 3 deep-extent fixture.
target/debug/mkext4 --out /tmp/s5-deep.img --preset deep-extent
/opt/homebrew/opt/e2fsprogs/sbin/e2fsck -nf /tmp/s5-deep.img
/opt/homebrew/opt/e2fsprogs/sbin/debugfs -R 'dump_extents /big' /tmp/s5-deep.img

# Tier 4 overlay fixture.
target/debug/mkext4 --out /tmp/s5-overlay.img --preset overlay
/opt/homebrew/opt/e2fsprogs/sbin/e2fsck -nf /tmp/s5-overlay.img
/opt/homebrew/opt/e2fsprogs/sbin/debugfs -R 'stat /upper/.wh.gone' /tmp/s5-overlay.img
/opt/homebrew/opt/e2fsprogs/sbin/debugfs -R 'stat /opaque/.wh..wh..opq' /tmp/s5-overlay.img

# Guest mount + ls probes for the overlay fixture.
codesign --force --sign - --entitlements entitlements.plist target/debug/vm-mount-check
target/debug/vm-mount-check --image /tmp/s5-overlay.img --ls-path /upper --ls-path /opaque
```

If Homebrew installed `e2fsprogs` somewhere else, adjust the `debugfs` and
`e2fsck` paths accordingly.

## Handoff notes

- The spike is still **single-group**. That is now the main remaining layout
  limitation.
- Depth-1 extent trees are supported, but only within the single-group budget.
  The serializer intentionally caps contiguous file extents at **4096 blocks
  (16 MiB)** so a depth-1 tree can be exercised and validated without dragging
  multi-group support into the spike.
- Whiteout/opaque support is implemented directly in the builder API:
  - `add_whiteout(path)` takes the logical deleted path and materializes the
    correct `.wh.<basename>` marker.
  - `add_opaque_dir(path)` materializes `.wh..wh..opq` inside the directory.
- The VM harness now has two guest-side probes:
  - `SPIKE_CAT_PATH=/path`
  - `SPIKE_LS_PATHS=/a,/b`
  The initrd must mount both `proc` and `devtmpfs` before those probes work.

## Remaining gaps

1. **Multi-block-group images.** Needed before real container rootfs sizes.
2. **Extent trees deeper than one level.** Depth-1 is enough for the spike;
   deeper fanout belongs with multi-group work.
3. **htree directories.** Linear directory blocks only.
4. **Metadata checksums.** `metadata_csum` is still off.
5. **Byte-for-byte parity with `mkfs.ext4` or `cctl rootfs create --ext4`.**
   FINDINGS now says exactly what still diverges and why.

## Done checklist

- [x] Acceptance criteria from `02-spike-plan.md` met inside the spike scope.
- [x] `sign-and-run.sh` exits 0 from a cold build and prints the `/hello` cat sentinel.
- [x] Debug and release builds both pass.
- [x] JOURNAL.md has the final decision trail and verification notes.
- [x] FINDINGS.md includes the structural diff vs `mkfs.ext4`.
- [x] State line above reads `🟢 Passed`.
- [ ] `spike-logs/README.md` index updated — left for the curator per the shared-docs rule.
