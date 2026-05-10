//! cache — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::error::Error;
#[allow(unused_imports)]
use std::fmt;
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::path::PathBuf;
pub(crate) fn default_cache_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("FIRKIN_CACHE") {
        return PathBuf::from(path).join("oci");
    }
    if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(path).join("firkin").join("oci");
    }
    if let Some(path) = std::env::var_os("HOME") {
        return PathBuf::from(path)
            .join(".cache")
            .join("firkin")
            .join("oci");
    }
    std::env::temp_dir().join("firkin").join("oci")
}
pub(crate) fn cache_error(operation: &'static str, path: &Path, error: impl fmt::Display) -> Error {
    Error::Cache {
        operation,
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}
pub(crate) fn json_error(operation: &'static str, error: impl fmt::Display) -> Error {
    Error::Json {
        operation,
        reason: error.to_string(),
    }
}
