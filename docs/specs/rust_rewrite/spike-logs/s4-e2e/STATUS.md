# S4 — status snapshot

Last updated: 2026-04-20

## State
- 🟢 **Passed — FULL acceptance.** `echo hello` round-trips end-to-end
  through vminitd; host reads `"hello\n"` (6 bytes) via VZVirtioSocketListener;
  vminitd reports process exit code 0. Debug + release builds both green.

## Repro

```bash
# Prereq: S3 passed; apple/containerization/bin/{vmlinux,init.block} exist.

# 1. Stage assets.
cd /Users/darin/vendor/github.com/https:/github.com/apple/containerization
docs/specs/rust_rewrite/spike-template/scaffold.sh 4 e2e
cp bin/init.block ~/tmp/rust-rewrite-spikes/s4-e2e/assets/init.block
ln -sf ~/tmp/rust-rewrite-spikes/s3-vminitd-build/assets/vmlinux \
       ~/tmp/rust-rewrite-spikes/s4-e2e/assets/vmlinux

# 2. Build the container rootfs (64 MiB, busybox-static).
mkdir -p /tmp/s4-rootfs-build
cat > /tmp/s4-rootfs-build/build.sh <<'EOF'
#!/bin/sh
set -eux
apk add --no-cache e2fsprogs util-linux busybox-static
rm -rf /build/rootfs
mkdir -p /build/rootfs/bin /build/rootfs/sbin /build/rootfs/etc \
         /build/rootfs/proc /build/rootfs/sys /build/rootfs/dev \
         /build/rootfs/tmp /build/rootfs/root
cp /bin/busybox.static /build/rootfs/bin/busybox
cd /build/rootfs
for applet in echo sh ls cat sleep true false env printf; do
    ln -sf /bin/busybox bin/$applet
done
echo 'root:x:0:0:root:/root:/bin/sh' > /build/rootfs/etc/passwd
echo 'root:x:0:' > /build/rootfs/etc/group
dd if=/dev/zero of=/out/rootfs.ext4 bs=1M count=64
mkfs.ext4 -F -L rootfs -d /build/rootfs /out/rootfs.ext4
e2fsck -fy /out/rootfs.ext4 || true
EOF
docker run --rm --platform linux/arm64 \
    -v /tmp/s4-rootfs-build:/work \
    -v ~/tmp/rust-rewrite-spikes/s4-e2e/assets:/out \
    alpine:3.20 sh -c 'cd /work && sh build.sh'

# 3. Copy src/main.rs, Cargo.toml, build.rs, proto/ from the spike dir on
# this machine (not in-repo). Main.rs has the listener + RPC sequence.

# 4. Run. Bound to 30s because vminitd never terminates on its own.
cd ~/tmp/rust-rewrite-spikes/s4-e2e
SPIKE_TIMEOUT_SECS=30 ./sign-and-run.sh
SPIKE_TIMEOUT_SECS=30 PROFILE=release ./sign-and-run.sh
```

Expected tail:
```
[ACC/stretch-min] Sync RPC OK in ~10ms
[ACC/stretch-min] Getenv(PATH) = None
[ACC/stretch-min] ContainerStatistics returned 0 containers
[ACC/target] Mkdir(/run/container/container-0/rootfs) OK
[ACC/target] Mount(/dev/vdb -> /run/container/container-0/rootfs) OK
[ACC/target] WriteFile(config.json) OK
[listener] tag=1 accepted connection
[listener] tag=2 accepted connection
[ACC/target] CreateProcess OK
[ACC/target] StartProcess OK; pid=79
[ACC/target] WaitProcess returned exit=0
[ACC/full] stdout (6B) = "hello\n"
[ACC] FULL acceptance met — echo hello round-tripped
```

## RPCs that round-trip

- `Sync` (stretch minimum)
- `Getenv` (stretch minimum)
- `ContainerStatistics` (stretch minimum)
- `Mkdir` (target)
- `Mount` (target — mounts /dev/vdb as ext4 at vminitd's bundle path)
- `WriteFile` (target — writes config.json)
- `CreateProcess` (target — with stdout/stderr vsock ports)
- `StartProcess` (full — pid returned)
- `WaitProcess` (full — exit=0)

Plus host-side `VZVirtioSocketListener` accepts two guest-initiated
vsock connections for stdout + stderr and streams bytes back.

## Acceptance criteria (from 02-spike-plan.md §S4)

- [x] vminitd reports process exit code 0 — **yes** (WaitProcess.exitCode=0).
- [x] Streamed stdout on host contains "hello\n" — **yes** (6 bytes exact).
- [x] `sign-and-run.sh` exits 0 — **yes**, both debug and release.

## Done checklist

- [x] Acceptance criteria met (see above)
- [x] `sign-and-run.sh` exits 0 cold (tested clean build → run)
- [x] Debug + release builds clean
- [x] JOURNAL.md has a final resolution entry
- [x] FINDINGS.md written (what worked, what surprised)
- [x] State line above reads "🟢 Passed"
- [ ] spike-logs/README.md index update — **flagging for curator** (per
      SPIKE_RUNBOOK.md, shared docs are updated by curator).
- [x] PRO_TIPS additions flagged in FINDINGS.md.

## Handoff notes

### For S7 (Rosetta)
- Lift this spike's harness verbatim; add `VZLinuxRosettaDirectoryShare`
  + a bind-mount for `/run/rosetta` into the container spec.
- The stdio-listener pattern here lets you assert on `uname -m` output
  end-to-end.

### For the real library (Phase 1)
- The **5 gotchas** below each map to a documentation/test item in the
  library port. Don't re-derive them.
- The Rust modules with "would survive into the real code": `dial_vsock`,
  `VsockConnector`, `StdioListenerDelegate`. About 180 LOC total.

### Unknowns still open
- **stdin**: we didn't test stdin-from-host-to-guest, but it's the same
  pattern in reverse (guest pulls from `hostCID:stdinPort`; host listens
  and pushes bytes). Worth confirming before declaring the transport
  layer "done".
- **Multiple concurrent containers**: we only ran one. Cgroup paths are
  container-scoped (`/sys/fs/cgroup/container/<id>`), so the pattern
  should extend, but we didn't prove it.
- **Image pull**: we bypassed `oci-client` entirely and built the rootfs
  locally with mkfs.ext4 -d. The real library will pull, unpack, and
  merge layers (that's S5's beat). Our rootfs proves the consumer end
  (mount + exec) works once you have an ext4 somehow.
