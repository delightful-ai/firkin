//! Host disk-image creation and conversion.

use std::path::{Path, PathBuf};
use std::process::Command;

use firkin_types::Size;

use crate::{Error, Result, invalid_config};

/// Host disk image format for a local VZ disk-image attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiskImageFormat {
    /// Raw byte-for-byte disk image.
    Raw,
    /// Apple Sparse Image Format.
    Asif,
}

impl DiskImageFormat {
    fn diskutil_name(self) -> &'static str {
        match self {
            Self::Raw => "RAW",
            Self::Asif => "ASIF",
        }
    }
}

/// Specification for creating an empty local disk image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlankDiskImage {
    path: PathBuf,
    size: Size,
    format: DiskImageFormat,
}

impl BlankDiskImage {
    /// Construct a blank disk-image creation request.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, size: Size, format: DiskImageFormat) -> Self {
        Self {
            path: path.into(),
            size,
            format,
        }
    }

    /// Construct a raw blank disk-image creation request.
    #[must_use]
    pub fn raw(path: impl Into<PathBuf>, size: Size) -> Self {
        Self::new(path, size, DiskImageFormat::Raw)
    }

    /// Construct an ASIF blank disk-image creation request.
    #[must_use]
    pub fn asif(path: impl Into<PathBuf>, size: Size) -> Self {
        Self::new(path, size, DiskImageFormat::Asif)
    }

    /// Return the output path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the requested virtual size.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// Return the requested host disk image format.
    #[must_use]
    pub const fn format(&self) -> DiskImageFormat {
        self.format
    }
}

/// Specification for converting one disk image into another host format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskImageConversion {
    source: PathBuf,
    destination: PathBuf,
    format: DiskImageFormat,
}

impl DiskImageConversion {
    /// Construct a disk-image conversion request.
    #[must_use]
    pub fn new(
        source: impl Into<PathBuf>,
        destination: impl Into<PathBuf>,
        format: DiskImageFormat,
    ) -> Self {
        Self {
            source: source.into(),
            destination: destination.into(),
            format,
        }
    }

    /// Construct a conversion request targeting ASIF.
    #[must_use]
    pub fn asif(source: impl Into<PathBuf>, destination: impl Into<PathBuf>) -> Self {
        Self::new(source, destination, DiskImageFormat::Asif)
    }

    /// Return the source image path.
    #[must_use]
    pub fn source(&self) -> &Path {
        &self.source
    }

    /// Return the destination image path.
    #[must_use]
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    /// Return the destination host disk-image format.
    #[must_use]
    pub const fn format(&self) -> DiskImageFormat {
        self.format
    }
}

/// Create an empty local disk image.
///
/// Raw images are created directly with the requested byte length. ASIF images
/// are created with `diskutil image create blank --format ASIF --fs None`
/// because ASIF is an Apple host disk-image format.
///
/// # Errors
///
/// Returns [`Error::InvalidConfig`] when the requested size is zero, the path
/// cannot be created, or `diskutil` rejects ASIF creation.
pub fn create_blank_disk_image(spec: &BlankDiskImage) -> Result<()> {
    if spec.size.as_bytes() == 0 {
        return invalid_config("blank disk image size must be > 0");
    }

    match spec.format {
        DiskImageFormat::Raw => create_blank_raw_disk_image(spec),
        DiskImageFormat::Asif => create_blank_asif_disk_image(spec),
    }
}

/// Convert a local disk image into another host disk-image format.
///
/// # Errors
///
/// Returns [`Error::InvalidConfig`] if `diskutil` cannot convert the image.
pub fn convert_disk_image(spec: &DiskImageConversion) -> Result<()> {
    let format = spec.format.diskutil_name();
    let output = Command::new("diskutil")
        .args(["image", "create", "from", "--format", format])
        .arg(spec.source())
        .arg(spec.destination())
        .output()
        .map_err(|source| Error::InvalidConfig {
            reason: format!(
                "failed to run diskutil to convert {} to {} as {format}: {source}",
                spec.source().display(),
                spec.destination().display()
            ),
        })?;

    if output.status.success() {
        return Ok(());
    }

    invalid_config(format!(
        "failed to convert disk image {} to {} as {format}: {}",
        spec.source().display(),
        spec.destination().display(),
        command_output_summary(&output)
    ))
}

/// Return whether this host `diskutil` advertises ASIF conversion support.
#[must_use]
pub fn diskutil_supports_asif_conversion() -> bool {
    let output = Command::new("diskutil")
        .args(["image", "create", "from", "--help"])
        .output();
    let Ok(output) = output else {
        return false;
    };
    let help = String::from_utf8_lossy(&output.stdout);
    help.contains("\"ASIF\"") || help.contains(" ASIF") || help.contains("[ASIF")
}

fn create_blank_raw_disk_image(spec: &BlankDiskImage) -> Result<()> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(spec.path())
        .map_err(|source| Error::InvalidConfig {
            reason: format!(
                "failed to create raw disk image {}: {source}",
                spec.path().display()
            ),
        })?;
    file.set_len(spec.size().as_bytes())
        .map_err(|source| Error::InvalidConfig {
            reason: format!(
                "failed to size raw disk image {}: {source}",
                spec.path().display()
            ),
        })
}

fn create_blank_asif_disk_image(spec: &BlankDiskImage) -> Result<()> {
    let size = spec.size().as_bytes().to_string();
    let output = Command::new("diskutil")
        .args([
            "image",
            "create",
            "blank",
            "--format",
            "ASIF",
            "--fs",
            "None",
            "--size",
            size.as_str(),
        ])
        .arg(spec.path())
        .output()
        .map_err(|source| Error::InvalidConfig {
            reason: format!(
                "failed to run diskutil for ASIF disk image {}: {source}",
                spec.path().display()
            ),
        })?;

    if output.status.success() {
        return Ok(());
    }

    invalid_config(format!(
        "failed to create ASIF disk image {}: {}",
        spec.path().display(),
        command_output_summary(&output)
    ))
}

fn command_output_summary(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !stdout.is_empty() {
        return stdout;
    }
    format!("diskutil exited with status {}", output.status)
}
