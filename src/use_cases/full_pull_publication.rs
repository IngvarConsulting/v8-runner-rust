//! A caller-owned publication of a full dump. The runner retains ownership of
//! its baseline and the original source identity while the caller publishes.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::domain::capability::{Provider, ProviderReceipt};
use crate::support::error::AppError;
use crate::support::path::{
    filesystem_object_identity, nearest_existing_canonical_path, open_file_identity,
};

/// The caller may replace the source leaf and create missing parent directories,
/// but cannot acknowledge a different directory or a retargeted existing parent.
pub(super) struct PreparedPublicationBinding {
    stage: std::fs::File,
    ancestor: std::fs::File,
    ancestor_path: PathBuf,
    parent_path: PathBuf,
    canonical_parent: PathBuf,
    canonical_target: PathBuf,
}

impl PreparedPublicationBinding {
    pub(super) fn capture(stage: &Path, target: &Path) -> Result<Self, AppError> {
        let parent_path = target.parent().ok_or_else(|| {
            AppError::Validation("external full-pull target has no parent".into())
        })?;
        let canonical_parent =
            nearest_existing_canonical_path(parent_path).map_err(publication_binding_error)?;
        let canonical_target = canonical_parent.join(target.file_name().ok_or_else(|| {
            AppError::Validation("external full-pull target has no file name".into())
        })?);
        let mut ancestor_path = parent_path.to_path_buf();
        loop {
            match std::fs::metadata(&ancestor_path) {
                Ok(_) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if !ancestor_path.pop() {
                        return Err(publication_binding_error(error));
                    }
                }
                Err(error) => return Err(publication_binding_error(error)),
            }
        }
        Ok(Self {
            stage: open_directory(stage).map_err(publication_binding_error)?,
            ancestor: open_directory(&ancestor_path).map_err(publication_binding_error)?,
            ancestor_path,
            parent_path: parent_path.to_path_buf(),
            canonical_parent,
            canonical_target,
        })
    }

    pub(super) fn canonical_target(&self) -> &Path {
        &self.canonical_target
    }

    pub(super) fn validate_published(&self, target: &Path) -> Result<(), AppError> {
        let inspected = (|| {
            let target_metadata = std::fs::symlink_metadata(target)?;
            Ok::<_, std::io::Error>(
                target_metadata.is_dir()
                    && !target_metadata.file_type().is_symlink()
                    && open_file_identity(&self.stage)? == filesystem_object_identity(target)?
                    && open_file_identity(&self.ancestor)?
                        == filesystem_object_identity(&self.ancestor_path)?
                    && nearest_existing_canonical_path(&self.parent_path)? == self.canonical_parent,
            )
        })();
        match inspected {
            Ok(true) => Ok(()),
            Ok(false) => Err(AppError::Runtime(
                "acknowledged full-pull publication does not name the prepared directory under its original parent".into(),
            )),
            Err(error) => Err(publication_binding_error(error)),
        }
    }

    pub(super) fn validate_stage(&self, stage: &Path) -> Result<(), AppError> {
        if open_file_identity(&self.stage).map_err(publication_binding_error)?
            == filesystem_object_identity(stage).map_err(publication_binding_error)?
        {
            Ok(())
        } else {
            Err(AppError::Runtime(
                "full-pull stage changed while preparing its baseline".into(),
            ))
        }
    }
}

fn open_directory(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        options
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
    }
    let directory = options.open(path)?;
    if !directory.metadata()?.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "publication binding is not a directory",
        ));
    }
    Ok(directory)
}

fn publication_binding_error(error: std::io::Error) -> AppError {
    AppError::Runtime(format!(
        "cannot verify full-pull publication binding: {error}"
    ))
}

#[derive(Debug, Clone, Copy)]
pub enum ExecutorKind {
    Local,
    Attached,
    Remote,
}

pub struct FullPullExecutor<'a> {
    pub provider: &'a ProviderReceipt,
    pub kind: ExecutorKind,
    pub executable: Option<&'a Path>,
    pub endpoint: Option<&'a str>,
}

pub struct FullPullReady<'a> {
    pub command: super::context::CommandName,
    pub requested_mode: crate::domain::dump::DumpMode,
    pub mode: crate::domain::dump::ReportedDumpMode,
    pub extension: Option<&'a str>,
    pub mode_reason: Option<&'a crate::domain::dump::DumpModeReason>,
    pub provider_dispatched: bool,
    pub source_set: &'a str,
    pub target: &'a Path,
    pub stage: &'a Path,
    pub snapshot_prepared: bool,
    pub snapshot_warning: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryCommit {
    Saved,
    NotSaved,
}

/// The implementation authorizes the actual executor and returns from publish
/// only after the caller has published. An uncertain outcome is an error; it
/// never authorizes a baseline write.
pub trait FullPullControl: std::fmt::Debug + Send + Sync {
    fn authorize(&self, executor: &FullPullExecutor<'_>) -> Result<(), AppError>;
    fn publish(&self, ready: &FullPullReady<'_>) -> Result<(), AppError>;
    fn memory_finished(&self, outcome: MemoryCommit);
}

#[derive(Debug)]
pub struct ExternalFullPull {
    stage: PathBuf,
    control: Arc<dyn FullPullControl>,
    generation: Mutex<Option<GenerationObservation>>,
}

#[derive(Debug)]
struct GenerationObservation {
    provider: Provider,
    before: Option<String>,
    after: Option<String>,
}

impl ExternalFullPull {
    pub fn new(stage: PathBuf, control: Arc<dyn FullPullControl>) -> Self {
        Self {
            stage,
            control,
            generation: Mutex::new(None),
        }
    }

    pub fn stage(&self) -> &Path {
        &self.stage
    }

    pub fn authorize(&self, executor: &FullPullExecutor<'_>) -> Result<(), AppError> {
        self.control.authorize(executor)
    }

    pub(super) fn generation_before(&self, provider: Provider, before: Option<String>) {
        *self
            .generation
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(GenerationObservation {
            provider,
            before,
            after: None,
        });
    }

    pub(super) fn generation_after(&self, after: Option<String>) {
        if let Some(observation) = self
            .generation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_mut()
        {
            observation.after = after;
        }
    }

    pub(super) fn publish(&self, ready: &FullPullReady<'_>) -> Result<(), AppError> {
        self.control.publish(ready)
    }

    pub(super) fn record_generation(
        &self,
        set: &crate::domain::source_set::SourceSetContext,
        work: &Path,
    ) -> Option<String> {
        self.generation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .and_then(|observation| {
                super::exchange_guard::record_after_dump(
                    set,
                    work,
                    observation.provider,
                    observation.before.as_deref(),
                    observation.after.as_deref(),
                )
            })
    }

    pub(super) fn memory_finished(&self, outcome: MemoryCommit) {
        self.control.memory_finished(outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_caller_can_create_missing_parents_before_publishing_the_held_stage() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join("stage");
        let target = root.path().join("new/nested/sources");
        fs::create_dir(&stage).unwrap();
        let binding = PreparedPublicationBinding::capture(&stage, &target).unwrap();
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::rename(&stage, &target).unwrap();
        binding.validate_published(&target).unwrap();
    }

    #[test]
    fn acknowledging_a_different_directory_does_not_authorize_its_baseline() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join("stage");
        let target = root.path().join("sources");
        fs::create_dir(&stage).unwrap();
        let binding = PreparedPublicationBinding::capture(&stage, &target).unwrap();
        fs::create_dir(&target).unwrap();
        assert!(binding.validate_published(&target).is_err());
        assert!(stage.is_dir());
        assert!(target.is_dir());
    }

    #[test]
    fn replacing_an_existing_parent_cannot_rebind_even_the_prepared_directory() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join("stage");
        let parent = root.path().join("parent");
        let target = parent.join("sources");
        fs::create_dir(&stage).unwrap();
        fs::create_dir(&parent).unwrap();
        let binding = PreparedPublicationBinding::capture(&stage, &target).unwrap();
        fs::rename(&parent, root.path().join("original-parent")).unwrap();
        fs::create_dir(&parent).unwrap();
        fs::rename(&stage, &target).unwrap();
        assert!(binding.validate_published(&target).is_err());
        assert!(target.is_dir());
    }
}
