#!/usr/bin/env bash
# Materialise a new spike directory from this template.
#
# Usage:
#   docs/specs/rust_rewrite/spike-template/scaffold.sh <N> <topic>
#
# Creates:
#   ~/tmp/rust-rewrite-spikes/s<N>-<topic>/   (code — not committed)
#   docs/specs/rust_rewrite/spike-logs/s<N>-<topic>/   (notes — committed)
#
# Populates the code dir with the template, seeds JOURNAL.md and STATUS.md.
# Idempotent for the notes dir (won't overwrite). Refuses to overwrite an
# existing code dir — you have to `rm -rf` it yourself if you want a fresh
# one.
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "Usage: $0 <N> <topic>" >&2
    echo "Example: $0 2 vsock-tonic" >&2
    exit 2
fi

N="$1"
TOPIC="$2"
NAME="s${N}-${TOPIC}"

THIS_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$THIS_DIR/../../../.." && pwd)"
SPEC_DIR="$REPO_ROOT/docs/specs/rust_rewrite"
NOTES_DIR="$SPEC_DIR/spike-logs/$NAME"
CODE_DIR="$HOME/tmp/rust-rewrite-spikes/$NAME"

# --- Code dir -------------------------------------------------------------
if [[ -e "$CODE_DIR" ]]; then
    echo "refusing to overwrite existing $CODE_DIR" >&2
    echo "rm -rf $CODE_DIR to start fresh" >&2
    exit 3
fi

mkdir -p "$CODE_DIR/src" "$CODE_DIR/init" "$CODE_DIR/assets"

# Copy template files
cp "$THIS_DIR/Cargo.toml" "$CODE_DIR/Cargo.toml"
sed -i.bak "s/SPIKE_NAME/${NAME}/" "$CODE_DIR/Cargo.toml" && rm "$CODE_DIR/Cargo.toml.bak"

cp "$THIS_DIR/src/main.rs" "$CODE_DIR/src/main.rs"
cp "$THIS_DIR/entitlements.plist" "$CODE_DIR/entitlements.plist"
cp "$THIS_DIR/sign-and-run.sh" "$CODE_DIR/sign-and-run.sh"
chmod +x "$CODE_DIR/sign-and-run.sh"

cp "$THIS_DIR/init/init.c" "$CODE_DIR/init/init.c"
cp "$THIS_DIR/init/build.sh" "$CODE_DIR/init/build.sh"
chmod +x "$CODE_DIR/init/build.sh"

# Kernel: prefer S3's kata kernel (ext4 + vsock + virtio_blk + devtmpfs all
# built in). Fall back to S1's Ubuntu kernel if kata isn't there. Last resort:
# fetch Ubuntu fresh via docker. Either way a usable kernel lands in
# assets/vmlinux before we return.
S3_KERNEL="$HOME/tmp/rust-rewrite-spikes/s3-vminitd-build/assets/vmlinux"
S1_KERNEL="$HOME/tmp/rust-rewrite-spikes/s1-boot/assets/vmlinux"
if [[ -f "$S3_KERNEL" ]]; then
    ln -s "$S3_KERNEL" "$CODE_DIR/assets/vmlinux"
    echo "linked $CODE_DIR/assets/vmlinux -> $S3_KERNEL (kata: ext4+vsock+virtio_blk built in)"
elif [[ -f "$S1_KERNEL" ]]; then
    ln -s "$S1_KERNEL" "$CODE_DIR/assets/vmlinux"
    echo "linked $CODE_DIR/assets/vmlinux -> $S1_KERNEL (Ubuntu; vsock/ext4 as modules — see PRO_TIPS §25)"
else
    echo "no s1-boot kernel; fetching Ubuntu arm64 linux-image-virtual via docker..."
    TMPOUT=$(mktemp -d)
    if docker run --rm --platform linux/arm64 -v "$TMPOUT:/out" ubuntu:24.04 bash -c '
        apt-get update -qq >/dev/null
        apt-get install -y --no-install-recommends linux-image-virtual >/dev/null 2>&1
        cp /boot/vmlinuz-*-generic /out/vmlinuz.gz.raw
    ' 2>&1 | tail -3; then
        gunzip -c "$TMPOUT/vmlinuz.gz.raw" > "$CODE_DIR/assets/vmlinux"
        rm -rf "$TMPOUT"
        echo "wrote $CODE_DIR/assets/vmlinux ($(du -h "$CODE_DIR/assets/vmlinux" | cut -f1))"
    else
        echo "WARNING: kernel fetch failed — assets/vmlinux is missing." >&2
        echo "Set SPIKE_KERNEL=/path/to/vmlinux when running, or re-scaffold with docker available." >&2
    fi
fi

# Build the initrd now so first `./sign-and-run.sh` works out of the box
echo "building initrd (docker; alpine arm64)..."
"$CODE_DIR/init/build.sh" >/dev/null 2>&1 || {
    echo "initrd build failed — see $CODE_DIR/init/build.sh. Not fatal;" \
         "re-run it after docker is up."
}

# --- Notes dir ------------------------------------------------------------
mkdir -p "$NOTES_DIR"

today="$(date +%Y-%m-%d)"

# JOURNAL stub
if [[ ! -f "$NOTES_DIR/JOURNAL.md" ]]; then
    cat > "$NOTES_DIR/JOURNAL.md" <<EOF
# Spike S${N} — ${TOPIC}

**Spike code**: \`~/tmp/rust-rewrite-spikes/${NAME}/\`
**Started**: ${today}

## The question

(Copy the exact question from \`02-spike-plan.md\` S${N}.)

## Acceptance

(Copy the acceptance criteria from \`02-spike-plan.md\` S${N}.)

## Plan

(Your plan goes here. Rough bullets are fine.)

## Current status

Scaffolded. No spike-specific code yet.

## Events

- ${today} — \`scaffold.sh\` run. Harness boots a VM; extending from here.
EOF
fi

# STATUS stub
if [[ ! -f "$NOTES_DIR/STATUS.md" ]]; then
    cat > "$NOTES_DIR/STATUS.md" <<EOF
# S${N} — status snapshot

Last updated: ${today}

## State
- 🟡 In progress — scaffolded from template, harness boots.

## Next action

(One concrete next step.)

## Repro
\`\`\`bash
cd ~/tmp/rust-rewrite-spikes/${NAME}
./sign-and-run.sh
\`\`\`

## Done checklist

- [ ] Acceptance criteria met (quote from 02-spike-plan.md §S${N})
- [ ] \`sign-and-run.sh\` exits 0 cold
- [ ] Debug + release builds clean
- [ ] JOURNAL.md has a final resolution entry
- [ ] FINDINGS.md written (what worked, what surprised)
- [ ] State line above reads "🟢 Passed"
- [ ] spike-logs/README.md index updated
- [ ] Any PRO_TIPS.md additions flagged

## Handoff notes

(What can the next claude / you-tomorrow pick up cold? Anything blocked?)
EOF
fi

# FINDINGS stub — create empty so Edit-based workflows don't need to Write
# a new file. (Observed both S2 and S3 sub-agents reporting harness friction
# when Write-ing a new FINDINGS.md; with a stub in place Edit just works.)
if [[ ! -f "$NOTES_DIR/FINDINGS.md" ]]; then
    cat > "$NOTES_DIR/FINDINGS.md" <<EOF
# S${N} — findings

(Write this up as you go, or at the end. Cover: what worked as planned,
gotchas we hit (flag any PRO_TIPS additions), reusable patterns, known
loose ends, time to solve.)
EOF
fi

echo ""
echo "✅ scaffolded ${NAME}"
echo "  code:  $CODE_DIR"
echo "  notes: $NOTES_DIR/{JOURNAL,STATUS,FINDINGS}.md (stubs)"
echo ""
echo "Next:"
echo "  cd $CODE_DIR"
echo "  ./sign-and-run.sh  # should boot VM and exit 0"
echo ""
echo "Then edit src/main.rs; see '// TODO(spike):' markers."
echo "Read $SPEC_DIR/PRO_TIPS.md before touching threads/codesigning."
echo ""
echo "IMPORTANT — stub-file rule: JOURNAL/STATUS/FINDINGS.md already exist."
echo "Claude Code requires Read before Edit. Read each stub first, then Edit."
echo "Do not Write them from scratch — that'll clobber the stub and lose structure."
