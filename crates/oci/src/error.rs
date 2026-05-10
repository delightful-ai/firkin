//! error — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_types::Platform;
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Crate-local result type.
pub type Result<T> = std::result::Result<T, Error>;
/// Errors produced by OCI primitives.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum Error {
    /// The image reference string is not valid.
    #[error("invalid OCI image reference {input:?}: {reason}")]
    InvalidReference {
        /// Original input.
        input: String,
        /// Human-readable reason.
        reason: &'static str,
    },
    /// A layer media type is not supported.
    #[error("unsupported OCI layer media type {0:?}")]
    UnsupportedLayerMediaType(String),
    /// No descriptor in a manifest list matched the requested platform.
    #[error(
        "no OCI manifest matched requested platform {target:?}; available platforms: {available:?}"
    )]
    NoMatchingManifest {
        /// Requested platform.
        target: Platform,
        /// Platforms advertised by the manifest list.
        available: Vec<Platform>,
    },
    /// The registry request failed.
    #[error("registry request for {reference} failed: {reason}")]
    Registry {
        /// Image reference being pulled.
        reference: String,
        /// Registry error text.
        reason: String,
    },
    /// The local content cache could not be read or written.
    #[error("OCI cache operation {operation} failed at {path}: {reason}")]
    Cache {
        /// Cache operation.
        operation: &'static str,
        /// Cache path.
        path: PathBuf,
        /// Error text.
        reason: String,
    },
    /// JSON serialization or parsing failed.
    #[error("OCI JSON operation {operation} failed: {reason}")]
    Json {
        /// JSON operation.
        operation: &'static str,
        /// Error text.
        reason: String,
    },
    /// Registry content failed digest verification.
    #[error("digest mismatch for {path}: expected {expected}, got {actual}")]
    DigestMismatch {
        /// Content path.
        path: PathBuf,
        /// Expected digest.
        expected: String,
        /// Actual digest.
        actual: String,
    },
    /// A descriptor from the registry was structurally invalid.
    #[error("invalid OCI descriptor {digest}: {reason}")]
    InvalidDescriptor {
        /// Descriptor digest.
        digest: String,
        /// Human-readable reason.
        reason: &'static str,
    },
    /// The pulled image config cannot describe the manifest layers.
    #[error("invalid OCI image config: {reason}")]
    InvalidImageConfig {
        /// Human-readable reason.
        reason: &'static str,
    },
    /// The manifest media type is not supported.
    #[error("unsupported OCI manifest media type {0:?}")]
    UnsupportedManifestMediaType(String),
}
