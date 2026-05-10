//! preflight — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::error::{Result, invalid_config};
#[allow(unused_imports)]
use std::process::Command;
/// Host CPU architecture relevant to Virtualization.framework guests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostArch {
    /// Apple Silicon host.
    Arm64,
    /// Intel Mac host.
    X86_64,
}
/// Host capability information useful before attempting a VZ boot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preflight {
    macos_version: String,
    architecture: HostArch,
    nested_virtualization_supported: bool,
    rosetta_available: bool,
    codesigned: Option<bool>,
    has_virtualization_entitlement: Option<bool>,
}
impl Preflight {
    /// Construct preflight information from probed fields.
    #[must_use]
    pub const fn new(
        macos_version: String,
        architecture: HostArch,
        nested_virtualization_supported: bool,
        rosetta_available: bool,
        codesigned: Option<bool>,
        has_virtualization_entitlement: Option<bool>,
    ) -> Self {
        Self {
            macos_version,
            architecture,
            nested_virtualization_supported,
            rosetta_available,
            codesigned,
            has_virtualization_entitlement,
        }
    }
    /// Return the host macOS product version string.
    #[must_use]
    pub fn macos_version(&self) -> &str {
        &self.macos_version
    }
    /// Return the host architecture.
    #[must_use]
    pub const fn architecture(&self) -> HostArch {
        self.architecture
    }
    /// Return whether nested virtualization is supported by this host class.
    #[must_use]
    pub const fn nested_virtualization_supported(&self) -> bool {
        self.nested_virtualization_supported
    }
    /// Return whether Rosetta support is available by this host class.
    #[must_use]
    pub const fn rosetta_available(&self) -> bool {
        self.rosetta_available
    }
    /// Return whether the current executable appears codesigned, if probed.
    #[must_use]
    pub const fn codesigned(&self) -> Option<bool> {
        self.codesigned
    }
    /// Return whether the current executable has the virtualization entitlement, if probed.
    #[must_use]
    pub const fn has_virtualization_entitlement(&self) -> Option<bool> {
        self.has_virtualization_entitlement
    }
}
/// Probe host capabilities needed by VZ-backed VMs.
///
/// # Errors
///
/// Returns [`Error::InvalidConfig`] if the current target architecture is not
/// one of the supported macOS host architectures.
pub fn preflight() -> Result<Preflight> {
    let architecture = host_arch()?;
    let version = Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|output| {
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    let signing = std::env::current_exe()
        .ok()
        .and_then(|path| signing::codesign_check(path).ok());
    Ok(Preflight::new(
        version,
        architecture,
        matches!(architecture, HostArch::Arm64),
        matches!(architecture, HostArch::Arm64),
        signing.as_ref().map(signing::CodeSignInfo::codesigned),
        signing
            .as_ref()
            .map(signing::CodeSignInfo::has_virtualization_entitlement),
    ))
}
/// Codesigning helpers for VZ entitlement diagnostics.
pub mod signing {
    use crate::{Error, Result};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    /// Codesign status for an executable.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct CodeSignInfo {
        path: PathBuf,
        codesigned: bool,
        has_virtualization_entitlement: bool,
    }
    impl CodeSignInfo {
        /// Return the checked path.
        #[must_use]
        pub fn path(&self) -> &Path {
            &self.path
        }
        /// Return whether codesign recognized a signature.
        #[must_use]
        pub const fn codesigned(&self) -> bool {
            self.codesigned
        }
        /// Return whether the binary has the VZ entitlement.
        #[must_use]
        pub const fn has_virtualization_entitlement(&self) -> bool {
            self.has_virtualization_entitlement
        }
    }
    /// Check codesigning and the virtualization entitlement on `path`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidConfig`] if `/usr/bin/codesign` cannot be run.
    pub fn codesign_check(path: impl AsRef<Path>) -> Result<CodeSignInfo> {
        let path = path.as_ref();
        let display = Command::new("/usr/bin/codesign")
            .args(["-dv", path.to_str().unwrap_or_default()])
            .output()
            .map_err(|source| Error::InvalidConfig {
                reason: format!("codesign check failed for {}: {source}", path.display()),
            })?;
        let codesigned = display.status.success();
        let entitlements = Command::new("/usr/bin/codesign")
            .args([
                "-d",
                "--entitlements",
                ":-",
                path.to_str().unwrap_or_default(),
            ])
            .output()
            .map_err(|source| Error::InvalidConfig {
                reason: format!(
                    "codesign entitlement check failed for {}: {source}",
                    path.display()
                ),
            })?;
        let entitlement_text = String::from_utf8_lossy(&entitlements.stdout);
        let has_virtualization_entitlement =
            entitlement_text.contains("com.apple.security.virtualization");
        Ok(CodeSignInfo {
            path: path.to_path_buf(),
            codesigned,
            has_virtualization_entitlement,
        })
    }
}
fn host_arch() -> Result<HostArch> {
    if cfg!(target_arch = "aarch64") {
        Ok(HostArch::Arm64)
    } else if cfg!(target_arch = "x86_64") {
        Ok(HostArch::X86_64)
    } else {
        invalid_config("unsupported host architecture")
    }
}
