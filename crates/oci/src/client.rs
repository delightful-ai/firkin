//! client — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::auth::Auth;
#[allow(unused_imports)]
use crate::bundle::{
    BundleMetadata, ImageBundle, Layer, LayerMetadata, file_digest, layer_from_descriptor,
    registry_auth, registry_error, registry_reference, split_digest_path,
};
#[allow(unused_imports)]
use crate::cache::{cache_error, default_cache_dir, json_error};
#[allow(unused_imports)]
use crate::descriptor::{
    Digest, MediaType, descriptor_from_index_entry, digest_string, select_manifest,
};
#[allow(unused_imports)]
use crate::error::{Error, Result};
#[allow(unused_imports)]
use crate::image_config::Image;
#[allow(unused_imports)]
use crate::reference::Reference;
#[allow(unused_imports)]
use firkin_types::{Platform, Size};
#[allow(unused_imports)]
use oci_client::Reference as RegistryReference;
#[allow(unused_imports)]
use oci_client::client::{Client as RegistryClient, ClientConfig as RegistryClientConfig};
#[allow(unused_imports)]
use oci_client::manifest::{OciDescriptor, OciImageIndex, OciImageManifest, OciManifest};
#[allow(unused_imports)]
use oci_client::secrets::RegistryAuth;
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::num::NonZeroUsize;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::time::Duration;
use tokio::io::AsyncWriteExt as _;
/// OCI registry client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    auth: Auth,
    pub(crate) platform: Platform,
    cache_dir: PathBuf,
    pub(crate) timeout: Duration,
    layer_concurrency: NonZeroUsize,
    user_agent: String,
}
impl Client {
    /// Start an OCI client builder.
    #[must_use]
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }
    /// Return authentication configuration.
    #[must_use]
    pub const fn auth(&self) -> &Auth {
        &self.auth
    }
    /// Return selected target platform.
    #[must_use]
    pub const fn platform(&self) -> &Platform {
        &self.platform
    }
    /// Return content cache directory.
    #[must_use]
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }
    /// Return per-request timeout.
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }
    /// Return layer download concurrency.
    #[must_use]
    pub const fn layer_concurrency(&self) -> NonZeroUsize {
        self.layer_concurrency
    }
    /// Return user-agent string.
    #[must_use]
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }
    /// Pull an image into the content cache.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the registry request fails, the selected platform
    /// is unavailable, the local cache cannot be written, or pulled content
    /// fails digest verification.
    pub async fn pull(&self, reference: &Reference) -> Result<ImageBundle> {
        if let Some(bundle) = self.load_cached_bundle_by_reference(reference)? {
            return Ok(bundle);
        }
        let registry_reference = registry_reference(reference);
        let registry = self.registry_client();
        let auth = registry_auth(&self.auth);
        let (manifest, digest) = self
            .pull_manifest_for_platform(&registry, &registry_reference, &auth)
            .await?;
        if let Some(bundle) = self.load_cached_bundle(reference, &digest)? {
            return Ok(bundle);
        }
        self.materialize_bundle(
            reference,
            &registry_reference,
            &registry,
            &auth,
            manifest,
            digest,
        )
        .await
    }
    fn registry_client(&self) -> RegistryClient {
        RegistryClient::new(RegistryClientConfig {
            max_concurrent_download: self.layer_concurrency.get(),
            read_timeout: Some(self.timeout),
            connect_timeout: Some(self.timeout),
            ..RegistryClientConfig::default()
        })
    }
    async fn pull_manifest_for_platform(
        &self,
        registry: &RegistryClient,
        reference: &RegistryReference,
        auth: &RegistryAuth,
    ) -> Result<(OciImageManifest, Digest)> {
        let (manifest, digest) = registry
            .pull_manifest(reference, auth)
            .await
            .map_err(|error| registry_error(reference, &error))?;
        match manifest {
            OciManifest::Image(manifest) => Ok((manifest, Digest::new(digest))),
            OciManifest::ImageIndex(index) => {
                self.pull_index_manifest(registry, reference, auth, index)
                    .await
            }
        }
    }
    async fn pull_index_manifest(
        &self,
        registry: &RegistryClient,
        reference: &RegistryReference,
        auth: &RegistryAuth,
        index: OciImageIndex,
    ) -> Result<(OciImageManifest, Digest)> {
        let descriptors = index
            .manifests
            .iter()
            .filter_map(descriptor_from_index_entry)
            .collect::<Vec<_>>();
        let selected = select_manifest(&descriptors, &self.platform)?;
        let selected_reference = reference.clone_with_digest(selected.digest().as_str().to_owned());
        let (manifest, digest) = registry
            .pull_image_manifest(&selected_reference, auth)
            .await
            .map_err(|error| registry_error(&selected_reference, &error))?;
        Ok((manifest, Digest::new(digest)))
    }
    fn load_cached_bundle(
        &self,
        reference: &Reference,
        digest: &Digest,
    ) -> Result<Option<ImageBundle>> {
        let root = self.bundle_dir(digest);
        let metadata_path = root.join("bundle.json");
        if !metadata_path.exists() {
            return Ok(None);
        }
        let bytes =
            fs::read(&metadata_path).map_err(|error| cache_error("read", &metadata_path, error))?;
        let metadata: BundleMetadata = serde_json::from_slice(&bytes)
            .map_err(|error| json_error("parse bundle metadata", error))?;
        self.bundle_from_metadata(root, reference, digest.clone(), metadata)
    }
    pub(crate) fn load_cached_bundle_by_reference(
        &self,
        reference: &Reference,
    ) -> Result<Option<ImageBundle>> {
        let root = self.cache_dir.join("bundles");
        if !root.exists() {
            return Ok(None);
        }
        let expected_reference = reference.canonical();
        for algorithm_entry in
            fs::read_dir(&root).map_err(|error| cache_error("read_dir", &root, error))?
        {
            let algorithm_entry =
                algorithm_entry.map_err(|error| cache_error("read_dir", &root, error))?;
            if !algorithm_entry
                .file_type()
                .map_err(|error| cache_error("file_type", &algorithm_entry.path(), error))?
                .is_dir()
            {
                continue;
            }
            let algorithm_dir = algorithm_entry.path();
            for digest_entry in fs::read_dir(&algorithm_dir)
                .map_err(|error| cache_error("read_dir", &algorithm_dir, error))?
            {
                let digest_entry =
                    digest_entry.map_err(|error| cache_error("read_dir", &algorithm_dir, error))?;
                let bundle_root = digest_entry.path();
                if !digest_entry
                    .file_type()
                    .map_err(|error| cache_error("file_type", &bundle_root, error))?
                    .is_dir()
                {
                    continue;
                }
                let metadata_path = bundle_root.join("bundle.json");
                if !metadata_path.exists() {
                    continue;
                }
                let bytes = fs::read(&metadata_path)
                    .map_err(|error| cache_error("read", &metadata_path, error))?;
                let metadata: BundleMetadata = serde_json::from_slice(&bytes)
                    .map_err(|error| json_error("parse bundle metadata", error))?;
                if metadata.reference != expected_reference || metadata.platform != self.platform {
                    continue;
                }
                let digest = Digest::new(metadata.digest.clone());
                if let Some(bundle) =
                    self.bundle_from_metadata(bundle_root, reference, digest, metadata)?
                {
                    return Ok(Some(bundle));
                }
            }
        }
        Ok(None)
    }
    fn bundle_from_metadata(
        &self,
        root: PathBuf,
        reference: &Reference,
        digest: Digest,
        metadata: BundleMetadata,
    ) -> Result<Option<ImageBundle>> {
        let mut layers = Vec::with_capacity(metadata.layers.len());
        for layer in metadata.layers {
            let path = self.blob_path(&layer.digest);
            if !path.exists() || file_digest(&path)? != layer.digest {
                return Ok(None);
            }
            layers.push(Layer::new(
                path,
                Digest::new(layer.digest),
                Digest::new(layer.uncompressed_digest),
                Size::bytes(layer.size),
                MediaType::new(layer.media_type),
            )?);
        }
        Ok(Some(ImageBundle::new(
            root,
            reference.clone(),
            digest,
            metadata.platform,
            metadata.config,
            layers,
        )))
    }
    async fn materialize_bundle(
        &self,
        reference: &Reference,
        registry_reference: &RegistryReference,
        registry: &RegistryClient,
        auth: &RegistryAuth,
        manifest: OciImageManifest,
        digest: Digest,
    ) -> Result<ImageBundle> {
        let root = self.bundle_dir(&digest);
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|error| cache_error("create_dir_all", &root, error))?;
        let manifest_path = root.join("manifest.json");
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| json_error("serialize manifest", error))?;
        write_atomic(&manifest_path, &manifest_bytes).await?;
        let config_path = self
            .ensure_blob(registry_reference, registry, auth, &manifest.config)
            .await?;
        let config_bytes = tokio::fs::read(&config_path)
            .await
            .map_err(|error| cache_error("read", &config_path, error))?;
        let image: Image = serde_json::from_slice(&config_bytes)
            .map_err(|error| json_error("parse image config", error))?;
        let config = image.config.unwrap_or_default();
        write_atomic(&root.join("config.json"), &config_bytes).await?;
        if image.rootfs.diff_ids.len() != manifest.layers.len() {
            return Err(Error::InvalidImageConfig {
                reason: "rootfs diff_id count does not match manifest layer count",
            });
        }
        let mut layers = Vec::with_capacity(manifest.layers.len());
        for (descriptor, diff_id) in manifest.layers.iter().zip(image.rootfs.diff_ids.iter()) {
            let path = self
                .ensure_blob(registry_reference, registry, auth, descriptor)
                .await?;
            layers.push(layer_from_descriptor(path, descriptor, diff_id)?);
        }
        let metadata = BundleMetadata {
            reference: reference.canonical(),
            digest: digest.as_str().to_owned(),
            platform: self.platform.clone(),
            config: config.clone(),
            layers: layers
                .iter()
                .map(|layer| LayerMetadata {
                    digest: layer.digest().as_str().to_owned(),
                    uncompressed_digest: layer.uncompressed_digest().as_str().to_owned(),
                    size: layer.size().as_bytes(),
                    media_type: layer.media_type().as_str().to_owned(),
                })
                .collect(),
        };
        let metadata_bytes = serde_json::to_vec_pretty(&metadata)
            .map_err(|error| json_error("serialize bundle metadata", error))?;
        write_atomic(&root.join("bundle.json"), &metadata_bytes).await?;
        Ok(ImageBundle::new(
            root,
            reference.clone(),
            digest,
            self.platform.clone(),
            config,
            layers,
        ))
    }
    async fn ensure_blob(
        &self,
        reference: &RegistryReference,
        registry: &RegistryClient,
        auth: &RegistryAuth,
        descriptor: &OciDescriptor,
    ) -> Result<PathBuf> {
        let digest = digest_string(&descriptor.digest)?;
        let path = self.blob_path(&digest);
        if path.exists() && file_digest(&path)? == digest {
            return Ok(path);
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| cache_error("create_dir_all", parent, error))?;
        }
        registry
            .store_auth_if_needed(reference.resolve_registry(), auth)
            .await;
        let tmp = path.with_extension("partial");
        let mut file = tokio::fs::File::create(&tmp)
            .await
            .map_err(|error| cache_error("create", &tmp, error))?;
        registry
            .pull_blob(reference, descriptor, &mut file)
            .await
            .map_err(|error| registry_error(reference, &error))?;
        file.flush()
            .await
            .map_err(|error| cache_error("flush", &tmp, error))?;
        drop(file);
        let actual = file_digest(&tmp)?;
        if actual != digest {
            return Err(Error::DigestMismatch {
                path: tmp,
                expected: digest,
                actual,
            });
        }
        tokio::fs::rename(&tmp, &path)
            .await
            .map_err(|error| cache_error("rename", &path, error))?;
        Ok(path)
    }
    fn blob_path(&self, digest: &str) -> PathBuf {
        let (algorithm, value) = split_digest_path(digest);
        self.cache_dir.join("blobs").join(algorithm).join(value)
    }
    fn bundle_dir(&self, digest: &Digest) -> PathBuf {
        let (algorithm, value) = split_digest_path(digest.as_str());
        self.cache_dir.join("bundles").join(algorithm).join(value)
    }
}
impl Default for Client {
    fn default() -> Self {
        Self::builder()
            .build()
            .expect("default OCI client should be valid")
    }
}
/// Builder for [`Client`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientBuilder {
    auth: Auth,
    pub(crate) platform: Platform,
    cache_dir: PathBuf,
    pub(crate) timeout: Duration,
    layer_concurrency: NonZeroUsize,
    user_agent: String,
}
impl ClientBuilder {
    /// Set authentication configuration.
    #[must_use]
    pub fn auth(mut self, auth: Auth) -> Self {
        self.auth = auth;
        self
    }
    /// Set target platform.
    #[must_use]
    pub fn platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        self
    }
    /// Set content cache directory.
    #[must_use]
    pub fn cache_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cache_dir = dir.into();
        self
    }
    /// Set per-request timeout.
    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    /// Set layer download concurrency.
    #[must_use]
    pub const fn layer_concurrency(mut self, n: NonZeroUsize) -> Self {
        self.layer_concurrency = n;
        self
    }
    /// Set user-agent string.
    #[must_use]
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }
    /// Build the client.
    ///
    /// # Errors
    ///
    /// Currently infallible; returns [`Error`] to preserve the planned surface for
    /// TLS/auth validation when the registry backend lands.
    pub fn build(self) -> Result<Client> {
        Ok(Client {
            auth: self.auth,
            platform: self.platform,
            cache_dir: self.cache_dir,
            timeout: self.timeout,
            layer_concurrency: self.layer_concurrency,
            user_agent: self.user_agent,
        })
    }
}
impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            auth: Auth::Anonymous,
            platform: Platform::current(),
            cache_dir: default_cache_dir(),
            timeout: Duration::from_secs(30),
            layer_concurrency: NonZeroUsize::new(4).expect("4 is nonzero"),
            user_agent: "firkin-oci/0.0.1-alpha".to_owned(),
        }
    }
}
async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| cache_error("create_dir_all", parent, error))?;
    }
    let tmp = path.with_extension("partial");
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|error| cache_error("write", &tmp, error))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|error| cache_error("rename", path, error))
}
