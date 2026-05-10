//! rosetta — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::error::{Result, RpcFamily, VminitdError};
#[allow(unused_imports)]
use crate::pb;
use crate::pb::sandbox_context_client::SandboxContextClient;
#[allow(unused_imports)]
use tonic::transport::Channel;
/// Guest-side Rosetta setup driven through vminitd.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosettaSetup {
    mount_path: String,
    pub(crate) source: String,
    binary_name: String,
    emulator: EmulatorConfig,
}
impl RosettaSetup {
    /// Construct the S7-verified amd64 Rosetta setup.
    #[must_use]
    pub fn amd64() -> Self {
        Self {
            mount_path: "/run/rosetta".into(),
            source: "rosetta".into(),
            binary_name: "rosetta".into(),
            emulator: EmulatorConfig::amd64(),
        }
    }
    /// Build the exact vminitd RPC request sequence.
    #[must_use]
    pub fn requests(&self) -> RosettaRequests {
        RosettaRequests {
            mkdir: pb::MkdirRequest {
                path: self.mount_path.clone(),
                all: true,
                perms: 0o755,
            },
            mount: pb::MountRequest {
                r#type: "virtiofs".into(),
                source: self.source.clone(),
                destination: self.mount_path.clone(),
                options: Vec::new(),
            },
            setup_emulator: pb::SetupEmulatorRequest {
                binary_path: format!("{}/{}", self.mount_path, self.binary_name),
                name: self.emulator.name.clone(),
                r#type: self.emulator.kind.clone(),
                offset: self.emulator.offset.clone(),
                magic: self.emulator.magic.clone(),
                mask: self.emulator.mask.clone(),
                flags: self.emulator.flags.clone(),
            },
        }
    }
    /// Apply the Rosetta setup through a vminitd client.
    ///
    /// # Errors
    ///
    /// Returns [`VminitdError::Rpc`] with the failing RPC family if vminitd
    /// rejects any step in the sequence.
    pub async fn apply_to(&self, client: &mut SandboxContextClient<Channel>) -> Result<()> {
        let requests = self.requests();
        client
            .mkdir(tonic::Request::new(requests.mkdir))
            .await
            .map_err(|source| VminitdError::Rpc {
                family: RpcFamily::Mkdir,
                source: Box::new(source),
            })?;
        client
            .mount(tonic::Request::new(requests.mount))
            .await
            .map_err(|source| VminitdError::Rpc {
                family: RpcFamily::Mount,
                source: Box::new(source),
            })?;
        client
            .setup_emulator(tonic::Request::new(requests.setup_emulator))
            .await
            .map_err(|source| VminitdError::Rpc {
                family: RpcFamily::SetupEmulator,
                source: Box::new(source),
            })?;
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct EmulatorConfig {
    name: String,
    pub(crate) kind: String,
    offset: String,
    magic: String,
    mask: String,
    pub(crate) flags: String,
}
impl EmulatorConfig {
    fn amd64() -> Self {
        Self {
            name: "x86_64".into(),
            kind: "M".into(),
            offset: String::new(),
            magic: "\\x7fELF\\x02\\x01\\x01\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x02\\x00\\x3e\\x00"
                .into(),
            mask: "\\xff\\xff\\xff\\xff\\xff\\xfe\\xfe\\x00\\xff\\xff\\xff\\xff\\xff\\xff\\xff\\xff\\xfe\\xff\\xff\\xff"
                .into(),
            flags: "CF".into(),
        }
    }
}
/// Requests needed to configure Rosetta through vminitd.
#[derive(Clone, Debug, PartialEq)]
pub struct RosettaRequests {
    /// Ensure the guest mountpoint exists.
    pub mkdir: pb::MkdirRequest,
    /// Mount the host Rosetta virtiofs share.
    pub mount: pb::MountRequest,
    /// Register amd64 ELF handling with `binfmt_misc`.
    pub setup_emulator: pb::SetupEmulatorRequest,
}
