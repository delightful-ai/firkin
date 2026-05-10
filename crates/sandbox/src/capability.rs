use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CapabilityName {
    RuntimeCreate,
    RuntimeAttach,
    RuntimeList,
    RuntimeDeadline,
    SandboxStop,
    SandboxKill,
    SandboxDelete,
    SnapshotCapture,
    SnapshotRestore,
    SnapshotDelete,
    SnapshotExport,
    SnapshotImport,
    PauseCapture,
    PauseResume,
    ProcessRun,
    ProcessStart,
    ProcessStream,
    ProcessStdin,
    ProcessSignal,
    ProcessPty,
    FilesystemRead,
    FilesystemWrite,
    FilesystemCopyIn,
    FilesystemCopyOut,
    FilesystemList,
    FilesystemWatch,
    PortsConnect,
    PortsExpose,
    PortsDomainProxy,
    TemplatePrepare,
    TemplateReady,
    TemplateFreshness,
    TemplateDataPlaneNone,
    TemplateDataPlaneEnvdInject,
    TemplateDataPlaneEnvdVerify,
    SandboxDataPlaneInit,
    WarmPoolPrewarm,
    WarmPoolCheckout,
    EventsSubscribe,
    MetricsHost,
    MetricsGuest,
    NetworkPolicy,
}

impl CapabilityName {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RuntimeCreate => "runtime.create",
            Self::RuntimeAttach => "runtime.attach",
            Self::RuntimeList => "runtime.list",
            Self::RuntimeDeadline => "runtime.deadline",
            Self::SandboxStop => "sandbox.stop",
            Self::SandboxKill => "sandbox.kill",
            Self::SandboxDelete => "sandbox.delete",
            Self::SnapshotCapture => "snapshot.capture",
            Self::SnapshotRestore => "snapshot.restore",
            Self::SnapshotDelete => "snapshot.delete",
            Self::SnapshotExport => "snapshot.export",
            Self::SnapshotImport => "snapshot.import",
            Self::PauseCapture => "pause.capture",
            Self::PauseResume => "pause.resume",
            Self::ProcessRun => "process.run",
            Self::ProcessStart => "process.start",
            Self::ProcessStream => "process.stream",
            Self::ProcessStdin => "process.stdin",
            Self::ProcessSignal => "process.signal",
            Self::ProcessPty => "process.pty",
            Self::FilesystemRead => "filesystem.read",
            Self::FilesystemWrite => "filesystem.write",
            Self::FilesystemCopyIn => "filesystem.copy_in",
            Self::FilesystemCopyOut => "filesystem.copy_out",
            Self::FilesystemList => "filesystem.list",
            Self::FilesystemWatch => "filesystem.watch",
            Self::PortsConnect => "ports.connect",
            Self::PortsExpose => "ports.expose",
            Self::PortsDomainProxy => "ports.domain_proxy",
            Self::TemplatePrepare => "template.prepare",
            Self::TemplateReady => "template.ready",
            Self::TemplateFreshness => "template.freshness",
            Self::TemplateDataPlaneNone => "template.data_plane.none",
            Self::TemplateDataPlaneEnvdInject => "template.data_plane.envd.inject",
            Self::TemplateDataPlaneEnvdVerify => "template.data_plane.envd.verify",
            Self::SandboxDataPlaneInit => "sandbox.data_plane.init",
            Self::WarmPoolPrewarm => "warm_pool.prewarm",
            Self::WarmPoolCheckout => "warm_pool.checkout",
            Self::EventsSubscribe => "events.subscribe",
            Self::MetricsHost => "metrics.host",
            Self::MetricsGuest => "metrics.guest",
            Self::NetworkPolicy => "network.policy",
        }
    }
}

impl fmt::Display for CapabilityName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CapabilityName {
    type Err = UnknownCapabilityName;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        ALL_CAPABILITY_NAMES
            .iter()
            .find(|name| name.as_str() == value)
            .cloned()
            .ok_or_else(|| UnknownCapabilityName(value.to_owned()))
    }
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("unknown sandbox capability `{0}`")]
pub struct UnknownCapabilityName(pub String);

pub const ALL_CAPABILITY_NAMES: &[CapabilityName] = &[
    CapabilityName::RuntimeCreate,
    CapabilityName::RuntimeAttach,
    CapabilityName::RuntimeList,
    CapabilityName::RuntimeDeadline,
    CapabilityName::SandboxStop,
    CapabilityName::SandboxKill,
    CapabilityName::SandboxDelete,
    CapabilityName::SnapshotCapture,
    CapabilityName::SnapshotRestore,
    CapabilityName::SnapshotDelete,
    CapabilityName::SnapshotExport,
    CapabilityName::SnapshotImport,
    CapabilityName::PauseCapture,
    CapabilityName::PauseResume,
    CapabilityName::ProcessRun,
    CapabilityName::ProcessStart,
    CapabilityName::ProcessStream,
    CapabilityName::ProcessStdin,
    CapabilityName::ProcessSignal,
    CapabilityName::ProcessPty,
    CapabilityName::FilesystemRead,
    CapabilityName::FilesystemWrite,
    CapabilityName::FilesystemCopyIn,
    CapabilityName::FilesystemCopyOut,
    CapabilityName::FilesystemList,
    CapabilityName::FilesystemWatch,
    CapabilityName::PortsConnect,
    CapabilityName::PortsExpose,
    CapabilityName::PortsDomainProxy,
    CapabilityName::TemplatePrepare,
    CapabilityName::TemplateReady,
    CapabilityName::TemplateFreshness,
    CapabilityName::TemplateDataPlaneNone,
    CapabilityName::TemplateDataPlaneEnvdInject,
    CapabilityName::TemplateDataPlaneEnvdVerify,
    CapabilityName::SandboxDataPlaneInit,
    CapabilityName::WarmPoolPrewarm,
    CapabilityName::WarmPoolCheckout,
    CapabilityName::EventsSubscribe,
    CapabilityName::MetricsHost,
    CapabilityName::MetricsGuest,
    CapabilityName::NetworkPolicy,
];

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capability {
    name: CapabilityName,
    status: CapabilityStatus,
}

impl Capability {
    pub fn supported(name: CapabilityName) -> Self {
        Self {
            name,
            status: CapabilityStatus::Supported,
        }
    }

    pub fn unsupported(name: CapabilityName, reason: CapabilityReason) -> Self {
        Self {
            name,
            status: CapabilityStatus::Unsupported(reason),
        }
    }

    pub const fn name(&self) -> &CapabilityName {
        &self.name
    }

    pub const fn status(&self) -> &CapabilityStatus {
        &self.status
    }

    pub const fn is_supported(&self) -> bool {
        matches!(self.status, CapabilityStatus::Supported)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapabilityStatus {
    Supported,
    Unsupported(CapabilityReason),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapabilityReason {
    Permanent { detail: String },
    BuildFeatureGated { feature: String },
    HostPreflightGated { prerequisite: String },
    RuntimeStateGated { state: String },
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    entries: BTreeMap<CapabilityName, CapabilityStatus>,
}

impl Capabilities {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn all_unsupported(reason: &CapabilityReason) -> Self {
        let entries = ALL_CAPABILITY_NAMES
            .iter()
            .cloned()
            .map(|name| (name, CapabilityStatus::Unsupported(reason.clone())))
            .collect();
        Self { entries }
    }

    pub fn with_supported(mut self, name: CapabilityName) -> Self {
        self.entries.insert(name, CapabilityStatus::Supported);
        self
    }

    pub fn with_unsupported(mut self, name: CapabilityName, reason: CapabilityReason) -> Self {
        self.entries
            .insert(name, CapabilityStatus::Unsupported(reason));
        self
    }

    pub fn status(&self, name: &CapabilityName) -> CapabilityStatus {
        self.entries.get(name).cloned().unwrap_or_else(|| {
            CapabilityStatus::Unsupported(CapabilityReason::Permanent {
                detail: "capability was not reported by backend".to_owned(),
            })
        })
    }

    pub fn supports(&self, name: &CapabilityName) -> bool {
        matches!(self.status(name), CapabilityStatus::Supported)
    }

    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        ALL_CAPABILITY_NAMES.iter().cloned().map(|name| {
            let status = self.status(&name);
            Capability { name, status }
        })
    }
}

pub type CapabilitySet = Capabilities;

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityRequirement {
    name: CapabilityName,
}

impl CapabilityRequirement {
    pub const fn new(name: CapabilityName) -> Self {
        Self { name }
    }

    pub const fn name(&self) -> &CapabilityName {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::{Capabilities, CapabilityName, CapabilityReason, CapabilityStatus};

    #[test]
    fn unknown_capability_is_unsupported() {
        let capabilities = Capabilities::new();
        assert!(matches!(
            capabilities.status(&CapabilityName::NetworkPolicy),
            CapabilityStatus::Unsupported(_)
        ));
    }

    #[test]
    fn explicit_support_is_visible() {
        let capabilities =
            Capabilities::new().with_supported(CapabilityName::TemplateDataPlaneEnvdVerify);
        assert!(capabilities.supports(&CapabilityName::TemplateDataPlaneEnvdVerify));
        assert!(!capabilities.supports(&CapabilityName::TemplateDataPlaneEnvdInject));
    }

    #[test]
    fn reason_classification_is_structured() {
        let capabilities = Capabilities::new().with_unsupported(
            CapabilityName::PauseResume,
            CapabilityReason::BuildFeatureGated {
                feature: "snapshot".to_owned(),
            },
        );
        assert!(matches!(
            capabilities.status(&CapabilityName::PauseResume),
            CapabilityStatus::Unsupported(CapabilityReason::BuildFeatureGated { .. })
        ));
    }
}
