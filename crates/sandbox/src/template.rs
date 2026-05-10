use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use time::OffsetDateTime;

use crate::backend::{BoxBackend, TemplateControl};
use crate::data_plane::{DataPlaneInfo, DataPlaneSpec};
use crate::error::Result;
use crate::ids::TemplateId;

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateSpec {
    id: Option<TemplateId>,
    source: TemplateSource,
    data_plane: DataPlaneSpec,
    env: BTreeMap<String, String>,
    setup: Vec<TemplateCommand>,
    start: Option<TemplateCommand>,
    ready: Vec<TemplateReadyProbe>,
    timeout: Option<Duration>,
}

impl TemplateSpec {
    pub fn oci(reference: impl Into<String>) -> Self {
        Self {
            id: None,
            source: TemplateSource::Oci(OciTemplateSource {
                reference: reference.into(),
            }),
            data_plane: DataPlaneSpec::envd().inject(),
            env: BTreeMap::new(),
            setup: Vec::new(),
            start: None,
            ready: Vec::new(),
            timeout: None,
        }
    }

    pub fn git(url: impl Into<String>) -> Self {
        Self::new(TemplateSource::Git(GitTemplateSource { url: url.into() }))
    }

    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self::new(TemplateSource::Local(LocalTemplateSource {
            path: path.into(),
        }))
    }

    fn new(source: TemplateSource) -> Self {
        Self {
            id: None,
            source,
            data_plane: DataPlaneSpec::envd().inject(),
            env: BTreeMap::new(),
            setup: Vec::new(),
            start: None,
            ready: Vec::new(),
            timeout: None,
        }
    }

    pub fn id(mut self, id: TemplateId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn data_plane(mut self, data_plane: impl Into<DataPlaneSpec>) -> Self {
        self.data_plane = data_plane.into();
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn setup(mut self, command: impl Into<String>) -> Self {
        self.setup.push(TemplateCommand::shell(command));
        self
    }

    pub fn start(mut self, command: impl Into<String>) -> Self {
        self.start = Some(TemplateCommand::shell(command));
        self
    }

    pub fn ready(mut self, command: impl Into<String>) -> Self {
        self.ready.push(TemplateReadyProbe::command(command));
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub const fn id_ref(&self) -> Option<&TemplateId> {
        self.id.as_ref()
    }

    pub const fn source(&self) -> &TemplateSource {
        &self.source
    }

    pub const fn data_plane_ref(&self) -> &DataPlaneSpec {
        &self.data_plane
    }

    pub fn env_vars(&self) -> &BTreeMap<String, String> {
        &self.env
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplateSource {
    Oci(OciTemplateSource),
    Git(GitTemplateSource),
    Local(LocalTemplateSource),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OciTemplateSource {
    pub reference: String,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitTemplateSource {
    pub url: String,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalTemplateSource {
    pub path: PathBuf,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplateCommand {
    Shell(String),
    Argv { program: String, args: Vec<String> },
}

impl TemplateCommand {
    pub fn shell(command: impl Into<String>) -> Self {
        Self::Shell(command.into())
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplateReadyProbe {
    Command(TemplateCommand),
    EnvdHealth,
}

impl TemplateReadyProbe {
    pub fn command(command: impl Into<String>) -> Self {
        Self::Command(TemplateCommand::shell(command))
    }
}

pub type TemplateEnv = BTreeMap<String, String>;

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateEntrypointPolicy {
    Preserve,
    Wrap,
    Replace,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateUserPolicy {
    Preserve,
    Require(StringPolicy),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringPolicy {
    Present,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedTemplate {
    id: TemplateId,
    source: TemplateSource,
    data_plane: DataPlaneInfo,
    prepared_at: OffsetDateTime,
    artifact: Option<String>,
    metadata: BTreeMap<String, String>,
}

impl PreparedTemplate {
    pub fn new(id: TemplateId, source: TemplateSource, data_plane: DataPlaneInfo) -> Self {
        Self {
            id,
            source,
            data_plane,
            prepared_at: OffsetDateTime::now_utc(),
            artifact: None,
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_artifact(mut self, artifact: impl Into<String>) -> Self {
        self.artifact = Some(artifact.into());
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    pub const fn id(&self) -> &TemplateId {
        &self.id
    }

    pub const fn source(&self) -> &TemplateSource {
        &self.source
    }

    pub const fn data_plane(&self) -> &DataPlaneInfo {
        &self.data_plane
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateInfo {
    pub id: TemplateId,
    pub state: TemplateState,
    pub source: TemplateSource,
    pub data_plane: DataPlaneInfo,
    pub prepared_at: Option<OffsetDateTime>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateState {
    Preparing,
    Ready,
    Failed,
    Deleted,
}

#[derive(Clone)]
pub struct TemplateClient {
    backend: BoxBackend,
}

impl TemplateClient {
    pub(crate) fn new(backend: BoxBackend) -> Self {
        Self { backend }
    }

    fn control(&self) -> &dyn TemplateControl {
        self.backend.templates()
    }

    pub async fn prepare(&self, spec: TemplateSpec) -> Result<PreparedTemplate> {
        self.control().prepare_template(spec).await
    }

    pub async fn get(&self, id: &TemplateId) -> Result<TemplateInfo> {
        self.control().get_template(id).await
    }

    pub async fn list(&self) -> Result<Vec<TemplateInfo>> {
        self.control().list_templates().await
    }

    pub async fn delete(&self, id: &TemplateId) -> Result<()> {
        self.control().delete_template(id).await
    }
}

impl From<Arc<dyn crate::backend::SandboxBackend>> for TemplateClient {
    fn from(backend: Arc<dyn crate::backend::SandboxBackend>) -> Self {
        Self::new(backend)
    }
}

#[cfg(test)]
mod tests {
    use crate::data_plane::{DataPlaneProvisioning, DataPlaneSpec};

    use super::TemplateSpec;

    #[test]
    fn oci_template_defaults_to_envd_injection() {
        let spec = TemplateSpec::oci("docker.io/library/rust:latest");
        let DataPlaneSpec::Envd(envd) = spec.data_plane_ref() else {
            panic!("expected envd");
        };
        assert_eq!(envd.provisioning(), DataPlaneProvisioning::Inject);
    }

    #[test]
    fn no_data_plane_is_explicit() {
        let spec = TemplateSpec::oci("example").data_plane(DataPlaneSpec::none());
        assert_eq!(spec.data_plane_ref(), &DataPlaneSpec::None);
    }
}
