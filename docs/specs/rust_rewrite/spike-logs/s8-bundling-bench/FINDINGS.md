# S8 — findings

Measurement-only spike. Three bundling strategies (A embed, B build.rs fetch,
C runtime fetch) were implemented end-to-end and timed against two blob sizes
(131 MiB vminitd ELF; 384 MiB `init.block`). Raw timings are in
[`JOURNAL.md`](./JOURNAL.md); this file is the decision table + rationale the
rest of the spec cites.

## Headline

**Default: Strategy A, embedding the 131 MiB vminitd ELF via `include_bytes!`.**
**Fallback: Strategy C (`--features runtime-download`) for binary-size-sensitive
consumers.** Strategy B collapses into the default-A path plus a `build.rs` that
populates the cache — it is a build-tooling detail, not a user-visible option.

Do **not** embed `init.block` (384 MiB). See § "Why not `init.block`" below.

## Decision table

| | Cold build | Warm touch-lib | Warm touch-main | Release binary | First-run latency | rlib size |
|---|---|---|---|---|---|---|
| A — embed vminitd ELF (131 MiB) | **4.85 s** | **4.51 s** | **0.49 s** | 133 MB | n/a | 677 MB |
| A — embed init.block (384 MiB) | 11.85 s | 21–41 s | 1.97 s | 387 MB | n/a | 2.0 GB |
| B — build.rs fetch init.block | 27.37 s (incl. ureq deps + fetch) | 53.12 s | 3.88 s | 387 MB | n/a | 2.0 GB |
| C — runtime fetch, no embed | 5.19 s (incl. ureq deps) | 0.26 s | 0.15 s | **1.0 MB** | +0.6 s loopback / ~11 s @ 100 Mbps (131 MiB) / ~32 s @ 100 Mbps (384 MiB) | 35 KB |

All three A/B/C variants hit every cold/warm/first-run tolerance listed in
`04-phase1-plan.md` **only when paired with the 131 MiB vminitd ELF.** The
384 MiB `init.block` variant of A busts warm-touch-lib (>20 s) and peak RSS
(7.23 GB during link).

## Why Strategy A is the default

- **Compile-time cost tracks blob size linearly.** Shrinking from 384 MiB to
  131 MiB cuts cold-build 2.4× and warm-touch-lib ~5×. The biggest single
  perf win is blob choice, not strategy choice.
- **Warm-touch-main is the realistic day-to-day dev loop.** 0.49 s with
  A/vminitd is unnoticeable; 1.97 s with A/init.block is borderline.
- **`ld` dead-strips unreferenced `include_bytes!` consts.** Measured: a
  consumer binary that never touches the `INIT_BLOCK` const links at 422 KB,
  not 131 MiB. So exposing the blob as a `pub const` in a leaf crate doesn't
  tax consumers who never spin up a VM.
- **`cargo check` is immune to the embed tax** (3.50 s cold on the 384 MiB
  variant vs 11.85 s for `cargo build`). IDE / type-check loops stay snappy.
- **Zero runtime network dependency.** Once the crate is vendored into a
  downstream `Cargo.lock`, building it is hermetic.

## Why not `init.block` (384 MiB)

- Warm-touch-lib: 21–41 s (variance), vs 4.51 s for the 131 MiB ELF.
- Peak RSS during link: 7.23 GB — uncomfortable on 16 GB dev laptops running
  a browser + IDE alongside. 32 GB machines are fine but the library should
  not require 32 GB to rebuild.
- rlib on disk: 2.0 GB. Unpleasant for consumers' `target/` size, even with
  dead-strip at link time.
- The `init.block` is a **deterministic function of the vminitd ELF**: same
  ELF → same ext4 bytes (modulo UUID which we pin). The `ext4` crate
  synthesizes it on-host at first VM boot, caches in `$XDG_CACHE_HOME` keyed
  by the ELF's SHA-256, and every subsequent VM boot on the host reuses it
  in O(stat) time. This converges with the S5 result (D-004): one EXT4 writer,
  two consumers (container rootfs + init.block). See [D-003](../../DECISIONS.md#d-003--embed-vminitd-elf-not-initblock).

## Why Strategy B collapses into A

Strategy B was "embed via `build.rs`-populated `$OUT_DIR/...`". Its
user-visible timings match A (strategy B's cold is 27 s only because it
compiles `ureq` for the first time). Once the blob is cached, a touch-main
edit rebuilds in 3.88 s — same shape as A, dominated by link cost on the
embedded const.

What B actually provides is a **cache-populator** for CI / fresh clones
where the ELF isn't checked in. That mechanism survives in the real library
as part of Strategy A (`firkin-vminitd-bytes/build.rs`), but it isn't a
separate user-selectable strategy. See [D-017](../../DECISIONS.md#d-017--vminitd-elf-distributed-via-pinned-download-not-checked-in).

## Why C stays as a feature, not a fallback-when-A-is-slow

- `ld` dead-strip already makes A cheap for consumers that don't instantiate
  a VM — the "binary-size" argument against A is largely neutralized at the
  final-binary level.
- `.rlib` on disk (677 MB for vminitd-A) still matters for **target-dir-
  sensitive CI workflows** and for vendored-into-another-product builds
  with strict binary-size budgets. C is the option for those.
- First-run latency for C is tolerable for 131 MiB on a 100 Mbps link
  (~11 s) but unpleasant if the user's first action is time-sensitive.
  Document that the feature exists; don't default to it.

## Surprises worth preserving

1. **Warm-touch-lib is sometimes slower than cold for the 384 MiB embed**
   (21–41 s vs 12 s). Cargo parallelizes the cold pipeline; warm serializes
   recompile-lib + relink-downstream-bin around the same 384 MB blob moving
   through rustc/ld.
2. **Peak RSS ≈ 18× blob size during build.** Rustc appears to hold several
   copies of the constant in memory. Budget accordingly for embedded blobs.
3. **Dead-strip is a measured feature, not speculation.** A sibling binary
   in the same crate that doesn't touch the const links at 422 KB vs the
   consumer-of-const binary's 387 MB. Applies to the default macOS linker.
4. **Strategy C's 609 ms "first-run" over localhost is the wall-clock floor.**
   Bandwidth math for 100 Mbps residential pushes that to ~11 s for 131 MiB /
   ~32 s for 384 MiB. Loopback benches are not predictive of real UX.
5. **`ureq` default-features-off costs 39 transitive deps, ~15 s cold.**
   Adding TLS roughly doubles that. Consider `reqwest` vs `ureq` for the
   real library only if the dep count matters more than the compile-time
   difference.

## Convergence with other spikes

- **With S3**: S3 produced the 131 MiB vminitd ELF and the 384 MiB
  init.block. S8 picks between them — vminitd only.
- **With S5**: S5 proved a Rust EXT4 writer is tractable. That writer is
  what synthesizes init.block on-host from the vminitd ELF, closing the
  loop: we ship one artifact (ELF), we own one EXT4 writer (the S5 port),
  they compose.
- **With D-003 / D-004 / D-017**: the bundling decision, the ext4-crate
  single-source-of-truth decision, and the "fetch pinned ELF on first
  build, don't check it in" refinement all trace back to numbers on this
  page.

## Acceptance (from task)

- [x] Decision table with real numbers.
- [x] All three strategies implemented to the point of producing
      measurements.
- [x] `STATUS.md` reflects recommendation + `build.rs` template.
- [x] `JOURNAL.md` has raw `/usr/bin/time` output.
- [x] `FINDINGS.md` (this file) narrates the decision.

## Handoff

- No new blockers for S1–S7 or S9.
- Real library setup (Phase 1) uses Strategy A with the 131 MiB vminitd
  ELF, embedded in `firkin-vminitd-bytes` (isolated leaf so the 131 MiB
  link tax stays off day-to-day edits in other crates — PRO_TIPS §30).
- The ELF itself is fetched via pinned download per [D-017](../../DECISIONS.md#d-017--vminitd-elf-distributed-via-pinned-download-not-checked-in);
  S8 validated the embedding strategy, not the distribution path.
- HTTP server used for bench has been killed; cache at
  `~/Library/Caches/s8-bundling-bench/` may be deleted.
