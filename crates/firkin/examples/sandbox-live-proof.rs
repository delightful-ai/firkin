//! Live proof harness for the public sandbox facade.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use firkin::sandbox::apple_vz::{AppleVzBackend, SingleNodeConfig};
use firkin::sandbox::{
    Command, DataPlaneSpec, Error as SandboxError, PauseOptions, PreparedTemplate, RestoreOptions,
    Runtime, Sandbox, SandboxPath, SandboxSpec, SnapshotRef, TemplatePrepareFailure, TemplateSpec,
    WarmPoolSpec,
};

#[derive(Clone, Debug)]
struct Step {
    name: &'static str,
    status: StepStatus,
    detail: String,
    elapsed_ms: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepStatus {
    Pass,
    Fail,
}

#[derive(Clone, Debug)]
struct ProofConfig {
    json_path: PathBuf,
    html_path: PathBuf,
    state_root: PathBuf,
    image: String,
}

impl ProofConfig {
    fn from_env() -> std::io::Result<Self> {
        let output_dir = std::env::var_os("FIRKIN_SANDBOX_PROOF_DIR").map_or_else(
            || PathBuf::from("target/firkin-live-evidence"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&output_dir)?;
        let state_root = std::env::var_os("FIRKIN_SANDBOX_STATE_ROOT")
            .map_or_else(|| output_dir.join("sandbox-state"), PathBuf::from);
        let image = std::env::var("FIRKIN_SANDBOX_LIVE_IMAGE")
            .unwrap_or_else(|_| "docker.io/library/busybox:latest".to_owned());
        Ok(Self {
            json_path: output_dir.join("sandbox-public-surface-proof.json"),
            html_path: output_dir.join("sandbox-public-surface-proof.html"),
            state_root,
            image,
        })
    }
}

impl StepStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
        }
    }
}

fn record_step<T, E>(
    steps: &mut Vec<Step>,
    name: &'static str,
    started: Instant,
    result: Result<T, E>,
    detail: impl FnOnce(&T) -> String,
) -> Option<T>
where
    E: std::fmt::Display,
{
    match result {
        Ok(value) => {
            steps.push(Step {
                name,
                status: StepStatus::Pass,
                detail: detail(&value),
                elapsed_ms: started.elapsed().as_millis(),
            });
            Some(value)
        }
        Err(error) => {
            steps.push(Step {
                name,
                status: StepStatus::Fail,
                detail: error.to_string(),
                elapsed_ms: started.elapsed().as_millis(),
            });
            None
        }
    }
}

fn push_step(
    steps: &mut Vec<Step>,
    name: &'static str,
    started: Instant,
    status: StepStatus,
    detail: impl Into<String>,
) {
    steps.push(Step {
        name,
        status,
        detail: detail.into(),
        elapsed_ms: started.elapsed().as_millis(),
    });
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ProofConfig::from_env()?;
    let backend =
        AppleVzBackend::from_config(SingleNodeConfig::new(&config.state_root, "cube.localhost"));
    let runtime = Runtime::build(backend).await?;
    let mut steps = Vec::new();
    run_sandbox_proof(&runtime, &config.image, &mut steps).await?;
    write_proof(&config.json_path, &config.html_path, &config.image, &steps)?;
    println!("{}", config.json_path.display());
    println!("{}", config.html_path.display());
    Ok(())
}

async fn run_sandbox_proof(
    runtime: &Runtime,
    image: &str,
    steps: &mut Vec<Step>,
) -> Result<(), Box<dyn std::error::Error>> {
    prove_default_envd_refusal(runtime, image, steps).await;
    let Some(template) = prepare_no_data_plane_template(runtime, image, steps).await else {
        return Ok(());
    };
    prewarm_template(runtime, &template, steps).await;
    let Some(sandbox) = create_sandbox(runtime, &template, steps).await else {
        cleanup_warm_pool(runtime).await;
        return Ok(());
    };
    prove_exec_stdout(
        &sandbox,
        "exec command",
        "printf sandbox-exec-ok",
        "sandbox-exec-ok",
        steps,
    )
    .await;
    prove_filesystem_round_trip(&sandbox, steps).await?;
    prove_unsupported_pause(&sandbox, steps).await;
    let Some(snapshot) = capture_snapshot(&sandbox, steps).await else {
        let _ = sandbox.stop().await;
        cleanup_warm_pool(runtime).await;
        return Ok(());
    };
    stop_sandbox(&sandbox, steps).await;
    let Some(restored) = restore_snapshot(runtime, snapshot.clone(), steps).await else {
        let _ = runtime.snapshots().delete(snapshot.id()).await;
        cleanup_warm_pool(runtime).await;
        return Ok(());
    };
    prove_exec_stdout(
        &restored,
        "exec after restore",
        "printf sandbox-restore-ok",
        "sandbox-restore-ok",
        steps,
    )
    .await;
    let _ = restored.stop().await;
    let _ = runtime.snapshots().delete(snapshot.id()).await;
    cleanup_warm_pool(runtime).await;
    Ok(())
}

async fn prove_default_envd_refusal(runtime: &Runtime, image: &str, steps: &mut Vec<Step>) {
    let started = Instant::now();
    let default_prepare = runtime
        .templates()
        .prepare(TemplateSpec::oci(image.to_owned()))
        .await;
    match default_prepare {
        Err(SandboxError::TemplatePrepareFailure(TemplatePrepareFailure::EnvdMissing {
            ..
        })) => {
            push_step(
                steps,
                "prepare default envd refusal",
                started,
                StepStatus::Pass,
                "default OCI data plane requires explicit envd handling",
            );
        }
        Err(error) => push_step(
            steps,
            "prepare default envd refusal",
            started,
            StepStatus::Fail,
            format!("unexpected error: {error}"),
        ),
        Ok(_) => push_step(
            steps,
            "prepare default envd refusal",
            started,
            StepStatus::Fail,
            "default envd-inject template unexpectedly prepared",
        ),
    }
}

async fn prepare_no_data_plane_template(
    runtime: &Runtime,
    image: &str,
    steps: &mut Vec<Step>,
) -> Option<PreparedTemplate> {
    let started = Instant::now();
    record_step(
        steps,
        "prepare no data plane template",
        started,
        runtime
            .templates()
            .prepare(TemplateSpec::oci(image.to_owned()).data_plane(DataPlaneSpec::none()))
            .await,
        |template| format!("template_id={}", template.id()),
    )
}

async fn prewarm_template(runtime: &Runtime, template: &PreparedTemplate, steps: &mut Vec<Step>) {
    let started = Instant::now();
    let _ = record_step(
        steps,
        "prewarm template",
        started,
        runtime
            .warm_pool()
            .prewarm(template, WarmPoolSpec::depth(1))
            .await,
        |report| format!("created={} ready={}", report.created, report.ready),
    );
}

async fn create_sandbox(
    runtime: &Runtime,
    template: &PreparedTemplate,
    steps: &mut Vec<Step>,
) -> Option<Sandbox> {
    let started = Instant::now();
    record_step(
        steps,
        "create sandbox",
        started,
        runtime
            .sandboxes()
            .create(SandboxSpec::from_template(template))
            .await,
        |sandbox| format!("sandbox_id={}", sandbox.id()),
    )
}

async fn prove_exec_stdout(
    sandbox: &Sandbox,
    name: &'static str,
    command: &'static str,
    expected_stdout: &'static str,
    steps: &mut Vec<Step>,
) {
    let started = Instant::now();
    let _ = record_step(
        steps,
        name,
        started,
        sandbox
            .exec(Command::shell(command))
            .await
            .and_then(|output| {
                if output.stdout.as_ref() == expected_stdout.as_bytes() {
                    Ok(output)
                } else {
                    Err(unexpected_stdout(name, sandbox, output.stdout.as_ref()))
                }
            }),
        |output| format!("stdout={}", String::from_utf8_lossy(&output.stdout)),
    );
}

async fn prove_filesystem_round_trip(
    sandbox: &Sandbox,
    steps: &mut Vec<Step>,
) -> Result<(), SandboxError> {
    let proof_path = SandboxPath::new("/tmp/firkin-sandbox-proof.txt")?;
    let started = Instant::now();
    let fs_round_trip = async {
        sandbox
            .fs()
            .write(proof_path.clone(), b"proof-data".to_vec())
            .await?;
        sandbox.fs().read(proof_path.clone()).await
    }
    .await
    .map(|bytes| bytes.to_vec());
    let _ = record_step(
        steps,
        "filesystem write/read",
        started,
        fs_round_trip,
        |bytes| format!("bytes={}", String::from_utf8_lossy(bytes)),
    );
    Ok(())
}

async fn prove_unsupported_pause(sandbox: &Sandbox, steps: &mut Vec<Step>) {
    let started = Instant::now();
    let pause = sandbox.pause(PauseOptions::default()).await;
    match pause {
        Err(SandboxError::UnsupportedCapability(error)) => push_step(
            steps,
            "unsupported pause refusal",
            started,
            StepStatus::Pass,
            format!("capability={}", error.capability),
        ),
        Err(error) => push_step(
            steps,
            "unsupported pause refusal",
            started,
            StepStatus::Fail,
            format!("unexpected error: {error}"),
        ),
        Ok(_) => push_step(
            steps,
            "unsupported pause refusal",
            started,
            StepStatus::Fail,
            "pause unexpectedly succeeded",
        ),
    }
}

async fn capture_snapshot(sandbox: &Sandbox, steps: &mut Vec<Step>) -> Option<SnapshotRef> {
    let started = Instant::now();
    record_step(
        steps,
        "capture snapshot",
        started,
        sandbox
            .snapshot(format!("sandbox-proof-{}", std::process::id()))
            .await,
        |snapshot| format!("snapshot_id={}", snapshot.id()),
    )
}

async fn stop_sandbox(sandbox: &Sandbox, steps: &mut Vec<Step>) {
    let started = Instant::now();
    let _ = record_step(steps, "stop sandbox", started, sandbox.stop().await, |()| {
        "stopped".to_owned()
    });
}

async fn restore_snapshot(
    runtime: &Runtime,
    snapshot: SnapshotRef,
    steps: &mut Vec<Step>,
) -> Option<Sandbox> {
    let started = Instant::now();
    record_step(
        steps,
        "restore snapshot",
        started,
        runtime
            .snapshots()
            .restore(snapshot.clone(), RestoreOptions::default())
            .await,
        |sandbox| format!("sandbox_id={}", sandbox.id()),
    )
}

fn unexpected_stdout(operation: &'static str, sandbox: &Sandbox, stdout: &[u8]) -> SandboxError {
    SandboxError::ProcessFailure(firkin::sandbox::ProcessFailure {
        operation,
        sandbox_id: Some(sandbox.id().clone()),
        process_id: None,
        reason: format!("unexpected stdout `{}`", String::from_utf8_lossy(stdout)),
        retry: firkin::sandbox::RetryClass::Unknown,
    })
}

async fn cleanup_warm_pool(runtime: &Runtime) {
    if let Ok(status) = runtime.warm_pool().status().await {
        for entry in status.entries {
            let _ = runtime.warm_pool().evict(entry.key, entry.ready).await;
        }
    }
}

fn write_proof(
    json_path: &Path,
    html_path: &Path,
    image: &str,
    steps: &[Step],
) -> std::io::Result<()> {
    let generated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let passed = steps.iter().all(|step| step.status == StepStatus::Pass);
    let mut json = String::new();
    writeln!(&mut json, "{{").unwrap();
    writeln!(&mut json, "  \"generated_at_unix\": {generated_at},").unwrap();
    writeln!(&mut json, "  \"image\": \"{}\",", json_escape(image)).unwrap();
    writeln!(
        &mut json,
        "  \"overall\": \"{}\",",
        if passed { "pass" } else { "fail" }
    )
    .unwrap();
    writeln!(&mut json, "  \"residual_risks\": [").unwrap();
    writeln!(
        &mut json,
        "    \"live Apple/VZ availability, signing, disk pressure, OCI registry access, and low sample count can affect this proof\""
    )
    .unwrap();
    writeln!(&mut json, "  ],").unwrap();
    writeln!(&mut json, "  \"steps\": [").unwrap();
    for (index, step) in steps.iter().enumerate() {
        writeln!(
            &mut json,
            "    {{\"name\":\"{}\",\"status\":\"{}\",\"elapsed_ms\":{},\"detail\":\"{}\"}}{}",
            json_escape(step.name),
            step.status.as_str(),
            step.elapsed_ms,
            json_escape(&step.detail),
            if index + 1 == steps.len() { "" } else { "," }
        )
        .unwrap();
    }
    writeln!(&mut json, "  ]").unwrap();
    writeln!(&mut json, "}}").unwrap();
    std::fs::write(json_path, json)?;

    let mut html = String::new();
    writeln!(
        &mut html,
        "<!doctype html><meta charset=\"utf-8\"><title>Firkin Sandbox Proof</title>"
    )
    .unwrap();
    writeln!(
        &mut html,
        "<style>body{{font-family:system-ui,sans-serif;margin:32px;max-width:1100px}}table{{border-collapse:collapse;width:100%}}td,th{{border:1px solid #ccc;padding:6px;text-align:left}}.pass{{color:#075c31}}.fail{{color:#9b1c1c}}</style>"
    )
    .unwrap();
    writeln!(
        &mut html,
        "<h1>Firkin Sandbox Public Surface Proof</h1><p>Image: <code>{}</code></p><p>Overall: <strong class=\"{}\">{}</strong></p>",
        html_escape(image),
        if passed { "pass" } else { "fail" },
        if passed { "pass" } else { "fail" }
    )
    .unwrap();
    writeln!(&mut html, "<table><thead><tr><th>Step</th><th>Status</th><th>Elapsed ms</th><th>Detail</th></tr></thead><tbody>").unwrap();
    for step in steps {
        writeln!(
            &mut html,
            "<tr><td>{}</td><td class=\"{}\">{}</td><td>{}</td><td><code>{}</code></td></tr>",
            html_escape(step.name),
            step.status.as_str(),
            step.status.as_str(),
            step.elapsed_ms,
            html_escape(&step.detail)
        )
        .unwrap();
    }
    writeln!(&mut html, "</tbody></table><p>Residual risk: live Apple/VZ availability, signing, disk pressure, OCI registry access, and low sample count.</p>").unwrap();
    std::fs::write(html_path, html)
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            _ => vec![ch],
        })
        .collect()
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
