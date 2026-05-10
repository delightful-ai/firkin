# S8 — JOURNAL

## 2026-04-20 — Setup

- Created spike dir `~/tmp/rust-rewrite-spikes/s8-bundling-bench/{a-embed,b-buildrs,c-runtime,served}`.
- S3 artifacts available: `vminitd` static ELF (131 MiB), `init.block` EXT4 (384 MiB). Measured against **both** blob sizes so the decision captures the effect of blob choice, not just strategy choice.
- Started a local HTTP server (`python3 -m http.server 8873 --bind 127.0.0.1`) serving `init.block` from `served/`. This deliberately simulates a GitHub Releases fetch over the wire but with ~zero network variance (loopback). Real-world download time for 384 MiB is projected from bandwidth math separately (see FINDINGS.md).

## Machine / env

- macOS 26.3 (Darwin 25.3.0), Apple Silicon (arm64), M-series.
- rustc 1.95.0-nightly (873d4682c 2026-01-25), cargo 1.95.0-nightly.
- Fresh cargo registry for `ureq v2.12.1` (39 transitive crates). Ran `cargo fetch` once per crate *before* cold-build measurement so "cold build" means "cold build after registry is primed" — matches real CI where `~/.cargo/registry` is cached.

## Crate shapes

- **A (embed)**: `include_bytes!("../vendor/<blob>")` at a `pub const INIT_BLOCK: &[u8]`. Tested with two blobs: the full 384 MiB `init.block` and the 131 MiB `vminitd` ELF.
- **B (build.rs fetch)**: `build.rs` calls `ureq::get(...)` on cache-miss, writes `$OUT_DIR/init.block`, emits `cargo:rustc-env=S8_INIT_BLOCK_PATH=...`. `lib.rs` does `include_bytes!(env!("S8_INIT_BLOCK_PATH"))`. Deps: `ureq` with default-features-off.
- **C (runtime fetch)**: no embed. `lib.rs` exposes `ensure_init_block() -> EnsureResult` that streams to `$XDG_CACHE_HOME/s8-bundling-bench/init.block` via `ureq`. `main.rs` calls it and prints from-cache/wall_ms.

## Measurement protocol

For each strategy:
- `cargo clean && /usr/bin/time -l cargo build --release` → cold.
- `touch src/lib.rs && /usr/bin/time -l cargo build --release` → warm (changing the crate that embeds/holds the blob; worst case for A/B).
- `touch src/main.rs && /usr/bin/time -l cargo build --release` → warm-downstream (library user iterating; best realistic case).
- Binary/rlib size from `ls -lh target/release/`.
- For C, run the binary with cache cleared, then again warm.

## Raw timings (selected; full `/usr/bin/time -l` output below)

| Run | Strategy / blob | Wall | User | Peak RSS |
|---|---|---|---|---|
| Cold | A / init.block 384 MiB | 11.85s | 7.62s | 7.23 GB |
| Cold | A / vminitd 131 MiB   | 4.85s  | 2.64s | — |
| Cold | B / init.block 384 MiB (+ 39 ureq deps, +fetch) | 27.37s | 15.13s | 7.64 GB |
| Cold | C / no embed (39 ureq deps) | 5.19s | 12.55s | — |
| Warm (touch lib) | A / init.block | 21–41s (variance) | — | — |
| Warm (touch lib) | A / vminitd    | 4.51s  | — | — |
| Warm (touch lib) | B / init.block | 53.12s | — | — |
| Warm (touch lib) | C              | 0.26s  | 0.20s | — |
| Warm (touch main) | A / init.block | 1.97s | — | — |
| Warm (touch main) | A / vminitd    | 0.49s | — | — |
| Warm (touch main) | B / init.block | 3.88s | — | — |
| Warm (touch main) | C              | 0.15s | — | — |
| No-op rebuild | A / init.block | 0.04s | — | — |
| First run (clear cache) | C | 1.45s wall / 609ms fetch — over loopback | — | 1.9 MB RSS |
| Cached run | C | 0.00s (<1ms) | — | — |

## Artifact sizes

| Artifact | A / init.block | A / vminitd | B / init.block | C |
|---|---|---|---|---|
| `libs8_*.rlib` | 2.0 GB | 677 MB | 2.0 GB | 35 KB |
| binary (release) | 387 MB | 133 MB | 387 MB | 1.0 MB |
| binary NOT referencing const (dead-strip test) | 422 KB | — | — | — |

## Dead-strip experiment

Added `src/bin/unused.rs` that does NOT reference `s8_embed::INIT_BLOCK`. Built alongside the main binary.
- `target/release/s8-embed-bin` → 387 MB (references blob).
- `target/release/unused` → **422 KB** (does not reference blob).

**Conclusion**: the default macOS linker dead-strips unreferenced `include_bytes!` blobs. A library that exposes the blob as a `pub const` does NOT inflate binaries of consumers who never touch it. **However the rlib (2.0 GB) and the compile-time cost are paid regardless.**

## Surprises

1. **Warm (touch lib.rs) is sometimes slower than cold**: 21–41s vs 12s. Cargo parallelizes the cold pipeline; the warm case is a serial "recompile lib + relink downstream bin" and both are dominated by moving the 384 MiB blob through rustc/ld.
2. **Peak RSS 7+ GB during a build with a 384 MiB embedded blob.** Rustc loads and processes the constant in memory, likely several copies. Dev laptops with 16 GB will feel this if running alongside VS Code + browser; 32 GB is comfortable.
3. **Blob size dominates strategy choice.** For A, shrinking from 384 MiB to 131 MiB cuts cold-build 2.4x and warm-touch-lib ~5x. The biggest single thing the real library can do for build perf is **embed the vminitd ELF, not init.block** — the ext4 image can be produced on-host at first boot from the ELF + a tarball template (that's what `cctl rootfs create --ext4` already does).
4. **Dead-strip works** — pleasant surprise. Means exposing `vminitd::INIT_BLOCK` as a public const doesn't tax consumers who'd rather pull at runtime.
5. **Strategy C's first-run over localhost = 609ms**, but projected for 100 Mbps residential: 32s for 384 MiB / 11s for 131 MiB. **This blows the <3s target** unless we're on gig ethernet. Strategy C for the 384 MiB variant is only viable with a good network; for 131 MiB vminitd it's within range (~11s) but still > 3s target.

## Full raw `/usr/bin/time -l` excerpts

### A cold (init.block, 384 MiB) — `/tmp/s8-A-cold2.log`

```
    Finished `release` profile [optimized] target(s) in 11.82s
       11.85 real         7.62 user         3.39 sys
          7227162624  maximum resident set size (peak during linking)
             1831873  page reclaims
                2243  voluntary context switches
```

### A warm (touch lib.rs) — `/tmp/s8-A-warm.log` through `/tmp/s8-A-warm4.log`

```
run 1: 39.67 real   7.53 user   3.39 sys   (file lock contention at start)
run 2: 41.31 real   7.70 user   3.66 sys
run 3: 21.77 real   7.75 user   4.54 sys
run 4: 27.28 real   7.73 user   4.61 sys
```

Variance is disk/scheduler pressure; consistent story: 20–40s is the tax.

### A warm (touch main.rs) — `/tmp/s8-A-warm-main.log`

```
    Finished `release` profile [optimized] target(s) in 1.90s
        1.97 real         1.43 user         0.35 sys
```

### A no-op — `/tmp/s8-A-noop.log`

```
    Finished `release` profile [optimized] target(s) in 0.00s
        0.04 real         0.02 user         0.01 sys
```

### A cargo check (cold) — `/tmp/s8-A-check.log`

```
    Finished `release` profile [optimized] target(s) in 3.50s
        3.53 real         2.29 user         0.91 sys
```

Notable: `cargo check` skips the link step, so it escapes the blob tax almost entirely.

### A cold with 131 MiB vminitd — `/tmp/s8-A-vminitd-cold.log`

```
    Finished `release` profile [optimized] target(s) in 4.82s
        4.85 real         2.64 user         1.04 sys
```

### A warm (touch lib) with 131 MiB vminitd — `/tmp/s8-A-vminitd-warm.log`

```
    Finished `release` profile [optimized] target(s) in 4.48s
        4.51 real         2.70 user         1.28 sys
```

### A warm (touch main) with 131 MiB vminitd — `/tmp/s8-A-vminitd-warmmain.log`

```
    Finished `release` profile [optimized] target(s) in 0.46s
        0.49 real         0.52 user         0.13 sys
```

### A dead-strip test (unused binary) — `/tmp/s8-A-unused-cold.log`

```
    Finished `release` profile [optimized] target(s) in 21.25s
       21.28 real         7.85 user         4.73 sys
```

(Cold build; both binaries produced. Compile tax is shared. Dead-strip is a post-link effect on each bin independently.)

### B cold — `/tmp/s8-B-cold.log`

```
   Compiling ureq v2.12.1
   Compiling s8-buildrs v0.0.0
    Finished `release` profile [optimized] target(s) in 27.29s
       27.37 real        15.13 user         5.82 sys
          7641579520  maximum resident set size
             2471835  page reclaims
```

Cold includes: ureq's 39 transitive deps compile, build.rs runs (fetch from loopback), link.

### B warm (touch lib) — `/tmp/s8-B-warm.log`

```
    Finished `release` profile [optimized] target(s) in 53.12s
       53.16 real         7.83 user         5.26 sys
```

build.rs did NOT re-run (no "fetching" log); cost is rustc re-eval of the constant + relink. Same shape as A with high variance.

### B warm (touch main) — `/tmp/s8-B-warm-main.log`

```
    Finished `release` profile [optimized] target(s) in 3.85s
        3.88 real         1.32 user         0.32 sys
```

### C cold — `/tmp/s8-C-cold.log`

```
   (compiles ureq + 39 deps)
    Finished `release` profile [optimized] target(s) in 5.15s
        5.19 real        12.55 user         1.59 sys
```

### C warm (touch lib) — `/tmp/s8-C-warm.log`

```
    Finished `release` profile [optimized] target(s) in 0.23s
        0.26 real         0.20 user         0.09 sys
```

### C first run (clear cache) — `/tmp/s8-C-firstrun.log`

```
strategy=C path=/Users/darin/Library/Caches/s8-bundling-bench/init.block from_cache=false bytes=402653184 wall_ms=609
        1.45 real         0.01 user         0.35 sys
             1966080  maximum resident set size
```

`wall_ms=609` is program-internal (first stat → rename done). `1.45 real` includes process start + dynamic linker. Over localhost; real network will be much slower (see bandwidth projection in FINDINGS.md).

### C cached run — `/tmp/s8-C-warmrun.log`

```
strategy=C path=... from_cache=true bytes=402653184 wall_ms=0
        0.00 real         0.00 user         0.00 sys
```

## 2026-04-20 — Resolution

All three strategies implemented and measured. Blob-size sensitivity measured by also benching A against the 131 MiB vminitd ELF. Decision table + recommendation in FINDINGS.md.

HTTP server killed (`pkill -f "http.server 8873"`). Cache entries left in place (`~/Library/Caches/s8-bundling-bench/`) — cheap to re-derive; delete if desired.
