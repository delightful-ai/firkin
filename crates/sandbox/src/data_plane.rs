use time::OffsetDateTime;

use crate::error::{InvalidSpec, InvalidSpecReason, Result};

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataPlaneSpec {
    None,
    Envd(EnvdDataPlaneSpec),
}

impl DataPlaneSpec {
    pub const fn none() -> Self {
        Self::None
    }

    pub fn envd() -> EnvdDataPlaneSpec {
        EnvdDataPlaneSpec::default()
    }
}

impl From<EnvdDataPlaneSpec> for DataPlaneSpec {
    fn from(spec: EnvdDataPlaneSpec) -> Self {
        Self::Envd(spec)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataPlaneKind {
    None,
    Envd,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataPlaneProvisioning {
    Inject,
    AlreadyPresent,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataPlaneInfo {
    None,
    Envd(PreparedEnvdDataPlane),
}

impl DataPlaneInfo {
    pub const fn kind(&self) -> DataPlaneKind {
        match self {
            Self::None => DataPlaneKind::None,
            Self::Envd(_) => DataPlaneKind::Envd,
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvdDataPlaneSpec {
    provisioning: DataPlaneProvisioning,
    version: Option<String>,
    source: Option<EnvdSource>,
    arch: Option<GuestArch>,
    port: ReservedPort,
    startup: EnvdStartup,
    init_mode: EnvdInitMode,
    default_user: Option<String>,
    health: EnvdHealthProbe,
}

impl Default for EnvdDataPlaneSpec {
    fn default() -> Self {
        Self {
            provisioning: DataPlaneProvisioning::Inject,
            version: None,
            source: None,
            arch: None,
            port: ReservedPort::ENVD_DEFAULT,
            startup: EnvdStartup::Supervised,
            init_mode: EnvdInitMode::NonFirecracker,
            default_user: None,
            health: EnvdHealthProbe::default(),
        }
    }
}

impl EnvdDataPlaneSpec {
    pub fn inject(mut self) -> DataPlaneSpec {
        self.provisioning = DataPlaneProvisioning::Inject;
        DataPlaneSpec::Envd(self)
    }

    pub fn already_present(mut self) -> DataPlaneSpec {
        self.provisioning = DataPlaneProvisioning::AlreadyPresent;
        DataPlaneSpec::Envd(self)
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn source(mut self, source: EnvdSource) -> Self {
        self.source = Some(source);
        self
    }

    pub fn arch(mut self, arch: GuestArch) -> Self {
        self.arch = Some(arch);
        self
    }

    pub fn port(mut self, port: u16) -> Result<Self> {
        self.port = ReservedPort::new(port)?;
        Ok(self)
    }

    pub fn startup(mut self, startup: EnvdStartup) -> Self {
        self.startup = startup;
        self
    }

    pub fn init_mode(mut self, mode: EnvdInitMode) -> Self {
        self.init_mode = mode;
        self
    }

    pub fn default_user(mut self, user: impl Into<String>) -> Self {
        self.default_user = Some(user.into());
        self
    }

    pub fn health(mut self, probe: EnvdHealthProbe) -> Self {
        self.health = probe;
        self
    }

    pub const fn provisioning(&self) -> DataPlaneProvisioning {
        self.provisioning
    }

    pub const fn port_value(&self) -> ReservedPort {
        self.port
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedEnvdDataPlane {
    pub version: String,
    pub commit: Option<String>,
    pub sha256: String,
    pub arch: GuestArch,
    pub port: ReservedPort,
    pub startup: EnvdStartup,
    pub init_mode: EnvdInitMode,
    pub default_user: Option<String>,
    pub health: EnvdHealthProbe,
    pub health_checked_at: OffsetDateTime,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvdSource {
    PinnedLayer { reference: String, sha256: String },
    PinnedArtifact { url: String, sha256: String },
    VendoredBuild { commit: String, sha256: String },
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EnvdStartup {
    #[default]
    Supervised,
    ImageEntrypointWrapped,
    AlreadyRunning,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EnvdInitMode {
    #[default]
    NonFirecracker,
    ExplicitMetadataProvider,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvdHealthProbe {
    path: String,
    expected_status: u16,
}

impl EnvdHealthProbe {
    pub fn new(path: impl Into<String>, expected_status: u16) -> Result<Self> {
        let path = path.into();
        if !path.starts_with('/') {
            return Err(InvalidSpec::new(
                "configure envd health probe",
                InvalidSpecReason::InvalidDataPlane("health path must be absolute".to_owned()),
            )
            .into());
        }
        Ok(Self {
            path,
            expected_status,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub const fn expected_status(&self) -> u16 {
        self.expected_status
    }
}

impl Default for EnvdHealthProbe {
    fn default() -> Self {
        Self {
            path: "/health".to_owned(),
            expected_status: 200,
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestArch {
    Aarch64,
    X86_64,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReservedPort(u16);

impl ReservedPort {
    pub const ENVD_DEFAULT: Self = Self(49_983);

    pub const fn get(self) -> u16 {
        self.0
    }

    pub fn new(port: u16) -> Result<Self> {
        if port == 0 {
            return Err(InvalidSpec::new(
                "configure reserved port",
                InvalidSpecReason::InvalidPort(port),
            )
            .into());
        }
        Ok(Self(port))
    }
}

impl std::fmt::Display for ReservedPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{DataPlaneProvisioning, DataPlaneSpec, EnvdHealthProbe};

    #[test]
    fn envd_inject_is_default() {
        let spec = DataPlaneSpec::envd().inject();
        let DataPlaneSpec::Envd(envd) = spec else {
            panic!("envd spec");
        };
        assert_eq!(envd.provisioning(), DataPlaneProvisioning::Inject);
        assert_eq!(envd.port_value().get(), 49_983);
    }

    #[test]
    fn envd_already_present_is_explicit() {
        let spec = DataPlaneSpec::envd().already_present();
        let DataPlaneSpec::Envd(envd) = spec else {
            panic!("envd spec");
        };
        assert_eq!(envd.provisioning(), DataPlaneProvisioning::AlreadyPresent);
    }

    #[test]
    fn health_probe_rejects_relative_path() {
        assert!(EnvdHealthProbe::new("health", 200).is_err());
    }
}
