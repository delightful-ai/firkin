//! Single-node backend orchestration tests.

#![cfg(any())]
// Scaffolding: this runtime integration test depends on firkin-single-node.
// It is disabled during the crates.io bootstrap to keep the publish graph acyclic.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use firkin_single_node::{
    CommandOutput, CommandRequest, Result, RuntimeCreatedSandbox, RuntimeDriver,
    RuntimeSnapshotRef, SandboxResources, SingleNodeBackend, SingleNodeConfig,
    SingleNodeCreateRequest, SingleNodeScheduler, SingleNodeSchedulerConfig, SnapshotRecord,
    StateStore, TemplateMetadata,
};
use firkin_types::Size;

#[derive(Default)]
struct RecordingDriver {
    creates: Mutex<Vec<String>>,
    deletes: Mutex<Vec<String>>,
    snapshots: Mutex<Vec<String>>,
    commands: Mutex<Vec<(String, String)>>,
    template_starts: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl RuntimeDriver for RecordingDriver {
    fn runtime_sandbox_exists(&self, sandbox_id: &str) -> Result<bool> {
        Ok(self
            .creates
            .lock()
            .unwrap()
            .iter()
            .any(|created| created == sandbox_id))
    }

    async fn create(&self, request: SingleNodeCreateRequest) -> Result<RuntimeCreatedSandbox> {
        self.creates
            .lock()
            .unwrap()
            .push(request.sandbox_id().to_owned());
        Ok(RuntimeCreatedSandbox {
            sandbox_id: request.sandbox_id().to_owned(),
            client_id: "client-1".to_owned(),
            envd_access_token: Some("envd-1".to_owned()),
            traffic_access_token: None,
        })
    }

    async fn delete(&self, sandbox_id: &str) -> Result<()> {
        self.deletes.lock().unwrap().push(sandbox_id.to_owned());
        Ok(())
    }

    async fn snapshot(&self, sandbox_id: &str, name: Option<String>) -> Result<RuntimeSnapshotRef> {
        self.snapshots.lock().unwrap().push(sandbox_id.to_owned());
        Ok(RuntimeSnapshotRef {
            snapshot_id: name.unwrap_or_else(|| "snapshot-1".to_owned()),
            source_sandbox_id: Some(sandbox_id.to_owned()),
            location: Some("/tmp/snapshot-1.vzstate".to_owned()),
            staging_dir: Some("/tmp/sandbox-1".to_owned()),
            machine_identifier: Some(vec![1, 2, 3]),
            network_macs: Some(vec!["aa:bb:cc:dd:ee:ff".to_owned()]),
        })
    }

    async fn run_command(
        &self,
        sandbox_id: &str,
        request: CommandRequest,
    ) -> Result<CommandOutput> {
        self.commands
            .lock()
            .unwrap()
            .push((sandbox_id.to_owned(), request.command().to_owned()));
        Ok(CommandOutput::new(b"ok\n".to_vec(), Vec::new(), 0))
    }

    async fn start_template_command(
        &self,
        sandbox_id: &str,
        command: String,
        _envs: std::collections::HashMap<String, String>,
    ) -> Result<()> {
        self.template_starts
            .lock()
            .unwrap()
            .push((sandbox_id.to_owned(), command));
        Ok(())
    }
}

#[tokio::test]
async fn single_node_backend_orchestrates_create_command_snapshot_and_delete() {
    let driver = Arc::new(RecordingDriver::default());
    let config = SingleNodeConfig::new("/tmp/firkin-single-node-test", "cube.localhost");
    let state = StateStore::new();
    let backend = SingleNodeBackend::with_driver_and_state(config, driver.clone(), state);
    let resources = SandboxResources::new(2, Size::gib(1));

    let created = backend
        .create(SingleNodeCreateRequest::new("sandbox-1", "base", resources))
        .await
        .unwrap();
    assert_eq!(created.sandbox_id, "sandbox-1");
    assert_eq!(backend.scheduler().admitted_len().unwrap(), 1);
    assert_eq!(backend.state().load_active().unwrap().len(), 1);

    let output = backend
        .run_command("sandbox-1", CommandRequest::new("echo ok"))
        .await
        .unwrap();
    assert_eq!(output.stdout(), b"ok\n");

    let snapshot = backend
        .snapshot("sandbox-1", Some("checkpoint".to_owned()))
        .await
        .unwrap();
    assert_eq!(snapshot.snapshot_id, "checkpoint");
    assert_eq!(backend.state().load_snapshots().unwrap().len(), 1);

    backend.delete("sandbox-1").await.unwrap();
    assert_eq!(backend.scheduler().admitted_len().unwrap(), 0);
    assert!(backend.state().load_active().unwrap().is_empty());
    assert_eq!(
        driver.deletes.lock().unwrap().as_slice(),
        &["sandbox-1".to_owned()]
    );
}

#[tokio::test]
async fn single_node_backend_updates_deadline_and_records_template_metadata() {
    let driver = Arc::new(RecordingDriver::default());
    let config = SingleNodeConfig::new("/tmp/firkin-single-node-test", "cube.localhost");
    let backend =
        SingleNodeBackend::with_driver_and_state(config, driver.clone(), StateStore::new());
    let resources = SandboxResources::new(2, Size::gib(1));

    backend
        .create(SingleNodeCreateRequest::new("sandbox-1", "base", resources))
        .await
        .unwrap();
    backend
        .update_deadline("sandbox-1", std::time::Duration::from_mins(15))
        .unwrap();
    let active = backend.active_records().unwrap();
    assert_eq!(active.len(), 1);
    assert!(active[0].end_at_unix_seconds > active[0].started_at_unix_seconds);

    backend
        .start_template_command(
            "sandbox-1",
            "python /workspace/server.py".to_owned(),
            std::collections::HashMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        driver.template_starts.lock().unwrap().as_slice(),
        &[(
            "sandbox-1".to_owned(),
            "python /workspace/server.py".to_owned()
        )]
    );

    let metadata = TemplateMetadata::default().with_start_command("python /workspace/server.py");
    backend
        .snapshot_with_metadata("sandbox-1", Some("checkpoint".to_owned()), metadata.clone())
        .await
        .unwrap();
    let snapshots = backend.snapshot_records().unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].template_metadata, metadata);
}

#[tokio::test]
async fn single_node_backend_runs_template_start_and_ready_commands() {
    let driver = Arc::new(RecordingDriver::default());
    let config = SingleNodeConfig::new("/tmp/firkin-single-node-template-start", "cube.localhost");
    let backend =
        SingleNodeBackend::with_driver_and_state(config, driver.clone(), StateStore::new());
    let resources = SandboxResources::new(2, Size::gib(1));
    backend
        .create(SingleNodeCreateRequest::new("sandbox-1", "base", resources))
        .await
        .unwrap();
    let metadata = TemplateMetadata::default()
        .with_start_command("python /workspace/server.py")
        .with_ready_command("test -f /tmp/ready");

    backend
        .run_template_start(
            "sandbox-1",
            &metadata,
            std::collections::HashMap::from([("AGENT_MODE".to_owned(), "inspect".to_owned())]),
        )
        .await
        .unwrap();

    assert_eq!(
        driver.template_starts.lock().unwrap().as_slice(),
        &[(
            "sandbox-1".to_owned(),
            "python /workspace/server.py".to_owned()
        )]
    );
    assert_eq!(
        driver.commands.lock().unwrap().as_slice(),
        &[("sandbox-1".to_owned(), "test -f /tmp/ready".to_owned())]
    );
}

#[tokio::test]
async fn single_node_backend_restores_existing_active_state() {
    let driver = Arc::new(RecordingDriver::default());
    driver.creates.lock().unwrap().push("sandbox-1".to_owned());
    let resources = SandboxResources::new(2, Size::gib(1));
    let state = StateStore::from_records(
        vec![firkin_single_node::ActiveSessionRecord {
            sandbox_id: "sandbox-1".to_owned(),
            template_id: "base".to_owned(),
            client_id: "client-1".to_owned(),
            envd_access_token: None,
            started_at_unix_seconds: 1,
            end_at_unix_seconds: i64::MAX,
            resources,
            runtime_attached: true,
        }],
        Vec::<SnapshotRecord>::new(),
        std::collections::HashMap::default(),
    );
    let config = SingleNodeConfig::new("/tmp/firkin-single-node-test", "cube.localhost");

    let backend = SingleNodeBackend::with_driver_and_state(config, driver, state);
    let session = backend.get("sandbox-1").unwrap();
    assert_eq!(session.sandbox_id(), "sandbox-1");
}

#[tokio::test]
async fn single_node_backend_can_use_injected_scheduler() {
    let driver = Arc::new(RecordingDriver::default());
    let config = SingleNodeConfig::new("/tmp/firkin-single-node-test", "cube.localhost");
    let scheduler = Arc::new(SingleNodeScheduler::new(SingleNodeSchedulerConfig::new(
        1,
        SandboxResources::new(2, Size::gib(1)),
    )));
    let backend = SingleNodeBackend::with_driver_scheduler_and_state(
        config,
        driver,
        scheduler,
        StateStore::new(),
    );

    backend
        .create(SingleNodeCreateRequest::new(
            "sandbox-1",
            "base",
            SandboxResources::new(2, Size::gib(1)),
        ))
        .await
        .unwrap();
    let rejected = backend
        .create(SingleNodeCreateRequest::new(
            "sandbox-2",
            "base",
            SandboxResources::new(1, Size::gib(1)),
        ))
        .await;
    assert!(rejected.is_err());
}

#[tokio::test]
async fn single_node_backend_with_injected_scheduler_persists_file_state() {
    let temp = tempfile::tempdir().unwrap();
    let driver = Arc::new(RecordingDriver::default());
    let scheduler = Arc::new(SingleNodeScheduler::new(SingleNodeSchedulerConfig::new(
        1,
        SandboxResources::new(2, Size::gib(1)),
    )));
    let config = SingleNodeConfig::new(temp.path(), "cube.localhost");
    let backend =
        SingleNodeBackend::with_driver_scheduler(config.clone(), driver.clone(), scheduler)
            .unwrap();

    backend
        .create(SingleNodeCreateRequest::new(
            "sandbox-1",
            "base",
            SandboxResources::new(2, Size::gib(1)),
        ))
        .await
        .unwrap();

    let recovered = SingleNodeBackend::with_driver_scheduler(
        config,
        driver,
        Arc::new(SingleNodeScheduler::new(SingleNodeSchedulerConfig::new(
            1,
            SandboxResources::new(2, Size::gib(1)),
        ))),
    )
    .unwrap();
    assert_eq!(
        recovered.get("sandbox-1").unwrap().sandbox_id(),
        "sandbox-1"
    );
}
