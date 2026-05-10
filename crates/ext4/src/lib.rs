#![forbid(unsafe_code)]
#![allow(missing_docs)]

//! EXT4 image writer for the firkin Rust containerization library.
//!
//! The crate-level API is [`Writer`]. The lower-level S5 spike builder remains
//! available while the library surface is filled in because its layout types are
//! useful for structural tests.

use std::fs::OpenOptions;
use std::io::{Cursor, Read, Seek, Write};
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};

use firkin_types::Size;

#[allow(clippy::all, clippy::pedantic)]
pub mod builder;
pub mod error;
pub mod init_block;
#[allow(clippy::all, clippy::pedantic)]
pub mod layout;
#[allow(clippy::all, clippy::pedantic)]
pub mod types;

pub use builder::{FileSystemBuilder, Xattr};
pub use error::{Ext4Error as Error, Result};
pub use types::{BlockNumber, BlockSize, FileMode, InodeNumber};

/// Per-layer compression hint for OCI layer extraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayerCompression {
    /// Uncompressed tar.
    None,
    /// Gzip-compressed tar.
    Gzip,
    /// Zstd-compressed tar.
    Zstd,
}

/// Sealed trait implemented by sources that can provide ordered OCI layers.
pub mod sealed {
    /// Marker for [`super::OciLayerSource`] implementors.
    pub trait Sealed {}
}

/// A source of ordered OCI layers for rootfs assembly.
pub trait OciLayerSource: sealed::Sealed {
    /// Ordered raw layer paths and their compression formats.
    fn layers(&self) -> impl Iterator<Item = (&Path, LayerCompression)> + '_;
}

bitflags::bitflags! {
    /// EXT4 feature set requested for an image.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Features: u64 {
        /// Extended attributes.
        const EXT_ATTR = 1 << 0;
        /// Sparse superblock backups.
        const SPARSE_SUPER2 = 1 << 1;
        /// Directory entries include file type.
        const FILETYPE = 1 << 2;
        /// Extent-based file allocation.
        const EXTENT = 1 << 3;
        /// Flexible block groups.
        const FLEX_BG = 1 << 4;
        /// Large file support.
        const LARGE_FILE = 1 << 5;
        /// Huge file support.
        const HUGE_FILE = 1 << 6;
        /// Extra inode size support.
        const EXTRA_ISIZE = 1 << 7;
        /// HTree indexed directories.
        const DIR_INDEX = 1 << 8;
        /// Meta block groups.
        const META_BG = 1 << 9;
        /// 64-bit block addressing.
        const BIT_64 = 1 << 10;
        /// Metadata checksums.
        const METADATA_CSUM = 1 << 11;
        /// Extent trees deeper than depth 1.
        const DEEP_EXTENTS = 1 << 12;
        /// High directory link counts.
        const DIR_NLINK = 1 << 13;
        /// Inline file data.
        const INLINE_DATA = 1 << 14;
    }
}

impl Features {
    /// The current ship set implemented by the writer.
    #[must_use]
    pub const fn default_set() -> Self {
        Self::EXT_ATTR
            .union(Self::SPARSE_SUPER2)
            .union(Self::FILETYPE)
            .union(Self::EXTENT)
            .union(Self::FLEX_BG)
            .union(Self::LARGE_FILE)
            .union(Self::HUGE_FILE)
            .union(Self::EXTRA_ISIZE)
    }

    /// The spike-validated feature set. This remains stable across releases.
    #[must_use]
    pub const fn spike_set() -> Self {
        Self::default_set()
    }

    /// The mkfs.ext4 parity target.
    #[must_use]
    pub const fn mkfs_parity_target() -> Self {
        Self::all()
    }
}

/// EXT4 image writer.
pub struct Writer {
    sink: Sink,
    block_size: BlockSize,
    size: Size,
    features: Features,
    fs: FileSystemBuilder,
}

enum Sink {
    File(PathBuf),
    Memory,
}

impl Writer {
    /// Open `path` for writing and pre-size the image to at least `size`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the image cannot be written or finalized.
    pub fn new(path: impl Into<PathBuf>, size: Size) -> Result<Self> {
        Ok(Self::build(
            Sink::File(path.into()),
            size,
            BlockSize::DEFAULT,
        ))
    }

    /// Build an in-memory image.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the image cannot be written or finalized.
    pub fn in_memory(size: Size) -> Result<Self> {
        Ok(Self::build(Sink::Memory, size, BlockSize::DEFAULT))
    }

    /// Set the image feature request.
    #[must_use]
    pub fn features(mut self, features: Features) -> Self {
        self.features = features;
        self
    }

    /// Set the block size.
    #[must_use]
    pub fn block_size(mut self, size: BlockSize) -> Self {
        self.block_size = size;
        self.fs = FileSystemBuilder::new(size).with_min_size(self.size.as_bytes());
        self
    }

    /// Set the filesystem UUID from raw bytes.
    #[must_use]
    pub fn uuid(mut self, uuid: [u8; 16]) -> Self {
        self.fs = self.fs.with_uuid(uuid);
        self
    }

    /// Write a directory.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the path is invalid or conflicts.
    pub fn write_dir(mut self, guest_path: impl AsRef<Path>, mode: u16) -> Result<Self> {
        self.fs
            .add_dir(&path_to_guest_string(guest_path.as_ref()), mode)?;
        Ok(self)
    }

    /// Recursively copy a host directory into the image at `guest_path`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the host tree cannot be read or a guest path
    /// cannot be represented in the output image.
    pub fn write_directory(
        mut self,
        guest_path: impl AsRef<Path>,
        host_source: impl AsRef<Path>,
    ) -> Result<Self> {
        let guest_root = path_to_guest_string(guest_path.as_ref());
        let host_root = host_source.as_ref();
        self.copy_host_tree(&guest_root, host_root)?;
        Ok(self)
    }

    /// Write layers from an OCI layer source.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when layer extraction is not yet supported or a layer
    /// cannot be read.
    pub fn write_oci_layers(self, src: &impl OciLayerSource) -> Result<Self> {
        self.write_layers_raw(src.layers())
    }

    /// Write raw layer paths with explicit compression hints.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when layer extraction is not yet supported.
    pub fn write_layers_raw<I, P>(self, layers: I) -> Result<Self>
    where
        I: IntoIterator<Item = (P, LayerCompression)>,
        P: AsRef<Path>,
    {
        let mut writer = self;
        for (path, compression) in layers {
            writer.extract_layer(path.as_ref(), compression)?;
        }
        Ok(writer)
    }

    /// Write a file.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the path is invalid, conflicts, or content is too large.
    pub fn write_file(
        mut self,
        guest_path: impl AsRef<Path>,
        content: &[u8],
        mode: u16,
    ) -> Result<Self> {
        self.fs
            .add_file(&path_to_guest_string(guest_path.as_ref()), content, mode)?;
        Ok(self)
    }

    /// Write a symlink.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the link path or target is invalid.
    pub fn write_symlink(
        mut self,
        guest_path: impl AsRef<Path>,
        target: impl AsRef<Path>,
    ) -> Result<Self> {
        self.fs.add_symlink(
            &path_to_guest_string(guest_path.as_ref()),
            &path_to_guest_string(target.as_ref()),
        )?;
        Ok(self)
    }

    /// Write an OCI whiteout marker for `guest_path`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the marker path is invalid or conflicts.
    pub fn write_whiteout(mut self, guest_path: impl AsRef<Path>) -> Result<Self> {
        self.fs
            .add_whiteout(&path_to_guest_string(guest_path.as_ref()))?;
        Ok(self)
    }

    /// Mark a directory opaque using the OCI overlay marker.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the marker path is invalid or conflicts.
    pub fn write_opaque_dir(mut self, guest_path: impl AsRef<Path>) -> Result<Self> {
        self.fs
            .add_opaque_dir(&path_to_guest_string(guest_path.as_ref()))?;
        Ok(self)
    }

    /// Write an extended attribute on an existing path.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the path does not exist or the xattr cannot be encoded.
    pub fn write_xattr(
        mut self,
        guest_path: impl AsRef<Path>,
        name: &str,
        value: &[u8],
    ) -> Result<Self> {
        self.fs
            .set_xattr(&path_to_guest_string(guest_path.as_ref()), name, value)?;
        Ok(self)
    }

    /// Flush the image and return the output path.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when this writer is in-memory or the file cannot be written.
    pub fn finalize(mut self) -> Result<PathBuf> {
        match self.sink {
            Sink::File(ref path) => {
                let output_path = path.clone();
                let mut out = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&output_path)
                    .map_err(|source| Error::Control {
                        what: "open",
                        source,
                    })?;
                self.write_to(&mut out)?;
                out.flush().map_err(|source| Error::Control {
                    what: "flush",
                    source,
                })?;
                Ok(output_path)
            }
            Sink::Memory => Err(Error::Control {
                what: "finalize called on in-memory writer",
                source: std::io::Error::other("use into_bytes for in-memory writers"),
            }),
        }
    }

    /// Consume an in-memory writer and return the image bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when this writer targets a file or the image cannot be written.
    pub fn into_bytes(mut self) -> Result<Vec<u8>> {
        match self.sink {
            Sink::Memory => {
                let mut out = Cursor::new(Vec::new());
                self.write_to(&mut out)?;
                Ok(out.into_inner())
            }
            Sink::File(_) => Err(Error::Control {
                what: "into_bytes called on file writer",
                source: std::io::Error::other("use finalize for file writers"),
            }),
        }
    }

    fn build(sink: Sink, size: Size, block_size: BlockSize) -> Self {
        Self {
            sink,
            block_size,
            size,
            features: Features::default_set(),
            fs: FileSystemBuilder::new(block_size).with_min_size(size.as_bytes()),
        }
    }

    fn write_to<W: Write + Seek>(&mut self, out: &mut W) -> Result<()> {
        self.reject_unsupported_features()?;
        self.fs.write(out)
    }

    fn copy_host_tree(&mut self, guest_path: &str, host_path: &Path) -> Result<()> {
        let metadata = std::fs::symlink_metadata(host_path).map_err(|source| Error::Control {
            what: "read host metadata",
            source,
        })?;
        let file_type = metadata.file_type();

        if file_type.is_dir() {
            if guest_path != "/" {
                self.fs
                    .add_dir(guest_path, mode_from_metadata(&metadata, 0o755))?;
            }

            let mut entries = std::fs::read_dir(host_path)
                .map_err(|source| Error::Control {
                    what: "read host directory",
                    source,
                })?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|source| Error::Control {
                    what: "read host directory entry",
                    source,
                })?;
            entries.sort_by_key(std::fs::DirEntry::file_name);

            for entry in entries {
                let name = entry.file_name();
                let child_guest = append_guest_path(guest_path, &name.to_string_lossy());
                self.copy_host_tree(&child_guest, &entry.path())?;
            }
            return Ok(());
        }

        if file_type.is_symlink() {
            let target = std::fs::read_link(host_path).map_err(|source| Error::Control {
                what: "read host symlink",
                source,
            })?;
            self.fs
                .add_symlink(guest_path, &path_to_guest_string(&target))?;
            return Ok(());
        }

        if file_type.is_file() {
            let content = std::fs::read(host_path).map_err(|source| Error::Control {
                what: "read host file",
                source,
            })?;
            self.fs
                .add_file(guest_path, &content, mode_from_metadata(&metadata, 0o644))?;
            return Ok(());
        }

        #[cfg(unix)]
        if file_type.is_char_device() {
            return Err(Error::UnsupportedFeature {
                feature: "copying host character devices".to_owned(),
            });
        }

        #[cfg(unix)]
        if file_type.is_block_device() {
            return Err(Error::UnsupportedFeature {
                feature: "copying host block devices".to_owned(),
            });
        }

        Err(Error::UnsupportedFeature {
            feature: "copying this host file type".to_owned(),
        })
    }

    fn extract_layer(&mut self, path: &Path, compression: LayerCompression) -> Result<()> {
        let file = std::fs::File::open(path).map_err(|source| Error::Control {
            what: "open OCI layer",
            source,
        })?;
        let reader: Box<dyn Read> = match compression {
            LayerCompression::None => Box::new(file),
            LayerCompression::Gzip => Box::new(flate2::read::GzDecoder::new(file)),
            LayerCompression::Zstd => {
                Box::new(zstd::stream::read::Decoder::new(file).map_err(|source| {
                    Error::Control {
                        what: "open zstd OCI layer",
                        source,
                    }
                })?)
            }
        };

        let mut archive = tar::Archive::new(reader);
        let entries = archive.entries().map_err(|source| Error::Control {
            what: "read OCI layer entries",
            source,
        })?;

        for entry in entries {
            let mut entry = entry.map_err(|source| Error::Control {
                what: "read OCI layer entry",
                source,
            })?;
            let entry_path = entry.path().map_err(|source| Error::Control {
                what: "read OCI layer entry path",
                source,
            })?;
            let guest_path = tar_path_to_guest_string(&entry_path)?;

            if handle_oci_whiteout(&mut self.fs, &guest_path)? {
                continue;
            }

            let entry_type = entry.header().entry_type();
            if entry_type.is_dir() {
                if guest_path != "/" {
                    self.fs
                        .add_dir(&guest_path, tar_entry_mode(&entry, 0o755)?)?;
                }
                continue;
            }

            if entry_type.is_file() {
                let mut content = Vec::new();
                entry
                    .read_to_end(&mut content)
                    .map_err(|source| Error::Control {
                        what: "read OCI layer file",
                        source,
                    })?;
                self.fs
                    .add_file(&guest_path, &content, tar_entry_mode(&entry, 0o644)?)?;
                continue;
            }

            if entry_type.is_symlink() {
                let target = entry.link_name().map_err(|source| Error::Control {
                    what: "read OCI layer symlink",
                    source,
                })?;
                let Some(target) = target else {
                    return Err(Error::UnsupportedFeature {
                        feature: format!("symlink without target at {guest_path}"),
                    });
                };
                self.fs
                    .add_symlink(&guest_path, &path_to_guest_string(&target))?;
                continue;
            }

            if entry_type.is_hard_link() {
                let target = entry.link_name().map_err(|source| Error::Control {
                    what: "read OCI layer hardlink",
                    source,
                })?;
                let Some(target) = target else {
                    return Err(Error::UnsupportedFeature {
                        feature: format!("hardlink without target at {guest_path}"),
                    });
                };
                self.fs
                    .add_hardlink(&guest_path, &tar_path_to_guest_string(&target)?)?;
                continue;
            }

            return Err(Error::UnsupportedFeature {
                feature: format!("OCI layer entry type {entry_type:?} at {guest_path}"),
            });
        }

        Ok(())
    }

    fn reject_unsupported_features(&self) -> Result<()> {
        let unsupported = self.features - Features::default_set();
        if unsupported.is_empty() {
            return Ok(());
        }
        Err(Error::UnsupportedFeature {
            feature: format!("{unsupported:?}"),
        })
    }
}

fn path_to_guest_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn tar_path_to_guest_string(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(Error::UnsupportedFeature {
                    feature: format!("unsafe OCI layer path {}", path.display()),
                });
            }
        }
    }

    if parts.is_empty() {
        return Ok("/".into());
    }

    Ok(format!("/{}", parts.join("/")))
}

fn tar_entry_mode<R: Read>(entry: &tar::Entry<'_, R>, fallback: u16) -> Result<u16> {
    let mode = entry.header().mode().map_err(|source| Error::Control {
        what: "read OCI layer mode",
        source,
    })?;
    Ok(u16::try_from(mode & 0o7777).unwrap_or(fallback))
}

fn handle_oci_whiteout(fs: &mut FileSystemBuilder, guest_path: &str) -> Result<bool> {
    let path = Path::new(guest_path);
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(false);
    };

    if name == ".wh..wh..opq" {
        let parent = guest_parent_path(path);
        fs.clear_dir(&parent)?;
        return Ok(true);
    }

    let Some(target_name) = name.strip_prefix(".wh.") else {
        return Ok(false);
    };

    let parent = guest_parent_path(path);
    let target = append_guest_path(&parent, target_name);
    fs.remove_path(&target)?;
    Ok(true)
}

fn guest_parent_path(path: &Path) -> String {
    path.parent()
        .map(path_to_guest_string)
        .filter(|parent| !parent.is_empty())
        .unwrap_or_else(|| "/".into())
}

fn append_guest_path(parent: &str, child: &str) -> String {
    if parent == "/" {
        format!("/{child}")
    } else {
        format!("{parent}/{child}")
    }
}

fn mode_from_metadata(metadata: &std::fs::Metadata, fallback: u16) -> u16 {
    #[cfg(unix)]
    {
        u16::try_from(metadata.permissions().mode() & 0o7777).unwrap_or(fallback)
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        fallback
    }
}
