//! network — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::error::{Error, Result, invalid_config};
#[cfg(target_vendor = "apple")]
#[allow(unused_imports)]
use crate::vz;
#[allow(unused_imports)]
use std::net::Ipv4Addr;
/// A single network attachment.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Network {
    /// Virtualization.framework NAT attachment.
    Nat,
    /// Shared vmnet attachment with a caller-selected subnet.
    VmnetShared {
        /// Subnet in CIDR form.
        subnet: Option<String>,
    },
}
impl Network {
    /// Use a vmnet shared network and let vmnet select the subnet.
    #[must_use]
    pub const fn vmnet_shared() -> Self {
        Self::VmnetShared { subnet: None }
    }
    /// Use a vmnet shared network with an explicit IPv4 subnet.
    #[must_use]
    pub fn vmnet_shared_subnet(subnet: impl Into<String>) -> Self {
        Self::VmnetShared {
            subnet: Some(subnet.into()),
        }
    }
}
/// Guest-visible network interface assigned to a running VM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkInterface {
    name: String,
    ipv4_address: Ipv4Addr,
    prefix: u8,
    gateway: Ipv4Addr,
}
impl NetworkInterface {
    /// Construct a network interface description.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        ipv4_address: Ipv4Addr,
        prefix: u8,
        gateway: Ipv4Addr,
    ) -> Self {
        Self {
            name: name.into(),
            ipv4_address,
            prefix,
            gateway,
        }
    }
    /// Return the guest interface name, for example `eth0`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Return the assigned IPv4 address.
    #[must_use]
    pub const fn ipv4_address(&self) -> Ipv4Addr {
        self.ipv4_address
    }
    /// Return the CIDR prefix length.
    #[must_use]
    pub const fn prefix(&self) -> u8 {
        self.prefix
    }
    /// Return the default gateway address.
    #[must_use]
    pub const fn gateway(&self) -> Ipv4Addr {
        self.gateway
    }
}
#[cfg(target_vendor = "apple")]
pub(crate) fn default_network_macs(count: usize) -> Vec<String> {
    (0..count).map(|_| vz::new_mac_address_string()).collect()
}
#[cfg(not(target_vendor = "apple"))]
pub(crate) fn default_network_macs(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| format!("02:00:00:00:00:{:02x}", index & 0xff))
        .collect()
}
pub(crate) fn validate_mac_address(mac: &str) -> Result<()> {
    let parts = mac.split(':').collect::<Vec<_>>();
    if parts.len() != 6
        || parts
            .iter()
            .any(|part| part.len() != 2 || !part.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return invalid_config(format!("network MAC address `{mac}` is invalid"));
    }
    Ok(())
}
pub(crate) fn validate_vmnet_subnet(subnet: &str) -> Result<()> {
    let Some((addr, prefix)) = subnet.split_once('/') else {
        return invalid_config(format!("vmnet subnet `{subnet}` is not CIDR notation"));
    };
    let prefix = prefix.parse::<u8>().map_err(|_| Error::InvalidConfig {
        reason: format!("vmnet subnet `{subnet}` has an invalid prefix"),
    })?;
    if prefix > 30 {
        return invalid_config(format!(
            "vmnet subnet `{subnet}` must leave room for gateway and guest addresses"
        ));
    }
    addr.parse::<Ipv4Addr>().map_err(|_| Error::InvalidConfig {
        reason: format!("vmnet subnet `{subnet}` has an invalid IPv4 address"),
    })?;
    Ok(())
}
