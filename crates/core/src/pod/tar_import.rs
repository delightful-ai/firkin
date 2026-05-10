//! tar import — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[cfg(test)]
#[allow(unused_imports)]
use crate::{Error, Result};
#[cfg(test)]
#[allow(unused_imports)]
use firkin_ext4::LayerCompression;
#[cfg(test)]
#[allow(unused_imports)]
use std::ffi::OsString;
#[cfg(test)]
#[allow(unused_imports)]
use std::io;
#[cfg(test)]
#[allow(unused_imports)]
use std::io::Read;
#[cfg(test)]
#[allow(unused_imports)]
use std::path::Path;
#[cfg(test)]
#[allow(unused_imports)]
use std::path::PathBuf;
#[cfg(test)]
#[allow(clippy::too_many_lines)]
pub(crate) fn rewrite_materialization_layer_archive(
    path: &Path,
    compression: LayerCompression,
) -> Result<tempfile::NamedTempFile> {
    let reader = layer_archive_reader(path, compression)?;
    let mut source = tar::Archive::new(reader);
    let archive = tempfile::NamedTempFile::new().map_err(|error| Error::RuntimeOperation {
        operation: "create pod rootfs layer archive",
        reason: error.to_string(),
    })?;
    let file = std::fs::File::create(archive.path()).map_err(|error| Error::RuntimeOperation {
        operation: "create pod rootfs layer archive",
        reason: error.to_string(),
    })?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let entries = source.entries().map_err(|error| Error::RuntimeOperation {
        operation: "rewrite pod rootfs layer",
        reason: format!("{}: {error}", path.display()),
    })?;
    for entry in entries {
        let mut entry = entry.map_err(|error| Error::RuntimeOperation {
            operation: "rewrite pod rootfs layer",
            reason: format!("{}: {error}", path.display()),
        })?;
        let entry_path = entry.path().map_err(|error| Error::RuntimeOperation {
            operation: "rewrite pod rootfs layer",
            reason: format!("{}: {error}", path.display()),
        })?;
        let entry_path = normalize_materialization_layer_path(&entry_path)?;
        if entry_path.as_os_str().is_empty() {
            continue;
        }
        let entry_type = entry.header().entry_type();
        if entry_type.is_file() {
            let metadata = MaterializationEntryMetadata::from_header(entry.header(), 0o644);
            let mut content = Vec::new();
            entry
                .read_to_end(&mut content)
                .map_err(|error| Error::RuntimeOperation {
                    operation: "rewrite pod rootfs layer",
                    reason: format!(
                        "read {} from {}: {error}",
                        entry_path.display(),
                        path.display()
                    ),
                })?;
            append_materialization_file(&mut builder, &entry_path, metadata, &content)?;
        } else if entry_type.is_dir() {
            let metadata = MaterializationEntryMetadata::from_header(entry.header(), 0o755);
            append_materialization_directory(&mut builder, &entry_path, metadata)?;
        } else if entry_type.is_symlink() {
            let metadata = MaterializationEntryMetadata::from_header(entry.header(), 0o777);
            let target = entry
                .link_name()
                .map_err(|error| Error::RuntimeOperation {
                    operation: "rewrite pod rootfs layer",
                    reason: format!(
                        "read symlink target for {} from {}: {error}",
                        entry_path.display(),
                        path.display()
                    ),
                })?
                .ok_or_else(|| Error::RuntimeOperation {
                    operation: "rewrite pod rootfs layer",
                    reason: format!(
                        "{} has symlink entry without a target",
                        entry_path.display()
                    ),
                })?
                .into_owned();
            append_materialization_symlink(&mut builder, &entry_path, &target, metadata)?;
        } else if entry_type.is_hard_link() {
            let metadata = MaterializationEntryMetadata::from_header(entry.header(), 0o777);
            let target = entry
                .link_name()
                .map_err(|error| Error::RuntimeOperation {
                    operation: "rewrite pod rootfs layer",
                    reason: format!(
                        "read hardlink target for {} from {}: {error}",
                        entry_path.display(),
                        path.display()
                    ),
                })?
                .ok_or_else(|| Error::RuntimeOperation {
                    operation: "rewrite pod rootfs layer",
                    reason: format!(
                        "{} has hardlink entry without a target",
                        entry_path.display()
                    ),
                })?;
            let target = normalize_materialization_layer_path(&target)?;
            if target.as_os_str().is_empty() {
                return Err(Error::RuntimeOperation {
                    operation: "rewrite pod rootfs layer",
                    reason: format!("{} hardlinks to an empty target path", entry_path.display()),
                });
            }
            let relative_target = relative_symlink_target(&entry_path, &target);
            append_materialization_symlink(&mut builder, &entry_path, &relative_target, metadata)?;
        } else {
            return Err(Error::RuntimeOperation {
                operation: "rewrite pod rootfs layer",
                reason: format!(
                    "{} contains unsupported tar entry type {:?} at {}",
                    path.display(),
                    entry_type,
                    entry_path.display()
                ),
            });
        }
    }
    let encoder = builder
        .into_inner()
        .map_err(|error| Error::RuntimeOperation {
            operation: "encode pod rootfs layer archive",
            reason: error.to_string(),
        })?;
    encoder.finish().map_err(|error| Error::RuntimeOperation {
        operation: "finish pod rootfs layer archive",
        reason: error.to_string(),
    })?;
    Ok(archive)
}
#[cfg(test)]
#[derive(Clone, Copy)]
struct MaterializationEntryMetadata {
    mode: u32,
    uid: Option<u64>,
    gid: Option<u64>,
    mtime: Option<u64>,
}
#[cfg(test)]
impl MaterializationEntryMetadata {
    fn from_header(header: &tar::Header, default_mode: u32) -> Self {
        Self {
            mode: header.mode().unwrap_or(default_mode),
            uid: header.uid().ok(),
            gid: header.gid().ok(),
            mtime: header.mtime().ok(),
        }
    }
    fn apply(self, header: &mut tar::Header) {
        header.set_mode(self.mode);
        if let Some(uid) = self.uid {
            header.set_uid(uid);
        }
        if let Some(gid) = self.gid {
            header.set_gid(gid);
        }
        if let Some(mtime) = self.mtime {
            header.set_mtime(mtime);
        }
    }
}
#[cfg(test)]
fn append_materialization_file<W: io::Write>(
    builder: &mut tar::Builder<W>,
    path: &Path,
    metadata: MaterializationEntryMetadata,
    content: &[u8],
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(
        u64::try_from(content.len()).map_err(|error| Error::RuntimeOperation {
            operation: "rewrite pod rootfs layer",
            reason: format!("{} is too large to archive: {error}", path.display()),
        })?,
    );
    metadata.apply(&mut header);
    header.set_cksum();
    builder
        .append_data(&mut header, path, content)
        .map_err(|error| Error::RuntimeOperation {
            operation: "rewrite pod rootfs layer",
            reason: format!("append regular file {}: {error}", path.display()),
        })?;
    Ok(())
}
#[cfg(test)]
fn append_materialization_directory<W: io::Write>(
    builder: &mut tar::Builder<W>,
    path: &Path,
    metadata: MaterializationEntryMetadata,
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_size(0);
    metadata.apply(&mut header);
    header.set_cksum();
    builder
        .append_data(&mut header, path, io::empty())
        .map_err(|error| Error::RuntimeOperation {
            operation: "rewrite pod rootfs layer",
            reason: format!("append directory {}: {error}", path.display()),
        })?;
    Ok(())
}
#[cfg(test)]
fn append_materialization_symlink<W: io::Write>(
    builder: &mut tar::Builder<W>,
    path: &Path,
    target: &Path,
    metadata: MaterializationEntryMetadata,
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    metadata.apply(&mut header);
    header.set_cksum();
    builder
        .append_link(&mut header, path, target)
        .map_err(|error| Error::RuntimeOperation {
            operation: "rewrite pod rootfs layer",
            reason: format!(
                "append symlink {} -> {}: {error}",
                path.display(),
                target.display()
            ),
        })?;
    Ok(())
}
#[cfg(test)]
fn normalize_materialization_layer_path(path: &Path) -> Result<PathBuf> {
    validate_materialization_layer_path(path)?;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        if let std::path::Component::Normal(name) = component {
            normalized.push(name);
        }
    }
    Ok(normalized)
}
#[cfg(test)]
fn validate_materialization_layer_path(path: &Path) -> Result<()> {
    if path.is_absolute() {
        return Err(Error::RuntimeOperation {
            operation: "scan pod rootfs layer",
            reason: format!("layer path {} is absolute", path.display()),
        });
    }
    for component in path.components() {
        match component {
            std::path::Component::Prefix(_) | std::path::Component::ParentDir => {
                return Err(Error::RuntimeOperation {
                    operation: "scan pod rootfs layer",
                    reason: format!("layer path {} escapes the rootfs", path.display()),
                });
            }
            std::path::Component::Normal(name)
                if name
                    .to_str()
                    .is_some_and(|name| name == ".wh..wh..opq" || name.starts_with(".wh.")) =>
            {
                return Err(Error::RuntimeOperation {
                    operation: "scan pod rootfs layer",
                    reason: format!(
                        "OCI whiteout entry {} requires guest-side whiteout application",
                        path.display()
                    ),
                });
            }
            _ => {}
        }
    }
    Ok(())
}
#[cfg(test)]
fn relative_symlink_target(path: &Path, target: &Path) -> PathBuf {
    let path_parent = path.parent().unwrap_or_else(|| Path::new(""));
    let path_components = normal_path_components(path_parent);
    let target_components = normal_path_components(target);
    let mut common = 0;
    while common < path_components.len()
        && common < target_components.len()
        && path_components[common] == target_components[common]
    {
        common += 1;
    }
    let mut relative = PathBuf::new();
    for _ in common..path_components.len() {
        relative.push("..");
    }
    for component in &target_components[common..] {
        relative.push(component);
    }
    relative
}
#[cfg(test)]
fn normal_path_components(path: &Path) -> Vec<OsString> {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => Some(name.to_os_string()),
            _ => None,
        })
        .collect()
}
#[cfg(test)]
fn layer_archive_reader(path: &Path, compression: LayerCompression) -> Result<Box<dyn Read>> {
    let file = std::fs::File::open(path).map_err(|error| Error::RuntimeOperation {
        operation: "open pod rootfs layer",
        reason: format!("{}: {error}", path.display()),
    })?;
    match compression {
        LayerCompression::None => Ok(Box::new(file)),
        LayerCompression::Gzip => Ok(Box::new(flate2::read::GzDecoder::new(file))),
        LayerCompression::Zstd => {
            let decoder = zstd::stream::read::Decoder::new(file).map_err(|error| {
                Error::RuntimeOperation {
                    operation: "open zstd pod rootfs layer",
                    reason: format!("{}: {error}", path.display()),
                }
            })?;
            Ok(Box::new(decoder))
        }
        _ => Err(Error::RuntimeOperation {
            operation: "open pod rootfs layer",
            reason: "unsupported layer compression".to_owned(),
        }),
    }
}
