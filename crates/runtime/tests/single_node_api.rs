//! Public API tests for the single-node runtime facade.

#![cfg(any())]
// Scaffolding: this runtime integration test depends on firkin-single-node.
// It is disabled during the crates.io bootstrap to keep the publish graph acyclic.

use std::path::PathBuf;
use std::time::Duration;

use firkin_single_node::{
    AppleVzLocalRuntimeDriver, CommandOutput, CommandRequest, Error, RuntimeDriver,
    SandboxResources, SingleNodeBackend, SingleNodeConfig, SingleNodeCreateRequest,
    SingleNodeRuntimeMode, TemplateMetadata,
};
use firkin_types::Size;

#[test]
fn single_node_api_models_default_cube_mapping() {
    let config = SingleNodeConfig::new(PathBuf::from("/tmp/firkin-single-node"), "cube.localhost");
    assert_eq!(config.root(), PathBuf::from("/tmp/firkin-single-node"));
    assert_eq!(config.domain(), "cube.localhost");
    assert_eq!(config.minimum_free_disk(), Size::gib(10));
    assert_eq!(config.warm_pool_minimum_free_disk(), Size::gib(20));

    let resources = SandboxResources::new(2, Size::gib(1));
    let request = SingleNodeCreateRequest::new("sandbox-id", "template-id", resources)
        .with_timeout(Duration::from_secs(30));
    assert_eq!(
        request.runtime_mode(),
        SingleNodeRuntimeMode::SingleVmBackedContainer
    );
    assert_eq!(request.resources(), &resources);
    assert_eq!(request.timeout(), Some(Duration::from_secs(30)));
    let request = request.with_env("AGENT_MODE", "inspect");
    assert_eq!(
        request.env().get("AGENT_MODE").map(String::as_str),
        Some("inspect")
    );

    let metadata = TemplateMetadata::default()
        .with_env("PATH", "/usr/bin")
        .with_start_command("python server.py")
        .with_ready_command("curl -sf localhost:8000/health");
    assert_eq!(
        metadata.envs().get("PATH").map(String::as_str),
        Some("/usr/bin")
    );
    assert_eq!(metadata.start_command(), Some("python server.py"));
    assert_eq!(
        metadata.ready_command(),
        Some("curl -sf localhost:8000/health")
    );

    let command = CommandRequest::new("echo hi")
        .with_cwd("/workspace")
        .with_env("A", "B")
        .with_stdin(b"input".to_vec());
    assert_eq!(command.command(), "echo hi");
    assert_eq!(command.cwd(), Some("/workspace"));
    assert_eq!(command.env().get("A").map(String::as_str), Some("B"));
    assert_eq!(command.stdin(), Some(&b"input"[..]));

    let output = CommandOutput::new(Vec::new(), Vec::new(), 0);
    assert!(output.success());

    let backend = SingleNodeBackend::from_config(config);
    assert_eq!(backend.config().domain(), "cube.localhost");
}

#[tokio::test]
async fn apple_vz_local_driver_rejects_unknown_short_template_ids_before_launch() {
    let driver = AppleVzLocalRuntimeDriver::new("docker.io/library/debian:bookworm-slim");
    let resources = SandboxResources::new(2, Size::gib(1));
    let error = driver
        .create(SingleNodeCreateRequest::new(
            "sandbox-id",
            "missing-template",
            resources,
        ))
        .await
        .expect_err("unknown short template id should fail before VZ launch");

    assert!(matches!(
        error,
        Error::SnapshotNotFound(message) if message.contains("missing-template")
    ));
}

#[tokio::test]
async fn apple_vz_local_driver_validates_snapshot_ids_before_filesystem_delete() {
    let driver = AppleVzLocalRuntimeDriver::new("docker.io/library/debian:bookworm-slim");
    let error = driver
        .delete_snapshot("../escape")
        .await
        .expect_err("invalid snapshot id should fail before touching disk");

    assert!(matches!(
        error,
        Error::InvalidRequest(message) if message.contains("../escape")
    ));
}

#[tokio::test]
async fn apple_vz_local_driver_reports_missing_sandbox_for_command() {
    let driver = AppleVzLocalRuntimeDriver::new("docker.io/library/debian:bookworm-slim");
    let error = driver
        .run_command("missing-sandbox", CommandRequest::new("echo hi"))
        .await
        .expect_err("command should require an attached runtime sandbox");

    assert!(matches!(
        error,
        Error::SandboxNotFound(message) if message.contains("missing-sandbox")
    ));
}
