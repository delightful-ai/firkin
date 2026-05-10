//! sandbox — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
use std::fmt::Write as _;
/// Route builder for E2B sandbox control-plane endpoints.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SandboxRoutes;
impl SandboxRoutes {
    /// Route for creating a sandbox.
    #[must_use]
    pub const fn create() -> &'static str {
        "/sandboxes"
    }
    /// Route for creating a follow-up sandbox from a continuation snapshot.
    #[must_use]
    pub const fn followup() -> &'static str {
        "/sandboxes/followups"
    }
    /// Route for listing sandboxes.
    #[must_use]
    pub const fn list_v2() -> &'static str {
        "/v2/sandboxes"
    }
    /// Route for querying metrics for multiple sandboxes.
    #[must_use]
    pub const fn metrics_many() -> &'static str {
        "/sandboxes/metrics"
    }
    /// Route for listing snapshots.
    #[must_use]
    pub const fn snapshots() -> &'static str {
        "/snapshots"
    }
    /// Route for getting a sandbox.
    #[must_use]
    pub fn get(sandbox_id: &str) -> String {
        format!("/sandboxes/{}", encode_path_segment(sandbox_id))
    }
    /// Route for deleting a sandbox.
    #[must_use]
    pub fn delete(sandbox_id: &str) -> String {
        Self::get(sandbox_id)
    }
    /// Route for connecting a paused sandbox.
    #[must_use]
    pub fn connect(sandbox_id: &str) -> String {
        format!("/sandboxes/{}/connect", encode_path_segment(sandbox_id))
    }
    /// Route for resuming a paused sandbox.
    #[must_use]
    pub fn resume(sandbox_id: &str) -> String {
        format!("/sandboxes/{}/resume", encode_path_segment(sandbox_id))
    }
    /// Route for pausing a sandbox.
    #[must_use]
    pub fn pause(sandbox_id: &str) -> String {
        format!("/sandboxes/{}/pause", encode_path_segment(sandbox_id))
    }
    /// Route for setting sandbox timeout.
    #[must_use]
    pub fn timeout(sandbox_id: &str) -> String {
        format!("/sandboxes/{}/timeout", encode_path_segment(sandbox_id))
    }
    /// Route for refreshing sandbox lifetime.
    #[must_use]
    pub fn refresh(sandbox_id: &str) -> String {
        format!("/sandboxes/{}/refreshes", encode_path_segment(sandbox_id))
    }
    /// Route for sandbox logs.
    #[must_use]
    pub fn logs_v2(sandbox_id: &str) -> String {
        format!("/v2/sandboxes/{}/logs", encode_path_segment(sandbox_id))
    }
    /// Route for sandbox-local metrics.
    #[must_use]
    pub fn metrics(sandbox_id: &str) -> String {
        format!("/sandboxes/{}/metrics", encode_path_segment(sandbox_id))
    }
    /// Route for creating a sandbox snapshot.
    #[must_use]
    pub fn create_snapshot(sandbox_id: &str) -> String {
        format!("/sandboxes/{}/snapshots", encode_path_segment(sandbox_id))
    }
    /// Route for deleting a snapshot-backed template id.
    #[must_use]
    pub fn delete_snapshot(snapshot_id: &str) -> String {
        format!("/templates/{}", encode_path_segment(snapshot_id))
    }
}
/// Percent-encode a path segment with uppercase hex escapes.
#[must_use]
pub fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}
