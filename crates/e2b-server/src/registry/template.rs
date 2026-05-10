//! template — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::routes::encode_path_segment;
#[allow(unused_imports)]
use firkin_e2b_contract::BackendError;
#[allow(unused_imports)]
use firkin_e2b_contract::PreparedTemplate;
#[allow(unused_imports)]
use firkin_e2b_wire::{
    AssignTemplateTags, AssignedTemplateTags, RemoveTemplateTags, TemplateAliasInfo,
    TemplateBuildInfo, TemplateBuildLogs, TemplateBuildRequest, TemplateBuildRequestInfo,
    TemplateBuildStatus, TemplateFileUpload, TemplateInstructionKind, TemplateUpdateInfo,
    TemplateUpdateRequest, TemplateWithBuilds,
};
#[allow(unused_imports)]
use firkin_e2b_wire::{
    BuildLogEntry, TemplateBuild, TemplateBuildStart, TemplateInfo, TemplateTag,
};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::BTreeMap;
/// Registry record for one E2B template.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TemplateRecord {
    /// Template summary.
    pub info: TemplateInfo,
    /// Builds for this template.
    pub builds: Vec<TemplateBuild>,
    /// Structured build logs keyed by build id.
    pub build_logs: BTreeMap<String, Vec<BuildLogEntry>>,
    /// Build start inputs keyed by build id.
    pub build_inputs: BTreeMap<String, TemplateBuildStart>,
    /// Runtime-prepared template artifacts keyed by build id.
    pub prepared_templates: BTreeMap<String, PreparedTemplate>,
    /// Tags keyed by tag name.
    pub tags: BTreeMap<String, TemplateTag>,
    /// Uploaded COPY archives keyed by file hash.
    pub uploaded_files: BTreeMap<String, Vec<u8>>,
}
/// In-memory E2B template registry.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct LocalTemplateRegistry {
    #[allow(missing_docs)]
    pub templates: BTreeMap<String, TemplateRecord>,
    #[allow(missing_docs)]
    pub aliases: BTreeMap<String, String>,
    next_template_id: u64,
    next_build_id: u64,
    #[allow(missing_docs)]
    pub now: String,
}
impl LocalTemplateRegistry {
    /// Create an empty template registry with a fixed timestamp.
    #[must_use]
    pub fn new(now: impl Into<String>) -> Self {
        Self {
            now: now.into(),
            ..Self::default()
        }
    }
    /// Request a new template build.
    pub fn request_build(&mut self, request: TemplateBuildRequest) -> TemplateBuildRequestInfo {
        self.next_template_id = self.next_template_id.saturating_add(1);
        self.next_build_id = self.next_build_id.saturating_add(1);
        let template_id = format!("tpl_{}", self.next_template_id);
        let build_id = format!("bld_{}", self.next_build_id);
        let names = request.name.into_iter().collect::<Vec<_>>();
        for name in &names {
            self.aliases.insert(name.clone(), template_id.clone());
        }
        let build = TemplateBuild {
            build_id: build_id.clone(),
            status: TemplateBuildStatus::Waiting,
            created_at: self.now.clone(),
            updated_at: self.now.clone(),
            finished_at: None,
            cpu_count: request.cpu_count.unwrap_or(2),
            memory_mb: request.memory_mb.unwrap_or(1024),
            disk_size_mb: None,
            envd_version: None,
        };
        let info = TemplateInfo {
            template_id: template_id.clone(),
            build_id: Some(build_id.clone()),
            public: false,
            aliases: Vec::new(),
            names: names.clone(),
            created_at: self.now.clone(),
            updated_at: self.now.clone(),
            last_spawned_at: None,
            spawn_count: 0,
            build_count: 1,
            envd_version: None,
            build_status: Some(TemplateBuildStatus::Waiting),
        };
        self.templates.insert(
            template_id.clone(),
            TemplateRecord {
                info,
                builds: vec![build],
                build_logs: BTreeMap::new(),
                build_inputs: BTreeMap::new(),
                prepared_templates: BTreeMap::new(),
                tags: request
                    .tags
                    .iter()
                    .cloned()
                    .map(|tag| {
                        (
                            tag.clone(),
                            TemplateTag {
                                tag,
                                build_id: build_id.clone(),
                                created_at: self.now.clone(),
                            },
                        )
                    })
                    .collect(),
                uploaded_files: BTreeMap::new(),
            },
        );
        TemplateBuildRequestInfo {
            template_id,
            build_id,
            public: false,
            names,
            tags: request.tags,
            aliases: Vec::new(),
        }
    }
    /// List registered templates.
    #[must_use]
    pub fn list(&self) -> Vec<TemplateInfo> {
        self.templates
            .values()
            .map(|record| record.info.clone())
            .collect()
    }
    /// Return a template and its builds.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the template is not registered.
    pub fn get(&self, template_id: &str) -> Result<TemplateWithBuilds, BackendError> {
        let record = self.template(template_id)?;
        Ok(TemplateWithBuilds {
            template_id: record.info.template_id.clone(),
            public: record.info.public,
            aliases: record.info.aliases.clone(),
            names: record.info.names.clone(),
            created_at: record.info.created_at.clone(),
            updated_at: record.info.updated_at.clone(),
            last_spawned_at: record.info.last_spawned_at.clone(),
            spawn_count: record.info.spawn_count,
            builds: record.builds.clone(),
        })
    }
    /// Delete a template, returning whether it existed.
    pub fn delete(&mut self, template_id: &str) -> bool {
        let Some(record) = self.templates.remove(template_id) else {
            return false;
        };
        for name in record.info.names {
            self.aliases.remove(&name);
        }
        true
    }
    /// Update template public visibility.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the template is not registered.
    pub fn update(
        &mut self,
        template_id: &str,
        request: TemplateUpdateRequest,
    ) -> Result<TemplateUpdateInfo, BackendError> {
        let now = self.now.clone();
        let record = self.template_mut(template_id)?;
        if let Some(public) = request.public {
            record.info.public = public;
        }
        record.info.updated_at = now;
        Ok(TemplateUpdateInfo {
            names: record.info.names.clone(),
        })
    }
    /// Return a synthetic file-upload response.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the template is not registered.
    pub fn file_upload(
        &self,
        template_id: &str,
        hash: &str,
        origin: Option<&str>,
    ) -> Result<TemplateFileUpload, BackendError> {
        let record = self.template(template_id)?;
        let present = record.uploaded_files.contains_key(hash);
        Ok(TemplateFileUpload {
            present,
            url: (!present).then(|| {
                format!(
                    "{}/templates/{}/files/{}/upload",
                    origin.unwrap_or("http://upload.local"),
                    encode_path_segment(template_id),
                    encode_path_segment(hash)
                )
            }),
        })
    }
    /// Store a file upload for a template.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the template is not registered.
    pub fn upload_file(
        &mut self,
        template_id: &str,
        hash: &str,
        bytes: Vec<u8>,
    ) -> Result<(), BackendError> {
        self.template_mut(template_id)?
            .uploaded_files
            .insert(hash.to_owned(), bytes);
        Ok(())
    }
    /// Return uploaded file bytes.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the template or upload is missing.
    pub fn uploaded_file(&self, template_id: &str, hash: &str) -> Result<&[u8], BackendError> {
        self.template(template_id)?
            .uploaded_files
            .get(hash)
            .map(Vec::as_slice)
            .ok_or_else(|| BackendError::NotFound(hash.to_owned()))
    }
    /// Mark a build as started.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the template or build is missing.
    pub fn start_build(
        &mut self,
        template_id: &str,
        build_id: &str,
        start: TemplateBuildStart,
    ) -> Result<(), BackendError> {
        let now = self.now.clone();
        let record = self.template_mut(template_id)?;
        validate_template_build_inputs(record, &start)?;
        let build = record
            .builds
            .iter_mut()
            .find(|build| build.build_id == build_id)
            .ok_or_else(|| BackendError::NotFound(build_id.to_owned()))?;
        build.status = TemplateBuildStatus::Building;
        build.updated_at.clone_from(&now);
        record.build_inputs.insert(build_id.to_owned(), start);
        record.info.build_status = Some(TemplateBuildStatus::Building);
        record.info.updated_at = now;
        Ok(())
    }
    /// Return the build start input for a build.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the template, build, or input is missing.
    pub fn build_input(
        &self,
        template_id: &str,
        build_id: &str,
    ) -> Result<&TemplateBuildStart, BackendError> {
        self.template(template_id)?
            .build_inputs
            .get(build_id)
            .ok_or_else(|| BackendError::NotFound(build_id.to_owned()))
    }
    /// Return uploaded COPY archives for a build start request.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when a required upload is missing.
    pub fn uploaded_files_for_build(
        &self,
        template_id: &str,
        start: &TemplateBuildStart,
    ) -> Result<BTreeMap<String, Vec<u8>>, BackendError> {
        let record = self.template(template_id)?;
        let mut files = BTreeMap::new();
        for step in &start.steps {
            if step.kind != TemplateInstructionKind::Copy {
                continue;
            }
            let hash = step.files_hash.as_deref().ok_or_else(|| {
                BackendError::Runtime("template COPY step is missing filesHash".to_owned())
            })?;
            let bytes = record
                .uploaded_files
                .get(hash)
                .ok_or_else(|| BackendError::NotFound(hash.to_owned()))?;
            files.insert(hash.to_owned(), bytes.clone());
        }
        Ok(files)
    }
    /// Store a prepared template artifact.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the template is not registered.
    pub fn set_prepared_template(
        &mut self,
        template_id: &str,
        prepared: PreparedTemplate,
    ) -> Result<(), BackendError> {
        self.template_mut(template_id)?
            .prepared_templates
            .insert(prepared.build_id.clone(), prepared);
        Ok(())
    }
    /// Return the latest prepared template for a template id or alias.
    pub fn latest_prepared_template(&self, template_id_or_alias: &str) -> Option<PreparedTemplate> {
        let template_id = self
            .aliases
            .get(template_id_or_alias)
            .map_or(template_id_or_alias, String::as_str);
        let record = self.templates.get(template_id)?;
        record
            .builds
            .iter()
            .rev()
            .find(|build| build.status == TemplateBuildStatus::Ready)
            .and_then(|build| record.prepared_templates.get(&build.build_id))
            .cloned()
    }
    /// Return the latest ready prepared template for every registered template.
    #[must_use]
    pub fn latest_prepared_templates(&self) -> Vec<PreparedTemplate> {
        self.templates
            .values()
            .filter_map(|record| {
                record
                    .builds
                    .iter()
                    .rev()
                    .find(|build| build.status == TemplateBuildStatus::Ready)
                    .and_then(|build| record.prepared_templates.get(&build.build_id))
                    .cloned()
            })
            .collect()
    }
    /// Set a build status.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the template or build is missing.
    pub fn set_build_status(
        &mut self,
        template_id: &str,
        build_id: &str,
        status: TemplateBuildStatus,
    ) -> Result<(), BackendError> {
        let now = self.now.clone();
        let record = self.template_mut(template_id)?;
        let build = record
            .builds
            .iter_mut()
            .find(|build| build.build_id == build_id)
            .ok_or_else(|| BackendError::NotFound(build_id.to_owned()))?;
        build.status = status;
        build.updated_at.clone_from(&now);
        if matches!(
            status,
            TemplateBuildStatus::Ready | TemplateBuildStatus::Error
        ) {
            build.finished_at = Some(now.clone());
        }
        record.info.build_status = Some(status);
        record.info.updated_at = now;
        Ok(())
    }
    /// Return build status response.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the template or build is missing.
    pub fn build_status(
        &self,
        template_id: &str,
        build_id: &str,
    ) -> Result<TemplateBuildInfo, BackendError> {
        let record = self.template(template_id)?;
        let build = record
            .builds
            .iter()
            .find(|build| build.build_id == build_id)
            .ok_or_else(|| BackendError::NotFound(build_id.to_owned()))?;
        let log_entries = record.build_logs.get(build_id).cloned().unwrap_or_default();
        Ok(TemplateBuildInfo {
            template_id: template_id.to_owned(),
            build_id: build_id.to_owned(),
            status: build.status,
            logs: log_entries
                .iter()
                .map(|entry| entry.message.clone())
                .collect(),
            log_entries,
            reason: None,
        })
    }
    /// Append a build log entry.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the template is missing.
    pub fn push_build_log(
        &mut self,
        template_id: &str,
        build_id: &str,
        entry: BuildLogEntry,
    ) -> Result<(), BackendError> {
        self.template(template_id)?;
        self.build_logs_mut(template_id, build_id).push(entry);
        Ok(())
    }
    /// Return build logs.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the template is missing.
    pub fn build_logs(
        &self,
        template_id: &str,
        build_id: &str,
    ) -> Result<TemplateBuildLogs, BackendError> {
        let record = self.template(template_id)?;
        Ok(TemplateBuildLogs {
            logs: record.build_logs.get(build_id).cloned().unwrap_or_default(),
        })
    }
    /// Resolve an alias.
    #[must_use]
    pub fn alias(&self, name: &str) -> Option<TemplateAliasInfo> {
        let template_id = self.aliases.get(name)?;
        let record = self.templates.get(template_id)?;
        Some(TemplateAliasInfo {
            template_id: template_id.clone(),
            public: record.info.public,
        })
    }
    /// Assign tags to a target.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the target template name/id is
    /// not registered.
    pub fn assign_tags(
        &mut self,
        request: AssignTemplateTags,
    ) -> Result<AssignedTemplateTags, BackendError> {
        let template_id = self
            .aliases
            .get(request.target.split(':').next().unwrap_or(&request.target))
            .cloned()
            .unwrap_or_else(|| request.target.clone());
        let now = self.now.clone();
        let record = self.template_mut(&template_id)?;
        let build_id = record.info.build_id.clone().unwrap_or_default();
        for tag in &request.tags {
            record.tags.insert(
                tag.clone(),
                TemplateTag {
                    tag: tag.clone(),
                    build_id: build_id.clone(),
                    created_at: now.clone(),
                },
            );
        }
        Ok(AssignedTemplateTags {
            tags: request.tags,
            build_id,
        })
    }
    /// Remove tags from a template name.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the name is not registered.
    pub fn remove_tags(&mut self, request: RemoveTemplateTags) -> Result<(), BackendError> {
        let template_id = self
            .aliases
            .get(&request.name)
            .cloned()
            .ok_or_else(|| BackendError::NotFound(request.name.clone()))?;
        let record = self.template_mut(&template_id)?;
        for tag in request.tags {
            record.tags.remove(&tag);
        }
        Ok(())
    }
    /// List tags for a template.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the template is not registered.
    pub fn tags(&self, template_id: &str) -> Result<Vec<TemplateTag>, BackendError> {
        Ok(self.template(template_id)?.tags.values().cloned().collect())
    }
    #[allow(missing_docs)]
    pub fn template(&self, template_id: &str) -> Result<&TemplateRecord, BackendError> {
        self.templates
            .get(template_id)
            .ok_or_else(|| BackendError::NotFound(template_id.to_owned()))
    }
    fn template_mut(&mut self, template_id: &str) -> Result<&mut TemplateRecord, BackendError> {
        self.templates
            .get_mut(template_id)
            .ok_or_else(|| BackendError::NotFound(template_id.to_owned()))
    }
    fn build_logs_mut(&mut self, template_id: &str, build_id: &str) -> &mut Vec<BuildLogEntry> {
        self.templates
            .get_mut(template_id)
            .expect("template checked before build_logs_mut")
            .build_logs
            .entry(build_id.to_owned())
            .or_default()
    }
}
fn validate_template_build_inputs(
    record: &TemplateRecord,
    start: &TemplateBuildStart,
) -> Result<(), BackendError> {
    for step in &start.steps {
        if step.kind != TemplateInstructionKind::Copy {
            continue;
        }
        let hash = step.files_hash.as_deref().ok_or_else(|| {
            BackendError::Runtime("template COPY step is missing filesHash".to_owned())
        })?;
        if !record.uploaded_files.contains_key(hash) {
            return Err(BackendError::NotFound(hash.to_owned()));
        }
    }
    Ok(())
}
