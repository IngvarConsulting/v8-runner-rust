//! The runner-owned file infobase used by both source-to-package commands.

use crate::change_detection::runner_dirs::RunnerDirs;
use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::change_detection::analyzer::{
    self, AnalysisOutcome, ChangeDetectionError, FullSnapshot, PreparedStateUpdate, SnapshotMemory,
};
use crate::change_detection::partial_load::{self, LoadDecision};
use crate::config::model::{AppConfig, SourceFormat, SourceSetConfig};
use crate::domain::artifacts::{BuildMode, BuildReport};
use crate::domain::capability::Provider;
use crate::domain::source_set::SourceSetContext;
use crate::platform::ibcmd::IbcmdError;
use crate::platform::locator::UtilityType;
use crate::platform::process::ProcessRunner;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::fs::{remove_path_if_exists, write_file_atomically};
use crate::support::path::{
    nearest_existing_canonical_path, open_file_identity, stable_path_identity,
};
use crate::support::temp::partial_list_file;
use crate::use_cases::context::{CommandName, ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::extension_identity::extension_name_key;
use crate::use_cases::interruption::{collecting_deferrals, interruption_before_safe_point};
use crate::use_cases::source_inventory::SourceSetInventory;
use crate::use_cases::throwaway_infobase::{
    edt_sources_to_xml, Builder, EdtConversion, FileInfobase,
};

const CACHE_LAYOUT: &str = "v8-runner build infobase v1\n";
const READY: &str = "ready.json";

pub(super) fn root(work_path: &Path) -> PathBuf {
    work_path.join("temp").join("build-infobase")
}

/// This check also runs in preview and before publication, with no writes.
pub(super) fn reject_overlap(work_path: &Path, candidate: &Path) -> Result<(), AppError> {
    let base = canonical(&root(work_path))?;
    let candidate = canonical(candidate)?;
    if path_contains(&base, &candidate) || path_contains(&candidate, &base) {
        return Err(overlap_error(&base, &candidate));
    }
    Ok(())
}

fn path_contains(parent: &Path, child: &Path) -> bool {
    let identity = stable_path_identity(parent);
    child
        .ancestors()
        .any(|ancestor| stable_path_identity(ancestor) == identity)
}

fn overlap_error(base: &Path, candidate: &Path) -> AppError {
    AppError::Validation(format!(
        "path '{}' overlaps the reserved build infobase '{}'",
        candidate.display(),
        base.display()
    ))
}

fn canonical(path: &Path) -> Result<PathBuf, AppError> {
    nearest_existing_canonical_path(path)
        .map_err(|error| io_failure("resolve build path", path, error))
}

fn io_failure(action: &str, path: &Path, error: io::Error) -> AppError {
    AppError::Runtime(format!("failed to {action} '{}': {error}", path.display()))
}

fn remove(path: &Path) -> Result<(), AppError> {
    remove_path_if_exists(path).map_err(|error| io_failure("remove", path, error))
}

/// Resolve relative to a checked parent, refusing links/junctions even inside the cache.
fn child(parent: &Path, name: &str) -> Result<PathBuf, AppError> {
    let path = parent.join(name);
    let resolved = canonical(&path)?;
    let expected = canonical(parent)?.join(name);
    if stable_path_identity(&resolved) != stable_path_identity(&expected)
        || std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_symlink())
    {
        return Err(AppError::Validation(format!(
            "build infobase path '{}' is a link or junction",
            path.display()
        )));
    }
    Ok(path)
}

fn mkdir(path: &Path) -> Result<(), AppError> {
    std::fs::create_dir_all(path).map_err(|error| io_failure("create directory", path, error))
}

#[derive(Default)]
pub(super) struct BuildSession {
    started: bool,
    force_pending: bool,
    /// `--full`: every EDT set of this run is translated again, each once.
    full_translation: bool,
    translated: BTreeSet<String>,
}

pub(super) struct BuildEnvironment<'a> {
    pub context: &'a ExecutionContext,
    pub config: &'a AppConfig,
    pub builder: &'a Builder,
    pub runner: &'a dyn ProcessRunner,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct DatabaseStamp {
    bytes: u64,
    modified_ns: u128,
    object: String,
}

fn database_stamp(root: &Path) -> Result<Option<DatabaseStamp>, AppError> {
    let path = child(&child(root, "ib")?, "1Cv8.1CD")?;
    Ok(std::fs::File::open(path).ok().and_then(|file| {
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() == 0 {
            return None;
        }
        Some(DatabaseStamp {
            bytes: metadata.len(),
            object: format!("{:?}", open_file_identity(&file).ok()?),
            modified_ns: metadata
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_nanos(),
        })
    }))
}

#[derive(Serialize, Deserialize)]
struct Ready {
    platform: String,
    database: DatabaseStamp,
    slots: BTreeSet<String>,
}

impl Ready {
    fn read(path: &Path) -> io::Result<Self> {
        serde_json::from_slice(&std::fs::read(path)?).map_err(io::Error::other)
    }

    fn save(&self, path: &Path) -> Result<(), AppError> {
        write_file_atomically(path, |file| {
            serde_json::to_writer(file, self).map_err(io::Error::other)
        })
        .map_err(|error| io_failure("save build infobase memory", path, error))
    }
}

enum Load {
    Skip,
    Partial(Vec<PathBuf>, PreparedStateUpdate),
    Full,
}

struct SetLoad<'a> {
    extension: Option<&'a str>,
    context: SourceSetContext,
    slot: String,
    load: Load,
}

fn slot_key(extension: Option<&str>) -> String {
    match extension {
        Some(name) => format!(
            "ext-{:x}",
            Sha256::digest(extension_name_key(name).as_bytes())
        ),
        None => "main".to_owned(),
    }
}

fn snapshot(
    set: &SourceSetConfig,
    path: PathBuf,
    key: &str,
    storage_dir: &Path,
    source_identity: &str,
    local_path: &Path,
) -> Result<SourceSetContext, AppError> {
    let storage = child(storage_dir, &format!("{key}.redb"))?;
    Ok(SourceSetContext::new(&set.name, path, key)
        .with_build_memory(storage, source_identity.to_owned())
        .with_local_path(local_path))
}

fn analyze(
    context: &ExecutionContext,
    work_path: &Path,
    sources: &SourceSetContext,
) -> Result<AnalysisOutcome, AppError> {
    let mut interrupted =
        || interruption_before_safe_point(context, "build source analysis").is_some();
    if let Some(error) = interruption_before_safe_point(context, "build source analysis") {
        return Err(error);
    }
    if analyzer::snapshot_memory(sources, work_path) != SnapshotMemory::Own {
        return Ok(AnalysisOutcome::Fallback);
    }
    match analyzer::analyze_context(sources, work_path, &mut interrupted).outcome {
        Ok(outcome) => Ok(outcome),
        Err(ChangeDetectionError::Interrupted) => Err(interruption_before_safe_point(
            context,
            "build source analysis",
        )
        .unwrap_or_else(|| AppError::Runtime("build source analysis interrupted".to_owned()))),
        Err(_) => Ok(AnalysisOutcome::Fallback),
    }
}

fn memory_error(error: ChangeDetectionError) -> AppError {
    AppError::Runtime(error.to_string())
}

pub(super) struct PreparedBuild {
    pub base: FileInfobase,
    pub report: BuildReport,
    pub warnings: Vec<String>,
}

impl PreparedBuild {
    /// Opening a file infobase to export a package can update its on-disk stamp
    /// even without a configuration change (Designer on Windows). A successful
    /// read may advance the timestamp when the file identity and size still match. If they
    /// do not, the export itself succeeded: its package stands, and only the memory of the
    /// base is dropped, so the next run starts fresh.
    pub(super) fn record_successful_access(&mut self) -> Result<(), AppError> {
        let root = self.base.root().to_path_buf();
        let root = root.as_path();
        let path = child(root, READY)?;
        let mut ready = Ready::read(&path)
            .map_err(|error| io_failure("read build infobase memory after export", &path, error))?;
        let current = database_stamp(root)?.ok_or_else(|| {
            AppError::Runtime("build infobase file disappeared during export".to_owned())
        })?;
        if ready.database.object != current.object || ready.database.bytes != current.bytes {
            remove(&path)?;
            self.warnings.push(
                "the build infobase file was replaced or changed size during the export; the package was published and the next build starts from a fresh base"
                    .to_owned(),
            );
            return Ok(());
        }
        if ready.database != current {
            ready.database = current;
            ready.save(&path)?;
        }
        Ok(())
    }

    pub(super) fn dump(
        &self,
        env: &BuildEnvironment<'_>,
        output: &Path,
        extension: Option<&str>,
        log_file: Option<PathBuf>,
    ) -> Result<PlatformCommandResult, AppError> {
        let outcome = self
            .base
            .dump_package(env.context, env.runner, output, extension, log_file);
        if interruption_before_safe_point(env.context, "build infobase recovery").is_none()
            && match &outcome {
                Ok(result) => {
                    result.process.outcome().is_err() && result.process.interruption.is_none()
                }
                Err(error) => error.cancellation().is_none(),
            }
        {
            // A platform failure can be the first signal of a damaged base whose file stamp
            // still matches. The next invocation must start fresh. Keep the original error.
            let invalidated = child(self.base.root(), READY).and_then(|ready| remove(&ready));
            if let Err(error) = invalidated {
                tracing::warn!(%error, "could not invalidate the build infobase");
            }
        }
        outcome
    }
}

impl BuildSession {
    pub(super) fn request_full(&mut self) {
        self.force_pending = !self.started;
        // The walk asks for every set of the run; the request outlives its first set.
        self.full_translation |= !self.started;
    }

    /// `--full` translates every EDT set of the run again, each of them once.
    fn takes_full_translation(&mut self, set: &str) -> bool {
        self.translated.insert(set.to_owned()) && self.full_translation
    }

    /// Synchronize the needed slots before exporting a package or building external files.
    /// Only this method owns the database and its memory. Publication is owned by the caller.
    pub(super) fn prepare(
        &mut self,
        env: &BuildEnvironment<'_>,
        set: &SourceSetConfig,
        extension: Option<&str>,
        parent: Option<&SourceSetConfig>,
        log_file: Option<PathBuf>,
    ) -> Result<PreparedBuild, AppError> {
        let forced = std::mem::take(&mut self.force_pending);
        self.started = true;
        let root = open_root(env)?;
        let base = FileInfobase::new(root.clone(), env.builder.clone());
        let ready_path = child(&root, READY)?;
        let platform = env.builder.platform_identity();
        let loaded = Ready::read(&ready_path).ok();
        let database = database_stamp(&root)?;
        let mut reset = match (&loaded, &database) {
            _ if forced => Some("full requested"),
            (Some(ready), _) if ready.platform != platform => Some("platform version changed"),
            (Some(ready), Some(stamp)) if ready.database == *stamp => None,
            (None, None) if !child(&root, "ib")?.exists() => Some("new build infobase"),
            _ => Some("build infobase or memory is invalid"),
        };
        let mut slots = loaded.map_or_else(BTreeSet::new, |ready| ready.slots);
        let memory = child(&root, "loaded")?;
        let mut loads = Vec::with_capacity(2);
        let mut warnings = Vec::new();
        for (set, extension) in parent
            .into_iter()
            .map(|parent| (parent, None))
            .chain(std::iter::once((set, extension)))
        {
            let translate_again = self.takes_full_translation(&set.name);
            let (xml, identity, notes) = sources_in_xml(env, &root, set, translate_again)?;
            warnings.extend(notes);
            let slot = slot_key(extension);
            let sources = snapshot(set, xml, &slot, &memory, &identity, &env.config.local_path)?;
            let load = if reset.is_none() && slots.contains(&slot) {
                match analyze(env.context, &env.config.work_path, &sources)? {
                    AnalysisOutcome::NoChanges => Load::Skip,
                    AnalysisOutcome::Changes { changes, prepared } => {
                        match partial_load::decide_build(
                            &changes,
                            sources.path(),
                            RunnerDirs::of(env.config),
                        ) {
                            LoadDecision::Partial(paths) => Load::Partial(paths, prepared),
                            LoadDecision::Full => Load::Full,
                        }
                    }
                    AnalysisOutcome::Fallback => Load::Full,
                }
            } else {
                Load::Full
            };
            if reset.is_none()
                && matches!(load, Load::Full)
                && (slots.contains(&slot) || extension.is_none() && !slots.is_empty())
            {
                reset = Some("no safe partial load for the existing configuration");
            }
            loads.push(SetLoad {
                extension,
                context: sources,
                slot,
                load,
            });
        }

        if reset.is_some() {
            for item in &mut loads {
                item.load = Load::Full;
            }
            slots.clear();
        }
        let report = if let Some(reason) = reset {
            BuildReport::new(BuildMode::Full, reason)
        } else if loads.iter().any(|item| matches!(item.load, Load::Full)) {
            BuildReport::new(BuildMode::Full, "source set is new to the build infobase")
        } else if loads
            .iter()
            .any(|item| matches!(item.load, Load::Partial(..)))
        {
            BuildReport::new(BuildMode::Partial, "source files changed")
        } else {
            BuildReport::new(BuildMode::Unchanged, "source files are unchanged")
        };
        if report.mode == BuildMode::Unchanged {
            return Ok(PreparedBuild {
                base,
                report,
                warnings,
            });
        }

        // Capture before dispatch. A user edit while the utility is working must not become
        // the recorded state. Remove readiness before changing the DB or any loaded snapshot.
        let snapshots = loads
            .iter()
            .map(|item| match item.load {
                Load::Full => {
                    analyzer::prepare_source_snapshot(&item.context, &env.config.work_path)
                        .map(Some)
                        .map_err(memory_error)
                }
                _ => Ok(None),
            })
            .collect::<Result<Vec<Option<FullSnapshot>>, AppError>>()?;
        if let Some(error) = interruption_before_safe_point(env.context, "build infobase update") {
            return Err(error);
        }
        remove(&ready_path)?;
        if reset.is_some() {
            for name in ["ib", "data", "loaded"] {
                remove(&child(&root, name)?)?;
            }
            let created = base.create(
                env.context,
                env.runner,
                InterruptionSafetyClass::GracefulThenKill,
            )?;
            if let Some(error) = interruption_before_safe_point(env.context, "sources load") {
                return Err(if created.is_empty() {
                    error
                } else {
                    error.with_context(created.join("; "))
                });
            }
            warnings.extend(created);
        }
        for (item, snapshot) in loads.into_iter().zip(snapshots) {
            if matches!(item.load, Load::Skip) {
                continue;
            }
            if let Some(error) = interruption_before_safe_point(env.context, "sources load") {
                return Err(error);
            }
            let ((), notes) = collecting_deferrals(|deferrals| {
                let load_result = match item.load {
                    Load::Skip => return Ok(()),
                    Load::Full => base.load_full(
                        env.context,
                        env.runner,
                        item.context.path(),
                        item.extension,
                        log_file.clone(),
                        InterruptionSafetyClass::GracefulThenKill,
                    )?,
                    Load::Partial(ref paths, _) => match env.builder.provider {
                        Provider::Designer => {
                            let list =
                                partial_list_file(&env.config.work_path).map_err(|error| {
                                    io_failure("prepare partial list", &root, error)
                                })?;
                            partial_load::write_list_file(paths, item.context.path(), list.path())
                                .map_err(|error| {
                                    io_failure("write partial list", list.path(), error)
                                })?;
                            let list = list.into_temp_path();
                            base.designer(
                                env.context,
                                env.runner,
                                log_file.clone(),
                                InterruptionSafetyClass::GracefulThenKill,
                            )
                            .load_config_from_files_partial_untouched(
                                item.context.path(),
                                &list,
                                item.extension,
                            )
                            .map_err(AppError::from)?
                        }
                        Provider::Ibcmd => {
                            let relative = partial_load::relative_paths(paths, item.context.path())
                                .map_err(|error| {
                                    io_failure("resolve partial paths", item.context.path(), error)
                                })?;
                            base.ibcmd(
                                env.context,
                                env.runner,
                                InterruptionSafetyClass::GracefulThenKill,
                            )
                            .config_import_partial(item.context.path(), &relative, item.extension)
                            .map_err(|error| match error {
                                IbcmdError::PartialImportCommandLineTooLong { file_count } => {
                                    AppError::SourceLoadNotStarted(format!(
                                        "cannot start partial ibcmd import of {file_count} files: operating-system command-line or executable path limit exceeded. Rerun the command for a full build in a fresh build infobase"
                                    ))
                                }
                                error => AppError::from(error),
                            })?
                        }
                        other => {
                            return Err(crate::use_cases::unimplemented_provider(
                                crate::domain::capability::Operation::Make,
                                other,
                            ))
                        }
                    },
                };
                deferrals.note_result("build infobase load", &load_result);
                ensure_platform_success(item.context.name(), &load_result)?;
                if let Some(error) = interruption_before_safe_point(env.context, "package dump") {
                    return Err(error);
                }
                Ok(())
            })?;
            warnings.extend(notes);
            if let Load::Partial(_, prepared) = item.load {
                analyzer::commit_success(&item.context, &root, &prepared).map_err(memory_error)?;
            }
            if let Some(snapshot) = snapshot {
                analyzer::commit_full_snapshot(&item.context, &root, &snapshot)
                    .map_err(memory_error)?;
            }
            slots.insert(item.slot);
        }
        let database = database_stamp(&root)?.ok_or_else(|| {
            AppError::Platform("platform did not produce a build infobase file".to_owned())
        })?;
        let state = Ready {
            platform,
            database,
            slots,
        };
        state.save(&ready_path)?;
        Ok(PreparedBuild {
            base,
            report,
            warnings,
        })
    }
}

pub(super) fn validate_project(config: &AppConfig) -> Result<(), AppError> {
    let inventory = SourceSetInventory::new(config);
    let work_path = canonical(&config.work_path)?;
    let base = canonical(&root(&config.work_path))?;
    for set in &config.source_sets {
        let source = canonical(&inventory.source_path(set))?;
        let contains_work = work_path
            .parent()
            .is_some_and(|parent| path_contains(&source, parent));
        if path_contains(&base, &source) || path_contains(&source, &base) && !contains_work {
            return Err(overlap_error(&base, &source));
        }
    }
    Ok(())
}

/// A process killed while creating the directory leaves nothing but its markers.
fn holds_only_its_markers(root: &Path) -> io::Result<bool> {
    for entry in std::fs::read_dir(root)? {
        let name = entry?.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(".gitignore") && !name.starts_with(".build-cache") {
            return Ok(false);
        }
    }
    Ok(true)
}

fn open_root(env: &BuildEnvironment<'_>) -> Result<PathBuf, AppError> {
    if let Some(error) = interruption_before_safe_point(env.context, "build infobase creation") {
        return Err(error);
    }
    let temp = child(&env.config.work_path, "temp")?;
    let root = child(&temp, "build-infobase")?;
    validate_project(env.config)?;
    let layout = child(&root, ".build-cache")?;
    let ignore = child(&root, ".gitignore")?;
    mkdir(&temp)?;
    match std::fs::create_dir(&root) {
        Ok(()) => {
            write_file_atomically(&ignore, |file| file.write_all(b"*\n"))
                .map_err(|error| io_failure("ignore build infobase in Git", &ignore, error))?;
            write_file_atomically(&layout, |file| file.write_all(CACHE_LAYOUT.as_bytes()))
                .map_err(|error| io_failure("record build cache layout", &layout, error))?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if std::fs::read_to_string(&layout).as_deref().ok() != Some(CACHE_LAYOUT) {
                if !holds_only_its_markers(&root)
                    .map_err(|error| io_failure("read build infobase directory", &root, error))?
                {
                    return Err(AppError::Validation(format!(
                        "build infobase directory '{}' has no recognized cache layout; it holds only a rebuildable cache, so remove the directory and repeat the command",
                        root.display()
                    )));
                }
                write_file_atomically(&layout, |file| file.write_all(CACHE_LAYOUT.as_bytes()))
                    .map_err(|error| io_failure("record build cache layout", &layout, error))?;
            }
            if std::fs::read_to_string(&ignore).as_deref().ok() != Some("*\n") {
                write_file_atomically(&ignore, |file| file.write_all(b"*\n"))
                    .map_err(|error| io_failure("ignore build infobase in Git", &ignore, error))?;
            }
        }
        Err(error) => return Err(io_failure("create build infobase directory", &root, error)),
    }
    // ibcmd also writes its own catalogue here when opening a Designer-created database.
    child(&root, "data")?;
    Ok(root)
}

pub(super) fn edt_identity(config: &AppConfig) -> String {
    PlatformUtilities::from_config(config)
        .locate(UtilityType::EdtCli)
        .map_or_else(
            |_| "no-edt".to_owned(),
            |location| {
                let path = location.path;
                stable_path_identity(&path.canonicalize().unwrap_or(path))
            },
        )
}

fn sources_in_xml(
    env: &BuildEnvironment<'_>,
    root: &Path,
    set: &SourceSetConfig,
    force_export: bool,
) -> Result<(PathBuf, String, Vec<String>), AppError> {
    let input = SourceSetInventory::new(env.config).source_path(set);
    let identity = format!(
        "{:?}:{}:{}:{}",
        env.config.format,
        set.purpose.as_str(),
        set.name,
        stable_path_identity(&canonical(&input)?)
    );
    if env.config.format == SourceFormat::Designer {
        return Ok((input, identity, Vec::new()));
    }
    // The XML is the output of this EDT: its installation is part of what the memory describes.
    let identity = format!("{identity}:{}", edt_identity(env.config));

    // Keep this cache across database replacement. The XML and the EDT inputs each have
    // a snapshot: a partial/externally changed export never authorizes a skipped translation.
    let exports = child(root, "xml")?;
    let key = format!("{:x}", Sha256::digest(set.name.as_bytes()));
    let target = child(&exports, &key)?;
    let memory = child(root, "exported")?;
    let local_path = &env.config.local_path;
    let edt = snapshot(
        set,
        input,
        &format!("{key}-edt"),
        &memory,
        &identity,
        local_path,
    )?;
    let xml = snapshot(
        set,
        target.clone(),
        &format!("{key}-xml"),
        &memory,
        &identity,
        local_path,
    )?;
    if !force_export
        && target.join("Configuration.xml").is_file()
        && matches!(
            analyze(env.context, &env.config.work_path, &edt)?,
            AnalysisOutcome::NoChanges
        )
        && matches!(
            analyze(env.context, &env.config.work_path, &xml)?,
            AnalysisOutcome::NoChanges
        )
    {
        return Ok((target, identity, Vec::new()));
    }
    let prepared =
        analyzer::prepare_source_snapshot(&edt, &env.config.work_path).map_err(memory_error)?;
    let edt_memory = edt
        .storage_path(root)
        .ok_or_else(|| AppError::Runtime("build cache has no EDT snapshot path".to_owned()))?;
    remove(&edt_memory)?;
    let timeout = if env.context.command() == CommandName::Convert {
        env.context.edt_timeout()
    } else {
        None
    };
    let warnings = edt_sources_to_xml(
        env.context,
        env.config,
        set,
        &target,
        EdtConversion::OneShot { timeout },
    )?;
    let generated = analyzer::prepare_full_snapshot(&xml, &target).map_err(memory_error)?;
    analyzer::commit_full_snapshot(&xml, root, &generated).map_err(memory_error)?;
    analyzer::commit_full_snapshot(&edt, root, &prepared).map_err(memory_error)?;
    Ok((target, identity, warnings))
}

pub(super) fn ensure_platform_success(
    source_set_name: &str,
    result: &PlatformCommandResult,
) -> Result<(), AppError> {
    let Err(code) = result.process.outcome() else {
        return Ok(());
    };

    let mut details = vec![format!(
        "package build failed for source-set '{source_set_name}' with exit code {code}"
    )];
    if !result.process.stdout.trim().is_empty() {
        details.push(format!("stdout: {}", result.process.stdout.trim()));
    }
    if !result.process.stderr.trim().is_empty() {
        details.push(format!("stderr: {}", result.process.stderr.trim()));
    }
    if let Some(log) = result
        .platform_log
        .as_deref()
        .map(str::trim)
        .filter(|log| !log.is_empty())
    {
        details.push(format!("platform log: {log}"));
    } else if let Some(path) = result.platform_log_path.as_ref() {
        details.push(format!("platform log path: {}", path.display()));
    }
    if let Some(error) = result.platform_log_read_error.as_deref() {
        details.push(error.to_owned());
    }

    Err(AppError::Platform(details.join("; ")))
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::child;
    use super::{holds_only_its_markers, BuildSession};

    #[test]
    fn a_full_build_translates_every_set_of_the_run_once() {
        let mut session = BuildSession::default();
        session.request_full();
        assert!(session.takes_full_translation("main"));
        session.started = true;

        // Обход без набора просит полную сборку у каждого набора прогона.
        session.request_full();
        assert!(!session.force_pending, "the base is rebuilt only once");
        assert!(session.takes_full_translation("ext"));
        assert!(!session.takes_full_translation("main"));
    }

    #[test]
    fn a_build_without_the_key_translates_only_what_changed() {
        let mut session = BuildSession::default();

        assert!(!session.takes_full_translation("main"));
    }

    /// Прерванное создание каталога не должно закрывать сборку навсегда, а чужой каталог
    /// раннер не присваивает.
    #[test]
    fn only_the_markers_of_a_killed_creation_count_as_a_new_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(holds_only_its_markers(dir.path()).expect("empty"));
        std::fs::write(dir.path().join(".gitignore"), "*\n").expect("marker");
        assert!(holds_only_its_markers(dir.path()).expect("marker only"));
        std::fs::write(dir.path().join("1Cv8.1CD"), "base").expect("foreign");
        assert!(!holds_only_its_markers(dir.path()).expect("foreign file"));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_inside_the_build_directory_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).expect("elsewhere");
        std::os::unix::fs::symlink(&elsewhere, dir.path().join("ib")).expect("link");

        let error = child(dir.path(), "ib").expect_err("a link is refused");

        assert!(error.to_string().contains("link or junction"), "{error}");
    }
}
