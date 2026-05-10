#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Pinned vminitd and vmexec ELF bytes.
//!
//! This crate is intentionally tiny and leaf-shaped so the large embedded ELF
//! artifacts have one owner. `firkin-core` consumes these bytes when it synthesizes the
//! cached `init.block` image.

/// Revision label from `build-tools/build-vminitd/pin.toml`.
pub const VMINITD_REVISION: &str = env!("VMINITD_REVISION");

/// SHA-256 of the pinned aarch64 vminitd ELF.
pub const VMINITD_SHA256: &str = env!("VMINITD_SHA256");

/// SHA-256 of the pinned aarch64 vmexec ELF.
pub const VMEXEC_SHA256: &str = env!("VMEXEC_SHA256");

/// Pinned aarch64 vminitd ELF bytes.
#[cfg(not(feature = "runtime-download"))]
pub const VMINITD_AARCH64: &[u8] = include_bytes!(env!("VMINITD_AARCH64_PATH"));

/// Pinned aarch64 vmexec ELF bytes.
#[cfg(not(feature = "runtime-download"))]
pub const VMEXEC_AARCH64: &[u8] = include_bytes!(env!("VMEXEC_AARCH64_PATH"));

/// Empty marker when `runtime-download` is enabled.
#[cfg(feature = "runtime-download")]
pub const VMINITD_AARCH64: &[u8] = &[];

/// Empty marker when `runtime-download` is enabled.
#[cfg(feature = "runtime-download")]
pub const VMEXEC_AARCH64: &[u8] = &[];

/// Return whether this build embedded the vminitd ELF.
#[must_use]
pub const fn embedded() -> bool {
    !VMINITD_AARCH64.is_empty() && !VMEXEC_AARCH64.is_empty()
}

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "runtime-download"))]
    use sha2::{Digest, Sha256};

    #[test]
    #[cfg(not(feature = "runtime-download"))]
    fn embedded_bytes_match_pinned_hash() {
        let actual = format!("{:x}", Sha256::digest(super::VMINITD_AARCH64));

        assert_eq!(actual, super::VMINITD_SHA256);
        let actual = format!("{:x}", Sha256::digest(super::VMEXEC_AARCH64));

        assert_eq!(actual, super::VMEXEC_SHA256);
        assert!(super::embedded());
    }

    #[test]
    #[cfg(feature = "runtime-download")]
    fn runtime_download_builds_without_embedded_bytes() {
        assert!(!super::embedded());
    }
}
