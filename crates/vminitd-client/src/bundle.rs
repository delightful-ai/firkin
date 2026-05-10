//! bundle — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::error::{Result, VminitdError};
#[allow(unused_imports)]
use crate::pb;
#[allow(unused_imports)]
use firkin_oci::Spec;
#[allow(unused_imports)]
use firkin_types::ContainerId;
/// vminitd's implicit bundle path for a container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerBundle {
    pub(crate) path: String,
    rootfs_path: String,
    config_json_path: String,
}
impl ContainerBundle {
    /// Construct the bundle paths vminitd derives from a container ID.
    #[must_use]
    pub fn for_id(id: &ContainerId) -> Self {
        let path = format!("/run/container/{id}");
        let rootfs_path = format!("{path}/rootfs");
        let config_json_path = format!("{path}/config.json");
        Self {
            path,
            rootfs_path,
            config_json_path,
        }
    }
    /// The bundle directory path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
    /// The rootfs mount path below the bundle.
    #[must_use]
    pub fn rootfs_path(&self) -> &str {
        &self.rootfs_path
    }
    /// The smoke-test config path below the bundle.
    #[must_use]
    pub fn config_json_path(&self) -> &str {
        &self.config_json_path
    }
    /// Build the `Mkdir` request for vminitd's implicit rootfs path.
    #[must_use]
    pub fn mkdir_rootfs_request(&self, perms: u32) -> pb::MkdirRequest {
        pb::MkdirRequest {
            path: self.rootfs_path.clone(),
            all: true,
            perms,
        }
    }
    /// Build the `Mount` request for a rootfs block device.
    #[must_use]
    pub fn mount_rootfs_request<I, S>(
        &self,
        source: impl Into<String>,
        options: I,
    ) -> pb::MountRequest
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        pb::MountRequest {
            r#type: "ext4".into(),
            source: source.into(),
            destination: self.rootfs_path.clone(),
            options: options.into_iter().map(Into::into).collect(),
        }
    }
    /// Build the `WriteFile(config.json)` request used before `CreateProcess`.
    ///
    /// # Errors
    ///
    /// Returns [`VminitdError::EncodeSpec`] if the OCI runtime spec cannot be
    /// serialized to JSON.
    pub fn write_config_request(&self, spec: &Spec) -> Result<pb::WriteFileRequest> {
        let data =
            serde_json::to_vec(spec).map_err(|source| VminitdError::EncodeSpec { source })?;
        Ok(pb::WriteFileRequest {
            path: self.config_json_path.clone(),
            data,
            mode: 0o644,
            flags: Some(pb::write_file_request::WriteFileFlags {
                create_parent_dirs: true,
                append: false,
                create_if_missing: true,
            }),
        })
    }
}
