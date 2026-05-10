# spike-template

Copy-pasteable starting point for any new spike in `02-spike-plan.md`.

The template is *exactly* enough code to boot a Linux VM and print
"hello from init". Your spike extends this — adding devices, background
runtimes, whatever the spike's question demands.

## Usage

```bash
# From repo root:
docs/specs/rust_rewrite/spike-template/scaffold.sh <N> <topic>
# e.g.
docs/specs/rust_rewrite/spike-template/scaffold.sh 2 vsock-tonic
```

What `scaffold.sh` does:

1. Creates `~/tmp/rust-rewrite-spikes/s<N>-<topic>/` (code, not committed).
2. Copies the template files in and stamps the spike name into `Cargo.toml`.
3. Stages a kernel in `assets/vmlinux`: symlinks s1-boot's if present, else
   fetches Ubuntu arm64 `linux-image-virtual` via docker.
4. Builds the initrd via docker/alpine (non-fatal if docker's down; rerun).
5. Creates `docs/specs/rust_rewrite/spike-logs/s<N>-<topic>/` with
   **`JOURNAL.md`, `STATUS.md`, and `FINDINGS.md` stubs**. Read each before
   Edit-ing — Claude Code requires Read-before-Edit.

Then:

```bash
cd ~/tmp/rust-rewrite-spikes/s<N>-<topic>
./sign-and-run.sh
# Expect: kernel boot log, "SPIKE: hello from init", exit 0.

# For long-running guests (vminitd, servers), bound with a watchdog:
SPIKE_TIMEOUT_SECS=10 ./sign-and-run.sh
# SIGTERM/SIGKILL at timeout → exit 0 (interpreted as "still running").
# Real non-zero exits before timeout propagate normally.
```

If that passes, the harness is good — start editing `src/main.rs`, following
the `// TODO(spike):` markers.

## What's in here

| File | Purpose |
|---|---|
| `Cargo.toml` | Path deps to the vendored objc2 workspace. Edit to add crates. |
| `src/main.rs` | Boot skeleton. Lifts the clean version of S1's main. Extend for your spike. |
| `entitlements.plist` | `com.apple.security.virtualization`. Don't commit alterations unless S6 changed. |
| `sign-and-run.sh` | build + ad-hoc codesign + run. Supports `SPIKE_TIMEOUT_SECS` watchdog for long-running guests. |
| `init/init.c` | Tiny static-musl init that powers off after printing hello. |
| `init/build.sh` | Docker recipe for building the init + packing a cpio initrd. |
| `scaffold.sh` | The materialiser. Run from repo root. |

## When you should NOT use this template

- **S5 (ext4)**: the spike is pure Rust byte-manipulation. Boot harness
  isn't needed until the validation step. Scaffold anyway, then gut
  `src/main.rs` of VZ code.
- **S8 (bundling numbers)**: pure measurement. Use a tiny placeholder cargo
  project instead of this template.

## Conventions reminder

- Code lives in `~/tmp/rust-rewrite-spikes/`. Not committed.
- Notes live in `docs/specs/rust_rewrite/spike-logs/`. Committed.
- Read `../PRO_TIPS.md` before touching threads, `define_class!`, or
  codesigning. Read `../SPIKE_RUNBOOK.md` for the full dev loop.
