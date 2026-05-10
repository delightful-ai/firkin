//! OCI image reference parsing behavior from the Rust rewrite plan.

use std::str::FromStr;

use firkin_oci::{Error, Reference};

#[test]
fn short_names_expand_to_docker_library_latest() {
    let reference = Reference::parse("busybox").expect("reference");

    assert_eq!(reference.registry(), "docker.io");
    assert_eq!(reference.namespace(), "library/busybox");
    assert_eq!(reference.name(), "busybox");
    assert_eq!(reference.tag(), Some("latest"));
    assert_eq!(reference.digest(), None);
    assert_eq!(reference.canonical(), "docker.io/library/busybox:latest");
}

#[test]
fn one_segment_namespace_defaults_to_docker_registry() {
    let reference = Reference::parse("team/service:1.2.3").expect("reference");

    assert_eq!(reference.registry(), "docker.io");
    assert_eq!(reference.namespace(), "team/service");
    assert_eq!(reference.name(), "service");
    assert_eq!(reference.tag(), Some("1.2.3"));
    assert_eq!(reference.canonical(), "docker.io/team/service:1.2.3");
}

#[test]
fn dotted_or_ported_first_segment_is_registry() {
    let dotted = Reference::parse("registry.example.com/team/service").expect("reference");
    let ported = Reference::parse("localhost:5000/team/service").expect("reference");

    assert_eq!(dotted.registry(), "registry.example.com");
    assert_eq!(dotted.namespace(), "team/service");
    assert_eq!(dotted.tag(), Some("latest"));
    assert_eq!(
        dotted.canonical(),
        "registry.example.com/team/service:latest"
    );

    assert_eq!(ported.registry(), "localhost:5000");
    assert_eq!(ported.namespace(), "team/service");
    assert_eq!(ported.canonical(), "localhost:5000/team/service:latest");
}

#[test]
fn digest_references_are_pinned_and_do_not_gain_latest() {
    let reference =
        Reference::parse("alpine@sha256:0123456789abcdef").expect("reference with digest");

    assert_eq!(reference.registry(), "docker.io");
    assert_eq!(reference.namespace(), "library/alpine");
    assert_eq!(reference.name(), "alpine");
    assert_eq!(reference.tag(), None);
    assert_eq!(reference.digest(), Some("sha256:0123456789abcdef"));
    assert!(reference.pinned());
    assert_eq!(
        reference.canonical(),
        "docker.io/library/alpine@sha256:0123456789abcdef"
    );
}

#[test]
fn tag_and_digest_round_trip_together() {
    let reference = Reference::parse("ghcr.io/org/app:prod@sha256:abc").expect("reference");

    assert_eq!(reference.tag(), Some("prod"));
    assert_eq!(reference.digest(), Some("sha256:abc"));
    assert_eq!(reference.canonical(), "ghcr.io/org/app:prod@sha256:abc");
}

#[test]
fn with_tag_and_digest_replace_existing_selector() {
    let reference = Reference::parse("busybox")
        .expect("reference")
        .with_tag("edge")
        .with_digest("sha256:deadbeef");

    assert_eq!(reference.tag(), None);
    assert_eq!(reference.digest(), Some("sha256:deadbeef"));
    assert_eq!(
        reference.canonical(),
        "docker.io/library/busybox@sha256:deadbeef"
    );

    let tagged = reference.with_tag("stable");
    assert_eq!(tagged.tag(), Some("stable"));
    assert_eq!(tagged.digest(), None);
    assert_eq!(tagged.canonical(), "docker.io/library/busybox:stable");
}

#[test]
fn display_and_from_str_use_canonical_form() {
    let reference = Reference::from_str("alpine:3.20").expect("from str");

    assert_eq!(reference.to_string(), reference.canonical());
    assert_eq!(reference.to_string(), "docker.io/library/alpine:3.20");
}

#[test]
fn invalid_references_are_rejected() {
    for input in [
        "",
        "/busybox",
        "busybox:",
        "busybox@",
        "busy box",
        "busybox@@sha256:abc",
    ] {
        assert!(matches!(
            Reference::parse(input),
            Err(Error::InvalidReference { .. })
        ));
    }
}
