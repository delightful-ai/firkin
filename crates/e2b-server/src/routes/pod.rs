//! pod — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::routes::sandbox::encode_path_segment;
/// Route builder for product pod control-plane endpoints.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PodRoutes;
impl PodRoutes {
    /// Route for creating a pod.
    #[must_use]
    pub const fn create() -> &'static str {
        "/pods"
    }
    /// Route for listing pods.
    #[must_use]
    pub const fn list() -> &'static str {
        "/pods"
    }
    /// Route for getting a pod.
    #[must_use]
    pub fn get(pod_id: &str) -> String {
        format!("/pods/{}", encode_path_segment(pod_id))
    }
    /// Route for deleting a pod.
    #[must_use]
    pub fn delete(pod_id: &str) -> String {
        Self::get(pod_id)
    }
    /// Route for adding a container to a pod.
    #[must_use]
    pub fn add_container(pod_id: &str) -> String {
        format!("/pods/{}/containers", encode_path_segment(pod_id))
    }
    /// Route for removing a container from a pod.
    #[must_use]
    pub fn delete_container(pod_id: &str, container_name: &str) -> String {
        format!(
            "/pods/{}/containers/{}",
            encode_path_segment(pod_id),
            encode_path_segment(container_name)
        )
    }
    /// Route for waiting for a container and collecting output.
    #[must_use]
    pub fn wait_container(pod_id: &str, container_name: &str) -> String {
        format!("{}/wait", Self::delete_container(pod_id, container_name))
    }
}
