//! log rotation — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use flate2::Compression;
#[allow(unused_imports)]
use flate2::write::GzEncoder;
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Log rotation error.
#[derive(Debug, ThisError)]
pub enum LogRotationError {
    /// Filesystem operation failed.
    #[error("log rotation filesystem operation failed while {operation}: {source}")]
    Io {
        /// Operation being attempted.
        operation: &'static str,
        /// Source error.
        #[source]
        source: io::Error,
    },
}
/// Log rotation plan for files directly under a log root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRotationPlan {
    pub(crate) root: PathBuf,
    max_bytes: u64,
    max_rotated_files: u32,
    compression: LogRotationCompression,
    rotate: Vec<PathBuf>,
}
impl LogRotationPlan {
    /// Build a log rotation plan for files larger than `max_bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`LogRotationError`] when the log directory cannot be read.
    pub fn for_log_dir(root: impl AsRef<Path>, max_bytes: u64) -> Result<Self, LogRotationError> {
        Self::for_log_dir_with_retention(root, max_bytes, 1)
    }
    /// Build a log rotation plan for files larger than `max_bytes`, retaining
    /// up to `max_rotated_files` generations per active log file.
    ///
    /// # Errors
    ///
    /// Returns [`LogRotationError`] when the log directory cannot be read.
    pub fn for_log_dir_with_retention(
        root: impl AsRef<Path>,
        max_bytes: u64,
        max_rotated_files: u32,
    ) -> Result<Self, LogRotationError> {
        let root = root.as_ref().to_path_buf();
        let mut rotate = Vec::new();
        for entry in fs::read_dir(&root).map_err(|source| LogRotationError::Io {
            operation: "read log directory",
            source,
        })? {
            let entry = entry.map_err(|source| LogRotationError::Io {
                operation: "read log entry",
                source,
            })?;
            let path = entry.path();
            let metadata = entry.metadata().map_err(|source| LogRotationError::Io {
                operation: "stat log entry",
                source,
            })?;
            if metadata.is_file() && metadata.len() > max_bytes && !is_rotated_log_path(&path) {
                rotate.push(path);
            }
        }
        rotate.sort();
        Ok(Self {
            root,
            max_bytes,
            max_rotated_files,
            compression: LogRotationCompression::None,
            rotate,
        })
    }
    /// Compress rotated log files using gzip.
    #[must_use]
    pub const fn with_gzip_compression(mut self) -> Self {
        self.compression = LogRotationCompression::Gzip;
        self
    }
    /// Return the log root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Return max log size in bytes.
    #[must_use]
    pub const fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
    /// Return max rotated generations retained per active log.
    #[must_use]
    pub const fn max_rotated_files(&self) -> u32 {
        self.max_rotated_files
    }
    /// Return paths rotated by the plan.
    #[must_use]
    pub fn rotate(&self) -> &[PathBuf] {
        &self.rotate
    }
    /// Return whether a path is rotated by the plan.
    #[must_use]
    pub fn rotates_path(&self, path: impl AsRef<Path>) -> bool {
        self.rotate
            .iter()
            .any(|candidate| candidate == path.as_ref())
    }
    /// Execute the log rotation plan.
    ///
    /// # Errors
    ///
    /// Returns [`LogRotationError`] when any planned rotate fails.
    pub fn execute(&self) -> Result<LogRotationReport, LogRotationError> {
        let mut rotated = Vec::new();
        for source_path in &self.rotate {
            if self.max_rotated_files == 0 {
                fs::remove_file(source_path).map_err(|source| LogRotationError::Io {
                    operation: "delete oversized log file",
                    source,
                })?;
                continue;
            }
            let last = rotated_log_path(source_path, self.max_rotated_files, self.compression);
            if last.exists() {
                fs::remove_file(&last).map_err(|source| LogRotationError::Io {
                    operation: "delete expired rotated log file",
                    source,
                })?;
            }
            for generation in (1..self.max_rotated_files).rev() {
                let from = rotated_log_path(source_path, generation, self.compression);
                if from.exists() {
                    let to = rotated_log_path(source_path, generation + 1, self.compression);
                    fs::rename(&from, &to).map_err(|source| LogRotationError::Io {
                        operation: "shift rotated log file",
                        source,
                    })?;
                }
            }
            let rotated_path = rotated_log_path(source_path, 1, self.compression);
            match self.compression {
                LogRotationCompression::None => {
                    fs::rename(source_path, &rotated_path).map_err(|source| {
                        LogRotationError::Io {
                            operation: "rotate log file",
                            source,
                        }
                    })?;
                }
                LogRotationCompression::Gzip => {
                    gzip_log_file(source_path, &rotated_path)?;
                }
            }
            rotated.push(rotated_path);
        }
        Ok(LogRotationReport { rotated })
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LogRotationCompression {
    #[default]
    None,
    Gzip,
}
fn rotated_log_path(
    source_path: &Path,
    generation: u32,
    compression: LogRotationCompression,
) -> PathBuf {
    let rotated = source_path.with_extension(format!(
        "{}.{}",
        source_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("log"),
        generation
    ));
    match compression {
        LogRotationCompression::None => rotated,
        LogRotationCompression::Gzip => rotated.with_extension(format!(
            "{}.gz",
            rotated
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("log")
        )),
    }
}
fn gzip_log_file(source_path: &Path, rotated_path: &Path) -> Result<(), LogRotationError> {
    let mut source_file = fs::File::open(source_path).map_err(|source| LogRotationError::Io {
        operation: "open log file for compression",
        source,
    })?;
    let temp_path = rotated_path.with_file_name(format!(
        "{}.tmp",
        rotated_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("rotated.log.gz")
    ));
    let temp_file = fs::File::create(&temp_path).map_err(|source| LogRotationError::Io {
        operation: "create compressed rotated log file",
        source,
    })?;
    let mut encoder = GzEncoder::new(temp_file, Compression::default());
    io::copy(&mut source_file, &mut encoder).map_err(|source| LogRotationError::Io {
        operation: "compress rotated log file",
        source,
    })?;
    let compressed = encoder.finish().map_err(|source| LogRotationError::Io {
        operation: "finish compressed rotated log file",
        source,
    })?;
    compressed
        .sync_all()
        .map_err(|source| LogRotationError::Io {
            operation: "sync compressed rotated log file",
            source,
        })?;
    fs::rename(&temp_path, rotated_path).map_err(|source| LogRotationError::Io {
        operation: "publish compressed rotated log file",
        source,
    })?;
    fs::remove_file(source_path).map_err(|source| LogRotationError::Io {
        operation: "delete compressed source log file",
        source,
    })
}
fn is_rotated_log_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.strip_suffix(".gz").unwrap_or(name))
        .and_then(|name| name.rsplit_once('.'))
        .and_then(|(_, generation)| generation.parse::<u32>().ok())
        .is_some()
}
/// Result of executing a log rotation plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRotationReport {
    rotated: Vec<PathBuf>,
}
impl LogRotationReport {
    /// Return rotated paths.
    #[must_use]
    pub fn rotated(&self) -> &[PathBuf] {
        &self.rotated
    }
    /// Return rotated path count.
    #[must_use]
    pub const fn rotated_count(&self) -> usize {
        self.rotated.len()
    }
}
