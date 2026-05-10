# Error model

> Covers: per-crate `thiserror` capability enums, leaf error types, `terrors::OneOf` internal plumbing, classification helpers, cross-crate flow, error-design discipline.
>
> Source philosophy: [`error_design.md`](../../../../../../src/personal/beads-rs/docs/philosophy/error_design.md) — "the other half of the map." Applied per [D-007](../DECISIONS.md#d-007--beads-rs-philosophy-for-rust-style).

---

## 1. Overview — the three-level model

Each of the four public crates (`core`, `vmm`, `oci`, `ext4`) has three layers of error representation:

1. **Leaf error types** — small `thiserror` structs / enums specific to a single failure mode. Local to a module; usually `pub(crate)`. Flow up through `#[source]` chains.
2. **Capability error enums** — one per crate, `thiserror` + `#[non_exhaustive]`. Domain-named variants. This is what users import and match on.
3. **Internal error sets** via `terrors::OneOf<(…)>` — per-function precision inside a crate. **Never escape the crate boundary.** Collapse at the capability trait / public fn boundary.

Two cross-cutting rules govern the design:

- **Variants are behaviors, fields are details.** If two failures require different caller handling, they are different variants. If two failures require the same handling but carry different context, they are one variant with structured fields.
- **No crate names in variant names.** `Reqwest(reqwest::Error)` is banned. Chain the underlying crate error via `#[source]`; name the variant by the *operation* that was trying to succeed.

---

## 2. `core::Error` — user-facing capability enum

The top-level error type 90% of users match on.

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Configuration rejected before any resource was acquired. Not retryable; fix inputs.
    #[error("invalid container configuration")]
    Config(#[from] ConfigError),

    /// VM could not be booted.
    #[error("VM failed to boot")]
    VmBoot(#[source] vmm::Error),

    /// Host cannot reach the in-guest agent (vminitd) via vsock.
    #[error("guest agent unreachable on vsock port {port}")]
    GuestAgentUnreachable {
        port: VsockPort,
        #[source] source: vmm::Error,
    },

    /// Guest agent accepted the connection but refused the specific operation.
    #[error("guest agent refused `{op}`")]
    GuestAgent {
        op: &'static str,
        #[source] source: GuestAgentError,
    },

    /// OCI image pull failed.
    #[error("could not pull image `{reference}`")]
    ImagePull {
        reference: String,
        #[source] source: oci::Error,
    },

    /// Rootfs assembly (OCI layers -> ext4) failed.
    #[error("rootfs assembly failed")]
    RootfsAssembly(#[source] ext4::Error),

    /// Container init or exec'd process operation failed.
    #[error("container process operation `{op}` failed")]
    Process {
        op: &'static str,
        #[source] source: ProcessError,
    },

    /// State-gated operation called on a container that isn't alive.
    #[error("container `{container}` is not running")]
    NotRunning { container: ContainerId },

    /// Operation on a container that already exited.
    #[error("container `{container}` exited with {status:?}")]
    ContainerExited { container: ContainerId, status: ExitStatus },

    /// User asked for a library-reserved vsock port.
    #[error("vsock port {port} is reserved: {reason}")]
    ReservedPort { port: VsockPort, reason: &'static str },

    /// File copy (copy_in / copy_out) failed mid-stream.
    #[error("{direction} `{path:?}` failed")]
    FileTransfer {
        direction: CopyDirection,
        path: PathBuf,
        #[source] source: std::io::Error,
    },

    /// Operation was cancelled via cascading lifecycle shutdown (VM or Container stop).
    /// See 09-cross-cutting.md § cancellation.
    #[error("operation cancelled because {reason:?}")]
    Cancelled { reason: CancelReason },

    /// An invariant the library maintains was violated. Rare; if you see this in
    /// production, it's a bug in this library, not a world state.
    #[error("internal error: {0}")]
    Internal(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyDirection { Into, Out }
```

### 2.1 Classification helpers on `core::Error`

```rust
impl Error {
    /// Transient errors are worth a retry with backoff.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Error::VmBoot(e) if e.is_transient())
            || matches!(self, Error::GuestAgentUnreachable { .. })
            || matches!(self, Error::ImagePull { source, .. } if source.is_transient())
            || matches!(self, Error::FileTransfer { source, .. }
                if matches!(source.kind(), std::io::ErrorKind::Interrupted | std::io::ErrorKind::TimedOut))
    }

    /// Config errors mean "fix inputs." Never retry blindly.
    pub fn is_config(&self) -> bool {
        matches!(self, Error::Config(_) | Error::ReservedPort { .. })
    }

    /// The specific subset of auth-related pull failures, via oci::Error::is_auth().
    pub fn is_auth(&self) -> bool {
        matches!(self, Error::ImagePull { source, .. } if source.is_auth())
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::ImagePull { source, .. } if source.is_not_found())
    }
}
```

---

## 3. `vmm::Error` — VZ-backed primitives

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// VZ configuration rejected before boot.
    #[error("VM configuration is invalid: {reason}")]
    InvalidConfig { reason: String },

    /// VM failed to boot.
    #[error("VM failed to boot: {reason}")]
    BootFailed {
        reason: &'static str,
        #[source] source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// VZ `validateWithError:` returned a rejection.
    #[error("VZ device configuration was rejected: {reason}")]
    InvalidDeviceConfig { reason: String },

    /// Nested virtualization requested but host doesn't support it.
    #[error("nested virtualization is not supported on this host")]
    NestedVirtNotSupported,

    /// Operation attempted while the VM is paused.
    #[error("VM is paused; resume before calling `{op}`")]
    VmPaused { op: &'static str },

    /// Binary does not have the virtualization entitlement.
    #[error("missing entitlement `{entitlement}` — rebuild and resign")]
    MissingEntitlement { entitlement: &'static str },

    /// Binary is not codesigned or signature is invalid.
    #[error("binary is not codesigned or signature is invalid")]
    CodeSignInvalid,

    /// Reserved vsock port used by caller.
    #[error("vsock port {port} is reserved for library use: {reason}")]
    ReservedPort { port: VsockPort, reason: &'static str },

    /// Vsock dial failure.
    #[error("failed to dial vsock port {port}")]
    VsockDial { port: VsockPort, #[source] source: std::io::Error },

    /// Vsock listen failure.
    #[error("failed to listen on vsock port {port}")]
    VsockListen { port: VsockPort, #[source] source: std::io::Error },

    /// VM-stop path failed.
    #[error("VM failed to stop cleanly")]
    StopFailed(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// Snapshot / restore failed.
    #[cfg(feature = "snapshot")]
    #[error("snapshot operation `{op}` failed")]
    Snapshot {
        op: &'static str,
        #[source] source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// VZ returned an error code we haven't characterized. Tombstone variant per §9
    /// ("No `Other(String)`") — each occurrence is a TODO to name the real behavior
    /// and split a typed variant off. Do not classify from this variant; treat it
    /// as a bug report trigger.
    #[error("unclassified VZ error {code}: {message}")]
    UnclassifiedVZ { code: i32, message: String },
}
```

### 3.1 Classification on `vmm::Error`

```rust
impl Error {
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Error::BootFailed { .. }
                | Error::VsockDial { .. }
                | Error::VsockListen { .. }
                | Error::StopFailed(_)
        )
    }

    pub fn is_config(&self) -> bool {
        matches!(
            self,
            Error::InvalidConfig { .. }
                | Error::InvalidDeviceConfig { .. }
                | Error::MissingEntitlement { .. }
                | Error::CodeSignInvalid
                | Error::ReservedPort { .. }
                | Error::NestedVirtNotSupported
        )
    }
}
```

---

## 4. `oci::Error` — registry and image operations

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("invalid image reference `{raw}`: {reason}")]
    InvalidReference { raw: String, reason: &'static str },

    #[error("registry at `{registry}` requires auth for `{reference}`")]
    Unauthorized { registry: String, reference: Reference },

    #[error("registry denied access to `{reference}`")]
    Forbidden { reference: Reference },

    #[error("image `{reference}` not found")]
    NotFound { reference: Reference },

    #[error("registry at `{registry}` is unreachable")]
    Transport {
        registry: String,
        #[source] source: std::io::Error,
    },

    #[error("manifest for `{reference}` is malformed")]
    BadManifest {
        reference: Reference,
        #[source] source: serde_json::Error,
    },

    #[error("unsupported media type `{media_type}`")]
    UnsupportedMediaType { media_type: String },

    /// No matching platform manifest in the list (e.g. arm64 client, amd64-only image).
    #[error("no manifest in `{reference}` matches platform {target}; available: {available:?}")]
    NoMatchingManifest {
        reference: Reference,
        target: Platform,
        available: Vec<Platform>,
    },

    #[error("digest mismatch for `{reference}`: expected {expected}, got {actual}")]
    DigestMismatch {
        reference: Reference,
        expected: String,
        actual: String,
    },

    #[error("image I/O error")]
    Io(#[source] std::io::Error),
}

impl Error {
    pub fn is_transient(&self) -> bool {
        matches!(self, Error::Transport { .. } | Error::Io(_))
    }

    pub fn is_auth(&self) -> bool {
        matches!(self, Error::Unauthorized { .. } | Error::Forbidden { .. })
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::NotFound { .. } | Error::NoMatchingManifest { .. })
    }
}
```

---

## 5. `ext4::Error` — EXT4 writer

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("I/O error writing ext4 image")]
    Io(#[source] std::io::Error),

    #[error("disk full while writing block {block:?}")]
    DiskFull { block: BlockNumber },

    #[error("OCI layer `{layer}` contains an unsupported entry: {reason}")]
    InvalidLayer { layer: String, reason: &'static str },

    #[error("filesystem feature `{feature}` is not supported by this writer")]
    UnsupportedFeature { feature: &'static str },

    #[error("image size {requested:?} is too small for content; need at least {needed:?}")]
    ImageTooSmall { requested: Size, needed: Size },

    #[error("whiteout `{path:?}` has no matching lower entry")]
    OrphanWhiteout { path: PathBuf },

    #[error("xattr `{name}` value exceeds inline capacity and block-store is disabled")]
    XattrTooLarge { name: String },

    #[error("directory `{path:?}` would exceed hardlink limit without DIR_NLINK feature")]
    DirNLinkRequired { path: PathBuf },
}
```

No classifier helpers on `ext4::Error` — every variant requires a config or input fix, not a retry. `is_config()` would be "always true" which is useless; better to omit.

---

## 6. Leaf errors — examples

Leaf errors live inside each crate, usually `pub(crate)` unless flowed through a public capability variant via `#[source]`.

### 6.1 `ConfigError` (in `core`)

```rust
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("container id `{raw}` is invalid: {reason}")]
    InvalidContainerId { raw: String, reason: &'static str },

    #[error("rootfs file does not exist at `{path:?}`")]
    RootfsMissing { path: PathBuf },

    #[error("rootfs at `{path:?}` is not an ext4 image (magic mismatch at offset {offset:#x})")]
    RootfsNotExt4 { path: PathBuf, offset: u64 },

    /// D-019: rootfs path supplied to vm.container(...).rootfs(...) was not
    /// pre-declared via VmConfig::builder().block_device(path) at VM-boot time.
    /// Applies only on `OnVm` / `OnVmArc` builders; ImplicitVm path does not
    /// require pre-declaration.
    #[error("rootfs `{path:?}` is not pre-declared on this VM's block_device list")]
    RootfsNotPreDeclared { path: PathBuf },

    /// D-019: OCI bundle rootfs is not supported on OnVm builders in v0.1.
    /// Pre-assemble with `ext4::Writer::write_oci_layers` and pre-declare
    /// the resulting path via `VmConfig::builder().block_device(path)`.
    #[error("Rootfs::OciBundle is not supported on OnVm / OnVmArc builders in v0.1; pre-assemble to an ext4 path and pre-declare it")]
    OciBundleOnMultiContainerVm,

    #[error("container has no command and no image_config provides one")]
    NoCommand,

    #[error("writable_layer must be a block device (got {kind})")]
    InvalidWritableLayer { kind: &'static str },

    #[error("memory {size:?} is below VZ minimum {min:?}")]
    MemoryTooSmall { size: Size, min: Size },

    #[error("terminal mode set but process.stdin / stdout override mismatch")]
    TerminalStdioMismatch,

    #[error("invalid seccomp policy: {reason}")]
    InvalidSeccomp { reason: String },
}
```

### 6.2 `GuestAgentError` (in `core`)

```rust
#[derive(Debug, thiserror::Error)]
pub enum GuestAgentError {
    #[error("vminitd gRPC transport error")]
    Transport(#[source] tonic::transport::Error),

    #[error("vminitd returned status {code:?} for `{op}`: {message}")]
    Status { op: String, code: tonic::Code, message: String },

    #[error("vminitd deadlined during `{op}` after {after:?}")]
    Timeout { op: String, after: std::time::Duration },

    #[error("vminitd response was malformed: {reason}")]
    MalformedResponse { reason: String },
}
```

### 6.3 `ProcessError` (in `core`)

```rust
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("process `{pid}` could not be signalled")]
    SignalFailed { pid: i32, #[source] source: GuestAgentError },

    #[error("process `{id}` has not yet started")]
    NotStarted { id: ProcessId },

    #[error("process `{id}` already started")]
    AlreadyStarted { id: ProcessId },

    #[error("wait timeout for process `{id}` after {after:?}")]
    WaitTimeout { id: ProcessId, after: std::time::Duration },
}
```

---

## 7. Internal error sets — `terrors::OneOf`

Inside a crate, a function may have a narrow set of failure modes. Naming an enum to say "one of these two leaves" is noise (`scatter.md § noise`). `terrors::OneOf` gives per-function precision:

```rust
use terrors::OneOf;

// Inside the core crate's orchestrator:
type PullAndAssembleErr = OneOf<(oci::Error, ext4::Error, ConfigError)>;

async fn pull_and_assemble(
    this: &Client,
    reference: &Reference,
) -> Result<PathBuf, PullAndAssembleErr> {
    // ?-propagation from oci::Client::pull, ext4::Writer::*, and our config checks.
    let bundle = this.oci.pull(reference).await?;     // oci::Error flows as ..A(e)
    let size = compute_needed_size(&bundle)?;         // ConfigError flows as ..C(e)
    let path = build_rootfs(&bundle, size)?;          // ext4::Error flows as ..B(e)
    Ok(path)
}
```

At the capability boundary, collapse back to `core::Error`:

```rust
impl Container {
    pub async fn spawn(self) -> Result<Container, core::Error> {
        match pull_and_assemble(&self.client, &self.reference).await {
            Ok(path) => { /* continue */ }
            Err(e) => return match e.as_enum() {
                E3::A(oci_err)  => Err(core::Error::ImagePull {
                    reference: self.reference.to_string(),
                    source: oci_err,
                }),
                E3::B(ext4_err) => Err(core::Error::RootfsAssembly(ext4_err)),
                E3::C(cfg_err)  => Err(core::Error::Config(ConfigError::from(cfg_err))),
            },
        }
        // ...
    }
}
```

### 7.1 When to reach for `OneOf` vs a named enum

| Situation | Use |
|---|---|
| 2–3 leaf types with no strong shared meaning | `OneOf` — it saves inventing a name that doesn't mean anything |
| 4+ variants | Named `enum` — the act of naming the type helps the reader |
| Variants have real behavioral meaning worth a name | Named `enum` |
| The set of variants will grow over time | Named `enum` with `#[non_exhaustive]` |
| The function is internal plumbing | `OneOf` is fine |
| The function is at a crate boundary | Never `OneOf` — collapse to the capability enum |

### 7.2 `OneOf` is never public

```rust
// Never:
pub async fn do_thing(&self) -> Result<T, OneOf<(A, B, C)>>;   // ← forbidden in public API

// Always at crate boundary:
pub async fn do_thing(&self) -> Result<T, core::Error>;         // ← OK
```

`OneOf` is plumbing. It's great for internal composition; it's terrible for users to match on.

---

## 8. Cross-crate error flow

Every capability variant in `core::Error` that wraps a lower crate's error uses `#[source]`, not `#[from]`, **when the *context* adds information** the lower error doesn't have. `#[from]` is reserved for zero-context conversions.

### 8.1 `#[source]` vs `#[from]` — the rule

- **`#[from]`**: automatically converts a lower error into this variant. Use when the upper variant carries no additional context beyond "this lower error occurred."
  ```rust
  #[error("invalid container configuration")]
  Config(#[from] ConfigError),     // no context to add; ConfigError IS the story
  ```

- **`#[source]`**: explicitly stores the lower error as the source; requires a hand-written match to map into the variant, because the variant carries context fields the mapper must fill in.
  ```rust
  #[error("could not pull image `{reference}`")]
  ImagePull {
      reference: String,             // ← context only the caller knows
      #[source] source: oci::Error,  // ← chain to the lower error
  },
  ```

### 8.2 Why we avoid `#[from]` liberally

`#[from]` lets `?` propagate implicitly, which is convenient, but it loses the opportunity to add context at the boundary. For `ImagePull` above, the variant needs to carry *which image* was being pulled — the caller knows that; the lower `oci::Error` doesn't. Using `#[from]` would drop that information.

This is the `scatter.md § lies` failure mode applied to errors: without the context field, the user has to reconstruct "which pull was this?" from call-site knowledge. Adding context at the boundary is the honest move.

### 8.3 The error chain

`std::error::Error::source()` walks the chain:

```
core::Error::ImagePull { reference: "busybox:latest", source: ... }
  → oci::Error::Transport { registry: "docker.io", source: ... }
    → std::io::Error (connection refused)
```

Rendering for logs / debugging uses a chain-walking format, which most ecosystems already provide (`anyhow::Chain`, `eyre::Report`, etc.). We don't prescribe a specific rendering — users render at their boundary.

---

## 9. What we don't do

- **No god-`Error`.** No single `Error` enum spanning all crates. `vmm::Error` doesn't wrap `oci::Error`; they're peer enums in peer crates. `core::Error` wraps both because `core` is the orchestrator.
- **No `anyhow::Error` in capability APIs.** `anyhow` is fine at the CLI top level ("log and exit"); not in library surfaces.
- **No crate-named variants.** `Reqwest(reqwest::Error)` / `Tonic(tonic::Status)` / `Serde(serde_json::Error)` are banned. Chain the underlying via `#[source]`; name the variant for the operation that was trying to succeed.
- **No `Other(String)`.** If something feels like it needs `Other`, it needs a named variant. Exception: literal tombstones for once-in-a-blue-moon VZ error codes we haven't characterized — name them `UnclassifiedVZ { code: i32, message: String }` and treat each occurrence as a TODO.
- **No logging from deep internals.** Crates emit `tracing::span!` + `tracing::event!` at event-worthy points, but *policy* (ERROR vs WARN vs DEBUG, stdout vs file) is the user's at their boundary. No `eprintln!`, no `log::error!` inside capability paths.
- **No panics for world-caused failures.** Missing file, network down, registry auth expired → proper error variants. Panics are reserved for violated internal invariants (bugs we want to crash on).

---

## 10. When you add a new `Result<T, Error>`

Checklist from `error_design.md § 8.1`, applied here:

1. Which capability is this part of? Is there an enum already, or does a new crate need one?
2. Who is the first real caller of this error? What decision do they need to make?
3. Does each distinct behavior have a variant? Or are you leaking a decision the caller has to reconstruct?
4. Can any precondition move into a type instead of an error? (E.g., `ContainerId` validation lives in the newtype constructor, not in every call that takes an id.)
5. Is a dependency detail leaking via the variant name? (E.g., `SerdeError(serde_json::Error)` → refactor to `BadManifest { reference, #[source] source }`.)
6. Does this variant need classification helpers (`is_transient`, `is_auth`, etc.) because some downstream site has to decide policy?

---

## 11. Example — the full flow for "spawn failed because the image wasn't found"

```rust
// User writes:
let result = Container::builder("foo")
    .rootfs(Rootfs::OciBundle(bundle))   // bundle pulled from a nonexistent image
    .spawn().await;
```

The internal chain:

1. `core::pull_and_assemble` returns `OneOf<(oci::Error, ext4::Error, ConfigError)>` = `..A(oci::Error::NotFound { reference })`.
2. `Container::spawn` collapses: `Err(core::Error::ImagePull { reference, source: oci::Error::NotFound { reference } })`.
3. User matches:

```rust
match result {
    Err(core::Error::ImagePull { reference, source }) if source.is_not_found() => {
        eprintln!("no such image: {reference}");
    }
    Err(core::Error::ImagePull { .. }) => { /* other pull errors */ }
    // ...
}
```

or with a chain walk:

```rust
if let Err(e) = result {
    let mut source: Option<&dyn std::error::Error> = Some(&e);
    while let Some(s) = source {
        eprintln!("caused by: {s}");
        source = s.source();
    }
}
// Prints:
// caused by: could not pull image `busybox:latest`
// caused by: image `busybox:latest` not found
```

Both idioms work; neither is privileged by the library.

---

## 12. Invariants worth locking

1. Four capability enums, one per public crate, all `thiserror` + `#[non_exhaustive]`.
2. Variants are behaviors, fields are details.
3. No crate names in variant names.
4. `#[source]` where context-adding; `#[from]` only for zero-context conversions.
5. `terrors::OneOf` internal-only; collapsed at capability boundary.
6. Classification helpers (`is_transient`, `is_config`, `is_auth`, `is_not_found`) only where they drive real policy decisions.
7. No `anyhow` in capability APIs. No god-Error. No `Other(String)`.
8. Panics for bugs. Errors for world states. Never the reverse.
9. Users render error chains at their boundary; library doesn't prescribe a format.

Proceed to [`06-ext4-crate.md`](./06-ext4-crate.md) for the EXT4 writer surface.
