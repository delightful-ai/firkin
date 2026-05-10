//! control plane — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::HttpBody;
#[allow(unused_imports)]
use crate::auth_error_to_http;
#[allow(unused_imports)]
use crate::backend::LocalRuntimeBackend;
#[allow(unused_imports)]
use crate::full_body;
#[allow(unused_imports)]
use crate::header_matches;
#[allow(unused_imports)]
use crate::lifecycle::{LifecycleClock, LifecycleScheduler, SystemLifecycleClock};
#[allow(unused_imports)]
use crate::state::LocalRuntimeStateStoreError;
#[allow(unused_imports)]
use firkin_e2b_contract::BackendError;
#[allow(unused_imports)]
use firkin_e2b_contract::RuntimeAdapter;
#[allow(unused_imports)]
use firkin_e2b_wire::ControlPlaneResponse;
#[allow(unused_imports)]
use firkin_e2b_wire::MethodNotAllowed;
#[allow(unused_imports)]
use firkin_e2b_wire::{ControlPlaneMethod, ControlPlaneRequest};
use http_body_util::BodyExt as _;
#[allow(unused_imports)]
use hyper::StatusCode;
#[allow(unused_imports)]
use hyper::body::Incoming;
#[allow(unused_imports)]
use hyper::header::CONTENT_TYPE;
#[allow(unused_imports)]
use hyper::header::HOST;
#[allow(unused_imports)]
use hyper::header::HeaderName;
#[allow(unused_imports)]
use hyper::header::HeaderValue;
#[allow(unused_imports)]
use hyper::service::service_fn;
#[allow(unused_imports)]
use hyper::{Request, Response};
#[allow(unused_imports)]
use hyper_util::rt::TokioIo;
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::convert::Infallible;
#[allow(unused_imports)]
use std::net::SocketAddr;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use tokio::net::TcpListener;
#[allow(unused_imports)]
use tokio::sync::Mutex;
/// Error returned by the transport-neutral E2B control-plane dispatcher.
#[derive(Debug, thiserror::Error)]
pub enum ControlPlaneError {
    /// Request route is malformed.
    #[error("bad request: {0}")]
    BadRequest(String),
    /// Route does not exist.
    #[error("route not found: {0}")]
    NotFound(String),
    /// Method is not allowed for a matched route.
    #[error("method not allowed")]
    MethodNotAllowed,
    /// Backend registry or runtime error.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// JSON encode/decode error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// State-store persistence error.
    #[error(transparent)]
    StateStore(#[from] LocalRuntimeStateStoreError),
}
impl From<MethodNotAllowed> for ControlPlaneError {
    fn from(_: MethodNotAllowed) -> Self {
        Self::MethodNotAllowed
    }
}
impl ControlPlaneError {
    /// Return the HTTP status code for this error.
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::BadRequest(_) | Self::Json(_) => 400,
            Self::NotFound(_) | Self::Backend(BackendError::NotFound(_)) => 404,
            Self::MethodNotAllowed => 405,
            Self::Backend(BackendError::AlreadyExists(_)) => 409,
            Self::Backend(BackendError::Runtime(_)) | Self::StateStore(_) => 500,
        }
    }
}
/// Hyper-backed HTTP server for the local E2B control-plane backend.
#[derive(Clone, Debug)]
pub struct ControlPlaneHttpServer<A> {
    #[allow(missing_docs)]
    pub backend: Arc<Mutex<LocalRuntimeBackend<A>>>,
    pub(crate) state_path: Option<PathBuf>,
    api_key: Option<String>,
}
impl<A> ControlPlaneHttpServer<A>
where
    A: RuntimeAdapter,
{
    /// Construct an HTTP server around a local runtime backend.
    #[must_use]
    pub fn new(backend: LocalRuntimeBackend<A>) -> Self {
        Self {
            backend: Arc::new(Mutex::new(backend)),
            state_path: None,
            api_key: None,
        }
    }
    /// Construct an HTTP server that saves control-plane state after
    /// successful mutating requests.
    #[must_use]
    pub fn new_persistent(backend: LocalRuntimeBackend<A>, path: impl Into<PathBuf>) -> Self {
        Self {
            backend: Arc::new(Mutex::new(backend)),
            state_path: Some(path.into()),
            api_key: None,
        }
    }
    /// Require SDK `x-api-key` control-plane authentication.
    #[must_use]
    pub fn with_required_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }
    /// Construct a persistent HTTP server from an existing state file.
    ///
    /// # Errors
    ///
    /// Returns filesystem or JSON decode errors.
    pub fn load_persistent(
        adapter: A,
        path: impl Into<PathBuf>,
    ) -> Result<Self, LocalRuntimeStateStoreError> {
        let path = path.into();
        Ok(Self::new_persistent(
            LocalRuntimeBackend::load_state_json(adapter, &path)?,
            path,
        ))
    }
    /// Return the shared backend used by the server.
    #[must_use]
    pub fn backend(&self) -> Arc<Mutex<LocalRuntimeBackend<A>>> {
        Arc::clone(&self.backend)
    }
    /// Return the configured state path, when persistence is enabled.
    #[must_use]
    pub fn state_path(&self) -> Option<&Path> {
        self.state_path.as_deref()
    }
    /// Construct a lifecycle scheduler for the shared backend.
    #[must_use]
    pub fn lifecycle_scheduler(&self, interval: Duration) -> LifecycleScheduler<A> {
        LifecycleScheduler {
            backend: Arc::clone(&self.backend),
            state_path: self.state_path.clone(),
            clock: SystemLifecycleClock,
            interval,
        }
    }
    /// Construct a lifecycle scheduler with a caller-provided clock.
    #[must_use]
    pub fn lifecycle_scheduler_with_clock<C>(
        &self,
        interval: Duration,
        clock: C,
    ) -> LifecycleScheduler<A, C>
    where
        C: LifecycleClock,
    {
        LifecycleScheduler {
            backend: Arc::clone(&self.backend),
            state_path: self.state_path.clone(),
            clock,
            interval,
        }
    }
    /// Serve control-plane HTTP requests on a listener.
    ///
    /// # Errors
    ///
    /// Returns listener accept errors.
    pub async fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (stream, _) = listener.accept().await?;
            let backend = Arc::clone(&self.backend);
            let state_path = self.state_path.clone();
            let api_key = self.api_key.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let backend = Arc::clone(&backend);
                    let state_path = state_path.clone();
                    let api_key = api_key.clone();
                    async move {
                        Ok::<_, Infallible>(
                            handle_http_request(
                                backend,
                                state_path.as_deref(),
                                api_key.as_deref(),
                                request,
                            )
                            .await,
                        )
                    }
                });
                let stream = TokioIo::new(stream);
                if let Err(_error) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(stream, service)
                    .with_upgrades()
                    .await
                {}
            });
        }
    }
    /// Bind and serve control-plane HTTP requests.
    ///
    /// # Errors
    ///
    /// Returns listener bind or accept errors.
    pub async fn bind_and_serve(
        addr: SocketAddr,
        backend: LocalRuntimeBackend<A>,
    ) -> std::io::Result<()> {
        let listener = TcpListener::bind(addr).await?;
        Self::new(backend).serve(listener).await
    }
}
async fn handle_http_request<A>(
    backend: Arc<Mutex<LocalRuntimeBackend<A>>>,
    state_path: Option<&Path>,
    api_key: Option<&str>,
    request: Request<Incoming>,
) -> Response<HttpBody>
where
    A: RuntimeAdapter,
{
    if let Some(api_key) = api_key
        && !header_matches(request.headers().get("x-api-key"), api_key)
    {
        return auth_error_to_http("missing or invalid x-api-key");
    }
    let converted = convert_http_request(request).await;
    let result = match converted {
        Ok(request) => {
            if let Some(result) = LocalRuntimeBackend::handle_concurrent_control_plane_create(
                Arc::clone(&backend),
                request.clone(),
            )
            .await
            {
                if result.is_ok()
                    && request.method.mutates_state()
                    && let Some(path) = state_path
                    && let Err(error) = backend.lock().await.save_state_json(path)
                {
                    return control_plane_error_to_http(&ControlPlaneError::StateStore(error));
                }
                return match result {
                    Ok(response) => control_plane_to_http(response),
                    Err(error) => control_plane_error_to_http(&error),
                };
            }
            let mut backend = backend.lock().await;
            let mutates_state = request.method.mutates_state();
            let result = backend.handle_control_plane(request).await;
            if result.is_ok()
                && mutates_state
                && let Some(path) = state_path
                && let Err(error) = backend.save_state_json(path)
            {
                return control_plane_error_to_http(&ControlPlaneError::StateStore(error));
            }
            result
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(response) => control_plane_to_http(response),
        Err(error) => control_plane_error_to_http(&error),
    }
}
async fn convert_http_request(
    request: Request<Incoming>,
) -> Result<ControlPlaneRequest, ControlPlaneError> {
    let (parts, body) = request.into_parts();
    let method = ControlPlaneMethod::from_str(parts.method.as_str())?;
    let origin = parts
        .headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .map(|host| format!("http://{host}"));
    let path = parts
        .uri
        .path_and_query()
        .map_or_else(|| "/".to_owned(), ToString::to_string);
    let bytes = body.collect().await.map_err(|error| {
        ControlPlaneError::BadRequest(format!("failed to read request body: {error}"))
    })?;
    let bytes = bytes.to_bytes();
    let body = (!bytes.is_empty()).then(|| bytes.to_vec());
    Ok(ControlPlaneRequest {
        method,
        path,
        body,
        origin,
    })
}
fn control_plane_to_http(response: ControlPlaneResponse) -> Response<HttpBody> {
    let mut builder = Response::builder().status(response.status);
    for (name, value) in response.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            builder = builder.header(name, value);
        }
    }
    builder
        .body(full_body(response.body))
        .expect("response builder accepts status/header values validated above")
}
fn control_plane_error_to_http(error: &ControlPlaneError) -> Response<HttpBody> {
    let status = StatusCode::from_u16(error.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = serde_json::to_vec(&BTreeMap::from([("message", error.to_string())]))
        .expect("string error body serializes");
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(full_body(body))
        .expect("static error response is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use firkin_e2b_contract::{
        BackendError, PausedSandbox, PortTarget, PreparedTemplate, RuntimeAdapter,
        RuntimeCapabilitySet, RuntimeSandbox, RuntimeTemplateBuild, SandboxRuntimeConfig,
        SnapshotRef, StartSandboxRequest,
    };
    use firkin_e2b_wire::{SandboxCreateRequest, SandboxLogs, SandboxMetric, TemplateBuildRequest};
    use firkin_types::SandboxNetworkPolicy;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::Barrier;

    #[derive(Clone, Debug)]
    struct BarrierStartAdapter {
        barrier: Arc<Barrier>,
        starts: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl RuntimeAdapter for BarrierStartAdapter {
        async fn preflight(&self) -> Result<RuntimeCapabilitySet, BackendError> {
            Ok(RuntimeCapabilitySet::default())
        }

        async fn prepare_template(
            &self,
            _request: TemplateBuildRequest,
        ) -> Result<PreparedTemplate, BackendError> {
            Err(BackendError::Runtime(
                "barrier adapter does not prepare templates".to_owned(),
            ))
        }

        async fn build_template(
            &self,
            _request: RuntimeTemplateBuild,
        ) -> Result<PreparedTemplate, BackendError> {
            Err(BackendError::Runtime(
                "barrier adapter does not build templates".to_owned(),
            ))
        }

        async fn start(
            &self,
            _request: StartSandboxRequest,
        ) -> Result<RuntimeSandbox, BackendError> {
            let index = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
            self.barrier.wait().await;
            Ok(RuntimeSandbox {
                config: SandboxRuntimeConfig {
                    sandbox_id: format!("sbx_concurrent_{index}"),
                    domain: "localhost".to_owned(),
                    envd_version: "test".to_owned(),
                    envd_access_token: None,
                    traffic_access_token: None,
                    started_at: "2026-05-08T00:00:00Z".to_owned(),
                    end_at: "2026-05-08T00:05:00Z".to_owned(),
                    cpu_count: 1,
                    memory_mb: 512,
                },
                exposed_ports: Vec::new(),
            })
        }

        async fn start_followup(
            &self,
            request: StartSandboxRequest,
            _snapshot: firkin_e2b_contract::FollowupSnapshot,
        ) -> Result<RuntimeSandbox, BackendError> {
            self.start(request).await
        }

        async fn stop(&self, _sandbox_id: &str) -> Result<(), BackendError> {
            Ok(())
        }

        async fn pause(&self, sandbox_id: &str) -> Result<PausedSandbox, BackendError> {
            Err(BackendError::Runtime(format!(
                "barrier adapter does not pause {sandbox_id}"
            )))
        }

        async fn resume(&self, paused: PausedSandbox) -> Result<RuntimeSandbox, BackendError> {
            Err(BackendError::Runtime(format!(
                "barrier adapter does not resume {}",
                paused.sandbox_id
            )))
        }

        async fn snapshot(
            &self,
            sandbox_id: &str,
            _name: Option<String>,
        ) -> Result<SnapshotRef, BackendError> {
            Err(BackendError::Runtime(format!(
                "barrier adapter does not snapshot {sandbox_id}"
            )))
        }

        async fn metrics(&self, _sandbox_id: &str) -> Result<Vec<SandboxMetric>, BackendError> {
            Ok(Vec::new())
        }

        async fn logs(&self, _sandbox_id: &str) -> Result<SandboxLogs, BackendError> {
            Ok(SandboxLogs::default())
        }

        async fn apply_network(
            &self,
            _sandbox_id: &str,
            _policy: SandboxNetworkPolicy,
        ) -> Result<(), BackendError> {
            Ok(())
        }

        async fn port_target(
            &self,
            sandbox_id: &str,
            port: u16,
        ) -> Result<PortTarget, BackendError> {
            Err(BackendError::Runtime(format!(
                "barrier adapter has no port target for {sandbox_id}:{port}"
            )))
        }
    }

    #[test]
    fn control_plane_create_requests_do_not_hold_backend_lock_across_start() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build test runtime")
            .block_on(async {
                let adapter = BarrierStartAdapter {
                    barrier: Arc::new(Barrier::new(2)),
                    starts: Arc::new(AtomicUsize::new(0)),
                };
                let server = ControlPlaneHttpServer::new(LocalRuntimeBackend::new(
                    adapter.clone(),
                    "2026-05-08T00:00:00Z",
                ));
                let listener = TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("bind test control plane");
                let addr = listener.local_addr().expect("read listener addr");
                let task = tokio::spawn(server.serve(listener));
                let body = serde_json::to_vec(&SandboxCreateRequest::default())
                    .expect("encode create request");

                let responses = tokio::time::timeout(Duration::from_secs(1), async {
                    let first = tokio::spawn(post_sandbox_create(addr, body.clone()));
                    let second = tokio::spawn(post_sandbox_create(addr, body));
                    [
                        first.await.expect("first create task joins"),
                        second.await.expect("second create task joins"),
                    ]
                })
                .await
                .expect("concurrent creates should both reach adapter start");

                task.abort();
                assert_eq!(adapter.starts.load(Ordering::SeqCst), 2);
                assert!(
                    responses
                        .into_iter()
                        .all(|response| response.starts_with("HTTP/1.1 200"))
                );
            });
    }

    async fn post_sandbox_create(addr: SocketAddr, body: Vec<u8>) -> String {
        let mut stream = tokio::net::TcpStream::connect(addr)
            .await
            .expect("connect test control plane");
        let request = format!(
            "POST /sandboxes HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("write test request head");
        stream.write_all(&body).await.expect("write test body");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .await
            .expect("read test response");
        response
    }
}
