# S7 — findings

Result: **🟢 FULL acceptance.** amd64 `/bin/uname -m` inside an arm64 VZ
VM returns `x86_64\n` to the host's stdio. ~60 LOC of net additions to
S4's harness (Rosetta directory share on the host; `Mount(virtiofs)` +
`SetupEmulator` on the guest).

## What worked exactly as expected

- **S4 harness lifted verbatim.** `dial_vsock` + `VsockConnector` +
  `StdioListenerDelegate` + the entire `CreateProcess → StartProcess →
  WaitProcess` flow: zero changes. The only edits to S4's `run_tests()`
  were (a) three new RPCs inserted between `Mount(ext4)` and
  `CreateProcess`, (b) args changed `/bin/echo hello` → `/bin/uname -m`,
  (c) expected stdout string updated.
- **`VZLinuxRosettaDirectoryShare` + `VZVirtioFileSystemDeviceConfiguration`**
  glue is a direct port of the Swift reference in
  `Sources/Containerization/VZVirtualMachineInstance.swift`:
  ```rust
  let share = VZLinuxRosettaDirectoryShare::initWithError(...)?;
  let fs   = VZVirtioFileSystemDeviceConfiguration::initWithTag(..., "rosetta");
  fs.setShare(Some(share.as_super()));
  config.setDirectorySharingDevices(&NSArray::from_slice(&[fs.as_super()]));
  ```
  Config validated first try; no mystery entitlement / provisioning
  dance on top of S1's `com.apple.security.virtualization`.
- **`SetupEmulator` RPC** — name: `SetupEmulator`, proto message
  `SetupEmulatorRequest { binary_path, name, type, offset, magic, mask,
  flags }`. Stable across vminitd builds we have access to; `Binfmt.Entry`
  fields map 1:1 to these. The implementation in `Server+GRPC.swift`
  just checks `Binfmt.mounted()` then calls `Binfmt.Entry.register(binaryPath:)`,
  which does a single write to `/proc/sys/fs/binfmt_misc/register`.
  Assuming vminitd has binfmt_misc pre-mounted (it does — see
  `vminitd/Sources/vminitd/AgentCommand.swift`), there are no hidden
  preconditions beyond the `/run/rosetta` virtiofs mount.
- **Magic + mask for amd64 ELF** — lifted verbatim from
  `Sources/ContainerizationOS/Linux/Binfmt.swift::Binfmt.Entry.amd64()`.
  The strings contain `\x…` sequences; binfmt_misc parses them at
  register time, so we pass them through as-is with a single extra
  backslash in the Rust literal (`"\\x7fELF..."`).
- **Flags `"CF"`** — `C` = credentials cleanup; `F` = fix-binary. `F` is
  critical: the kernel opens `/run/rosetta/rosetta` at register time
  and keeps the fd, so the container's mount namespace doesn't need
  `/run/rosetta` visible. That means the OCI spec's `mounts[]` is
  identical to S4's.
- **kata kernel has `CONFIG_BINFMT_MISC=y`** — the register succeeded
  without any guest-side module-loading gymnastics. (Not separately
  verified via `/proc/config.gz`, but the RPC returned success on the
  first attempt.)
- **Watchdog pattern** from S3's `sign-and-run.sh` — no changes.
  `SPIKE_TIMEOUT_SECS=30` is plenty; the container exits ~4s after VM
  start (kernel boot + vminitd init + mount dance + exec).

## Rosetta license flow

On this M4 Max (macOS 26.3), `VZLinuxRosettaDirectoryShare::availability()`
initially returned `.NotInstalled`. A one-off helper binary
(`src/bin/install-rosetta.rs`) that calls
`installRosettaWithCompletionHandler:` succeeded **without showing any
interactive dialog** — availability transitioned to `.Installed` and
subsequent runs happy-path through. Possible explanations:

1. This machine has previously had Rosetta for macOS installed/accepted;
   the Rosetta-for-Linux download uses the same EULA state.
2. Calling programmatically from an entitled binary (`com.apple.security.virtualization`)
   skips the GUI consent.
3. Apple's SLA reviewer has already agreed, so it's a silent download.

Apple's own Swift flow (`VZVirtualMachineInstance.prestart()`) **does**
anticipate a prompt — it calls `installRosetta()` on a background Task and
rethrows if the user declines. So: **the acceptance criterion here is
"we wrote an installer binary and documented the flow"** — we don't have
concrete evidence of the prompt, but we've shown the install path works
programmatically.

**Recommendation for Phase 1**: call `installRosetta…` lazily (first time
the user passes `--rosetta`) and emit a clear error that names the
user-visible system settings panel if the call returns `notInstalled`
afterward. Don't block VM start on the license dance.

## Gotchas (**proposed PRO_TIPS additions** at the bottom)

### 1. Rosetta install returns success without prompt on pre-accepted Macs

Documented above. Not a code bug; just a testing hazard — CI on a
cold-state Mac may hit a GUI prompt and stall forever. Plan to:
- Use the availability check up-front.
- Provide an `install-rosetta` one-shot that can be run from Terminal
  where a prompt would actually surface.
- Don't assume programmatic install always works silently.

### 2. `binary_path` must be inside the virtiofs mount

`Binfmt.Entry.amd64().register(binaryPath: "/run/rosetta/rosetta")` is
the only path that works. The virtiofs share exposes Rosetta's host
bundle inside `/run/rosetta/`; the interpreter is the file named
`rosetta` in that share. The `F` (fix-binary) flag makes binfmt_misc
open it at register time, so later umounts / namespace changes don't
break emulation — but if you typo the path to `register`, you get a
silent "binfmt registered, lookup fails at exec time" failure with no
log. Copy-paste from `Vminitd+Rosetta.swift`.

### 3. Don't forget: the `flags` string in `SetupEmulatorRequest` is the
   **binfmt_misc flags char set**, not anything protobuf-ish

I.e. literal `"CF"` goes straight into the registration line. vminitd
concatenates this verbatim:
`":x86_64:M::\x7fELF...:/run/rosetta/rosetta:CF"`. Getting this wrong
will silently break the `F` semantics and you'll chase phantom "file
not found" errors at container exec.

### 4. The proto is `offset`-as-string, not int

`SetupEmulatorRequest.offset` is a `string`, not an integer. Empty string
means "offset 0". This matches the binfmt_misc registration format
literally (which is `string` in the kernel's parser), but is surprising
against the Swift type's `String` annotation for a numeric concept. Just
pass `""` unless you know what you're doing.

### 5. Availability enum has a fourth "unknown" possibility Swift handles
   but Rust doesn't surface

Swift's `@unknown default` branch (see `VZVirtualMachineInstance.swift`)
catches a future enum value that wasn't compiled in. The Rust enum is
`VZLinuxRosettaAvailability(NSInteger)` — any int is a valid value.
Defensive programming: treat anything outside `{NotSupported,
NotInstalled, Installed}` as `NotSupported` with a log.

## Reusable patterns for the real library

### Guest-side `enable_rosetta()` helper

```rust
async fn enable_rosetta(client: &mut SandboxContextClient<Channel>) -> Result<()> {
    const ROSETTA_PATH: &str = "/run/rosetta";
    client.mkdir(MkdirRequest { path: ROSETTA_PATH.into(), all: true, perms: 0o755 }).await?;
    client.mount(MountRequest {
        r#type: "virtiofs".into(),
        source: "rosetta".into(),
        destination: ROSETTA_PATH.into(),
        options: vec![],
    }).await?;
    client.setup_emulator(SetupEmulatorRequest {
        binary_path: format!("{ROSETTA_PATH}/rosetta"),
        name: "x86_64".into(),
        r#type: "M".into(),
        offset: "".into(),
        magic: AMD64_ELF_MAGIC.into(),
        mask:  AMD64_ELF_MASK.into(),
        flags: "CF".into(),
    }).await?;
    Ok(())
}
```

Matches `Vminitd+Rosetta.swift` line-for-line.

### Host-side Rosetta directory share

```rust
fn attach_rosetta(config: &VZVirtualMachineConfiguration) -> Result<()> {
    use VZLinuxRosettaAvailability::*;
    let a = unsafe { VZLinuxRosettaDirectoryShare::availability() };
    match a {
        NotSupported => bail!("Rosetta for Linux not supported on this host"),
        NotInstalled => bail!("Rosetta for Linux not installed — run install-rosetta"),
        Installed => {}
        _ => bail!("unknown Rosetta availability: {:?}", a),
    }
    let share = unsafe { VZLinuxRosettaDirectoryShare::initWithError(VZLinuxRosettaDirectoryShare::alloc()) }?;
    let fs = unsafe {
        VZVirtioFileSystemDeviceConfiguration::initWithTag(
            VZVirtioFileSystemDeviceConfiguration::alloc(),
            &NSString::from_str("rosetta"),
        )
    };
    unsafe { fs.setShare(Some((&*share).as_super())) };
    unsafe {
        config.setDirectorySharingDevices(
            &NSArray::from_slice(&[(&*fs).as_super() as &VZDirectorySharingDeviceConfiguration]),
        );
    }
    // Leak share/fs to keep them alive past this scope; the real
    // library should hold them as struct fields instead.
    Box::leak(Box::new(share));
    Box::leak(Box::new(fs));
    Ok(())
}
```

### amd64 ELF binfmt constants

```rust
const AMD64_ELF_MAGIC: &str = "\\x7fELF\\x02\\x01\\x01\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x02\\x00\\x3e\\x00";
const AMD64_ELF_MASK:  &str = "\\xff\\xff\\xff\\xff\\xff\\xfe\\xfe\\x00\\xff\\xff\\xff\\xff\\xff\\xff\\xff\\xff\\xfe\\xff\\xff\\xff";
```

These are the exact strings the kernel's binfmt_misc parser expects; no
further escaping needed at the Rust side.

## Diff from S4 in numbers

```
 src/main.rs         | ~60 lines added, 3 changed (args + header + expected)
 src/bin/probe.rs    | +20 (new; Rosetta availability check)
 src/bin/install-rosetta.rs | +80 (new; one-shot install helper)
 assets/rootfs.ext4  | rebuilt amd64 instead of arm64
 Cargo.toml          | name renamed only (no new deps)
```

No new Rust dependencies needed on top of S4's.

## Known loose ends

- **AOT translation cache**. `VZLinuxRosettaCachingOptions` (abstract-
  socket or unix-socket variants) would accelerate repeated amd64
  workloads. Orthogonal to "does it run?" which is what S7 asked.
- **Fresh-machine license prompt**. My Mac evidently has Rosetta's
  EULA pre-accepted. Need to exercise this on a truly cold Mac to see
  the dialog (or not). Not a blocker; the code path is identical
  either way once the user agrees.
- **amd64 dynamic executables / multi-arch layer containers.** We tested
  a fully static amd64 busybox. Real images have dynamic linkers + lots
  of ld.so paths. Expected to Just Work since Rosetta handles ELF
  interpretation at kernel load time, but unverified here.
- **Concurrent containers on Rosetta.** Same virtiofs share, same
  binfmt_misc registration — should be one-time setup that covers all
  subsequent amd64 containers in the VM lifetime. Unverified.

## Time to solve

~50 minutes of focused work:
- ~10 min reading S4's FINDINGS + PRO_TIPS + Rosetta Swift reference.
- ~5 min scaffolding S7 and copying S4 verbatim.
- ~8 min building amd64 rootfs + verifying it's x86-64 ELF via debugfs.
- ~5 min on `install-rosetta` helper (one-shot binary).
- ~10 min wiring up `setDirectorySharingDevices` + Mount(virtiofs) +
  SetupEmulator RPCs.
- ~3 min on first run → full acceptance first try.
- ~10 min on documentation.

Under budget (spec said 1 day / 1–2 hours).

## Proposed PRO_TIPS additions

(For the curator to fold into `PRO_TIPS.md`. Each is in "here's the
trap, here's the fix" shape.)

### Proposed §28 — Rosetta directory share (from S7)

The working sequence for Rosetta cross-arch execution:

1. **Host — availability gate**:
   ```rust
   let a = unsafe { VZLinuxRosettaDirectoryShare::availability() };
   match a {
       VZLinuxRosettaAvailability::NotSupported => bail!("not supported"),
       VZLinuxRosettaAvailability::NotInstalled => bail!("run install-rosetta"),
       VZLinuxRosettaAvailability::Installed => {}
       _ => bail!("unknown availability"),
   }
   ```
2. **Host — one-shot install**:
   ```rust
   let (tx, rx) = oneshot::channel();  // or mutex+condvar
   let block = RcBlock::new(move |err: *mut NSError| {
       tx.send(if err.is_null() { Ok(()) } else { Err(nserror_desc(&*err)) });
   });
   unsafe { VZLinuxRosettaDirectoryShare::installRosettaWithCompletionHandler(&block); }
   // Completion fires on "an arbitrary queue" per docs — a Mutex+Condvar is safer than oneshot
   // because any tokio runtime hop may be absent.
   ```
   May surface a GUI dialog on first call from a Mac that hasn't accepted
   the Rosetta EULA; runs silently otherwise. Don't assume it's always
   non-interactive.
3. **Host — attach share**:
   ```rust
   let share = unsafe { VZLinuxRosettaDirectoryShare::initWithError(...) }?;
   let fs = unsafe { VZVirtioFileSystemDeviceConfiguration::initWithTag(..., "rosetta") };
   unsafe { fs.setShare(Some(share.as_super())) };
   unsafe { config.setDirectorySharingDevices(&NSArray::from_slice(&[fs.as_super()])); }
   ```
4. **Guest — mount + register**:
   ```
   Mount(type=virtiofs, source="rosetta", destination="/run/rosetta")
   SetupEmulator(binary_path="/run/rosetta/rosetta", name="x86_64", type="M",
                 offset="", magic=<amd64 ELF magic>, mask=<amd64 ELF mask>, flags="CF")
   ```
   `magic`/`mask` are the exact strings from
   `Sources/ContainerizationOS/Linux/Binfmt.swift::Binfmt.Entry.amd64()`.
   `flags="CF"` (F = fix-binary) is what keeps the rosetta fd alive through
   the container's mount-namespace switch — so you do NOT need to bind-mount
   `/run/rosetta` into the container rootfs.

Required features on `objc2-virtualization` (all on by default):
`VZDirectoryShare`, `VZDirectorySharingDeviceConfiguration`,
`VZLinuxRosettaDirectoryShare`, `VZVirtioFileSystemDeviceConfiguration`.
