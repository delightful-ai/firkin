//! descriptor — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::bundle::split_digest_path;
#[allow(unused_imports)]
use crate::error::{Error, Result};
#[allow(unused_imports)]
use firkin_ext4::LayerCompression;
#[allow(unused_imports)]
use firkin_types::Platform;
#[allow(unused_imports)]
use firkin_types::Size;
#[allow(unused_imports)]
use firkin_types::{Arch, Os};
#[allow(unused_imports)]
use oci_client::manifest::ImageIndexEntry;
#[allow(unused_imports)]
use std::fmt;
/// OCI digest string.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Digest(String);
impl Digest {
    /// Construct a digest value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    /// Return the digest string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl From<String> for Digest {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}
impl From<&str> for Digest {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}
/// OCI layer media type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MediaType(String);
impl MediaType {
    /// OCI image manifest media type.
    pub const OCI_IMAGE_MANIFEST: &'static str = "application/vnd.oci.image.manifest.v1+json";
    /// OCI image index media type.
    pub const OCI_IMAGE_INDEX: &'static str = "application/vnd.oci.image.index.v1+json";
    /// Docker image manifest media type.
    pub const DOCKER_IMAGE_MANIFEST: &'static str =
        "application/vnd.docker.distribution.manifest.v2+json";
    /// Docker image manifest-list media type.
    pub const DOCKER_MANIFEST_LIST: &'static str =
        "application/vnd.docker.distribution.manifest.list.v2+json";
    /// Uncompressed Docker/OCI tar layer media type.
    pub const TAR: &'static str = "application/vnd.oci.image.layer.v1.tar";
    /// Gzip-compressed OCI tar layer media type.
    pub const TAR_GZIP: &'static str = "application/vnd.oci.image.layer.v1.tar+gzip";
    /// Zstd-compressed OCI tar layer media type.
    pub const TAR_ZSTD: &'static str = "application/vnd.oci.image.layer.v1.tar+zstd";
    /// Gzip-compressed Docker tar layer media type.
    pub const DOCKER_TAR_GZIP: &'static str = "application/vnd.docker.image.rootfs.diff.tar.gzip";
    /// Construct a media type value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    /// Return the media type string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub(crate) fn compression(&self) -> Result<LayerCompression> {
        match self.0.as_str() {
            Self::TAR => Ok(LayerCompression::None),
            Self::TAR_GZIP | Self::DOCKER_TAR_GZIP => Ok(LayerCompression::Gzip),
            Self::TAR_ZSTD => Ok(LayerCompression::Zstd),
            other => Err(Error::UnsupportedLayerMediaType(other.to_owned())),
        }
    }
}
/// Platform information carried by a manifest-list descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestPlatform {
    pub(crate) platform: Platform,
}
impl ManifestPlatform {
    /// Construct descriptor platform metadata.
    #[must_use]
    pub const fn new(platform: Platform) -> Self {
        Self { platform }
    }
    /// Return the platform.
    #[must_use]
    pub const fn platform(&self) -> &Platform {
        &self.platform
    }
}
/// OCI descriptor used for manifests and blobs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub(crate) media_type: MediaType,
    pub(crate) digest: Digest,
    pub(crate) size: Size,
    pub(crate) platform: Option<ManifestPlatform>,
}
impl Descriptor {
    /// Construct a descriptor.
    #[must_use]
    pub fn new(
        media_type: MediaType,
        digest: impl Into<String>,
        size: u64,
        platform: Option<ManifestPlatform>,
    ) -> Self {
        Self {
            media_type,
            digest: Digest::new(digest),
            size: Size::bytes(size),
            platform,
        }
    }
    /// Return descriptor media type.
    #[must_use]
    pub const fn media_type(&self) -> &MediaType {
        &self.media_type
    }
    /// Return descriptor digest.
    #[must_use]
    pub const fn digest(&self) -> &Digest {
        &self.digest
    }
    /// Return descriptor size.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }
    /// Return platform metadata, if any.
    #[must_use]
    pub const fn platform(&self) -> Option<&ManifestPlatform> {
        self.platform.as_ref()
    }
}
/// Select a manifest descriptor from a manifest list for `target`.
///
/// # Errors
///
/// Returns [`Error::NoMatchingManifest`] when no descriptor platform matches.
pub fn select_manifest<'a>(
    descriptors: &'a [Descriptor],
    target: &Platform,
) -> Result<&'a Descriptor> {
    if let Some(exact) = descriptors.iter().find(|descriptor| {
        descriptor
            .platform()
            .is_some_and(|p| p.platform() == target)
    }) {
        return Ok(exact);
    }
    descriptors
        .iter()
        .find(|descriptor| {
            descriptor.platform().is_some_and(|p| {
                p.platform().os == target.os
                    && p.platform().arch == target.arch
                    && (p.platform().variant.is_none() || target.variant.is_none())
            })
        })
        .ok_or_else(|| Error::NoMatchingManifest {
            target: target.clone(),
            available: descriptors
                .iter()
                .filter_map(|descriptor| descriptor.platform().map(ManifestPlatform::platform))
                .cloned()
                .collect(),
        })
}
pub(crate) fn descriptor_from_index_entry(entry: &ImageIndexEntry) -> Option<Descriptor> {
    Some(Descriptor::new(
        MediaType::new(entry.media_type.clone()),
        entry.digest.clone(),
        u64::try_from(entry.size).ok()?,
        entry.platform.as_ref().and_then(platform_from_registry),
    ))
}
fn platform_from_registry(platform: &oci_client::manifest::Platform) -> Option<ManifestPlatform> {
    let os = match platform.os.to_string().as_str() {
        "linux" => Os::Linux,
        _ => return None,
    };
    let arch = match platform.architecture.to_string().as_str() {
        "amd64" => Arch::Amd64,
        "arm64" => Arch::Arm64,
        "arm" => Arch::Arm,
        "riscv64" => Arch::Riscv64,
        "ppc64le" => Arch::Ppc64le,
        "s390x" => Arch::S390x,
        _ => return None,
    };
    Some(ManifestPlatform::new(Platform {
        os,
        arch,
        variant: platform.variant.clone(),
    }))
}
pub(crate) fn digest_string(digest: &str) -> Result<String> {
    let (algorithm, value) = split_digest_path(digest);
    if algorithm != "sha256" {
        return Err(Error::InvalidDescriptor {
            digest: digest.to_owned(),
            reason: "only sha256 descriptors are supported",
        });
    }
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::InvalidDescriptor {
            digest: digest.to_owned(),
            reason: "sha256 digest must contain 64 hexadecimal characters",
        });
    }
    Ok(digest.to_owned())
}
