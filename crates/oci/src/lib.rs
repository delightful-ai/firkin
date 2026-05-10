#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! OCI image primitives.
#[cfg(test)]
#[allow(unused_imports)]
use firkin_types::Platform;
pub use image_config::{History, Image, ImageConfig, RootfsConfig};
pub use runtime_spec::{
    BoxSize, Hook, Hooks, Linux, LinuxCapabilities, LinuxDevice, LinuxDeviceCgroup,
    LinuxHugepageLimit, LinuxIDMapping, LinuxNamespace, LinuxNamespaceType, LinuxPersonality,
    LinuxResources, LinuxSeccomp, LinuxSeccompAction, LinuxSeccompArch, LinuxSeccompArg,
    LinuxSeccompFlag, LinuxSeccompOperator, LinuxSeccompProfile, LinuxSyscall, Mount, PosixRlimit,
    Process, Root, Spec, User,
};
#[allow(unused_imports)]
use sha2::{Digest as ShaDigest, Sha256};
#[cfg(test)]
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use tokio::io::AsyncWriteExt;
#[allow(missing_docs)]
pub(crate) mod image_config;
#[allow(missing_docs)]
pub(crate) mod runtime_spec;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pull_can_load_warmed_tag_cache_without_registry_manifest() {
        let cache = tempfile::tempdir().expect("cache dir");
        let reference = Reference::parse("busybox").expect("reference");
        let layer_bytes = b"cached layer";
        let layer_digest = test_digest(layer_bytes);
        let manifest_digest = test_digest(b"manifest");
        let layer_path = cache
            .path()
            .join("blobs")
            .join("sha256")
            .join(layer_digest.trim_start_matches("sha256:"));
        fs::create_dir_all(layer_path.parent().expect("blob parent")).expect("blob dir");
        fs::write(&layer_path, layer_bytes).expect("blob write");
        let bundle_root = cache
            .path()
            .join("bundles")
            .join("sha256")
            .join(manifest_digest.trim_start_matches("sha256:"));
        fs::create_dir_all(&bundle_root).expect("bundle dir");
        let metadata = BundleMetadata {
            reference: reference.canonical(),
            digest: manifest_digest.clone(),
            platform: Platform::linux_arm64(),
            config: ImageConfig::default(),
            layers: vec![LayerMetadata {
                digest: layer_digest.clone(),
                uncompressed_digest: test_digest(b"uncompressed"),
                size: layer_bytes.len() as u64,
                media_type: MediaType::TAR_GZIP.to_owned(),
            }],
        };
        fs::write(
            bundle_root.join("bundle.json"),
            serde_json::to_vec(&metadata).expect("metadata json"),
        )
        .expect("metadata write");
        let client = Client::builder()
            .cache_dir(cache.path())
            .platform(Platform::linux_arm64())
            .build()
            .expect("client");
        let bundle = client
            .load_cached_bundle_by_reference(&reference)
            .expect("cache lookup")
            .expect("cached bundle");
        assert_eq!(bundle.reference(), &reference);
        assert_eq!(bundle.digest().as_str(), manifest_digest);
        assert_eq!(bundle.layers()[0].path(), layer_path);
    }
    fn test_digest(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }
}
pub mod auth;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use auth::*;
pub mod bundle;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use bundle::*;
pub mod cache;
#[allow(unused_imports)]
pub(crate) use cache::*;
pub mod client;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use client::*;
pub mod descriptor;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use descriptor::*;
pub mod error;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use error::*;
pub mod reference;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use reference::*;
