# Spike runbook — conventions & how to work

Read this **first** if you're about to start a spike. Its companions:

- **`PRO_TIPS.md`** — technical gotchas.
- **`CONCEPTS.md`** — vocabulary (VZ, virtio, vsock, codesigning, etc.).
- **`DECISIONS.md`** — architectural decisions with rationale.

## The contract

Each spike produces **three** things:

1. **Working spike code** at `~/tmp/rust-rewrite-spikes/s<N>-<topic>/` —
   standalone cargo binary. Not committed; lives on disk.
2. **Notes** at `docs/specs/rust_rewrite/spike-logs/s<N>-<topic>/`,
   committed to this repo. Three files minimum:
   - `JOURNAL.md` — chronological events, decisions, things you tried.
   - `STATUS.md` — current state + repro recipe + handoff notes.
   - `FINDINGS.md` — what you learned. Writes most of itself from JOURNAL.
3. **Updates to shared docs** when you discover something a future spike
   needs: add to `PRO_TIPS.md`, update `spike-logs/README.md`'s index,
   update this runbook if conventions drift.

## Filesystem layout

```
~/tmp/rust-rewrite-spikes/             # code, never committed
├── s1-boot/                            # ✅ reference implementation
│   ├── Cargo.toml
│   ├── src/main.rs
│   ├── init/                           # guest-side code (static musl)
│   ├── assets/                         # kernel + initrd + anything binary
│   ├── entitlements.plist
│   └── sign-and-run.sh
├── s2-vsock-tonic/
├── s3-vminitd-build/
└── ...

docs/specs/rust_rewrite/                # this dir, committed
├── 00-notes.md                         # architecture overview
├── 01-ecosystem-verification.md        # dependency audit
├── 02-spike-plan.md                    # the 8 spikes (you came here from)
├── 03-project-layout.md                # what the real library will look like
├── PRO_TIPS.md                         # technical gotchas
├── SPIKE_RUNBOOK.md                    # this file
├── spike-template/                     # copy-pasteable starting point
│   ├── README.md
│   ├── Cargo.toml
│   ├── src/main.rs
│   ├── entitlements.plist
│   ├── sign-and-run.sh
│   └── scaffold.sh                     # materialise a new spike dir
└── spike-logs/
    ├── README.md                       # index
    ├── s1-boot/                        # JOURNAL + STATUS + FINDINGS
    └── s<N>-<topic>/
```

## Starting a new spike

```bash
# From repo root:
docs/specs/rust_rewrite/spike-template/scaffold.sh <N> <topic>
# e.g.
docs/specs/rust_rewrite/spike-template/scaffold.sh 2 vsock-tonic
```

This creates:
- `~/tmp/rust-rewrite-spikes/s2-vsock-tonic/` populated from the template
- `docs/specs/rust_rewrite/spike-logs/s2-vsock-tonic/` with stub JOURNAL/STATUS

You get a known-good "boots a VM" binary as your starting point. Build on it.

## While you work

- **Keep `JOURNAL.md` open and current.** Timestamp new entries. Log
  failed hypotheses too — future-you wants to know what you ruled out.
- **Update `STATUS.md` every time the state changes materially.** What's
  blocking? What's the next concrete thing? Can someone pick this up cold?
- **Don't leave a broken build at end-of-day.** If it doesn't compile,
  either fix it or write down the exact error + your current theory in
  JOURNAL before stopping.
- **If you hit a gotcha, put it in `PRO_TIPS.md`.** The next person
  deserves to not re-hit it.

### Stub-file rule (important)

`scaffold.sh` pre-creates `JOURNAL.md`, `STATUS.md`, and `FINDINGS.md`
stubs in your notes dir. Claude Code enforces **Read-before-Edit** — if
you try to `Edit` a file you haven't `Read` first, the tool errors.

- **Read each stub first, then Edit it.** Never `Write` from scratch — you'll
  clobber the stub's structure.
- If you think the harness is blocking `Write` on a specific filename,
  it's probably actually the Read-before-Edit rule biting you from the
  other direction (you `Write`-overwrote an unread file, then tried to
  `Edit` it later). Read the file first.

## Done looks like

Copy-paste this checklist into `STATUS.md`:

```
- [ ] Acceptance criteria from 02-spike-plan.md met (quote them, mark each)
- [ ] `sign-and-run.sh` exits 0 from a cold-cloned state
- [ ] Debug and release builds both pass (`cargo build` and `cargo build --release`)
- [ ] JOURNAL.md has a final entry describing the resolution
- [ ] FINDINGS.md written — what worked, what surprised, reusable patterns
- [ ] STATUS.md "State" line reads 🟢 Passed with the day's date
- [ ] spike-logs/README.md index updated
- [ ] Any PRO_TIPS.md additions land
```

## Parallelization rules

Multiple claudes can work spikes concurrently if:

- Each writes **only** to their own `s<N>-<topic>/` directories
  (both in `~/tmp/` and in `spike-logs/`).
- Shared docs (`PRO_TIPS.md`, `spike-logs/README.md`) are updated by
  the curator after merge, not concurrently. If you discover something
  that belongs in PRO_TIPS while working, write it into your spike's
  `FINDINGS.md` and flag it in STATUS — the curator folds it in.
- Asset reuse: kernel (`assets/vmlinux`) is identical across spikes. Copy
  or symlink from `../s1-boot/assets/vmlinux`; don't re-download.

## Which spike starts from what

| Spike | Starting point |
|---|---|
| S1 | ✅ done. Reference for everyone else. |
| S2 | `scaffold.sh 2 vsock-tonic`. Extend with VZVirtioSocketDeviceConfiguration; add guest-side tonic binary. |
| S3 | `scaffold.sh 3 vminitd-build`. Most of the work is Swift toolchain + `make`; the Rust side only boots the resulting `init.block`. |
| S4 | Needs S1+S2+S3 passing. `scaffold.sh 4 e2e` then copy vsock & vminitd pieces in. |
| S5 | **Standalone.** No VM harness needed until validation. `scaffold.sh 5 ext4` then delete `src/main.rs` VZ stuff; pure bytes-and-bits Rust. |
| S6 | Partial — S1 answered the NAT case. Remaining work is vmnet entitlements; `scaffold.sh 6 vmnet-entitlements` when we tackle it. |
| S7 | Needs S4. Extend with `VZLinuxRosettaDirectoryShare`. |
| S8 | **Measurement only.** No Rust spike; `scaffold.sh 8 bundling-bench` and produce a decision table. |

## Git / jj hygiene

- This repo uses `jj` (see global CLAUDE.md). If you `jj git init` hasn't
  been run, you may need to.
- **Don't commit spike code** (`~/tmp/...` isn't in the repo anyway).
- **Do commit spike notes** (`docs/specs/rust_rewrite/spike-logs/...`) —
  they're the durable artifact.
- Don't commit without being asked.

## Docker

OrbStack/docker is assumed available. All cross-compilation of
guest-side Linux artifacts (init, vminitd-alternative, test tonic servers)
goes through docker. See PRO_TIPS.md §7 for recipes.

## Sanity checks before you declare done

```bash
cd ~/tmp/rust-rewrite-spikes/s<N>-<topic>

# Cold build passes
cargo clean && cargo build 2>&1 | tail -20

# Release build passes
cargo build --release 2>&1 | tail -5

# Spike runs end-to-end
./sign-and-run.sh

# Echo the run output into JOURNAL as a final entry:
./sign-and-run.sh 2>&1 | tail -20 | \
  sed 's/^/    /' >> "<path to spike-logs>/JOURNAL.md"
```
