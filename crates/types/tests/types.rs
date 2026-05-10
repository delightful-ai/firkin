#![allow(missing_docs)]

use std::num::{NonZeroU16, NonZeroU32};
use std::str::FromStr;

use firkin_types::{
    Arch, BlockDeviceId, ContainerId, Hostname, InvalidContainerId, InvalidHostname,
    InvalidNetworkPolicyRule, InvalidPortSandboxHost, InvalidProcessId, InvalidVirtiofsTag,
    NamespaceKind, NetworkPolicyRule, Os, Platform, PortSandboxHost, ProcessId,
    SandboxNetworkPolicy, Size, VirtiofsTag, VmId, VsockPort, container_id, hostname, virtiofs_tag,
};

#[test]
fn container_id_accepts_documented_character_set() {
    let id = ContainerId::new("web_01.alpha-beta").unwrap();

    assert_eq!(id.as_str(), "web_01.alpha-beta");
    assert_eq!(id.to_string(), "web_01.alpha-beta");
    assert_eq!(ContainerId::from_str("web_01.alpha-beta").unwrap(), id);
    assert_eq!(container_id!("web"), ContainerId::new("web").unwrap());
}

#[test]
fn container_id_rejects_empty_too_long_and_forbidden_characters() {
    assert_eq!(ContainerId::new("").unwrap_err(), InvalidContainerId::Empty);

    let too_long = "a".repeat(65);
    assert_eq!(
        ContainerId::new(too_long.clone()).unwrap_err(),
        InvalidContainerId::TooLong(too_long)
    );

    assert_eq!(
        ContainerId::new("bad/id").unwrap_err(),
        InvalidContainerId::ForbiddenChars("bad/id".to_string())
    );
}

#[test]
fn process_id_uses_container_id_rules_but_remains_a_distinct_type() {
    let pid = ProcessId::new("init.0").unwrap();

    assert_eq!(pid.as_str(), "init.0");
    assert_eq!(pid.to_string(), "init.0");
    assert_eq!(ProcessId::from_str("init.0").unwrap(), pid);
    assert_eq!(
        ProcessId::new("bad/id").unwrap_err(),
        InvalidProcessId::ForbiddenChars("bad/id".to_string())
    );
}

#[test]
fn vm_id_is_uuid_backed_and_displayed_as_uuid() {
    let id = VmId::new();

    assert_eq!(id.to_string(), id.as_uuid().to_string());
}

#[test]
fn namespace_kind_renders_vminitd_spec_strings() {
    assert_eq!(NamespaceKind::Pid.as_spec_str(), "pid");
    assert_eq!(NamespaceKind::Mount.as_spec_str(), "mount");
    assert_eq!(NamespaceKind::Network.as_spec_str(), "network");
    assert_eq!(NamespaceKind::Ipc.as_spec_str(), "ipc");
    assert_eq!(NamespaceKind::Uts.as_spec_str(), "uts");
    assert_eq!(NamespaceKind::User.as_spec_str(), "user");
    assert_eq!(NamespaceKind::Cgroup.as_spec_str(), "cgroup");
    assert_eq!(NamespaceKind::Time.as_spec_str(), "time");
}

#[test]
fn block_device_id_is_an_opaque_nonzero_slot() {
    let id = BlockDeviceId::from_slot(NonZeroU32::new(3).unwrap());

    assert_eq!(id.slot(), NonZeroU32::new(3).unwrap());
    assert_eq!(id.to_string(), "bd#3");
}

#[test]
fn vsock_port_is_an_unvalidated_newtype() {
    let port = VsockPort::new(1024);

    assert_eq!(port.get(), 1024);
    assert_eq!(VsockPort::from(1024), port);
}

#[test]
fn virtiofs_tag_accepts_printable_ascii_with_vz_length_cap() {
    let tag = VirtiofsTag::new("cargo-cache").unwrap();

    assert_eq!(tag.as_str(), "cargo-cache");
    assert_eq!(tag.to_string(), "cargo-cache");
    assert_eq!(virtiofs_tag!("cargo-cache"), tag);
}

#[test]
fn virtiofs_tag_rejects_empty_long_and_non_printable_tags() {
    assert_eq!(VirtiofsTag::new("").unwrap_err(), InvalidVirtiofsTag::Empty);

    let too_long = "a".repeat(37);
    assert_eq!(
        VirtiofsTag::new(too_long.clone()).unwrap_err(),
        InvalidVirtiofsTag::TooLong(too_long)
    );

    assert_eq!(
        VirtiofsTag::new("bad\n").unwrap_err(),
        InvalidVirtiofsTag::ForbiddenChars("bad\n".to_string())
    );
}

#[test]
fn hostname_validates_rfc1123_shape() {
    let hostname = Hostname::new("web-1.local").unwrap();

    assert_eq!(hostname.as_str(), "web-1.local");
    assert_eq!(hostname.to_string(), "web-1.local");
    assert_eq!(Hostname::from_str("web-1.local").unwrap(), hostname);
    assert_eq!(hostname!("web-1.local"), hostname);
}

#[test]
fn hostname_rejects_empty_bad_label_forbidden_chars_and_numeric_only() {
    assert_eq!(Hostname::new("").unwrap_err(), InvalidHostname::Empty);

    let too_long = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(64)
    );
    assert_eq!(
        Hostname::new(too_long.clone()).unwrap_err(),
        InvalidHostname::TooLong(too_long)
    );

    assert_eq!(
        Hostname::new("web..local").unwrap_err(),
        InvalidHostname::BadLabel {
            label: String::new()
        }
    );

    assert_eq!(
        Hostname::new("-web.local").unwrap_err(),
        InvalidHostname::ForbiddenChars("-web.local".to_string())
    );

    assert_eq!(
        Hostname::new("123").unwrap_err(),
        InvalidHostname::NumericOnly("123".to_string())
    );
}

#[test]
fn port_sandbox_host_parses_e2b_proxy_hostname_shape() {
    let domain = hostname!("cube.localhost");
    let route = PortSandboxHost::parse_for_domain("49983-sbx_01.cube.localhost", &domain).unwrap();

    assert_eq!(route.port(), NonZeroU16::new(49983).unwrap());
    assert_eq!(route.sandbox_id(), &container_id!("sbx_01"));
    assert_eq!(route.domain(), &domain);
    assert_eq!(route.to_string(), "49983-sbx_01.cube.localhost");
}

#[test]
fn port_sandbox_host_rejects_debug_and_malformed_hosts() {
    let domain = hostname!("cube.localhost");

    assert_eq!(
        PortSandboxHost::parse_for_domain("localhost:49983", &domain).unwrap_err(),
        InvalidPortSandboxHost::DomainMismatch {
            host: "localhost:49983".to_owned(),
            expected: "cube.localhost".to_owned(),
        }
    );
    assert_eq!(
        PortSandboxHost::parse_for_domain("49983.cube.localhost", &domain).unwrap_err(),
        InvalidPortSandboxHost::MissingSeparator {
            host: "49983.cube.localhost".to_owned(),
        }
    );
    assert_eq!(
        PortSandboxHost::parse_for_domain("0-sbx.cube.localhost", &domain).unwrap_err(),
        InvalidPortSandboxHost::InvalidPort {
            port: "0".to_owned(),
        }
    );
    assert_eq!(
        PortSandboxHost::parse_for_domain("49983-bad/id.cube.localhost", &domain).unwrap_err(),
        InvalidPortSandboxHost::InvalidSandboxId(InvalidContainerId::ForbiddenChars(
            "bad/id".to_owned()
        ))
    );
}

#[test]
fn sandbox_network_policy_matches_local_e2b_sdk_defaults_and_network_shape() {
    let default_policy = SandboxNetworkPolicy::e2b_default();

    assert_eq!(default_policy.allow_internet_access(), Some(true));
    assert!(default_policy.allow_out().is_empty());
    assert!(default_policy.deny_out().is_empty());
    assert_eq!(default_policy.allow_public_traffic(), None);
    assert_eq!(default_policy.mask_request_host(), None);
    assert!(!default_policy.requires_policy_engine());

    let allow = NetworkPolicyRule::new("api.example.com").unwrap();
    let deny = NetworkPolicyRule::new("169.254.169.254").unwrap();
    let policy = SandboxNetworkPolicy::new(
        Some(false),
        [allow.clone()],
        [deny.clone()],
        Some(true),
        Some("sandbox.internal".to_owned()),
    );

    assert_eq!(policy.allow_internet_access(), Some(false));
    assert_eq!(policy.allow_out(), &[allow]);
    assert_eq!(policy.deny_out(), &[deny]);
    assert_eq!(policy.allow_public_traffic(), Some(true));
    assert_eq!(policy.mask_request_host(), Some("sandbox.internal"));
    assert!(policy.requires_policy_engine());
}

#[cfg(feature = "serde")]
#[test]
fn sandbox_network_policy_serializes_to_local_e2b_sdk_json_shape() {
    let policy = SandboxNetworkPolicy::new(
        Some(false),
        [NetworkPolicyRule::new("api.example.com").unwrap()],
        [NetworkPolicyRule::new("169.254.169.254").unwrap()],
        Some(true),
        Some("sandbox.internal".to_owned()),
    );
    let json = serde_json::to_value(&policy).unwrap();

    assert_eq!(json["allow_internet_access"], false);
    assert_eq!(json["allowOut"][0], "api.example.com");
    assert_eq!(json["denyOut"][0], "169.254.169.254");
    assert_eq!(json["allowPublicTraffic"], true);
    assert_eq!(json["maskRequestHost"], "sandbox.internal");
}

#[test]
fn network_policy_rule_rejects_empty_rules() {
    assert_eq!(
        NetworkPolicyRule::new(" \t").unwrap_err(),
        InvalidNetworkPolicyRule::Empty
    );
    assert_eq!(
        NetworkPolicyRule::from_str("api.example.com")
            .unwrap()
            .as_str(),
        "api.example.com"
    );
}

#[test]
fn size_uses_binary_units_and_saturating_subtraction() {
    assert_eq!(Size::bytes(1536).as_bytes(), 1536);
    assert_eq!(Size::kib(1).as_bytes(), 1024);
    assert_eq!(Size::mib(2).as_bytes(), 2 * 1024 * 1024);
    assert_eq!(Size::gib(3).as_bytes(), 3 * 1024 * 1024 * 1024);
    assert_eq!(Size::tib(4).as_bytes(), 4 * 1024 * 1024 * 1024 * 1024);
    assert_eq!((Size::mib(2) + Size::kib(512)).as_kib(), 2560);
    assert_eq!((Size::kib(1) - Size::kib(2)).as_bytes(), 0);
    assert_eq!(Size::bytes(1536).to_string(), "1.5 KiB");
}

#[test]
fn platform_current_maps_host_arch_to_linux_container_platform() {
    let current = Platform::current();

    assert_eq!(current.os, Os::Linux);
    if cfg!(target_arch = "aarch64") {
        assert_eq!(current, Platform::linux_arm64_v8());
    } else if cfg!(target_arch = "x86_64") {
        assert_eq!(current, Platform::linux_amd64());
    }
}

#[test]
fn platform_convenience_constructors_match_spec() {
    assert_eq!(
        Platform::linux_amd64(),
        Platform {
            os: Os::Linux,
            arch: Arch::Amd64,
            variant: None
        }
    );
    assert_eq!(
        Platform::linux_arm64(),
        Platform {
            os: Os::Linux,
            arch: Arch::Arm64,
            variant: None
        }
    );
    assert_eq!(
        Platform::linux_arm64_v8(),
        Platform {
            os: Os::Linux,
            arch: Arch::Arm64,
            variant: Some("v8".to_string())
        }
    );
}
