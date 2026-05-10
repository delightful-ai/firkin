# `oci` crate

> Covers: public API of the `oci` crate — `Client`, `ClientBuilder`, `Reference`, `ImageBundle` (renamed from `Bundle` per [D-020](../DECISIONS.md#d-020--ocibundle-renamed-to-ociimagebundle)), `Layer`, `Auth`, `Platform`, plus re-exports from `oci-spec`. Multi-arch manifest list handling, gzip/zstd layer support, content-addressable on-disk cache.
>
> Wraps: `oci-client` (registry protocol) + `oci-spec` (typed manifest / config structs).
>
> Depends on: `firkin-types` for shared value types (`Platform`/`Os`/`Arch`/`Size`); `firkin-ext4` for the `LayerCompression` enum used at the ext4 hand-off.

---

## 1. Scope

`oci` is a **facade + glue** over two ecosystem crates:

- **`oci-client`** (formerly `oci-distribution`; now `oras-project/rust-oci-client`) — the registry protocol client. HTTP, auth negotiation, manifest fetching, layer download.
- **`oci-spec`** — typed OCI image-spec JSON (manifests, configs, descriptors).

D-007's "don't reinvent" applies: we don't write our own HTTP-to-registry client or re-type OCI manifests. We *wrap* these crates to:

1. Add multi-arch manifest-list handling with platform selection.
2. Add zstd layer decompression (modern images use it; gzip alone is insufficient).
3. Provide `ImageBundle` (a pulled-image-on-disk abstraction) that implements `ext4::OciLayerSource` (per [D-024](../DECISIONS.md#d-024--ext4ocilayersource-trait-decouples-oci-from-ext4-name-in-method-signatures)) so `ext4::Writer::write_oci_layers(&bundle)` just works. Named `ImageBundle` (not `Bundle`) per [D-020](../DECISIONS.md#d-020--ocibundle-renamed-to-ociimagebundle) to avoid collision with the OCI *runtime* bundle concept.
4. Present our own error enum (`oci::Error`) rather than exposing `oci-client`'s.
5. Make `Auth` integrate with macOS Keychain optionally.

**The surface we own**: `Client`, `ClientBuilder`, `Reference`, `ImageBundle`, `Layer`, `Auth`, `Credentials`, `AuthProvider`, `TlsConfig`, `oci::Error`. **Consumed from `firkin-types`**: `Platform`, `Os`, `Arch`, `Size`. **Re-exports from `oci-spec`**: `Manifest`, `ImageConfig`, `Descriptor`, `MediaType`, `Digest`.

---

## 2. `Reference` — image references

```rust
pub struct Reference { /* private */ }

impl Reference {
    /// Parse a reference. Accepts:
    /// - `docker.io/library/busybox:latest`
    /// - `ghcr.io/foo/bar@sha256:abc...`
    /// - short forms that default registry / namespace / tag per Docker rules:
    ///   - `busybox`        -> `docker.io/library/busybox:latest`
    ///   - `alpine:3.18`    -> `docker.io/library/alpine:3.18`
    ///   - `foo/bar`        -> `docker.io/foo/bar:latest`
    ///   - `reg.io/x/y`     -> `reg.io/x/y:latest`
    ///   - `foo@sha256:abc` -> `docker.io/library/foo@sha256:abc`
    pub fn parse(s: impl AsRef<str>) -> Result<Self, Error>;

    pub fn registry(&self) -> &str;          // "docker.io"
    pub fn namespace(&self) -> &str;         // "library/busybox"
    pub fn name(&self) -> &str;              // "busybox"
    pub fn tag(&self) -> Option<&str>;       // Some("latest")
    pub fn digest(&self) -> Option<&Digest>; // Some(...) if `@sha256:...` present

    pub fn with_tag(self, tag: impl Into<String>) -> Self;
    pub fn with_digest(self, digest: Digest) -> Self;

    /// True iff the reference is pinned to a specific digest (immutable).
    pub fn pinned(&self) -> bool;

    /// Canonical string form; reversible with parse().
    pub fn canonical(&self) -> String;
}

impl std::fmt::Display for Reference { /* canonical */ }
impl std::str::FromStr for Reference { /* parse() */ }
```

**Short-form expansion rules** (documented on `parse`):

1. Zero `/` characters → `docker.io/library/` prefix.
2. One `/` and no `.` or `:` in first segment → `docker.io/` prefix.
3. One `/` with `.` or `:` in first segment → treat first segment as registry.
4. Two or more `/` → first segment is registry.
5. No `:tag` and no `@digest` → append `:latest`.

Matches Docker's own rules exactly.

---

## 3. `Client` — the registry client

```rust
pub struct Client { /* private */ }

impl Client {
    /// Anonymous client, current-platform, default cache directory.
    pub fn default() -> Self;

    /// For auth/TLS/concurrency knobs.
    pub fn builder() -> ClientBuilder;
}

pub struct ClientBuilder { /* private */ }

impl ClientBuilder {
    pub fn auth(self, auth: Auth) -> Self;
    pub fn mirror(self, registry: impl Into<String>, mirror_url: impl Into<String>) -> Self;
    pub fn platform(self, platform: Platform) -> Self;   // default Platform::current()
    pub fn cache_dir(self, dir: impl Into<PathBuf>) -> Self;
    pub fn timeout(self, dur: Duration) -> Self;          // per-request timeout
    pub fn layer_concurrency(self, n: NonZeroUsize) -> Self;  // parallel layer downloads; default NonZeroUsize::new(4).unwrap()
    pub fn tls(self, cfg: TlsConfig) -> Self;             // custom CAs, client certs
    pub fn user_agent(self, ua: impl Into<String>) -> Self;

    pub fn build(self) -> Result<Client, Error>;
}
```

### 3.1 `Client` operations

```rust
impl Client {
    /// Pull an image; returns an ImageBundle on disk. Idempotent: repeated pulls
    /// of the same reference return cached results if content is still valid
    /// (digest match).
    pub async fn pull(&self, reference: &Reference) -> Result<ImageBundle, Error>;

    /// Metadata-only: fetch manifest + config without downloading layers.
    pub async fn inspect(&self, reference: &Reference) -> Result<Image, Error>;

    /// HEAD-style existence check. No body transfer.
    pub async fn exists(&self, reference: &Reference) -> Result<bool, Error>;
}

pub struct Image {
    pub reference: Reference,
    pub manifest: Manifest,
    pub config: ImageConfig,
    pub digest: Digest,
    pub platform: Platform,
}
```

### 3.2 `pull` behavior

1. **Resolve manifest.** GET the manifest at the reference. If it's a **manifest list** (multi-arch), filter by `builder.platform`, pick the matching manifest, follow its digest.
2. **Check cache.** If `$cache/bundles/<image-digest>/` exists, validate digests; if good, return `ImageBundle` pointing at the cached layout.
3. **Download layers.** For each layer descriptor not in `$cache/blobs/sha256/`, HTTP-GET the blob to a content-addressable path. Respect `layer_concurrency` — download N layers in parallel via `tokio::JoinSet`.
4. **Coalesce concurrent pulls.** `flock` on `$cache/bundles/<image-digest>/.lock` ensures two concurrent `pull()` calls for the same image don't download twice.
5. **Materialize bundle.** Write `manifest.json`, `config.json`, `layers.toml` into `$cache/bundles/<image-digest>/`.
6. Return `ImageBundle`.

### 3.3 Cancellation

`pull()` is cancel-safe. Dropping the future:
- Aborts in-flight layer downloads.
- Leaves partially-downloaded blobs in `$cache/blobs/sha256/`; they're content-addressable so a future `pull()` either finds them complete (if they finished before cancellation) or re-downloads.
- Releases the `ImageBundle` flock.

---

## 4. `Auth` — credentials

```rust
pub enum Auth {
    /// Anonymous pull — sufficient for most public images.
    Anonymous,

    /// Static credentials, applied to every registry request.
    Static(Credentials),

    /// Read credentials from a Docker-style config file (e.g., `~/.docker/config.json`).
    /// Supports `credHelpers`, `credsStore`, `auths` sections.
    DockerConfig(PathBuf),

    /// macOS Keychain — look up credentials via Security.framework.
    /// Compile-gated by the `keychain` feature; default-on for macOS targets.
    #[cfg(feature = "keychain")]
    Keychain,

    /// Runtime callback — library invokes this for each registry contact.
    /// Useful for OAuth refresh, workload identity, etc.
    Callback(Arc<dyn AuthProvider>),
}

#[derive(Clone, Debug)]
pub struct Credentials {
    pub username: String,
    pub password: String,         // or token
}

pub trait AuthProvider: Send + Sync {
    /// Return credentials for a given registry, if any. None → try anonymous.
    fn credentials_for(&self, registry: &str) -> Option<Credentials>;
}
```

### 4.1 `AuthProvider` contract

`credentials_for(registry)` is called **synchronously** from the HTTP request path. Implementations that need to fetch credentials asynchronously (OAuth refresh) should pre-cache and refresh in a background task, serving from cache in this sync call.

### 4.2 `keychain` feature

```toml
# Default-on for macOS; off elsewhere:
[features]
default = ["keychain"]
keychain = ["dep:security-framework"]

[target.'cfg(not(target_os = "macos"))'.dependencies]
# Users on non-macOS who want `keychain` must supply their own adapter.
```

On macOS with `keychain` enabled, `Auth::Keychain` looks up per-registry credentials via `security-framework` crate → `Security.framework` → the user's login keychain. Users can pre-populate keychain entries via `docker login` (which writes Keychain entries on macOS by default) or the `security` CLI.

### 4.3 Why keychain as a feature, not a sibling crate

Two options were considered:

| Option | Tradeoff |
|---|---|
| Feature flag (chosen) | Users who don't want `Security.framework` in their build turn it off. Keeps `oci` as a single crate. |
| Sibling `oci-auth-keychain` crate | Even cleaner separation; but introduces a dep users almost always want, doubling the "install" surface. |

Feature-gated inside `oci` won on ergonomics — one-crate-imports-for-the-common-case.

---

## 5. `Platform` and multi-arch manifest lists

Defined in full in [`04-value-types.md § Platform`](./04-value-types.md). Summary:

```rust
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
    pub variant: Option<String>,
}

impl Platform {
    pub fn current() -> Self;         // Linux + host arch (Arm64 or Amd64)
    pub fn linux_amd64() -> Self;
    pub fn linux_arm64() -> Self;
    pub fn linux_arm64_v8() -> Self;
}
```

### 5.1 Manifest-list behavior

When `Client::pull()` encounters a **manifest list** (media type `application/vnd.oci.image.index.v1+json` or `application/vnd.docker.distribution.manifest.list.v2+json`):

1. Parse the list to get per-platform descriptors.
2. Filter by `builder.platform`:
   - Exact match on `(os, arch, variant)` is preferred.
   - Fall back to `(os, arch)` if variant is `None` on either side.
   - No fallback across `os` or `arch`.
3. If exactly one candidate: proceed with its manifest.
4. If no candidate: return `Error::NoMatchingManifest { reference, target, available: Vec<Platform> }`.

The `available` field in the error lets the caller recover gracefully — e.g., on arm64 pulling an amd64-only image, they can detect "amd64 is in available" and opt into Rosetta by pulling again with `.platform(Platform::linux_amd64())` then booting with `VmConfig::rosetta(true)`.

### 5.2 Explicit platform override for Rosetta use

```rust
// Default — picks the current platform:
let client = Client::default();  // on M-series Mac: filters to linux/arm64/v8

// Force amd64 for Rosetta use:
let amd64_client = Client::builder()
    .platform(Platform::linux_amd64())
    .build()?;
let bundle = amd64_client.pull(&reference).await?;

// Later boot with Rosetta enabled:
let vm = VirtualMachine::new(
    VmConfig::builder().rosetta(true).build()?
).boot().await?;
```

---

## 6. Layer compression — gzip + zstd + uncompressed

Media types supported out of the box (audit A.3 commitment):

| Media type | Handler |
|---|---|
| `application/vnd.oci.image.layer.v1.tar` | passthrough |
| `application/vnd.oci.image.layer.v1.tar+gzip` | `flate2` |
| `application/vnd.oci.image.layer.v1.tar+zstd` | `zstd` |
| `application/vnd.docker.image.rootfs.diff.tar.gzip` | `flate2` (Docker-compat) |
| anything else | returns `Error::UnsupportedMediaType { media_type }` at manifest-parse time |

Each `Layer` in an `ImageBundle` has `layer.media_type()` returning the concrete type; decompression is transparent when `ext4::Writer::write_oci_layers` or `bundle.extract_layer(i, dest)` iterates.

Unsupported media types fail fast at manifest-parse time (not mid-pull when we'd have already transferred bytes).

---

## 7. `ImageBundle` — pulled image on disk

Named `ImageBundle` to distinguish from the OCI *runtime* bundle (`config.json` + `rootfs/` consumed by runc). Per [D-020](../DECISIONS.md#d-020--ocibundle-renamed-to-ociimagebundle).

```rust
pub struct ImageBundle { /* private */ }

impl ImageBundle {
    pub fn root(&self) -> &Path;              // $cache/bundles/<digest>/
    pub fn reference(&self) -> &Reference;
    pub fn manifest(&self) -> &Manifest;
    pub fn config(&self) -> &ImageConfig;
    pub fn digest(&self) -> &Digest;          // of the manifest
    pub fn platform(&self) -> &Platform;
    pub fn total_size(&self) -> Size;         // sum of layer compressed sizes

    pub fn layers(&self) -> &[Layer];
    pub fn layer_paths(&self) -> impl Iterator<Item = &Path> + '_;

    /// Extract a single layer into `dest`, handling whiteouts + opaque-dir markers.
    /// Useful for debugging; `ext4::Writer::write_oci_layers(&bundle)` is the
    /// production path.
    pub async fn extract_layer(&self, index: usize, dest: impl AsRef<Path>) -> Result<(), Error>;
}

// D-024: ImageBundle is an ext4 layer source. oci depends on ext4; ext4 stays
// free of oci/oci-spec. `write_oci_layers(&bundle)` dispatches through this impl.
impl ext4::sealed::Sealed for ImageBundle {}
impl ext4::OciLayerSource for ImageBundle {
    fn layers(&self) -> impl Iterator<Item = (&Path, ext4::LayerCompression)> + '_ {
        self.layers().iter().map(|l| (l.path(), l.compression()))
    }
}

pub struct Layer { /* private */ }

impl Layer {
    pub fn path(&self) -> &Path;                // concrete file on disk (compressed form)
    pub fn digest(&self) -> &Digest;            // of the compressed form
    pub fn uncompressed_digest(&self) -> &Digest; // DiffID (from image config)
    pub fn size(&self) -> Size;                 // compressed size
    pub fn media_type(&self) -> &MediaType;

    /// Translation from this layer's MediaType to the compression variant
    /// ext4 understands. Pure function; no side effects.
    pub fn compression(&self) -> ext4::LayerCompression;
}
```

### 7.1 On-disk cache layout

```
$cache/
└── firkin/
    └── oci/
        ├── blobs/
        │   └── sha256/
        │       ├── ab12cd...tar.gz     ← content-addressable, shared across bundles
        │       ├── ef34gh...tar.zst
        │       └── ...
        └── bundles/
            └── <manifest-digest>/
                ├── manifest.json
                ├── config.json
                ├── layers.toml         ← ordered list of blob paths
                └── .lock               ← flock during materialization
```

**Content-addressable sharing**: the same layer across 10 different images is 1 physical file. Pulling a new image that reuses a layer is zero bytes downloaded — we already have it.

### 7.2 Concurrent pulls

Protected by `flock(bundle_dir/.lock)` during materialization. A second `pull()` for the same manifest digest waits for the first to finish, then finds it cached. No double-download, no corruption.

(The "bundle" in `bundle_dir` is the image bundle's on-disk materialization; the flock scope is per-`ImageBundle`.)

For **different** images that share layers, the blob directory uses per-blob `flock`s; parallel pulls that happen to share a layer download it exactly once.

### 7.3 `extract_layer` — debugging escape hatch

```rust
/// Extract layer[i] to dest. Handles:
/// - gzip/zstd decompression based on media type
/// - whiteouts (.wh.file -> delete dest/file, .wh..wh..opq -> opaque dir marker)
/// - standard tar entries (regular files, symlinks, hardlinks, device nodes)
///
/// Useful when you want to inspect a single layer's contents for debugging,
/// or when ext4::Writer::write_oci_layers is the wrong shape for your use case
/// (e.g., you want to diff two layers).
pub async fn extract_layer(&self, index: usize, dest: impl AsRef<Path>) -> Result<(), Error>;
```

Yes, this duplicates functionality users could build with `tar + flate2/zstd`. We keep it because (a) it's a ~50 LOC convenience that correctly handles OCI whiteout semantics, which raw tar doesn't, and (b) it's useful for tests and debugging flows that span the library.

---

## 8. Re-exports from `oci-spec`

Users import OCI types from `oci::*` without knowing `oci-spec` exists:

```rust
pub use oci_spec::image::{
    ImageManifest      as Manifest,
    ImageConfiguration as ImageConfig,
    Descriptor,
    MediaType,
    Digest,
};
```

**Why re-export rather than re-type**: `oci-spec` is already canonical, correct, and well-maintained. Re-typing would mean every OCI spec addition (a new media type, a new manifest field) requires syncing across two crates for no gain. D-007 "don't reinvent" applies.

**Cost**: users who pin our `oci` crate get an implicit pin on `oci-spec`'s version family. We document this in the `oci` crate README and pin it in `workspace.dependencies` for deliberate bumps.

---

## 9. `TlsConfig` — custom certificate handling

```rust
pub struct TlsConfig { /* private */ }

impl TlsConfig {
    pub fn builder() -> TlsConfigBuilder;
}

pub struct TlsConfigBuilder { /* private */ }

impl TlsConfigBuilder {
    pub fn additional_ca(self, pem: &[u8]) -> Self;            // add trusted CA
    pub fn additional_cas(self, pems: impl IntoIterator<Item = Vec<u8>>) -> Self;
    pub fn client_cert(self, cert_pem: &[u8], key_pem: &[u8]) -> Self;
    pub fn danger_accept_invalid(self) -> Self;                // for dev; logs warn
    pub fn build(self) -> Result<TlsConfig, Error>;
}
```

Default: system trust store. No client cert. Valid certs only.

`danger_accept_invalid` is named explicitly to make reviewers nervous. Logs `tracing::warn!` at `ClientBuilder::build` time.

---

## 10. Error surface

Defined in [`05-error-model.md § oci::Error`](./05-error-model.md). Variants summary:

- `InvalidReference { raw, reason }`
- `Unauthorized { registry, reference }`
- `Forbidden { reference }`
- `NotFound { reference }`
- `Transport { registry, source }`
- `BadManifest { reference, source }`
- `UnsupportedMediaType { media_type }`
- `NoMatchingManifest { reference, target, available }`
- `DigestMismatch { reference, expected, actual }`
- `Io(source)`

Classifiers: `is_transient()`, `is_auth()`, `is_not_found()`.

---

## 11. What isn't here — explicit non-goals

- **Image `push`.** Pull-only. `oci-client` supports push; we don't expose it. Future sibling `oci-push` crate if needed; would layer on top of `oci::ImageBundle`.
- **Image building** (Dockerfile / Buildah). Sibling-tool concern.
- **Signature verification** (sigstore / cosign / Notary). Deferred to v2 (audit B.4).
- **Partial-pull / lazy-load formats** (stargz, eStargz). Different runtime model.
- **Manifest v1** (legacy Docker). Skipped — deprecated ≥10 years.
- **Image squashing to flat tar as a separate verb.** Layer assembly into ext4 IS the "squash"; if users want a flat tar, they call `ext4::Writer::write_oci_layers` + a tar-from-ext4 utility they provide.
- **Rate-limit handling** (Docker Hub 100-pull-per-6-hours, etc.). Returns `Error::Transport` with the 429 status in `source`; caller decides retry policy. We don't bake a retry policy in.
- **Offline / pre-pulled mode.** No explicit API; equivalent is set `cache_dir` to a pre-warmed directory and all pulls hit cache.

---

## 12. Worked examples

### 12.1 Simple pull

```rust
use firkin::oci::{Client, Reference};

let client = Client::default();
let reference = Reference::parse("docker.io/library/busybox:latest")?;
let bundle: firkin::oci::ImageBundle = client.pull(&reference).await?;

println!("pulled {} layers, {:?} total", bundle.layers().len(), bundle.total_size());
```

### 12.2 Authenticated pull via Docker config

```rust
let client = Client::builder()
    .auth(Auth::DockerConfig(dirs::home_dir().unwrap().join(".docker/config.json")))
    .build()?;

let bundle = client.pull(&Reference::parse("ghcr.io/my-org/private:v1.2.3")?).await?;
```

### 12.3 Pull amd64 on arm64 for Rosetta use

```rust
let amd64_client = Client::builder()
    .platform(Platform::linux_amd64())
    .build()?;

let bundle = amd64_client.pull(&Reference::parse("tensorflow/tensorflow:latest")?).await?;

// Use the ImplicitVm path — Rosetta is configured explicitly on the builder.
// Rootfs::OciBundle is only valid on ImplicitVm builders (D-023 — compile error on OnVm).
let c = Container::builder("tf")
    .image_config(bundle.config())          // env/cmd/cwd defaults from OCI
    .rootfs(Rootfs::oci_bundle(bundle))     // bundle consumed here
    .rosetta(true)
    .spawn().await?;
```

### 12.4 Callback-based auth (OAuth flow)

```rust
use std::sync::Arc;

struct MyOauthProvider { /* ... */ }

impl AuthProvider for MyOauthProvider {
    fn credentials_for(&self, registry: &str) -> Option<Credentials> {
        if registry == "my-registry.example.com" {
            Some(Credentials {
                username: "oauth2".into(),
                password: self.current_access_token(),   // uses a cached + background-refreshed token
            })
        } else {
            None
        }
    }
}

let client = Client::builder()
    .auth(Auth::Callback(Arc::new(MyOauthProvider::new())))
    .build()?;
```

### 12.5 Inspect without pulling layers

```rust
let client = Client::default();
let image = client.inspect(&Reference::parse("nginx:latest")?).await?;

println!("command: {:?}", image.config.config().as_ref().and_then(|c| c.cmd()));
println!("labels: {:?}", image.config.config().as_ref().and_then(|c| c.labels()));
// image.digest() + image.manifest() available too
```

---

## 13. Invariants worth locking

1. `oci` is a facade over `oci-client` + `oci-spec`, not a reinvention.
2. `Client::pull()` is idempotent, content-addressable, concurrent-safe via file locks.
3. Multi-arch manifest lists auto-select by configured `Platform`; `NoMatchingManifest` on miss.
4. gzip + zstd + uncompressed layer types supported (audit A.3); translated to `ext4::LayerCompression` by `oci::Layer::compression()` on the oci side of the boundary (D-024).
5. `Auth` supports anonymous, static, docker-config, macOS keychain (feature-gated), dynamic callback.
6. `ImageBundle` (not `Bundle` — D-020) owns on-disk layout; blobs content-addressable + shared across image bundles. `ImageBundle` impls `ext4::OciLayerSource` (D-024); `oci` depends on `ext4`, not the reverse. No `.layers_for_ext4()` method — the trait impl replaces it.
7. Re-exports from `oci-spec`: `Manifest`, `ImageConfig`, `Descriptor`, `MediaType`, `Digest`.
8. `Platform` / `Os` / `Arch` / `Size` come from `firkin-types` (D-015).
9. No push, no build, no signature verification in v1.

Proceed to [`08-vmm-crate.md`](./08-vmm-crate.md) for the VZ-backed primitives crate boundary.
