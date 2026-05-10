//! pods — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::collections::BTreeSet;
/// Pod lifecycle state exposed by the local control plane.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(private_interfaces)]
pub enum PodState {
    /// Pod has at least one running container.
    Running,
    /// Pod was stopped and should no longer accept container operations.
    Stopped,
}
/// Container lifecycle state exposed inside a pod.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PodContainerState {
    /// Container is running inside the pod VM.
    Running,
    /// Container was stopped or removed.
    Stopped,
}
/// Request to mount a pod `emptyDir` into one container.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodVolumeMountRequest {
    /// Pod `emptyDir` name.
    pub name: String,
    /// Container path where the volume is mounted.
    pub path: String,
    /// Whether the mount is read-only.
    #[serde(rename = "readOnly", default)]
    pub read_only: bool,
}
/// Pod-local `emptyDir` declaration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodEmptyDir {
    /// Volume name referenced by container mounts.
    pub name: String,
}
/// Host disk image format used for the product pod store.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PodStoreImageFormat {
    /// Raw local disk image.
    #[default]
    Raw,
    /// Apple Sparse Image Format local disk image.
    Asif,
}
/// Pod-store trim policy.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PodTrimPolicy {
    /// Do not run guest fstrim automatically.
    None,
    /// Run guest fstrim after container removal.
    #[default]
    OnRemove,
    /// Run guest fstrim when the pod stops.
    OnStop,
    /// Only run guest fstrim through an explicit future API.
    Manual,
}
/// Product pod-store configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodStoreOptions {
    /// Pod-store virtual disk size in bytes.
    #[serde(rename = "sizeBytes", default = "default_pod_store_size_bytes")]
    pub size_bytes: u64,
    /// Host disk image format.
    #[serde(rename = "imageFormat", default)]
    pub image_format: PodStoreImageFormat,
    /// Guest trim policy.
    #[serde(rename = "trimPolicy", default)]
    pub trim_policy: PodTrimPolicy,
    /// Whether OCI templates are materialized once and shared through overlay.
    #[serde(rename = "sharedRootfs", default = "default_shared_rootfs")]
    pub shared_rootfs: bool,
}
impl Default for PodStoreOptions {
    fn default() -> Self {
        Self {
            size_bytes: default_pod_store_size_bytes(),
            image_format: PodStoreImageFormat::Raw,
            trim_policy: PodTrimPolicy::OnRemove,
            shared_rootfs: true,
        }
    }
}
/// Request to create one container inside a product pod.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct PodContainerCreateRequest {
    /// Container name, unique within the pod.
    pub name: String,
    /// Template ID used as the container rootfs source.
    #[serde(rename = "templateID")]
    pub template_id: String,
    /// Optional command override.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    /// Environment variables for the container process.
    #[serde(
        rename = "envVars",
        default,
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub env_vars: BTreeMap<String, String>,
    /// Pod `emptyDir` mounts requested by the container.
    #[serde(
        rename = "emptyDirMounts",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub empty_dir_mounts: Vec<PodVolumeMountRequest>,
    /// Whether the runtime should capture stdout/stderr for a later wait.
    #[serde(rename = "captureOutput", default)]
    pub capture_output: bool,
}
/// Request to create a product pod.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct PodCreateRequest {
    /// Optional caller-provided pod ID. The runtime assigns one when omitted.
    #[serde(rename = "podID", default, skip_serializing_if = "Option::is_none")]
    pub pod_id: Option<String>,
    /// Timeout in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// Caller metadata.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    /// Pod-local `emptyDir` volumes.
    #[serde(rename = "emptyDirs", default, skip_serializing_if = "Vec::is_empty")]
    pub empty_dirs: Vec<PodEmptyDir>,
    /// Pod-store storage/reclaim options.
    #[serde(rename = "podStore", default)]
    pub pod_store: PodStoreOptions,
    /// Initial containers to start in the pod.
    pub containers: Vec<PodContainerCreateRequest>,
}
impl PodCreateRequest {
    /// Validate names and volume references before runtime work starts.
    ///
    /// # Errors
    ///
    /// Returns a string describing the first invalid declaration.
    pub fn validate(&self) -> Result<(), String> {
        if self.containers.is_empty() {
            return Err("pod requires at least one container".to_owned());
        }
        if self.pod_store.size_bytes == 0 {
            return Err("podStore.sizeBytes must be greater than zero".to_owned());
        }
        let mut empty_dirs = BTreeSet::new();
        for volume in &self.empty_dirs {
            if !empty_dirs.insert(volume.name.as_str()) {
                return Err(format!("pod declares duplicate emptyDir `{}`", volume.name));
            }
        }
        let mut containers = BTreeSet::new();
        for container in &self.containers {
            if !containers.insert(container.name.as_str()) {
                return Err(format!(
                    "pod declares duplicate container `{}`",
                    container.name
                ));
            }
            validate_container_mounts(&empty_dirs, container)?;
        }
        Ok(())
    }
}
const fn default_pod_store_size_bytes() -> u64 {
    7 * 1024 * 1024 * 1024
}
const fn default_shared_rootfs() -> bool {
    true
}
/// Container info returned by pod routes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct PodContainerInfo {
    /// Container name.
    pub name: String,
    /// Template ID used to create the container.
    #[serde(rename = "templateID")]
    pub template_id: String,
    /// Current container state.
    pub state: PodContainerState,
    /// Pod `emptyDir` mounts visible in this container.
    #[serde(
        rename = "emptyDirMounts",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub empty_dir_mounts: Vec<PodVolumeMountRequest>,
}
impl PodContainerInfo {
    /// Construct running container info from a create request.
    #[must_use]
    pub fn running(request: &PodContainerCreateRequest) -> Self {
        Self {
            name: request.name.clone(),
            template_id: request.template_id.clone(),
            state: PodContainerState::Running,
            empty_dir_mounts: request.empty_dir_mounts.clone(),
        }
    }
}
/// Captured output from a completed product pod container.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodContainerOutput {
    /// Captured stdout bytes.
    pub stdout: Vec<u8>,
    /// Captured stderr bytes.
    pub stderr: Vec<u8>,
    /// Process exit code, or 128 when the runtime reports no normal exit code.
    #[serde(rename = "exitCode")]
    pub exit_code: i32,
}
impl PodContainerOutput {
    /// Construct captured pod container output.
    #[must_use]
    pub const fn new(stdout: Vec<u8>, stderr: Vec<u8>, exit_code: i32) -> Self {
        Self {
            stdout,
            stderr,
            exit_code,
        }
    }

    /// Return true when the process exited successfully.
    #[must_use]
    pub const fn success(&self) -> bool {
        self.exit_code == 0
    }
}
/// Pod info returned by product pod routes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct PodInfo {
    /// Pod ID.
    #[serde(rename = "podID")]
    pub pod_id: String,
    /// Caller metadata.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    /// RFC3339 start timestamp.
    #[serde(rename = "startedAt")]
    pub started_at: String,
    /// RFC3339 timeout/deadline timestamp.
    #[serde(rename = "endAt")]
    pub end_at: String,
    /// Current pod state.
    pub state: PodState,
    /// Pod-local `emptyDir` volumes.
    #[serde(rename = "emptyDirs", default, skip_serializing_if = "Vec::is_empty")]
    pub empty_dirs: Vec<PodEmptyDir>,
    /// Containers currently registered in the pod.
    pub containers: Vec<PodContainerInfo>,
}
/// Validate one add-container request against the pod's declared emptyDirs.
///
/// # Errors
///
/// Returns a string describing the first invalid mount.
#[allow(private_interfaces)]
pub fn validate_pod_container_request(
    empty_dirs: &[PodEmptyDir],
    container: &PodContainerCreateRequest,
) -> Result<(), String> {
    let names = empty_dirs
        .iter()
        .map(|volume| volume.name.as_str())
        .collect::<BTreeSet<_>>();
    validate_container_mounts(&names, container)
}
fn validate_container_mounts(
    empty_dirs: &BTreeSet<&str>,
    container: &PodContainerCreateRequest,
) -> Result<(), String> {
    for mount in &container.empty_dir_mounts {
        if !empty_dirs.contains(mount.name.as_str()) {
            return Err(format!(
                "container `{}` references unknown emptyDir `{}`",
                container.name, mount.name
            ));
        }
    }
    Ok(())
}
