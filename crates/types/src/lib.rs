#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Shared value types for the firkin Rust containerization library.

use std::fmt;
use std::num::{NonZeroU16, NonZeroU32};
use std::ops::{Add, Sub};
use std::str::FromStr;

/// A validated container identity.
///
/// Container IDs are used as guest hostnames, path segments under
/// `/run/container`, and cgroup path fragments. Construction validates the
/// intersection of those constraints.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContainerId(String);

impl ContainerId {
    /// Construct a container ID with validation.
    ///
    /// Accepted IDs are non-empty, at most 64 ASCII characters, and contain
    /// only `[a-zA-Z0-9_.-]`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidContainerId`] when the ID is empty, too long, or
    /// contains forbidden characters.
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidContainerId> {
        let s = s.into();
        validate_id_runtime(&s).map_err(|error| error.into_container_error(s.clone()))?;
        Ok(Self(s))
    }

    /// Return the ID as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Construct from a string literal already validated by `container_id!`.
    #[doc(hidden)]
    #[must_use]
    pub fn __from_validated_literal(s: &'static str) -> Self {
        Self(s.to_owned())
    }
}

impl FromStr for ContainerId {
    type Err = InvalidContainerId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for ContainerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Failure to construct a [`ContainerId`].
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidContainerId {
    /// The ID was empty.
    #[error("container id is empty")]
    Empty,
    /// The ID exceeded 64 characters.
    #[error("container id `{0}` is too long (max 64 chars)")]
    TooLong(String),
    /// The ID contained a character outside `[a-zA-Z0-9_.-]`.
    #[error("container id `{0}` contains forbidden characters")]
    ForbiddenChars(String),
}

/// A validated process identity.
///
/// `ProcessId` uses the same character rules as [`ContainerId`] but remains a
/// distinct type so process and container identities cannot be mixed.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProcessId(String);

impl ProcessId {
    /// Construct a process ID with validation.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidProcessId`] when the ID is empty, too long, or contains
    /// forbidden characters.
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidProcessId> {
        let s = s.into();
        validate_id_runtime(&s).map_err(|error| error.into_process_error(s.clone()))?;
        Ok(Self(s))
    }

    /// Return the ID as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ProcessId {
    type Err = InvalidProcessId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Failure to construct a [`ProcessId`].
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidProcessId {
    /// The ID was empty.
    #[error("process id is empty")]
    Empty,
    /// The ID exceeded 64 characters.
    #[error("process id `{0}` is too long (max 64 chars)")]
    TooLong(String),
    /// The ID contained a character outside `[a-zA-Z0-9_.-]`.
    #[error("process id `{0}` contains forbidden characters")]
    ForbiddenChars(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdValidationError {
    Empty,
    TooLong,
    ForbiddenChars,
}

impl IdValidationError {
    fn into_container_error(self, value: String) -> InvalidContainerId {
        match self {
            Self::Empty => InvalidContainerId::Empty,
            Self::TooLong => InvalidContainerId::TooLong(value),
            Self::ForbiddenChars => InvalidContainerId::ForbiddenChars(value),
        }
    }

    fn into_process_error(self, value: String) -> InvalidProcessId {
        match self {
            Self::Empty => InvalidProcessId::Empty,
            Self::TooLong => InvalidProcessId::TooLong(value),
            Self::ForbiddenChars => InvalidProcessId::ForbiddenChars(value),
        }
    }
}

fn validate_id_runtime(s: &str) -> Result<(), IdValidationError> {
    if s.is_empty() {
        return Err(IdValidationError::Empty);
    }
    if s.chars().count() > 64 {
        return Err(IdValidationError::TooLong);
    }
    if !s.bytes().all(is_id_byte) {
        return Err(IdValidationError::ForbiddenChars);
    }
    Ok(())
}

/// A library-generated VM identity.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VmId(uuid::Uuid);

impl VmId {
    /// Generate a fresh UUID-backed VM ID.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    /// Return the underlying UUID.
    #[must_use]
    pub fn as_uuid(&self) -> &uuid::Uuid {
        &self.0
    }
}

impl Default for VmId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for VmId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Linux namespace kinds accepted by vminitd's runtime spec decoder.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NamespaceKind {
    /// PID namespace.
    Pid,
    /// Mount namespace.
    Mount,
    /// Network namespace.
    Network,
    /// IPC namespace.
    Ipc,
    /// UTS namespace.
    Uts,
    /// User namespace.
    User,
    /// Cgroup namespace.
    Cgroup,
    /// Time namespace.
    Time,
}

impl NamespaceKind {
    /// Return the lowercase OCI/runtime-spec string vminitd accepts.
    #[must_use]
    pub const fn as_spec_str(self) -> &'static str {
        match self {
            Self::Pid => "pid",
            Self::Mount => "mount",
            Self::Network => "network",
            Self::Ipc => "ipc",
            Self::Uts => "uts",
            Self::User => "user",
            Self::Cgroup => "cgroup",
            Self::Time => "time",
        }
    }
}

/// Opaque handle for a VM block-device slot.
///
/// Users obtain this from the VM configuration builder and pass it back through
/// the container rootfs API; they do not name paths twice.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockDeviceId(NonZeroU32);

impl BlockDeviceId {
    /// Construct a block-device ID from a non-zero VM slot.
    ///
    /// This is public so sibling crates can construct IDs, but it is hidden from
    /// generated docs because the public user path is `VmConfigBuilder`.
    #[doc(hidden)]
    #[must_use]
    pub const fn from_slot(slot: NonZeroU32) -> Self {
        Self(slot)
    }

    /// Return the opaque numeric slot.
    #[must_use]
    pub const fn slot(self) -> NonZeroU32 {
        self.0
    }
}

impl fmt::Display for BlockDeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bd#{}", self.0)
    }
}

/// A vsock port number.
///
/// The newtype itself does not validate reserved ranges. Dial/listen sites own
/// the policy because configuration code may still need to name reserved ports.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct VsockPort(u32);

impl VsockPort {
    /// Construct a vsock port.
    #[must_use]
    pub const fn new(port: u32) -> Self {
        Self(port)
    }

    /// Return the numeric port.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for VsockPort {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

/// A virtiofs tag accepted by Virtualization.framework.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VirtiofsTag(String);

impl VirtiofsTag {
    /// Construct a virtiofs tag with validation.
    ///
    /// Tags must be non-empty printable ASCII and at most 36 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidVirtiofsTag`] when the tag is empty, too long, or
    /// contains non-printable or non-ASCII characters.
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidVirtiofsTag> {
        let s = s.into();
        validate_virtiofs_tag_runtime(&s)?;
        Ok(Self(s))
    }

    /// Return the tag as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Construct from a string literal already validated by `virtiofs_tag!`.
    #[doc(hidden)]
    #[must_use]
    pub fn __from_validated_literal(s: &'static str) -> Self {
        Self(s.to_owned())
    }
}

impl fmt::Display for VirtiofsTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Failure to construct a [`VirtiofsTag`].
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidVirtiofsTag {
    /// The tag was empty.
    #[error("virtiofs tag is empty")]
    Empty,
    /// The tag exceeded Virtualization.framework's 36-byte cap.
    #[error("virtiofs tag `{0}` exceeds 36 bytes")]
    TooLong(String),
    /// The tag contained a non-printable or non-ASCII character.
    #[error("virtiofs tag `{0}` contains forbidden characters")]
    ForbiddenChars(String),
}

fn validate_virtiofs_tag_runtime(s: &str) -> Result<(), InvalidVirtiofsTag> {
    if s.is_empty() {
        return Err(InvalidVirtiofsTag::Empty);
    }
    if s.len() > 36 {
        return Err(InvalidVirtiofsTag::TooLong(s.to_owned()));
    }
    if !s.bytes().all(is_printable_ascii) {
        return Err(InvalidVirtiofsTag::ForbiddenChars(s.to_owned()));
    }
    Ok(())
}

/// A validated Linux hostname.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hostname(String);

impl Hostname {
    /// Construct a hostname with RFC 1123-style validation.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidHostname`] when the hostname is empty, too long, has an
    /// invalid label, contains forbidden characters, or is a bare number.
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidHostname> {
        let s = s.into();
        validate_hostname_runtime(&s)?;
        Ok(Self(s))
    }

    /// Return the hostname as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Construct from a string literal already validated by `hostname!`.
    #[doc(hidden)]
    #[must_use]
    pub fn __from_validated_literal(s: &'static str) -> Self {
        Self(s.to_owned())
    }
}

impl FromStr for Hostname {
    type Err = InvalidHostname;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for Hostname {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Failure to construct a [`Hostname`].
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidHostname {
    /// The hostname was empty.
    #[error("hostname is empty")]
    Empty,
    /// The hostname exceeded 253 bytes.
    #[error("hostname `{0}` exceeds 253 bytes")]
    TooLong(String),
    /// A label was empty or longer than 63 bytes.
    #[error("hostname label `{label}` is empty or >63 bytes")]
    BadLabel {
        /// The invalid label.
        label: String,
    },
    /// A label contained an invalid character or leading/trailing dash.
    #[error("hostname `{0}` contains a label with forbidden characters")]
    ForbiddenChars(String),
    /// The hostname was a bare number.
    #[error("hostname `{0}` is purely numeric (use an IP type instead)")]
    NumericOnly(String),
}

/// Parsed `{port}-{sandboxID}.{domain}` host used by E2B-style local proxies.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PortSandboxHost {
    port: NonZeroU16,
    sandbox_id: ContainerId,
    domain: Hostname,
}

impl PortSandboxHost {
    /// Construct a proxy host route from validated parts.
    #[must_use]
    pub fn new(port: NonZeroU16, sandbox_id: ContainerId, domain: Hostname) -> Self {
        Self {
            port,
            sandbox_id,
            domain,
        }
    }

    /// Parse `{port}-{sandboxID}.{domain}` for a configured local proxy domain.
    ///
    /// This rejects debug-mode `localhost:{port}` shortcuts. Full E2B/Cube
    /// compatibility requires host-based multi-sandbox routing.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidPortSandboxHost`] when the host is not under
    /// `expected_domain`, lacks the `port-sandbox` prefix, or has invalid port
    /// or sandbox-id components.
    pub fn parse_for_domain(
        host: &str,
        expected_domain: &Hostname,
    ) -> Result<Self, InvalidPortSandboxHost> {
        let suffix = expected_domain.as_str();
        let prefix = host
            .strip_suffix(suffix)
            .and_then(|head| head.strip_suffix('.'))
            .ok_or_else(|| InvalidPortSandboxHost::DomainMismatch {
                host: host.to_owned(),
                expected: suffix.to_owned(),
            })?;
        let (port, sandbox_id) =
            prefix
                .split_once('-')
                .ok_or_else(|| InvalidPortSandboxHost::MissingSeparator {
                    host: host.to_owned(),
                })?;
        if sandbox_id.is_empty() || sandbox_id.contains('.') {
            return Err(InvalidPortSandboxHost::InvalidSandboxId(
                InvalidContainerId::ForbiddenChars(sandbox_id.to_owned()),
            ));
        }
        let port = port
            .parse::<u16>()
            .ok()
            .and_then(NonZeroU16::new)
            .ok_or_else(|| InvalidPortSandboxHost::InvalidPort {
                port: port.to_owned(),
            })?;
        let sandbox_id =
            ContainerId::new(sandbox_id).map_err(InvalidPortSandboxHost::InvalidSandboxId)?;

        Ok(Self::new(port, sandbox_id, expected_domain.clone()))
    }

    /// Return the target guest/container TCP port.
    #[must_use]
    pub const fn port(&self) -> NonZeroU16 {
        self.port
    }

    /// Return the sandbox id.
    #[must_use]
    pub const fn sandbox_id(&self) -> &ContainerId {
        &self.sandbox_id
    }

    /// Return the configured proxy domain.
    #[must_use]
    pub const fn domain(&self) -> &Hostname {
        &self.domain
    }
}

impl fmt::Display for PortSandboxHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}.{}", self.port, self.sandbox_id, self.domain)
    }
}

/// Failure to parse a [`PortSandboxHost`].
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidPortSandboxHost {
    /// Host does not end in the configured proxy domain.
    #[error("host `{host}` is not under expected proxy domain `{expected}`")]
    DomainMismatch {
        /// Host header value.
        host: String,
        /// Expected local proxy domain.
        expected: String,
    },
    /// Host is missing the `port-sandbox` separator.
    #[error("host `{host}` is missing `port-sandbox` separator")]
    MissingSeparator {
        /// Host header value.
        host: String,
    },
    /// Port component is not a non-zero TCP port.
    #[error("proxy host port `{port}` is invalid")]
    InvalidPort {
        /// Invalid port text.
        port: String,
    },
    /// Sandbox id component is invalid.
    #[error("proxy host sandbox id is invalid: {0}")]
    InvalidSandboxId(#[source] InvalidContainerId),
}

/// E2B/Cube sandbox network fields that need real enforcement by a runtime.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct SandboxNetworkPolicy {
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "allow_internet_access",
            skip_serializing_if = "Option::is_none"
        )
    )]
    allow_internet_access: Option<bool>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    allow_out: Vec<NetworkPolicyRule>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    deny_out: Vec<NetworkPolicyRule>,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    allow_public_traffic: Option<bool>,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    mask_request_host: Option<String>,
}

impl SandboxNetworkPolicy {
    /// Construct a network policy from the local E2B Rust SDK field set.
    #[must_use]
    pub fn new(
        allow_internet_access: Option<bool>,
        allow_out: impl IntoIterator<Item = NetworkPolicyRule>,
        deny_out: impl IntoIterator<Item = NetworkPolicyRule>,
        allow_public_traffic: Option<bool>,
        mask_request_host: Option<String>,
    ) -> Self {
        Self {
            allow_internet_access,
            allow_out: allow_out.into_iter().collect(),
            deny_out: deny_out.into_iter().collect(),
            allow_public_traffic,
            mask_request_host,
        }
    }

    /// Return the E2B create-request default policy.
    #[must_use]
    pub fn e2b_default() -> Self {
        Self {
            allow_internet_access: Some(true),
            ..Self::default()
        }
    }

    /// Return whether outbound internet access was explicitly requested.
    #[must_use]
    pub const fn allow_internet_access(&self) -> Option<bool> {
        self.allow_internet_access
    }

    /// Return explicit outbound allow rules.
    #[must_use]
    pub fn allow_out(&self) -> &[NetworkPolicyRule] {
        &self.allow_out
    }

    /// Return explicit outbound deny rules.
    #[must_use]
    pub fn deny_out(&self) -> &[NetworkPolicyRule] {
        &self.deny_out
    }

    /// Return whether public inbound traffic is requested.
    #[must_use]
    pub const fn allow_public_traffic(&self) -> Option<bool> {
        self.allow_public_traffic
    }

    /// Return the host value that proxied requests should mask.
    #[must_use]
    pub fn mask_request_host(&self) -> Option<&str> {
        self.mask_request_host.as_deref()
    }

    /// Return whether this policy needs enforcement beyond plain network attach.
    #[must_use]
    pub fn requires_policy_engine(&self) -> bool {
        self.allow_internet_access == Some(false)
            || !self.allow_out.is_empty()
            || !self.deny_out.is_empty()
            || self.allow_public_traffic == Some(true)
            || self.mask_request_host.is_some()
    }
}

/// One target rule from E2B/Cube `allowOut` or `denyOut`.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NetworkPolicyRule(String);

impl NetworkPolicyRule {
    /// Construct a non-empty network policy rule.
    ///
    /// Rules intentionally stay as strings here because the E2B/Cube surface can
    /// carry hostnames, CIDRs, and service-shaped targets. Runtime adapters are
    /// responsible for lowering them to PF, guest firewall, or Linux/Cube policy
    /// backends.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidNetworkPolicyRule`] when `rule` is empty after trimming.
    pub fn new(rule: impl Into<String>) -> Result<Self, InvalidNetworkPolicyRule> {
        let rule = rule.into();
        if rule.trim().is_empty() {
            return Err(InvalidNetworkPolicyRule::Empty);
        }
        Ok(Self(rule))
    }

    /// Return the original rule text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for NetworkPolicyRule {
    type Err = InvalidNetworkPolicyRule;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for NetworkPolicyRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Failure to construct a [`NetworkPolicyRule`].
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidNetworkPolicyRule {
    /// The rule was empty.
    #[error("network policy rule is empty")]
    Empty,
}

fn validate_hostname_runtime(s: &str) -> Result<(), InvalidHostname> {
    if s.is_empty() {
        return Err(InvalidHostname::Empty);
    }
    if s.len() > 253 {
        return Err(InvalidHostname::TooLong(s.to_owned()));
    }
    if s.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(InvalidHostname::NumericOnly(s.to_owned()));
    }

    for label in s.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(InvalidHostname::BadLabel {
                label: label.to_owned(),
            });
        }
        if label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(InvalidHostname::ForbiddenChars(s.to_owned()));
        }
    }

    Ok(())
}

/// A byte size.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct Size(u64);

impl Size {
    /// Construct from bytes.
    #[must_use]
    pub const fn bytes(n: u64) -> Self {
        Self(n)
    }

    /// Construct from kibibytes.
    #[must_use]
    pub const fn kib(n: u64) -> Self {
        Self(n.saturating_mul(1024))
    }

    /// Construct from mebibytes.
    #[must_use]
    pub const fn mib(n: u64) -> Self {
        Self(n.saturating_mul(1024 * 1024))
    }

    /// Construct from gibibytes.
    #[must_use]
    pub const fn gib(n: u64) -> Self {
        Self(n.saturating_mul(1024 * 1024 * 1024))
    }

    /// Construct from tebibytes.
    #[must_use]
    pub const fn tib(n: u64) -> Self {
        Self(n.saturating_mul(1024 * 1024 * 1024 * 1024))
    }

    /// Return bytes.
    #[must_use]
    pub const fn as_bytes(self) -> u64 {
        self.0
    }

    /// Return whole kibibytes, truncating.
    #[must_use]
    pub const fn as_kib(self) -> u64 {
        self.0 / 1024
    }

    /// Return whole mebibytes, truncating.
    #[must_use]
    pub const fn as_mib(self) -> u64 {
        self.0 / (1024 * 1024)
    }

    /// Return whole gibibytes, truncating.
    #[must_use]
    pub const fn as_gib(self) -> u64 {
        self.0 / (1024 * 1024 * 1024)
    }
}

impl Add for Size {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0.saturating_add(rhs.0))
    }
}

impl Sub for Size {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0.saturating_sub(rhs.0))
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = self.0;
        let (unit, label) = match bytes {
            0..=1023 => return write!(f, "{bytes} B"),
            1024..=1_048_575 => (1024_u64, "KiB"),
            1_048_576..=1_073_741_823 => (1_048_576_u64, "MiB"),
            1_073_741_824..=1_099_511_627_775 => (1_073_741_824_u64, "GiB"),
            _ => (1_099_511_627_776_u64, "TiB"),
        };

        if bytes.is_multiple_of(unit) {
            write!(f, "{} {label}", bytes / unit)
        } else {
            let tenths = ((u128::from(bytes) * 10) + (u128::from(unit) / 2)) / u128::from(unit);
            let whole = tenths / 10;
            let decimal = tenths % 10;
            if decimal == 0 {
                write!(f, "{whole} {label}")
            } else {
                write!(f, "{whole}.{decimal} {label}")
            }
        }
    }
}

/// An OCI operating system.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Os {
    /// Linux containers.
    Linux,
}

/// An OCI CPU architecture.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Arch {
    /// `amd64`.
    Amd64,
    /// `arm64`.
    Arm64,
    /// `arm`.
    Arm,
    /// `riscv64`.
    Riscv64,
    /// `ppc64le`.
    Ppc64le,
    /// `s390x`.
    S390x,
}

/// An OCI platform used for manifest-list selection.
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Platform {
    /// The container OS.
    pub os: Os,
    /// The container CPU architecture.
    pub arch: Arch,
    /// Optional OCI architecture variant.
    pub variant: Option<String>,
}

impl Platform {
    /// Return the current host's Linux-container platform.
    #[must_use]
    pub fn current() -> Self {
        #[cfg(target_arch = "aarch64")]
        {
            Self::linux_arm64_v8()
        }
        #[cfg(target_arch = "x86_64")]
        {
            Self::linux_amd64()
        }
        #[cfg(target_arch = "arm")]
        {
            Self {
                os: Os::Linux,
                arch: Arch::Arm,
                variant: None,
            }
        }
        #[cfg(target_arch = "riscv64")]
        {
            Self {
                os: Os::Linux,
                arch: Arch::Riscv64,
                variant: None,
            }
        }
        #[cfg(all(target_arch = "powerpc64", target_endian = "little"))]
        {
            Self {
                os: Os::Linux,
                arch: Arch::Ppc64le,
                variant: None,
            }
        }
        #[cfg(target_arch = "s390x")]
        {
            Self {
                os: Os::Linux,
                arch: Arch::S390x,
                variant: None,
            }
        }
        #[cfg(not(any(
            target_arch = "aarch64",
            target_arch = "x86_64",
            target_arch = "arm",
            target_arch = "riscv64",
            all(target_arch = "powerpc64", target_endian = "little"),
            target_arch = "s390x"
        )))]
        compile_error!("unsupported target architecture for OCI platform detection");
    }

    /// Return `linux/amd64`.
    #[must_use]
    pub fn linux_amd64() -> Self {
        Self {
            os: Os::Linux,
            arch: Arch::Amd64,
            variant: None,
        }
    }

    /// Return `linux/arm64`.
    #[must_use]
    pub fn linux_arm64() -> Self {
        Self {
            os: Os::Linux,
            arch: Arch::Arm64,
            variant: None,
        }
    }

    /// Return `linux/arm64/v8`.
    #[must_use]
    pub fn linux_arm64_v8() -> Self {
        Self {
            os: Os::Linux,
            arch: Arch::Arm64,
            variant: Some("v8".to_owned()),
        }
    }
}

/// Validate a container ID literal at compile time.
#[doc(hidden)]
pub const fn __validate_container_id_literal(s: &str) {
    validate_id_literal(s);
}

/// Validate a virtiofs tag literal at compile time.
#[doc(hidden)]
pub const fn __validate_virtiofs_tag_literal(s: &str) {
    let bytes = s.as_bytes();
    assert!(!bytes.is_empty(), "virtiofs tag literal is empty");
    assert!(bytes.len() <= 36, "virtiofs tag literal exceeds 36 bytes");

    let mut i = 0;
    while i < bytes.len() {
        assert!(
            is_printable_ascii(bytes[i]),
            "virtiofs tag literal contains forbidden characters"
        );
        i += 1;
    }
}

/// Validate a hostname literal at compile time.
#[doc(hidden)]
pub const fn __validate_hostname_literal(s: &str) {
    let bytes = s.as_bytes();
    assert!(!bytes.is_empty(), "hostname literal is empty");
    assert!(bytes.len() <= 253, "hostname literal exceeds 253 bytes");
    assert!(
        !is_numeric_literal(bytes),
        "hostname literal is purely numeric"
    );

    let mut label_len = 0_usize;
    let mut label_start = 0_usize;
    let mut i = 0_usize;
    while i <= bytes.len() {
        if i == bytes.len() || bytes[i] == b'.' {
            assert!(
                !(label_len == 0 || label_len > 63),
                "hostname literal contains an empty or overlong label"
            );
            assert!(
                !(bytes[label_start] == b'-' || bytes[i - 1] == b'-'),
                "hostname literal contains a label with forbidden characters"
            );
            label_len = 0;
            label_start = i + 1;
        } else {
            assert!(
                bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-',
                "hostname literal contains a label with forbidden characters"
            );
            label_len += 1;
        }
        i += 1;
    }
}

const fn validate_id_literal(s: &str) {
    let bytes = s.as_bytes();
    assert!(!bytes.is_empty(), "id literal is empty");
    assert!(bytes.len() <= 64, "id literal exceeds 64 bytes");

    let mut i = 0;
    while i < bytes.len() {
        assert!(
            is_id_byte(bytes[i]),
            "id literal contains forbidden characters"
        );
        i += 1;
    }
}

const fn is_id_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.' || byte == b'-'
}

const fn is_printable_ascii(byte: u8) -> bool {
    byte >= 0x20 && byte <= 0x7e
}

const fn is_numeric_literal(bytes: &[u8]) -> bool {
    let mut i = 0_usize;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            return false;
        }
        i += 1;
    }
    true
}

/// Construct a [`ContainerId`] from a compile-time validated string literal.
#[macro_export]
macro_rules! container_id {
    ($s:literal) => {{
        const _: () = $crate::__validate_container_id_literal($s);
        $crate::ContainerId::__from_validated_literal($s)
    }};
}

/// Construct a [`VirtiofsTag`] from a compile-time validated string literal.
#[macro_export]
macro_rules! virtiofs_tag {
    ($s:literal) => {{
        const _: () = $crate::__validate_virtiofs_tag_literal($s);
        $crate::VirtiofsTag::__from_validated_literal($s)
    }};
}

/// Construct a [`Hostname`] from a compile-time validated string literal.
#[macro_export]
macro_rules! hostname {
    ($s:literal) => {{
        const _: () = $crate::__validate_hostname_literal($s);
        $crate::Hostname::__from_validated_literal($s)
    }};
}
