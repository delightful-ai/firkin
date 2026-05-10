//! volume — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_e2b_contract::BackendError;
#[allow(unused_imports)]
use firkin_e2b_wire::VolumeEntryStat;
#[allow(unused_imports)]
use firkin_e2b_wire::VolumeInfo;
#[allow(unused_imports)]
use firkin_e2b_wire::{
    VolumeAndToken, VolumeCreateRequest, VolumeFileType, VolumeMetadataRequest, VolumeWriteOptions,
};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::BTreeMap;
/// Registry record for one E2B volume.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VolumeRecord {
    /// SDK-visible volume info.
    pub info: VolumeInfo,
    /// Volume access token.
    pub token: String,
    /// Stored volume entries keyed by normalized absolute path.
    pub entries: BTreeMap<String, VolumeContentEntry>,
}
/// In-memory volume content entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VolumeContentEntry {
    /// Entry stat.
    pub stat: VolumeEntryStat,
    /// File bytes for regular files.
    pub data: Vec<u8>,
}
/// In-memory E2B volume registry.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct LocalVolumeRegistry {
    #[allow(missing_docs)]
    pub volumes: BTreeMap<String, VolumeRecord>,
    next_volume_id: u64,
}
impl LocalVolumeRegistry {
    /// Create an empty volume registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Create a volume record.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::AlreadyExists`] when a generated id collides.
    pub fn create(&mut self, request: VolumeCreateRequest) -> Result<VolumeAndToken, BackendError> {
        self.next_volume_id = self.next_volume_id.saturating_add(1);
        let volume_id = format!("vol_{}", self.next_volume_id);
        if self.volumes.contains_key(&volume_id) {
            return Err(BackendError::AlreadyExists(volume_id));
        }
        let token = format!("tok_{}", self.next_volume_id);
        let info = VolumeInfo {
            volume_id: volume_id.clone(),
            name: request.name,
        };
        self.volumes.insert(
            volume_id,
            VolumeRecord {
                info: info.clone(),
                token: token.clone(),
                entries: BTreeMap::from([("/".to_owned(), root_volume_entry())]),
            },
        );
        Ok(VolumeAndToken {
            volume_id: info.volume_id,
            name: info.name,
            token,
        })
    }
    /// Return volume info and token.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the volume is not registered.
    pub fn get(&self, volume_id: &str) -> Result<VolumeAndToken, BackendError> {
        let record = self
            .volumes
            .get(volume_id)
            .ok_or_else(|| BackendError::NotFound(volume_id.to_owned()))?;
        Ok(VolumeAndToken {
            volume_id: record.info.volume_id.clone(),
            name: record.info.name.clone(),
            token: record.token.clone(),
        })
    }
    /// Return all registered volume infos.
    #[must_use]
    pub fn list(&self) -> Vec<VolumeInfo> {
        self.volumes
            .values()
            .map(|record| record.info.clone())
            .collect()
    }
    /// Delete a volume, returning whether it existed.
    pub fn delete(&mut self, volume_id: &str) -> bool {
        self.volumes.remove(volume_id).is_some()
    }
    /// List volume content under a path.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the volume or path is missing.
    pub fn list_dir(
        &self,
        volume_id: &str,
        path: &str,
    ) -> Result<Vec<VolumeEntryStat>, BackendError> {
        let path = normalize_volume_path(path);
        let record = self.volume(volume_id)?;
        let entry = record
            .entries
            .get(&path)
            .ok_or_else(|| BackendError::NotFound(path.clone()))?;
        if entry.stat.kind != VolumeFileType::Directory {
            return Err(BackendError::NotFound(path));
        }
        let prefix = if path == "/" {
            "/".to_owned()
        } else {
            format!("{path}/")
        };
        Ok(record
            .entries
            .iter()
            .filter(|(candidate, _)| *candidate != &path)
            .filter(|(candidate, _)| candidate.starts_with(&prefix))
            .filter(|(candidate, _)| !candidate[prefix.len()..].contains('/'))
            .map(|(_, entry)| entry.stat.clone())
            .collect())
    }
    /// Create a directory under a volume.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the volume is missing.
    pub fn make_dir(
        &mut self,
        volume_id: &str,
        path: &str,
        opts: VolumeWriteOptions,
    ) -> Result<VolumeEntryStat, BackendError> {
        let path = normalize_volume_path(path);
        if opts.force != Some(true) && self.volume(volume_id)?.entries.contains_key(&path) {
            return Err(BackendError::AlreadyExists(path));
        }
        let entry = volume_entry_stat(
            &path,
            VolumeFileType::Directory,
            0,
            opts.mode.unwrap_or(0o755),
            opts.uid.unwrap_or(0),
            opts.gid.unwrap_or(0),
        );
        self.volume_mut(volume_id)?.entries.insert(
            path,
            VolumeContentEntry {
                stat: entry.clone(),
                data: Vec::new(),
            },
        );
        Ok(entry)
    }
    /// Return stat for a volume content path.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the volume or path is missing.
    pub fn path_info(&self, volume_id: &str, path: &str) -> Result<VolumeEntryStat, BackendError> {
        let path = normalize_volume_path(path);
        Ok(self
            .volume(volume_id)?
            .entries
            .get(&path)
            .ok_or(BackendError::NotFound(path))?
            .stat
            .clone())
    }
    /// Update metadata for a volume content path.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the volume or path is missing.
    pub fn update_metadata(
        &mut self,
        volume_id: &str,
        path: &str,
        metadata: VolumeMetadataRequest,
    ) -> Result<VolumeEntryStat, BackendError> {
        let path = normalize_volume_path(path);
        let entry = self
            .volume_mut(volume_id)?
            .entries
            .get_mut(&path)
            .ok_or(BackendError::NotFound(path))?;
        if let Some(uid) = metadata.uid {
            entry.stat.uid = uid;
        }
        if let Some(gid) = metadata.gid {
            entry.stat.gid = gid;
        }
        if let Some(mode) = metadata.mode {
            entry.stat.mode = mode;
        }
        VOLUME_CONTENT_TIMESTAMP.clone_into(&mut entry.stat.ctime);
        Ok(entry.stat.clone())
    }
    /// Read file bytes from a volume content path.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the volume or file is missing.
    pub fn read_file(&self, volume_id: &str, path: &str) -> Result<Vec<u8>, BackendError> {
        let path = normalize_volume_path(path);
        let entry = self
            .volume(volume_id)?
            .entries
            .get(&path)
            .ok_or_else(|| BackendError::NotFound(path.clone()))?;
        if entry.stat.kind != VolumeFileType::File {
            return Err(BackendError::NotFound(path));
        }
        Ok(entry.data.clone())
    }
    /// Write file bytes into a volume content path.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the volume is missing.
    pub fn write_file(
        &mut self,
        volume_id: &str,
        path: &str,
        data: Vec<u8>,
        opts: VolumeWriteOptions,
    ) -> Result<VolumeEntryStat, BackendError> {
        let path = normalize_volume_path(path);
        if opts.force != Some(true) && self.volume(volume_id)?.entries.contains_key(&path) {
            return Err(BackendError::AlreadyExists(path));
        }
        let size = i64::try_from(data.len()).unwrap_or(i64::MAX);
        let stat = volume_entry_stat(
            &path,
            VolumeFileType::File,
            size,
            opts.mode.unwrap_or(0o644),
            opts.uid.unwrap_or(0),
            opts.gid.unwrap_or(0),
        );
        self.volume_mut(volume_id)?.entries.insert(
            path,
            VolumeContentEntry {
                stat: stat.clone(),
                data,
            },
        );
        Ok(stat)
    }
    /// Remove a volume content path.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the volume or path is missing.
    pub fn remove_path(&mut self, volume_id: &str, path: &str) -> Result<(), BackendError> {
        let path = normalize_volume_path(path);
        if path == "/" {
            return Err(BackendError::NotFound(path));
        }
        let removed = self.volume_mut(volume_id)?.entries.remove(&path).is_some();
        if removed {
            Ok(())
        } else {
            Err(BackendError::NotFound(path))
        }
    }
    fn volume(&self, volume_id: &str) -> Result<&VolumeRecord, BackendError> {
        self.volumes
            .get(volume_id)
            .ok_or_else(|| BackendError::NotFound(volume_id.to_owned()))
    }
    fn volume_mut(&mut self, volume_id: &str) -> Result<&mut VolumeRecord, BackendError> {
        self.volumes
            .get_mut(volume_id)
            .ok_or_else(|| BackendError::NotFound(volume_id.to_owned()))
    }
}
const VOLUME_CONTENT_TIMESTAMP: &str = "2026-05-03T12:00:00Z";
fn root_volume_entry() -> VolumeContentEntry {
    VolumeContentEntry {
        stat: volume_entry_stat("/", VolumeFileType::Directory, 0, 0o755, 0, 0),
        data: Vec::new(),
    }
}
fn volume_entry_stat(
    path: &str,
    kind: VolumeFileType,
    size: i64,
    mode: u32,
    uid: u32,
    gid: u32,
) -> VolumeEntryStat {
    VolumeEntryStat {
        name: volume_path_name(path),
        kind,
        path: path.to_owned(),
        size,
        mode,
        uid,
        gid,
        atime: VOLUME_CONTENT_TIMESTAMP.to_owned(),
        mtime: VOLUME_CONTENT_TIMESTAMP.to_owned(),
        ctime: VOLUME_CONTENT_TIMESTAMP.to_owned(),
        target: None,
    }
}
fn volume_path_name(path: &str) -> String {
    if path == "/" {
        return "/".to_owned();
    }
    path.rsplit('/')
        .find(|component| !component.is_empty())
        .unwrap_or("/")
        .to_owned()
}
pub(crate) fn normalize_volume_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_owned();
    }
    let mut normalized = String::with_capacity(trimmed.len() + 1);
    normalized.push('/');
    normalized.push_str(trimmed.trim_matches('/'));
    normalized
}
