//! OCI client surface and manifest-selection tests.

use std::num::NonZeroUsize;
use std::time::Duration;

use firkin_oci::{
    Auth, Client, Descriptor, Error, ManifestPlatform, MediaType, Reference, select_manifest,
};
use firkin_types::Platform;

#[test]
fn client_builder_defaults_to_current_platform_and_cache() {
    let client = Client::builder().build().unwrap();

    assert_eq!(client.auth(), &Auth::Anonymous);
    assert_eq!(client.platform(), &Platform::current());
    assert_eq!(client.layer_concurrency(), NonZeroUsize::new(4).unwrap());
    assert!(client.cache_dir().ends_with("firkin/oci"));
}

#[test]
fn client_builder_accepts_explicit_platform_cache_and_timeout() {
    let cache = tempfile::tempdir().unwrap();
    let client = Client::builder()
        .platform(Platform::linux_amd64())
        .cache_dir(cache.path())
        .timeout(Duration::from_secs(7))
        .layer_concurrency(NonZeroUsize::new(2).unwrap())
        .user_agent("firkin-test")
        .build()
        .unwrap();

    assert_eq!(client.platform(), &Platform::linux_amd64());
    assert_eq!(client.cache_dir(), cache.path());
    assert_eq!(client.timeout(), Duration::from_secs(7));
    assert_eq!(client.layer_concurrency(), NonZeroUsize::new(2).unwrap());
    assert_eq!(client.user_agent(), "firkin-test");
}

#[test]
fn manifest_selection_prefers_exact_variant_then_arch_match() {
    let exact = descriptor(
        "sha256:exact",
        ManifestPlatform::new(Platform::linux_arm64_v8()),
    );
    let arch_match = descriptor(
        "sha256:arch",
        ManifestPlatform::new(Platform::linux_arm64()),
    );

    assert_eq!(
        select_manifest(
            &[arch_match.clone(), exact.clone()],
            &Platform::linux_arm64_v8()
        )
        .unwrap(),
        &exact
    );
    assert_eq!(
        select_manifest(
            std::slice::from_ref(&arch_match),
            &Platform::linux_arm64_v8()
        )
        .unwrap(),
        &arch_match
    );
}

#[test]
fn manifest_selection_reports_available_platforms() {
    let available = vec![descriptor(
        "sha256:amd64",
        ManifestPlatform::new(Platform::linux_amd64()),
    )];
    let error = select_manifest(&available, &Platform::linux_arm64()).expect_err("no match");

    assert!(matches!(
        error,
        Error::NoMatchingManifest { target, available } if target == Platform::linux_arm64()
            && available == vec![Platform::linux_amd64()]
    ));
}

#[tokio::test]
#[ignore = "live registry smoke; run manually when changing the registry backend"]
async fn live_pull_busybox_materializes_a_cached_bundle() {
    let cache = tempfile::tempdir().unwrap();
    let reference = Reference::parse("busybox").unwrap();
    let client = Client::builder()
        .cache_dir(cache.path())
        .platform(Platform::linux_amd64())
        .build()
        .unwrap();

    let bundle = client
        .pull(&reference)
        .await
        .expect("busybox should pull from docker hub");

    assert_eq!(bundle.reference(), &reference);
    assert_eq!(bundle.platform(), &Platform::linux_amd64());
    assert!(bundle.root().join("manifest.json").exists());
    assert!(bundle.root().join("config.json").exists());
    assert!(bundle.root().join("bundle.json").exists());
    assert!(!bundle.layers().is_empty());
    assert!(bundle.layer_paths().all(std::path::Path::exists));

    let cached = client.pull(&reference).await.expect("cached pull");
    assert_eq!(cached.digest(), bundle.digest());
    assert_eq!(cached.layers(), bundle.layers());
}

fn descriptor(digest: &str, platform: ManifestPlatform) -> Descriptor {
    Descriptor::new(
        MediaType::new(MediaType::OCI_IMAGE_MANIFEST),
        digest,
        123,
        Some(platform),
    )
}
