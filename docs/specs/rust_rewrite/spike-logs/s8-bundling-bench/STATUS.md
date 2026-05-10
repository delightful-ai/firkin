# S8 — status snapshot

Last updated: 2026-04-20

## State
- 🟢 **Passed.** Decision table produced with real numbers across three strategies and two blob sizes (131 MiB vminitd ELF + 384 MiB init.block). Raw `/usr/bin/time -l` logs preserved inline in `JOURNAL.md`.

## Recommendation (one-liner)

**Default: Strategy A (embed via `include_bytes!`) with the 131 MiB `vminitd` ELF.**
**Fallback: Strategy C (`--features runtime-download`) for binary-size-sensitive consumers.**
**Strategy B is a build-tooling detail (how CI populates `vendor/`), not a user-visible option.**

See parent-agent final-message summary for the full decision table + reasoning; `JOURNAL.md` for raw timings.

## Repro

```bash
# Prereqs: S3's vminitd + init.block on disk (see s3-vminitd-build/STATUS.md).
mkdir -p ~/tmp/rust-rewrite-spikes/s8-bundling-bench/{a-embed,b-buildrs,c-runtime,served}
# (contents as written in the spike dir; each mini-crate stands alone)

# Serve the blob for B/C:
cp /path/to/apple/containerization/bin/init.block \
   ~/tmp/rust-rewrite-spikes/s8-bundling-bench/served/
python3 -m http.server 8873 --bind 127.0.0.1 \
   --directory ~/tmp/rust-rewrite-spikes/s8-bundling-bench/served &

# Per strategy:
for s in a-embed b-buildrs c-runtime; do
  cd ~/tmp/rust-rewrite-spikes/s8-bundling-bench/$s
  cargo clean && /usr/bin/time -l cargo build --release 2>&1 | tee cold.log
  touch src/lib.rs && /usr/bin/time -l cargo build --release 2>&1 | tee warm-lib.log
  touch src/main.rs && /usr/bin/time -l cargo build --release 2>&1 | tee warm-main.log
done

# For C only — first-run vs cached:
rm -rf ~/Library/Caches/s8-bundling-bench
/usr/bin/time -l ~/tmp/rust-rewrite-spikes/s8-bundling-bench/c-runtime/target/release/s8-runtime-bin
/usr/bin/time -l ~/tmp/rust-rewrite-spikes/s8-bundling-bench/c-runtime/target/release/s8-runtime-bin
```

## Recommended `build.rs` template for the real `core/` crate

```rust
// core/build.rs — Strategy A (embed) with B (build.rs fetch) as the cache
// populator for CI / fresh clones where vendor/ isn't checked in.

use std::env;
use std::fs;
use std::path::PathBuf;

// Pin to a specific vminitd release. Keep in sync with build-tools/build-vminitd/pin.toml.
const VMINITD_REV: &str = "<git-sha>";
const VMINITD_SHA256: &str = "<hex>";
const VMINITD_URL_TMPL: &str = "https://github.com/<org>/<repo>/releases/download/vminitd-{rev}/vminitd-{triple}";

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let triple = env::var("TARGET").unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // 1. Prefer a checked-in vendored ELF for hermetic/CI builds.
    let vendored = manifest_dir
        .join("../../vendor/vminitd")
        .join(&triple)
        .join("vminitd");
    let cached   = out_dir.join("vminitd");

    let source_path = if vendored.exists() {
        vendored
    } else if cached.exists() {
        cached.clone()
    } else {
        // 2. Fetch from pinned GH release into $OUT_DIR. Verify SHA-256.
        let url = VMINITD_URL_TMPL
            .replace("{rev}", VMINITD_REV)
            .replace("{triple}", &triple);
        let bytes = ureq::get(&url).call().expect("fetch vminitd")
            .into_reader();
        // TODO: streaming write + sha256 verify before rename.
        let tmp = out_dir.join("vminitd.part");
        // ... write bytes, verify, rename to `cached` ...
        cached.clone()
    };

    // 3. Emit path for include_bytes! in lib.rs.
    println!("cargo:rustc-env=VMINITD_ELF_PATH={}", source_path.display());
    println!("cargo:rerun-if-changed={}", source_path.display());
    println!("cargo:rerun-if-changed=build.rs");
}
```

```rust
// core/src/vminitd.rs (default features = bundled)
#[cfg(feature = "bundled-vminitd")]
pub const VMINITD_ELF: &[u8] = include_bytes!(env!("VMINITD_ELF_PATH"));

#[cfg(feature = "runtime-download")]
pub async fn ensure_vminitd() -> anyhow::Result<PathBuf> { /* XDG cache fetch */ }
```

Feature flags in `core/Cargo.toml`:
```toml
[features]
default = ["bundled-vminitd"]
bundled-vminitd = []
runtime-download = ["dep:ureq"]
```

## Acceptance (from task)

- [x] `FINDINGS.md`-equivalent decision table with real numbers — in parent-agent final message + JOURNAL.md.
- [x] All three strategies implemented to the point of producing measurements.
- [x] `STATUS.md` reflects recommendation + `build.rs` template (this file).
- [x] `JOURNAL.md` has raw `/usr/bin/time` output.

## Proposed PRO_TIPS additions (for curator)

1. `include_bytes!` cost scales with blob size: ~0.1s/MiB cold, ~0.2s/MiB when enclosing crate touched. Mitigate by shrinking blob + isolating the embed in a leaf crate.
2. Default macOS linker dead-strips unreferenced `include_bytes!` consts from final binaries (rlibs still carry them, affecting `target/` size).
3. Peak RSS during embed build ≈ 18× blob size. Budget ≥8 GB free memory for 384 MiB embeds.
4. `cargo check` is immune to the embed tax — day-to-day dev loop unaffected.
5. ureq default-features-off = 39 deps / ~15s cold; adding TLS roughly doubles that.

## Handoff

- No new blockers for S1-S7.
- Real library setup (Phase 1): use Strategy A with 131 MiB vminitd ELF. `ext4` crate synthesizes init.block on-host. Expose `core::VMINITD_ELF` const + optional `ensure_vminitd()` behind `runtime-download` feature.
- HTTP server used for bench is killed; cache at `~/Library/Caches/s8-bundling-bench/` may be deleted.
