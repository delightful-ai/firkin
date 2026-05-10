//! network — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::error::{Result, RpcFamily, VminitdError};
#[allow(unused_imports)]
use crate::pb;
use crate::pb::sandbox_context_client::SandboxContextClient;
#[allow(unused_imports)]
use firkin_types::NamespaceKind;
#[allow(unused_imports)]
use serde::Serialize;
#[allow(unused_imports)]
use std::net::Ipv4Addr;
#[allow(unused_imports)]
use tonic::transport::Channel;
/// OCI namespace entry shaped for vminitd's strict Swift decoder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LinuxNamespace {
    #[serde(rename = "type")]
    pub(crate) kind: &'static str,
    pub(crate) path: &'static str,
}
impl LinuxNamespace {
    /// Construct an unshared namespace entry.
    ///
    /// vminitd's Swift decoder requires the `path` key to be present even when
    /// its value is empty.
    #[must_use]
    pub const fn unshare(kind: NamespaceKind) -> Self {
        Self {
            kind: kind.as_spec_str(),
            path: "",
        }
    }
    /// Namespace type string accepted by the OCI runtime spec.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        self.kind
    }
    /// Namespace path. Empty means unshare.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        self.path
    }
}
/// Guest network settings driven through vminitd netlink RPCs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkConfig {
    ipv4_address: Ipv4Addr,
    prefix: u8,
    gateway: Ipv4Addr,
    dns_location: String,
    nameservers: Vec<String>,
    mtu: Option<u32>,
}
impl NetworkConfig {
    /// Construct a network configuration.
    #[must_use]
    pub fn new<I, S>(
        ipv4_address: Ipv4Addr,
        prefix: u8,
        gateway: Ipv4Addr,
        dns_location: impl Into<String>,
        nameservers: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            ipv4_address,
            prefix,
            gateway,
            dns_location: dns_location.into(),
            nameservers: nameservers.into_iter().map(Into::into).collect(),
            mtu: None,
        }
    }
    /// Return a copy with an MTU override.
    #[must_use]
    pub const fn with_mtu(mut self, mtu: u32) -> Self {
        self.mtu = Some(mtu);
        self
    }
    /// Build the exact vminitd RPC request sequence.
    #[must_use]
    pub fn requests(&self, interface: impl Into<String>) -> NetworkRequests {
        let interface = interface.into();
        NetworkRequests {
            loopback_link: pb::IpLinkSetRequest {
                interface: "lo".into(),
                up: true,
                mtu: None,
            },
            address: pb::IpAddrAddRequest {
                interface: interface.clone(),
                ipv4_address: format!("{}/{}", self.ipv4_address, self.prefix),
            },
            interface_link: pb::IpLinkSetRequest {
                interface: interface.clone(),
                up: true,
                mtu: self.mtu,
            },
            default_route: pb::IpRouteAddDefaultRequest {
                interface,
                ipv4_gateway: self.gateway.to_string(),
            },
            dns: pb::ConfigureDnsRequest {
                location: self.dns_location.clone(),
                nameservers: self.nameservers.clone(),
                domain: None,
                search_domains: Vec::new(),
                options: Vec::new(),
            },
        }
    }
    /// Apply the network configuration through a vminitd client.
    ///
    /// # Errors
    ///
    /// Returns [`VminitdError::Rpc`] with the failing RPC family if vminitd
    /// rejects any step in the sequence.
    pub async fn apply_to(
        &self,
        client: &mut SandboxContextClient<Channel>,
        interface: impl Into<String>,
    ) -> Result<()> {
        let requests = self.requests(interface);
        client
            .ip_link_set(tonic::Request::new(requests.loopback_link))
            .await
            .map_err(|source| VminitdError::Rpc {
                family: RpcFamily::IpLinkSet,
                source: Box::new(source),
            })?;
        client
            .ip_addr_add(tonic::Request::new(requests.address))
            .await
            .map_err(|source| VminitdError::Rpc {
                family: RpcFamily::IpAddrAdd,
                source: Box::new(source),
            })?;
        client
            .ip_link_set(tonic::Request::new(requests.interface_link))
            .await
            .map_err(|source| VminitdError::Rpc {
                family: RpcFamily::IpLinkSet,
                source: Box::new(source),
            })?;
        client
            .ip_route_add_default(tonic::Request::new(requests.default_route))
            .await
            .map_err(|source| VminitdError::Rpc {
                family: RpcFamily::IpRouteAddDefault,
                source: Box::new(source),
            })?;
        client
            .configure_dns(tonic::Request::new(requests.dns))
            .await
            .map_err(|source| VminitdError::Rpc {
                family: RpcFamily::ConfigureDns,
                source: Box::new(source),
            })?;
        Ok(())
    }
}
/// Netlink/DNS requests needed to configure guest networking through vminitd.
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkRequests {
    /// Bring up `lo`.
    pub loopback_link: pb::IpLinkSetRequest,
    /// Add the CIDR-formatted IPv4 address to the target interface.
    pub address: pb::IpAddrAddRequest,
    /// Bring up the target interface.
    pub interface_link: pb::IpLinkSetRequest,
    /// Add the default route through the vmnet gateway.
    pub default_route: pb::IpRouteAddDefaultRequest,
    /// Configure resolver state under the guest root.
    pub dns: pb::ConfigureDnsRequest,
}
