//! Domain proxy tests for the single-node runtime facade.

use std::net::SocketAddr;

use firkin_e2b_contract::{BackendError, PortTarget, RuntimeAdapter};
use firkin_single_node::{DomainProxyAdapter, PortRegistry, SingleNodeBackend, SingleNodeConfig};
use firkin_types::hostname;

#[tokio::test]
async fn port_registry_routes_and_removes_sandbox_targets() {
    let registry = PortRegistry::default();
    let target: SocketAddr = "127.0.0.1:32123".parse().unwrap();

    registry.route_tcp("sbx-one", 49983, target).await;
    assert_eq!(
        registry.target("sbx-one", 49983).await.unwrap(),
        PortTarget::Tcp {
            host: "127.0.0.1".to_string(),
            port: 32123,
        }
    );

    registry.remove_sandbox("sbx-one").await;
    let error = registry.target("sbx-one", 49983).await.unwrap_err();
    assert!(matches!(error, BackendError::NotFound(_)));
}

#[tokio::test]
async fn domain_proxy_adapter_exposes_only_domain_proxy_runtime_capability() {
    let registry = PortRegistry::default();
    let adapter = DomainProxyAdapter::new(registry);

    let capabilities = adapter.preflight().await.unwrap();
    assert_eq!(capabilities.backend, "apple-vz");
    assert_eq!(capabilities.supported, vec!["domain-host-proxy"]);
    assert!(capabilities.unsupported.is_empty());
}

#[test]
fn single_node_backend_constructs_domain_proxy_server() {
    let backend = SingleNodeBackend::from_config(SingleNodeConfig::new(
        std::env::temp_dir().join("firkin-single-node-proxy-test"),
        "cube.localhost",
    ));
    let proxy = backend.domain_proxy(PortRegistry::default(), hostname!("cube.localhost"));

    assert_eq!(proxy.domain().as_str(), "cube.localhost");
}
