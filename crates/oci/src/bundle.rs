//! bundle — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::auth::Auth;
#[allow(unused_imports)]
use crate::cache::cache_error;
#[allow(unused_imports)]
use crate::descriptor::{Digest, MediaType};
#[allow(unused_imports)]
use crate::error::{Error, Result};
#[allow(unused_imports)]
use crate::image_config::ImageConfig;
#[allow(unused_imports)]
use crate::reference::Reference;
#[allow(unused_imports)]
use firkin_ext4::LayerCompression;
#[allow(unused_imports)]
use firkin_ext4::OciLayerSource;
#[allow(unused_imports)]
use firkin_types::Platform;
#[allow(unused_imports)]
use firkin_types::Size;
#[allow(unused_imports)]
use oci_client::Reference as RegistryReference;
#[allow(unused_imports)]
use oci_client::manifest::OciDescriptor;
#[allow(unused_imports)]
use oci_client::secrets::RegistryAuth;
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
#[allow(unused_imports)]
use sha2::Sha256;
#[allow(unused_imports)]
use std::fs;
use std::io::Read as _;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
/// Pulled OCI layer stored on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub(crate) path: PathBuf,
    pub(crate) digest: Digest,
    pub(crate) uncompressed_digest: Digest,
    pub(crate) size: Size,
    pub(crate) media_type: MediaType,
    compression: LayerCompression,
}
impl Layer {
    /// Construct a layer record.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsupportedLayerMediaType`] if the media type cannot
    /// be translated to an ext4 layer compression format.
    pub fn new(
        path: impl Into<PathBuf>,
        digest: Digest,
        uncompressed_digest: Digest,
        size: Size,
        media_type: MediaType,
    ) -> Result<Self> {
        let compression = media_type.compression()?;
        Ok(Self {
            path: path.into(),
            digest,
            uncompressed_digest,
            size,
            media_type,
            compression,
        })
    }
    /// Return the raw layer path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Return the compressed layer digest.
    #[must_use]
    pub const fn digest(&self) -> &Digest {
        &self.digest
    }
    /// Return the uncompressed layer digest.
    #[must_use]
    pub const fn uncompressed_digest(&self) -> &Digest {
        &self.uncompressed_digest
    }
    /// Return compressed size.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }
    /// Return the layer media type.
    #[must_use]
    pub const fn media_type(&self) -> &MediaType {
        &self.media_type
    }
    /// Return the compression format understood by `firkin-ext4`.
    #[must_use]
    pub const fn compression(&self) -> LayerCompression {
        self.compression
    }
}
/// Pulled OCI image bundle stored on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageBundle {
    pub(crate) root: PathBuf,
    pub(crate) reference: Reference,
    pub(crate) digest: Digest,
    pub(crate) platform: Platform,
    pub(crate) config: ImageConfig,
    pub(crate) layers: Vec<Layer>,
}
impl ImageBundle {
    /// Construct an image bundle record.
    #[must_use]
    pub fn new(
        root: impl Into<PathBuf>,
        reference: Reference,
        digest: Digest,
        platform: Platform,
        config: ImageConfig,
        layers: Vec<Layer>,
    ) -> Self {
        Self {
            root: root.into(),
            reference,
            digest,
            platform,
            config,
            layers,
        }
    }
    /// Return the bundle root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Return the original reference.
    #[must_use]
    pub const fn reference(&self) -> &Reference {
        &self.reference
    }
    /// Return the manifest digest.
    #[must_use]
    pub const fn digest(&self) -> &Digest {
        &self.digest
    }
    /// Return the selected platform.
    #[must_use]
    pub const fn platform(&self) -> &Platform {
        &self.platform
    }
    /// Return the OCI image config selected for this bundle.
    #[must_use]
    pub const fn config(&self) -> &ImageConfig {
        &self.config
    }
    /// Return pulled layers in application order.
    #[must_use]
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }
    /// Return total compressed layer size.
    #[must_use]
    pub fn total_size(&self) -> Size {
        self.layers
            .iter()
            .fold(Size::bytes(0), |total, layer| total + layer.size())
    }
    /// Iterate over layer paths.
    pub fn layer_paths(&self) -> impl Iterator<Item = &Path> + '_ {
        self.layers.iter().map(Layer::path)
    }
}
impl firkin_ext4::sealed::Sealed for ImageBundle {}
impl OciLayerSource for ImageBundle {
    fn layers(&self) -> impl Iterator<Item = (&Path, LayerCompression)> + '_ {
        self.layers
            .iter()
            .map(|layer| (layer.path(), layer.compression()))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BundleMetadata {
    pub(crate) reference: String,
    pub(crate) digest: String,
    pub(crate) platform: Platform,
    pub(crate) config: ImageConfig,
    pub(crate) layers: Vec<LayerMetadata>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LayerMetadata {
    pub(crate) digest: String,
    pub(crate) uncompressed_digest: String,
    pub(crate) size: u64,
    pub(crate) media_type: String,
}
pub(crate) fn registry_reference(reference: &Reference) -> RegistryReference {
    match (&reference.tag, &reference.digest) {
        (Some(tag), Some(digest)) => RegistryReference::with_tag_and_digest(
            reference.registry.clone(),
            reference.namespace.clone(),
            tag.clone(),
            digest.clone(),
        ),
        (Some(tag), None) => RegistryReference::with_tag(
            reference.registry.clone(),
            reference.namespace.clone(),
            tag.clone(),
        ),
        (None, Some(digest)) => RegistryReference::with_digest(
            reference.registry.clone(),
            reference.namespace.clone(),
            digest.clone(),
        ),
        (None, None) => RegistryReference::with_tag(
            reference.registry.clone(),
            reference.namespace.clone(),
            "latest".to_owned(),
        ),
    }
}
pub(crate) fn registry_auth(auth: &Auth) -> RegistryAuth {
    match auth {
        Auth::Anonymous => RegistryAuth::Anonymous,
    }
}
pub(crate) fn registry_error(
    reference: &RegistryReference,
    error: &oci_client::errors::OciDistributionError,
) -> Error {
    Error::Registry {
        reference: reference.to_string(),
        reason: error.to_string(),
    }
}
pub(crate) fn layer_from_descriptor(
    path: PathBuf,
    descriptor: &OciDescriptor,
    diff_id: &str,
) -> Result<Layer> {
    let size = u64::try_from(descriptor.size).map_err(|_| Error::InvalidDescriptor {
        digest: descriptor.digest.clone(),
        reason: "descriptor size is negative",
    })?;
    Layer::new(
        path,
        Digest::new(descriptor.digest.clone()),
        Digest::new(diff_id.to_owned()),
        Size::bytes(size),
        MediaType::new(descriptor.media_type.clone()),
    )
}
pub(crate) fn split_digest_path(digest: &str) -> (&str, &str) {
    digest.split_once(':').unwrap_or(("unknown", digest))
}
pub(crate) fn file_digest(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|error| cache_error("open", path, error))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut buf)
            .map_err(|error| cache_error("read", path, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}
