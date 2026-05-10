//! domain proxy — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::HttpBody;
#[allow(unused_imports)]
use crate::backend::{BoxError, LocalRuntimeBackend};
#[allow(unused_imports)]
use crate::control_plane::ControlPlaneHttpServer;
#[allow(unused_imports)]
use crate::full_body;
#[allow(unused_imports)]
use firkin_e2b_contract::PortProxyStream;
#[allow(unused_imports)]
use firkin_e2b_contract::RuntimeAdapter;
#[allow(unused_imports)]
use firkin_types::PortSandboxHost;
#[allow(unused_imports)]
use firkin_types::{ContainerId, Hostname};
use http_body_util::BodyExt as _;
#[allow(unused_imports)]
use http_body_util::Full;
#[allow(unused_imports)]
use hyper::Method;
#[allow(unused_imports)]
use hyper::Uri;
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
use hyper::header::{CONNECTION, UPGRADE};
#[allow(unused_imports)]
use hyper::service::service_fn;
#[allow(unused_imports)]
use hyper::{Request, Response, StatusCode};
#[allow(unused_imports)]
use hyper_util::rt::TokioIo;
#[allow(unused_imports)]
use rustls::ServerConfig;
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::convert::Infallible;
use std::fmt::Write as _;
#[allow(unused_imports)]
use std::io::{Cursor, Read as _, Write as _};
#[allow(unused_imports)]
use std::net::SocketAddr;
#[allow(unused_imports)]
use std::num::NonZeroU16;
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
#[allow(unused_imports)]
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
#[allow(unused_imports)]
use tokio::net::TcpListener;
#[allow(unused_imports)]
use tokio::sync::Mutex;
#[allow(unused_imports)]
use tokio_rustls::TlsAcceptor;
/// Hyper-backed HTTP proxy for E2B `{port}-{sandboxID}.{domain}` traffic.
#[derive(Clone, Debug)]
pub struct DomainProxyHttpServer<A> {
    #[allow(missing_docs)]
    pub backend: Arc<Mutex<LocalRuntimeBackend<A>>>,
    #[allow(missing_docs)]
    pub domain: Hostname,
}
impl<A> DomainProxyHttpServer<A>
where
    A: RuntimeAdapter,
{
    /// Construct an HTTP proxy around a local runtime backend.
    #[must_use]
    pub fn new(backend: Arc<Mutex<LocalRuntimeBackend<A>>>, domain: Hostname) -> Self {
        Self { backend, domain }
    }
    /// Construct a proxy sharing a control-plane server backend.
    #[must_use]
    pub fn from_control_plane(server: &ControlPlaneHttpServer<A>, domain: Hostname) -> Self {
        Self::new(server.backend(), domain)
    }
    /// Return the configured E2B proxy domain.
    #[must_use]
    pub const fn domain(&self) -> &Hostname {
        &self.domain
    }
    /// Serve proxied HTTP requests on a listener.
    ///
    /// # Errors
    ///
    /// Returns listener accept errors.
    pub async fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (stream, _) = listener.accept().await?;
            let backend = Arc::clone(&self.backend);
            let domain = self.domain.clone();
            tokio::spawn(async move {
                serve_domain_proxy_stream(stream, backend, domain).await;
            });
        }
    }
    /// Serve HTTPS proxied HTTP requests on a listener.
    ///
    /// # Errors
    ///
    /// Returns listener accept errors, PEM decode errors, or TLS configuration errors.
    pub async fn serve_tls(
        self,
        listener: TcpListener,
        identity: DomainProxyTlsIdentity,
    ) -> Result<(), DomainProxyTlsError> {
        let acceptor = TlsAcceptor::from(Arc::new(identity.server_config()?));
        loop {
            let (stream, _) = listener.accept().await?;
            let acceptor = acceptor.clone();
            let backend = Arc::clone(&self.backend);
            let domain = self.domain.clone();
            tokio::spawn(async move {
                if let Ok(stream) = acceptor.accept(stream).await {
                    serve_domain_proxy_stream(stream, backend, domain).await;
                }
            });
        }
    }
    /// Bind and serve proxied HTTP requests.
    ///
    /// # Errors
    ///
    /// Returns listener bind or accept errors.
    pub async fn bind_and_serve(
        addr: SocketAddr,
        backend: Arc<Mutex<LocalRuntimeBackend<A>>>,
        domain: Hostname,
    ) -> std::io::Result<()> {
        let listener = TcpListener::bind(addr).await?;
        Self::new(backend, domain).serve(listener).await
    }
}
/// PEM certificate and private-key material for the E2B domain proxy TLS listener.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainProxyTlsIdentity {
    cert_chain_pem: Vec<u8>,
    private_key_pem: Vec<u8>,
}
impl DomainProxyTlsIdentity {
    /// Construct a TLS identity from PEM-encoded certificate chain and private key bytes.
    #[must_use]
    pub fn from_pem(
        cert_chain_pem: impl Into<Vec<u8>>,
        private_key_pem: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            cert_chain_pem: cert_chain_pem.into(),
            private_key_pem: private_key_pem.into(),
        }
    }
    /// Construct a TLS identity from PEM files.
    ///
    /// # Errors
    ///
    /// Returns filesystem errors while reading either file.
    pub fn from_pem_files(cert_path: &Path, key_path: &Path) -> std::io::Result<Self> {
        Ok(Self::from_pem(
            std::fs::read(cert_path)?,
            std::fs::read(key_path)?,
        ))
    }
    fn server_config(&self) -> Result<ServerConfig, DomainProxyTlsError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cert_chain = rustls_pemfile::certs(&mut Cursor::new(&self.cert_chain_pem))
            .collect::<Result<Vec<_>, _>>()?;
        if cert_chain.is_empty() {
            return Err(DomainProxyTlsError::Pem(
                "TLS certificate file did not contain any certificates".to_owned(),
            ));
        }
        let private_key = rustls_pemfile::private_key(&mut Cursor::new(&self.private_key_pem))?
            .ok_or_else(|| {
                DomainProxyTlsError::Pem(
                    "TLS private-key file did not contain a supported key".to_owned(),
                )
            })?;
        let mut config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_chain, private_key)?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(config)
    }
}
/// Errors returned while serving HTTPS E2B domain proxy traffic.
#[derive(Debug, thiserror::Error)]
pub enum DomainProxyTlsError {
    /// Listener or TLS file I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// PEM file content error.
    #[error("pem error: {0}")]
    Pem(String),
    /// TLS configuration error.
    #[error("tls error: {0}")]
    Tls(#[from] rustls::Error),
}
async fn serve_domain_proxy_stream<A, S>(
    stream: S,
    backend: Arc<Mutex<LocalRuntimeBackend<A>>>,
    domain: Hostname,
) where
    A: RuntimeAdapter,
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let service = service_fn(move |request| {
        let backend = Arc::clone(&backend);
        let domain = domain.clone();
        async move { Ok::<_, Infallible>(handle_domain_proxy_request(backend, &domain, request).await) }
    });
    let stream = TokioIo::new(stream);
    if let Err(_error) = hyper::server::conn::http1::Builder::new()
        .serve_connection(stream, service)
        .with_upgrades()
        .await
    {}
}
async fn handle_domain_proxy_request<A>(
    backend: Arc<Mutex<LocalRuntimeBackend<A>>>,
    domain: &Hostname,
    request: Request<Incoming>,
) -> Response<HttpBody>
where
    A: RuntimeAdapter,
{
    let route = match request_route(&request, domain) {
        Ok(route) => route,
        Err(error) => return proxy_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let sandbox_id = route.sandbox_id().as_str().to_owned();
    let adapter = { backend.lock().await.adapter().clone() };
    let target = adapter
        .port_target(sandbox_id.as_str(), route.port().get())
        .await;
    let target = match target {
        Ok(target) => target,
        Err(error) => {
            return proxy_error_to_http(StatusCode::BAD_GATEWAY, &error.to_string());
        }
    };
    proxy_to_stream(request, move || {
        let adapter = adapter.clone();
        async move {
            adapter
                .connect_port_target(sandbox_id.as_str(), target)
                .await
                .map_err(|error| error.to_string())
        }
    })
    .await
}
fn request_route<B>(request: &Request<B>, domain: &Hostname) -> Result<PortSandboxHost, String> {
    match request_host(request).and_then(|host| {
        PortSandboxHost::parse_for_domain(&host, domain).map_err(|error| error.to_string())
    }) {
        Ok(route) => Ok(route),
        Err(host_error) => request_header_route(request, domain).map_err(|header_error| {
            format!("{host_error}; header route unavailable: {header_error}")
        }),
    }
}

fn request_header_route<B>(
    request: &Request<B>,
    domain: &Hostname,
) -> Result<PortSandboxHost, String> {
    let sandbox_id = request
        .headers()
        .get("e2b-sandbox-id")
        .ok_or_else(|| "missing e2b-sandbox-id header".to_owned())?
        .to_str()
        .map_err(|error| format!("invalid e2b-sandbox-id header: {error}"))?;
    let port = request
        .headers()
        .get("e2b-sandbox-port")
        .ok_or_else(|| "missing e2b-sandbox-port header".to_owned())?
        .to_str()
        .map_err(|error| format!("invalid e2b-sandbox-port header: {error}"))?
        .parse::<u16>()
        .ok()
        .and_then(NonZeroU16::new)
        .ok_or_else(|| "invalid e2b-sandbox-port header".to_owned())?;
    let sandbox_id = ContainerId::new(sandbox_id)
        .map_err(|error| format!("invalid e2b-sandbox-id header: {error}"))?;
    Ok(PortSandboxHost::new(port, sandbox_id, domain.clone()))
}

fn request_host<B>(request: &Request<B>) -> Result<String, String> {
    let host = request
        .headers()
        .get(HOST)
        .ok_or_else(|| "missing host header".to_owned())?
        .to_str()
        .map_err(|error| format!("invalid host header: {error}"))?;
    Ok(host
        .rsplit_once(':')
        .and_then(|(name, port)| port.parse::<u16>().ok().map(|_| name))
        .unwrap_or(host)
        .to_owned())
}

async fn proxy_to_stream<C, Fut>(request: Request<Incoming>, connect: C) -> Response<HttpBody>
where
    C: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<PortProxyStream, String>> + Send,
{
    if is_websocket_upgrade(&request) {
        return proxy_websocket_to_stream(request, connect).await;
    }
    if request.method() == Method::CONNECT {
        return proxy_connect_to_stream(request, connect);
    }
    proxy_http_to_stream(request, connect).await
}
async fn proxy_http_to_stream<C, Fut>(request: Request<Incoming>, connect: C) -> Response<HttpBody>
where
    C: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<PortProxyStream, String>> + Send,
{
    let (parts, body) = request.into_parts();
    let body = match body.collect().await {
        Ok(body) => body.to_bytes(),
        Err(error) => {
            return proxy_error_to_http(
                StatusCode::BAD_REQUEST,
                &format!("failed to read proxy request body: {error}"),
            );
        }
    };
    let stream = match connect().await {
        Ok(stream) => stream,
        Err(error) => {
            return proxy_error_to_http(StatusCode::BAD_GATEWAY, &error);
        }
    };
    let (mut sender, connection) =
        match hyper::client::conn::http1::handshake(TokioIo::new(stream)).await {
            Ok(parts) => parts,
            Err(error) => {
                return proxy_error_to_http(
                    StatusCode::BAD_GATEWAY,
                    &format!("failed to open proxy target connection: {error}"),
                );
            }
        };
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut builder = Request::builder().method(parts.method);
    let uri = parts.uri.path_and_query().map_or_else(
        || Uri::from_static("/"),
        |path| {
            path.as_str()
                .parse::<Uri>()
                .expect("request path-and-query is a valid relative URI")
        },
    );
    builder = builder.uri(uri);
    for (name, value) in &parts.headers {
        builder = builder.header(name, value);
    }
    let upstream = match builder.body(Full::new(body)) {
        Ok(request) => request,
        Err(error) => {
            return proxy_error_to_http(
                StatusCode::BAD_REQUEST,
                &format!("failed to build proxy request: {error}"),
            );
        }
    };
    let response = match sender.send_request(upstream).await {
        Ok(response) => response,
        Err(error) => {
            return proxy_error_to_http(
                StatusCode::BAD_GATEWAY,
                &format!("proxy target request failed: {error}"),
            );
        }
    };
    let (parts, body) = response.into_parts();
    let mut builder = Response::builder().status(parts.status);
    for (name, value) in &parts.headers {
        builder = builder.header(name, value);
    }
    builder
        .body(
            body.map_err(|error| -> BoxError { Box::new(error) })
                .boxed(),
        )
        .expect("proxy target response status/header values were already validated")
}
async fn proxy_websocket_to_stream<C, Fut>(
    mut request: Request<Incoming>,
    connect: C,
) -> Response<HttpBody>
where
    C: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<PortProxyStream, String>> + Send,
{
    let request_head = match websocket_request_head(&request) {
        Ok(head) => head,
        Err(error) => return proxy_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let on_upgrade = hyper::upgrade::on(&mut request);
    let mut target = match connect().await {
        Ok(stream) => stream,
        Err(error) => {
            return proxy_error_to_http(StatusCode::BAD_GATEWAY, &error);
        }
    };
    if let Err(error) = target.write_all(request_head.as_bytes()).await {
        return proxy_error_to_http(
            StatusCode::BAD_GATEWAY,
            &format!("failed to write websocket upgrade request: {error}"),
        );
    }
    let (status, headers, buffered) = match read_http_response_head(&mut target).await {
        Ok(response) => response,
        Err(error) => {
            return proxy_error_to_http(
                StatusCode::BAD_GATEWAY,
                &format!("failed to read websocket upgrade response: {error}"),
            );
        }
    };
    if status != StatusCode::SWITCHING_PROTOCOLS {
        return proxy_error_to_http(
            StatusCode::BAD_GATEWAY,
            &format!("websocket target rejected upgrade with status {status}"),
        );
    }
    tokio::spawn(async move {
        let Ok(upgraded) = on_upgrade.await else {
            return;
        };
        let mut upgraded = TokioIo::new(upgraded);
        if !buffered.is_empty() && upgraded.write_all(&buffered).await.is_err() {
            return;
        }
        let _ = tokio::io::copy_bidirectional(&mut upgraded, &mut target).await;
    });
    let mut builder = Response::builder().status(status);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    builder
        .body(full_body(Vec::new()))
        .expect("websocket upgrade response status/header values were already validated")
}
fn is_websocket_upgrade(request: &Request<Incoming>) -> bool {
    request.method() == Method::GET
        && header_has_token(request.headers().get(CONNECTION), "upgrade")
        && header_eq(request.headers().get(UPGRADE), "websocket")
        && request.headers().contains_key("sec-websocket-key")
}
fn websocket_request_head(request: &Request<Incoming>) -> Result<String, String> {
    if !is_websocket_upgrade(request) {
        return Err("request is not a websocket upgrade".to_owned());
    }
    let target = request
        .uri()
        .path_and_query()
        .map_or("/", hyper::http::uri::PathAndQuery::as_str);
    let mut head = format!("GET {target} HTTP/1.1\r\n");
    for (name, value) in request.headers() {
        let value = value
            .to_str()
            .map_err(|error| format!("invalid websocket header {name}: {error}"))?;
        write!(&mut head, "{name}: {value}\r\n").expect("writing to String cannot fail");
    }
    head.push_str("\r\n");
    Ok(head)
}
async fn read_http_response_head<S>(
    stream: &mut S,
) -> std::io::Result<(StatusCode, Vec<(HeaderName, HeaderValue)>, Vec<u8>)>
where
    S: AsyncRead + Unpin,
{
    const MAX_RESPONSE_HEAD: usize = 64 * 1024;
    let mut bytes = Vec::new();
    let header_end = loop {
        if bytes.len() > MAX_RESPONSE_HEAD {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "response headers exceeded 64KiB",
            ));
        }
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed before response headers",
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_header_end(&bytes) {
            break index;
        }
    };
    let head = std::str::from_utf8(&bytes[..header_end]).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("response head was not UTF-8: {error}"),
        )
    })?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "missing response status line",
        )
    })?;
    let status = parse_response_status(status_line)?;
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("malformed response header: {line}"),
            ));
        };
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid response header name: {error}"),
            )
        })?;
        let value = HeaderValue::from_str(value.trim_start()).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid response header value: {error}"),
            )
        })?;
        headers.push((name, value));
    }
    let buffered = bytes[header_end + 4..].to_vec();
    Ok((status, headers, buffered))
}
fn parse_response_status(status_line: &str) -> std::io::Result<StatusCode> {
    let mut parts = status_line.split_whitespace();
    let Some(version) = parts.next() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "missing response version",
        ));
    };
    if !version.starts_with("HTTP/") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid response version: {version}"),
        ));
    }
    let code = parts.next().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "missing response status")
    })?;
    let code = code.parse::<u16>().map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid response status: {error}"),
        )
    })?;
    StatusCode::from_u16(code).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unsupported response status: {error}"),
        )
    })
}
fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}
fn header_has_token(value: Option<&HeaderValue>, expected: &str) -> bool {
    value
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case(expected))
        })
}
fn header_eq(value: Option<&HeaderValue>, expected: &str) -> bool {
    value
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}
fn proxy_connect_to_stream<C, Fut>(request: Request<Incoming>, connect: C) -> Response<HttpBody>
where
    C: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<PortProxyStream, String>> + Send,
{
    tokio::spawn(async move {
        let Ok(upgraded) = hyper::upgrade::on(request).await else {
            return;
        };
        let Ok(mut target) = connect().await else {
            return;
        };
        let mut upgraded = TokioIo::new(upgraded);
        let _ = tokio::io::copy_bidirectional(&mut upgraded, &mut target).await;
    });
    Response::builder()
        .status(StatusCode::OK)
        .body(full_body(Vec::new()))
        .expect("static CONNECT response is valid")
}
fn proxy_error_to_http(status: StatusCode, message: &str) -> Response<HttpBody> {
    let body = serde_json::to_vec(&BTreeMap::from([("message", message)]))
        .expect("string error body serializes");
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(full_body(body))
        .expect("static proxy error response is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_route_falls_back_to_sdk_sandbox_headers_for_local_override_url() {
        let domain = Hostname::new("cube.localhost").expect("domain");
        let request = Request::builder()
            .uri("http://127.0.0.1:49999/health")
            .header(HOST, "127.0.0.1:49999")
            .header("e2b-sandbox-id", "sbx_firkin_42")
            .header("e2b-sandbox-port", "49983")
            .body(())
            .expect("request");

        let route = request_route(&request, &domain).expect("header route");

        assert_eq!(route.sandbox_id().as_str(), "sbx_firkin_42");
        assert_eq!(route.port().get(), 49983);
    }

    #[test]
    fn request_route_rejects_invalid_sdk_sandbox_header() {
        let domain = Hostname::new("cube.localhost").expect("domain");
        let request = Request::builder()
            .uri("http://127.0.0.1:49999/health")
            .header(HOST, "127.0.0.1:49999")
            .header("e2b-sandbox-id", "bad/id")
            .header("e2b-sandbox-port", "49983")
            .body(())
            .expect("request");

        let error = request_route(&request, &domain).expect_err("invalid id");

        assert!(error.contains("invalid e2b-sandbox-id header"));
    }
}
