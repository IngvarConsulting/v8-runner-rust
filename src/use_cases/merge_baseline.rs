use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::change_detection::analyzer::{self, FullSnapshot};
use crate::config::model::AppConfig;
use crate::domain::capability::Provider;
use crate::domain::source_set::SourceSetContext;
use crate::domain::status::{GenerationAfter, HolderStatus};
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::path::{filesystem_object_identity, FilesystemObjectIdentity};

use super::agent_session::{GenerationLedger, TokenScope};
use super::context::ExecutionContext;
use super::exchange_guard;
use super::generation_reader::{read_with_record_tool, RecordToolAnswer};
use super::result::{Generations, UseCaseError, UseCaseErrorKind};
use super::version_file::{RunnerVersionFile, VERSION_FILE_NAME};

const STORAGE_PREFIX: &str = "working-merge-";
const RECEIPT_FILE: &str = "baseline.json";
const RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_PREPARED: usize = 16;

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeBaselineReceipt {
    pub id: String,
    pub generation: String,
    pub provider: Provider,
    #[schemars(required, extend("type" = ["string", "null"]))]
    pub base_generation: Option<String>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Binding {
    identity: String,
    project: PathBuf,
    base_file: FilesystemObjectIdentity,
    owners: Vec<HolderStatus>,
}

impl Binding {
    fn of(config: &AppConfig, set: &SourceSetContext) -> Result<Self, AppError> {
        let identity = set.storage_identity().ok_or_else(|| {
            AppError::Validation("the selected set has no infobase memory".into())
        })?;
        let base = super::infobase_owner::file_infobase_dir(config)
            .ok_or_else(|| AppError::Validation("working merge requires a file infobase".into()))?;
        let holders = super::infobase_owner::holders(config).ok_or_else(|| {
            AppError::Validation("the file infobase owner cannot be verified".into())
        })?;
        let owners = holders.owners.ok_or_else(|| {
            AppError::Validation(
                holders
                    .reason
                    .unwrap_or_else(|| "unknown infobase owner".into()),
            )
        })?;
        if owners.iter().any(|owner| !owner.this_copy) {
            return Err(AppError::Validation(
                "a different working copy holds the selected infobase".into(),
            ));
        }
        let project = config.working_copy.as_deref().unwrap_or(&config.base_path);
        Ok(Self {
            identity: identity.to_owned(),
            project: crate::support::path::nearest_existing_canonical_path(project)
                .map_err(storage_error)?,
            base_file: filesystem_object_identity(&base.join("1Cv8.1CD")).map_err(storage_error)?,
            owners,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct PreparedBaseline {
    receipt: MergeBaselineReceipt,
    binding: Binding,
    snapshot: FullSnapshot,
}

#[derive(Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeBaselineResult {
    pub source_set: String,
    pub generation: String,
    pub provider: Provider,
    pub baseline: BaselineState,
}

#[derive(Clone, Copy, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BaselineState {
    Ready,
    Accepted,
}

impl std::fmt::Display for BaselineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ready => "ready",
            Self::Accepted => "accepted",
        })
    }
}

fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 256
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b":._-".contains(&byte))
}

/// Retain only the runner's memory of the original export. The mutable caller stage,
/// and later the merged local tree, are never used to calculate hashes at acceptance.
pub(super) fn prepare(
    config: &AppConfig,
    set: &SourceSetContext,
    stage: &Path,
    provider: Provider,
    generation: &str,
    version: Option<File>,
) -> Result<MergeBaselineReceipt, AppError> {
    if !valid_token(generation)
        || !TokenScope::measured(provider)
        || (exchange_guard::token_scope(config, set) == TokenScope::OwnTool
            && provider != Provider::Ibcmd)
    {
        // Only ibcmd is measured to report the loaded extension configuration.
        return Err(AppError::Validation(
            "the snapshot provider cannot confirm the working configuration generation".into(),
        ));
    }
    let binding = Binding::of(config, set)?;
    let snapshot = analyzer::prepare_full_snapshot(set, stage)
        .map_err(|error| AppError::Runtime(error.to_string()))?;
    let id = Uuid::new_v4();
    let receipt = MergeBaselineReceipt {
        id: id.to_string(),
        generation: generation.to_owned(),
        provider,
        base_generation: exchange_guard::recorded_generation(set)
            .filter(|record| {
                !record.merge_pending()
                    && record.after != GenerationAfter::FailedBuild
                    && valid_token(&record.token)
                    && exchange_guard::token_scope(config, set).compares(record.tool, provider)
            })
            .map(|record| record.token),
    };
    let prepared = PreparedBaseline {
        receipt,
        binding,
        snapshot,
    };
    let root = crate::support::temp::temp_root(&config.work_path).map_err(storage_error)?;
    prune_prepared(&root).map_err(storage_error)?;
    let path = receipt_path(&root, id);
    crate::support::temp::create_private_dir(&path).map_err(storage_error)?;
    let result = write_prepared(&path, &prepared, version);
    if result.is_err() {
        let _ = remove_prepared(&path);
    }
    result?;
    Ok(prepared.receipt)
}

fn write_prepared(
    path: &Path,
    prepared: &PreparedBaseline,
    version: Option<File>,
) -> Result<(), AppError> {
    if let Some(mut version) = version {
        use std::io::{Seek, SeekFrom};
        version.seek(SeekFrom::Start(0)).map_err(storage_error)?;
        crate::support::fs::write_file_atomically(&path.join(VERSION_FILE_NAME), |file| {
            std::io::copy(&mut version, file).map(|_| ())
        })
        .map_err(storage_error)?;
    }
    crate::support::fs::write_file_atomically(&path.join(RECEIPT_FILE), |file| {
        serde_json::to_writer(file, prepared).map_err(std::io::Error::other)
    })
    .map_err(storage_error)
}

fn receipt_path(root: &Path, id: Uuid) -> PathBuf {
    root.join(format!("{STORAGE_PREFIX}{id}"))
}

fn prune_prepared(root: &Path) -> std::io::Result<()> {
    let mut fresh = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let matches = entry.file_name().to_str().is_some_and(|name| {
            name.strip_prefix(STORAGE_PREFIX)
                .is_some_and(|id| Uuid::parse_str(id).is_ok())
        });
        if !matches || !entry.file_type()?.is_dir() {
            continue;
        }
        let modified = entry.metadata()?.modified()?;
        if modified.elapsed().is_ok_and(|age| age > RETENTION) {
            remove_prepared(&entry.path())?;
        } else {
            fresh.push((modified, entry.path()));
        }
    }
    let to_remove = (fresh.len() + 1).saturating_sub(MAX_PREPARED);
    fresh.sort_unstable();
    for (_, path) in fresh.into_iter().take(to_remove) {
        remove_prepared(&path)?;
    }
    Ok(())
}

fn remove_prepared(path: &Path) -> std::io::Result<()> {
    for name in [RECEIPT_FILE, VERSION_FILE_NAME] {
        match fs::remove_file(path.join(name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            result => result?,
        }
    }
    fs::remove_dir(path)
}

pub(crate) fn accept(
    context: &ExecutionContext,
    config: &AppConfig,
    set: &SourceSetContext,
    id: Uuid,
    check_only: bool,
) -> Result<MergeBaselineResult, AppError> {
    let root = crate::support::temp::temp_root_path(&config.work_path);
    let path = receipt_path(&root, id);
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::Validation(
                "working merge baseline is no longer available; fetch again".into(),
            )
        } else {
            storage_error(error)
        }
    })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata
            .modified()
            .map_err(storage_error)?
            .elapsed()
            .is_ok_and(|age| age > RETENTION)
    {
        return Err(AppError::Validation(
            "working merge baseline is no longer available; fetch again".into(),
        ));
    }
    let file = File::open(path.join(RECEIPT_FILE)).map_err(storage_error)?;
    let prepared: PreparedBaseline = serde_json::from_reader(std::io::BufReader::new(file))
        .map_err(|error| {
            AppError::Runtime(format!("cannot read working merge baseline: {error}"))
        })?;
    let receipt = prepared.receipt;
    if receipt.id != id.to_string() || prepared.binding != Binding::of(config, set)? {
        return Err(AppError::Validation(
            "working merge baseline belongs to a different infobase, owner or source set".into(),
        ));
    }
    let mut utilities = PlatformUtilities::from_config(config);
    let answer = read_with_record_tool(
        context,
        config,
        &mut utilities,
        receipt.provider,
        "working-merge-generation",
        exchange_guard::extension_of(config, set),
    )?;
    let current = match answer {
        RecordToolAnswer::Token(token) if token == receipt.generation => token,
        RecordToolAnswer::Token(token) => {
            return Err(AppError::Refused(Box::new(
                UseCaseError::new(
                    UseCaseErrorKind::NonFastForward,
                    "the working configuration changed after the snapshot; fetch again",
                )
                .with_generations(Generations {
                    base: token,
                    local: receipt.generation,
                }),
            )));
        }
        RecordToolAnswer::NoAnswer(why) => {
            return Err(AppError::Refused(Box::new(UseCaseError::new(
                UseCaseErrorKind::NonFastForward,
                format!("cannot confirm the snapshot generation: {why}; fetch again"),
            ))));
        }
    };
    if let Some(error) =
        super::interruption::pending_interruption_error(context, "working merge acceptance")
    {
        return Err(error);
    }
    let already_accepted = exchange_guard::recorded_generation(set).is_some_and(|record| {
        record.tool == receipt.provider
            && record.token == current
            && record
                .merge_baseline
                .as_ref()
                .is_some_and(|merge| merge.id == receipt.id && merge.pending_generation.is_none())
    });
    if !check_only && !already_accepted {
        let ledger = GenerationLedger::of(set).ok_or_else(|| {
            AppError::Validation("working merge requires an infobase generation ledger".into())
        })?;
        let version = match File::open(path.join(VERSION_FILE_NAME)) {
            Ok(version) => Some(version),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(storage_error(error)),
        };
        let version_file = RunnerVersionFile::of(set).ok_or_else(|| {
            AppError::Validation("working merge requires an infobase version file binding".into())
        })?;
        let mark = match exchange_guard::recorded_generation(set) {
            Some(record)
                if record.merge_baseline.as_ref().is_some_and(|merge| {
                    merge.id == receipt.id
                        && merge.pending_generation.as_deref() == Some(current.as_str())
                }) =>
            {
                record.mark()
            }
            _ => exchange_guard::dump_apply_mark(config, set, &ledger, receipt.provider, &current),
        };
        // Commit guard first: an interrupted multi-file update cannot authorize a push.
        ledger.record_merge(&receipt, mark, true)?;
        analyzer::commit_full_snapshot(set, &config.work_path, &prepared.snapshot)
            .map_err(|error| AppError::Runtime(error.to_string()))?;
        version_file.record_prepared(version)?;
        ledger.record_merge(&receipt, mark, false)?;
    }
    Ok(MergeBaselineResult {
        source_set: set.name().to_owned(),
        generation: current,
        provider: receipt.provider,
        baseline: if check_only {
            BaselineState::Ready
        } else {
            BaselineState::Accepted
        },
    })
}

fn storage_error(error: std::io::Error) -> AppError {
    AppError::Runtime(format!("working merge baseline storage: {error}"))
}
