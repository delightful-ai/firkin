//! OCI image bundle and ext4 handoff tests.

use firkin_ext4::{LayerCompression, OciLayerSource};
use firkin_oci::{Digest, ImageBundle, ImageConfig, Layer, MediaType, Reference};
use firkin_types::{Platform, Size};

#[test]
fn layer_media_types_map_to_ext4_compression() {
    let tar = layer("sha256:tar", MediaType::TAR).expect("tar layer");
    let gzip = layer("sha256:gzip", MediaType::TAR_GZIP).expect("gzip layer");
    let docker_gzip =
        layer("sha256:docker-gzip", MediaType::DOCKER_TAR_GZIP).expect("docker gzip layer");
    let zstd = layer("sha256:zstd", MediaType::TAR_ZSTD).expect("zstd layer");

    assert_eq!(tar.compression(), LayerCompression::None);
    assert_eq!(gzip.compression(), LayerCompression::Gzip);
    assert_eq!(docker_gzip.compression(), LayerCompression::Gzip);
    assert_eq!(zstd.compression(), LayerCompression::Zstd);
}

#[test]
fn unsupported_layer_media_type_is_rejected_at_layer_construction() {
    let error = layer("sha256:bad", "application/vnd.example.bad").expect_err("unsupported");

    assert!(
        error
            .to_string()
            .contains("unsupported OCI layer media type")
    );
}

#[test]
fn image_bundle_reports_layers_and_ext4_source_pairs() {
    let reference = Reference::parse("busybox").expect("reference");
    let layers = vec![
        layer("sha256:one", MediaType::TAR).expect("layer one"),
        layer("sha256:two", MediaType::TAR_GZIP).expect("layer two"),
    ];
    let config = ImageConfig {
        entrypoint: Some(vec!["/bin/sh".to_owned()]),
        cmd: Some(vec!["-c".to_owned(), "echo hi".to_owned()]),
        ..ImageConfig::default()
    };
    let bundle = ImageBundle::new(
        "/tmp/bundle",
        reference,
        Digest::new("sha256:manifest"),
        Platform::linux_arm64(),
        config,
        layers,
    );

    assert_eq!(bundle.root(), std::path::Path::new("/tmp/bundle"));
    assert_eq!(bundle.digest().as_str(), "sha256:manifest");
    assert_eq!(
        bundle.config().command_args(),
        vec!["/bin/sh".to_owned(), "-c".to_owned(), "echo hi".to_owned()]
    );
    assert_eq!(bundle.total_size(), Size::bytes(200));
    assert_eq!(
        bundle
            .layer_paths()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>(),
        vec!["/tmp/sha256:one.tar", "/tmp/sha256:two.tar"]
    );
    assert_eq!(
        OciLayerSource::layers(&bundle).collect::<Vec<_>>(),
        vec![
            (
                std::path::Path::new("/tmp/sha256:one.tar"),
                LayerCompression::None
            ),
            (
                std::path::Path::new("/tmp/sha256:two.tar"),
                LayerCompression::Gzip
            ),
        ]
    );
}

fn layer(digest: &str, media_type: impl Into<String>) -> firkin_oci::Result<Layer> {
    Layer::new(
        format!("/tmp/{digest}.tar"),
        Digest::new(digest),
        Digest::new(format!("{digest}-diff")),
        Size::bytes(100),
        MediaType::new(media_type),
    )
}
