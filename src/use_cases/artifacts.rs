//! `make`: пакет из исходников, собранный во временной базе раннера.
//!
//! База проекта в сборке не участвует
//! (`INV.USE-CASES.MAKE-BUILDS-PACKAGES-FROM-SOURCES-IN-A-THROWAWAY-BASE`): исходники набора
//! — для формата EDT сперва переведённые `1cedtcli` в XML — собирает исполнитель цепочки
//! `make` (`ibcmd`, иначе Конфигуратор) в [`ThrowawayInfobase`]. Внешние обработки и отчёты
//! собирает Конфигуратор в своей базе, куда сперва загружена основная конфигурация. База
//! служит одному прогону — у каждого исполнителя своя ([`MakeSession`]): `make <SET>`
//! создаёт свою, обход без набора ([`execute_all`]) — общую на все наборы, и прогон её
//! убирает.

use std::path::PathBuf;
use std::time::Instant;

mod all;

pub use self::all::execute_all;
use tracing::debug;

use crate::config::model::{AppConfig, SourceFormat, SourceSetConfig, SourceSetPurpose};
use crate::domain::artifact::{
    ArtifactKind, ArtifactRef, ArtifactSet, ARTIFACT_ROLE_PACKAGE_FILE, ARTIFACT_ROLE_PLATFORM_LOG,
    ARTIFACT_ROLE_STAGE_FILE,
};
use crate::domain::artifacts::{ArtifactBuildMetadata, ArtifactBuildMode, ArtifactsResult};
use crate::domain::capability::{Operation, Provider, ProviderPlan};
use crate::domain::execution::{
    ExecutionError, ExecutionInterruptionPhase, ExecutionOutcome, ExecutionStatus,
};
use crate::domain::runner::RunnerKind;
use crate::platform::designer::DesignerDsl;
use crate::platform::locator::UtilityType;
use crate::platform::process::ProcessRunner;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::fs::{acquire_advisory_lock, write_temp_dir_metadata, TempDirKind};
use crate::support::path::{
    hashed_lock_path, is_filesystem_root, nearest_existing_canonical_path, stable_path_identity,
};
use crate::support::temp::platform_logs_dir;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::dump_config::run_external_dump_designer;
use crate::use_cases::extension_identity::platform_extension_name;
use crate::use_cases::external_artifacts::{
    discover_designer_external_artifacts, prepare_edt_external_artifacts, sanitize_file_stem,
    source_set_external_kind, ExternalArtifactDescriptor,
};
use crate::use_cases::interruption::{
    deferred_command_interruption_details, deferred_interruption_warning_for_command,
    interruption_before_safe_point, record_cancellation,
};
use crate::use_cases::progress::log_live_stage;
use crate::use_cases::provider_selection::SelectedProvider;
use crate::use_cases::request::{ArtifactsModeRequest, ArtifactsRequest};
use crate::use_cases::result::{stamp_dispatch, UseCaseFailure, UseCaseResult};
use crate::use_cases::source_inventory::SourceSetInventory;
use crate::use_cases::throwaway_infobase::{Builder, Package, ThrowawayInfobase};

use super::staged_publication::{
    cleanup_owned_orphan_files, interruption_before_publish, StagedPublication,
    StagedPublicationOutcome,
};

const UNSUPPORTED_PROFILE_ERROR: &str =
    "make supports only the cf, cfe, epf and erf runner profiles of the designer backend";
const ARTIFACTS_BACKUP_PREFIX: &str = ".artifacts-backup";

/// Прогон `make`: исполнители процессов и временные базы, общие для всех его наборов.
///
/// База у прогона одна на исполнителя: её создаёт тот, кто в ней собирает. Пакеты собирает
/// исполнитель цепочки `make`, внешние обработки — всегда Конфигуратор; если пакеты собирал
/// `ibcmd`, у внешних своя база Конфигуратора — базу `ibcmd` Конфигуратор не открывает.
pub struct MakeSession {
    utilities: PlatformUtilities,
    bases: SessionBases,
}

/// Временные базы прогона и исходники EDT, уже переведённые в XML: перевод набора делается
/// один раз за прогон и живёт в каталоге той базы, где сделан, до её уборки.
#[derive(Default)]
struct SessionBases {
    bases: Vec<ThrowawayInfobase>,
    xml: std::collections::BTreeMap<String, PathBuf>,
}

impl MakeSession {
    pub(super) fn new(config: &AppConfig) -> Self {
        Self {
            utilities: PlatformUtilities::from_config(config),
            bases: SessionBases::default(),
        }
    }

    /// Убирает временные базы прогона. Неудачи — предупреждения для ответа.
    pub(super) fn close(self) -> Vec<String> {
        self.bases
            .bases
            .into_iter()
            .flat_map(ThrowawayInfobase::close)
            .collect()
    }
}

/// Предупреждения уборки временных баз ложатся в диагностику ответа набора, на котором
/// прогон кончился, — удачного или нет (`INV.MAKE-NAMES-A-FAILED-CLEANUP`).
pub(super) fn note_cleanup_warning(result: &mut ArtifactsResult, warnings: Vec<String>) {
    result.execution.diagnostics.extend(warnings);
}

pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &ArtifactsRequest,
) -> UseCaseResult<ArtifactsResult> {
    let mut session = MakeSession::new(config);
    let mut outcome = run_artifacts(context, config, args, &mut session);
    let warning = session.close();
    if let Some(payload) = crate::use_cases::result::payload_mut(&mut outcome) {
        note_cleanup_warning(payload, warning);
    }
    stamp_dispatch(outcome, context.work())
}

/// Сборка одного набора в прогоне `session`: временную базу прогона она создаёт при первой
/// нужде и оставляет следующему набору.
pub fn execute_in(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &ArtifactsRequest,
    session: &mut MakeSession,
) -> UseCaseResult<ArtifactsResult> {
    stamp_dispatch(
        run_artifacts(context, config, args, session),
        context.work(),
    )
}

type ArtifactsExecutionFailure = UseCaseFailure<ArtifactsResult>;

#[derive(Debug, Clone)]
struct ResolvedArtifactsTarget {
    mode: ArtifactBuildMode,
    source_set_name: String,
    extension: Option<String>,
    output_path: PathBuf,
    source_path: PathBuf,
    is_directory_output: bool,
    canonical_output_path: PathBuf,
    canonical_base_path: PathBuf,
    canonical_work_path: PathBuf,
    target_identity: String,
    lock_path: PathBuf,
}

impl ResolvedArtifactsTarget {
    fn is_external(&self) -> bool {
        matches!(
            self.mode,
            ArtifactBuildMode::ExternalDataProcessorEpf | ArtifactBuildMode::ExternalReportErf
        )
    }
}

fn run_artifacts(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &ArtifactsRequest,
    session: &mut MakeSession,
) -> UseCaseResult<ArtifactsResult> {
    debug!(
        command = context.command().as_str(),
        transport = ?context.transport(),
        mode = ?args.mode,
        source_set = args.source_set.as_deref().unwrap_or("<auto>"),
        extension = args.extension.as_deref().unwrap_or("<none>"),
        "executing artifacts use case"
    );
    let started = Instant::now();
    let mode = map_mode(args.mode);

    if let Some(error) = validate_supported_matrix(args) {
        return Err(ArtifactsExecutionFailure::with_payload(
            error,
            empty_result(
                mode,
                started,
                None,
                args.extension.clone(),
                PathBuf::from(&args.output_path),
                Some(UNSUPPORTED_PROFILE_ERROR.to_owned()),
            ),
        ));
    }

    let resolved = match resolve_target(config, args) {
        Ok(resolved) => resolved,
        Err(error) => {
            let message = error.to_string();
            return Err(ArtifactsExecutionFailure::with_payload(
                error,
                empty_result(
                    mode,
                    started,
                    args.source_set.clone(),
                    args.extension.clone(),
                    PathBuf::from(&args.output_path),
                    Some(message),
                ),
            ));
        }
    };

    if let Err(error) = validate_publish_target(&resolved) {
        let message = error.to_string();
        return Err(ArtifactsExecutionFailure::with_payload(
            error,
            empty_result(
                resolved.mode,
                started,
                Some(resolved.source_set_name.clone()),
                resolved.extension.clone(),
                resolved.output_path.clone(),
                Some(message),
            ),
        ));
    }

    // Внешние обработки и отчёты собирает только Конфигуратор: у `ibcmd` такой команды нет,
    // поэтому ключ `providers.make` их не касается.
    let plan = if resolved.is_external() {
        ProviderPlan::Default {
            chain: vec![Provider::Designer],
        }
    } else {
        config.provider_plan(Operation::Make)
    };
    let selected = match crate::use_cases::provider_selection::select_from(
        config,
        &mut session.utilities,
        Operation::Make,
        plan,
    ) {
        Ok(selected) => selected,
        Err((error, receipt)) => {
            let message = error.to_string();
            let mut result = empty_result(
                resolved.mode,
                started,
                Some(resolved.source_set_name.clone()),
                resolved.extension.clone(),
                resolved.output_path.clone(),
                Some(message),
            );
            result.provider = Some(receipt);
            return Err(ArtifactsExecutionFailure::with_payload(error, result));
        }
    };
    let receipt = selected.receipt.clone();
    let outcome =
        run_artifacts_selected(context, config, args, started, resolved, session, selected);
    crate::use_cases::provider_selection::attach(outcome, &receipt)
}

fn run_artifacts_selected(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &ArtifactsRequest,
    started: Instant,
    resolved: ResolvedArtifactsTarget,
    session: &mut MakeSession,
    selected: SelectedProvider,
) -> UseCaseResult<ArtifactsResult> {
    let Some(binary) = selected.location.map(|location| location.path) else {
        return Err(UseCaseFailure::without_payload(
            crate::use_cases::unimplemented_provider(Operation::Make, selected.provider),
        ));
    };
    let builder = Builder {
        provider: selected.provider,
        binary,
    };

    if args.dry_run {
        // Исходники EDT сборка сперва переводит `1cedtcli`: без него превью отказывает так
        // же, как отказал бы прогон.
        if config.format == SourceFormat::Edt {
            if let Err(error) = session.utilities.locate(UtilityType::EdtCli) {
                let error = AppError::from(error);
                let message = error.to_string();
                return Err(ArtifactsExecutionFailure::with_payload(
                    error,
                    empty_result(
                        resolved.mode,
                        started,
                        Some(resolved.source_set_name.clone()),
                        resolved.extension.clone(),
                        resolved.output_path.clone(),
                        Some(message),
                    ),
                ));
            }
        }
        crate::use_cases::progress::log_live_stage(
            "make: preview",
            "[Artifacts] preview only, nothing built or published",
        );
        // The artifacts lock below is this command's first filesystem write, and the
        // executor is already located, so an absent platform refuses in the preview.
        let metadata = ArtifactBuildMetadata {
            artifact_type: resolved.mode,
            output_path: resolved.output_path.clone(),
            file_names: resolved
                .output_path
                .file_name()
                .map(|value| vec![value.to_string_lossy().into_owned()])
                .unwrap_or_default(),
            published: false,
        };
        return Ok(ArtifactsResult {
            provider: None,
            provider_dispatched: false,
            mode: resolved.mode,
            source_set: Some(resolved.source_set_name.clone()),
            extension: resolved.extension.clone(),
            duration_ms: started.elapsed().as_millis() as u64,
            execution: ExecutionOutcome::new(ExecutionStatus::Succeeded)
                .with_payload(metadata)
                .with_diagnostics(vec![format!(
                    "would build {:?} into '{}' via {} in a throwaway infobase; nothing published",
                    resolved.mode,
                    resolved.output_path.display(),
                    builder.binary.display()
                )]),
        });
    }

    let lock_guard = match acquire_advisory_lock(&resolved.lock_path) {
        Ok(lock_guard) => lock_guard,
        Err(error) => {
            let message = format!(
                "failed to acquire artifacts lock '{}': {error}",
                resolved.lock_path.display()
            );
            return Err(ArtifactsExecutionFailure::with_payload(
                AppError::Runtime(message.clone()),
                empty_result(
                    resolved.mode,
                    started,
                    Some(resolved.source_set_name.clone()),
                    resolved.extension.clone(),
                    resolved.output_path.clone(),
                    Some(message),
                ),
            ));
        }
    };

    if let Err(error) = cleanup_orphan_files(&resolved) {
        let message = error.to_string();
        return Err(ArtifactsExecutionFailure::with_payload(
            error,
            empty_result(
                resolved.mode,
                started,
                Some(resolved.source_set_name.clone()),
                resolved.extension.clone(),
                resolved.output_path.clone(),
                Some(message),
            ),
        ));
    }

    let execution_result = if resolved.is_external() {
        run_external_build(context, config, &resolved, builder, session)
    } else {
        run_package_build(context, config, &resolved, builder, session)
    };
    drop(lock_guard);

    match execution_result {
        Ok((platform_result, mut artifacts, message)) => {
            let platform_log_path = platform_result.platform_log_path.clone();
            if let Some(path) = platform_log_path.as_ref() {
                artifacts.push(
                    ArtifactRef::new(ArtifactKind::PlatformLog, path)
                        .with_role(ARTIFACT_ROLE_PLATFORM_LOG),
                );
            }
            let metadata = ArtifactBuildMetadata {
                artifact_type: resolved.mode,
                output_path: resolved.output_path.clone(),
                file_names: published_file_names(&artifacts),
                published: true,
            };
            let execution = published_execution(context, artifacts, metadata, message);
            Ok(ArtifactsResult {
                provider: None,
                provider_dispatched: false,
                mode: resolved.mode,
                source_set: Some(resolved.source_set_name),
                extension: resolved.extension,
                duration_ms: started.elapsed().as_millis() as u64,
                execution,
            })
        }
        Err((error, artifacts, platform_log_path)) => Err(export_refusal(
            &resolved,
            started,
            error,
            artifacts,
            platform_log_path,
        )),
    }
}

/// Отказ сборки артефакта формой команды. Отмену и её место называет ошибка: безопасная
/// точка — `command_boundary`, снятый экспорт — `provider_command`; статус, ошибку
/// `cancelled` и запись ставит `record_cancellation`. Отказ, пришедший, когда
/// отмена уже ожидала, остаётся отказом со своим кодом: сигнал сюда не доходит вовсе.
fn export_refusal(
    resolved: &ResolvedArtifactsTarget,
    started: Instant,
    error: AppError,
    mut artifacts: ArtifactSet,
    platform_log_path: Option<PathBuf>,
) -> ArtifactsExecutionFailure {
    let message = error.to_string();
    if artifacts.get_by_role(ARTIFACT_ROLE_PLATFORM_LOG).is_none() {
        if let Some(path) = platform_log_path.as_ref() {
            artifacts.push(
                ArtifactRef::new(ArtifactKind::PlatformLog, path)
                    .with_role(ARTIFACT_ROLE_PLATFORM_LOG),
            );
        }
    }
    let metadata = ArtifactBuildMetadata {
        artifact_type: resolved.mode,
        output_path: resolved.output_path.clone(),
        file_names: published_file_names(&artifacts),
        published: false,
    };
    let artifact_for_error = artifacts
        .get_by_role(ARTIFACT_ROLE_PLATFORM_LOG)
        .or_else(|| artifacts.get_by_role(ARTIFACT_ROLE_STAGE_FILE))
        .map(|path| ArtifactRef::new(ArtifactKind::Other("diagnostic".to_owned()), path));
    let mut execution = ExecutionOutcome::new(ExecutionStatus::Failed)
        .with_artifacts(artifacts.clone())
        .with_payload(metadata);
    match error.cancellation() {
        Some(at) => {
            execution.diagnostics.push(message.clone());
            record_cancellation(
                &mut execution,
                at,
                ExecutionInterruptionPhase::ProviderCommand,
                message,
            );
        }
        None => execution.errors.push(ExecutionError {
            code: "designer_export_failed".to_owned(),
            message,
            details: Vec::new(),
            artifact: artifact_for_error,
            retryable: false,
        }),
    }
    let payload = ArtifactsResult {
        provider: None,
        provider_dispatched: false,
        mode: resolved.mode,
        source_set: Some(resolved.source_set_name.clone()),
        extension: resolved.extension.clone(),
        duration_ms: started.elapsed().as_millis() as u64,
        execution,
    };
    ArtifactsExecutionFailure::with_payload(error, payload)
}

/// Исход публикации артефактов: и успех, и отказ несут уже разложенный набор, чтобы
/// вызывающий мог убрать за собой и назвать, что успело лечь на диск.
type PublicationAttempt = Result<
    (PlatformCommandResult, ArtifactSet, PublicationOutcome),
    (AppError, ArtifactSet, Option<PathBuf>),
>;

/// Временная база прогона того исполнителя, что собирает: созданная им раньше в этом прогоне
/// или новая. База другого исполнителя не берётся никогда.
fn session_base<'s>(
    context: &ExecutionContext,
    config: &AppConfig,
    session: &'s mut SessionBases,
    builder: Builder,
    runner: &dyn ProcessRunner,
) -> Result<(&'s mut ThrowawayInfobase, &'s mut XmlCache), AppError> {
    let SessionBases { bases, xml } = session;
    let index = match bases
        .iter()
        .position(|base| base.provider() == builder.provider)
    {
        Some(index) => index,
        None => {
            bases.push(ThrowawayInfobase::create(
                context,
                &config.work_path,
                builder,
                runner,
            )?);
            bases.len() - 1
        }
    };
    Ok((&mut bases[index], xml))
}

/// Исходники EDT, переведённые в XML за прогон, по имени набора.
type XmlCache = std::collections::BTreeMap<String, PathBuf>;

/// Основная конфигурация во временной базе Конфигуратора: расширения и внешние обработки
/// он собирает поверх неё. База её получает один раз за прогон; `ibcmd` её не загружает.
fn ensure_configuration_in(
    context: &ExecutionContext,
    config: &AppConfig,
    xml: &mut XmlCache,
    base: &mut ThrowawayInfobase,
    runner: &dyn ProcessRunner,
) -> Result<(), (AppError, Option<PathBuf>)> {
    let inventory = SourceSetInventory::new(config);
    let configuration = configuration_source_set(&inventory).map_err(|error| (error, None))?;
    if !base.needs_configuration(&configuration.name) {
        return Ok(());
    }
    let parent_dir =
        sources_in_xml(context, config, xml, base, configuration).map_err(|error| (error, None))?;
    // Свой журнал `/Out`: загрузка основной конфигурации не затирается журналом набора.
    let log_file = platform_logs_dir(&config.work_path)
        .map(|dir| {
            dir.join(format!(
                "artifacts-{}-configuration.log",
                configuration.name
            ))
        })
        .map_err(|error| {
            (
                AppError::Runtime(format!("failed to create platform logs dir: {error}")),
                None,
            )
        })?;
    let loaded = base
        .load_configuration(
            context,
            runner,
            &configuration.name,
            &parent_dir,
            Some(log_file),
        )
        .map_err(|error| (error, None))?;
    ensure_platform_success(&configuration.name, &loaded)
        .map_err(|error| (error, loaded.platform_log_path.clone()))
}

/// Исполнитель процессов для утилиты исполнителя; исполнителю без утилиты `make` нечего
/// делать.
fn runner_of<'u>(
    config: &AppConfig,
    utilities: &'u PlatformUtilities,
    provider: Provider,
) -> Result<&'u dyn ProcessRunner, AppError> {
    crate::use_cases::provider_selection::utilities_of(provider, config)
        .and_then(|needed| needed.into_iter().next())
        .map(|utility| utilities.runner_for(utility))
        .ok_or_else(|| crate::use_cases::unimplemented_provider(Operation::Make, provider))
}

/// Каталог XML набора: у формата Конфигуратора — сами исходники, у формата EDT — их перевод
/// `1cedtcli` шагом сборки (`build_project::execute_edt_export_step`) в каталог временной
/// базы.
fn sources_in_xml(
    context: &ExecutionContext,
    config: &AppConfig,
    xml: &mut XmlCache,
    base: &ThrowawayInfobase,
    source_set: &SourceSetConfig,
) -> Result<PathBuf, AppError> {
    let inventory = SourceSetInventory::new(config);
    match config.format {
        SourceFormat::Designer => Ok(inventory.source_path(source_set)),
        // Перевод одного набора за прогон делается один раз: в обходе `ibcmd` с внешними
        // наборами основная конфигурация нужна и базе `ibcmd`, и базе Конфигуратора.
        SourceFormat::Edt if xml.contains_key(&source_set.name) => {
            Ok(xml[&source_set.name].clone())
        }
        SourceFormat::Edt => {
            let edt_context = inventory.edt_context(&source_set.name).ok_or_else(|| {
                AppError::Runtime(format!(
                    "missing EDT context for source-set '{}'",
                    source_set.name
                ))
            })?;
            let mut utilities = PlatformUtilities::from_config(config);
            let location = utilities
                .locate(UtilityType::EdtCli)
                .map_err(AppError::from)?;
            let edt = crate::platform::edt::EdtDsl::new(
                location.path,
                config.work_path.join("edt-workspace"),
                utilities.runner_for(UtilityType::EdtCli),
                context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
            );
            let target = base.xml_dir(&source_set.name);
            log_live_stage("make: edt export", "[EDT] converting the sources to XML");
            crate::use_cases::build_project::execute_edt_export_step(
                context,
                config,
                &edt,
                source_set,
                edt_context,
                &target,
                "make",
            )?;
            xml.insert(source_set.name.clone(), target.clone());
            Ok(target)
        }
    }
}

/// Пакет `.cf` или `.cfe` из исходников набора во временной базе прогона.
fn run_package_build(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedArtifactsTarget,
    builder: Builder,
    session: &mut MakeSession,
) -> PublicationAttempt {
    if let Some(error) = interruption_before_safe_point(
        context,
        format!(
            "artifact export for source-set '{}' and output '{}'",
            resolved.source_set_name,
            resolved.output_path.display()
        ),
    ) {
        return Err((error, ArtifactSet::default(), None));
    }
    let runner = runner_of(config, &session.utilities, builder.provider)
        .map_err(|error| (error, ArtifactSet::default(), None))?;
    build_package_in(
        context,
        config,
        resolved,
        builder,
        &mut session.bases,
        runner,
    )
}

/// Сборка пакета в базе прогона `base` процессами `runner`.
fn build_package_in(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedArtifactsTarget,
    builder: Builder,
    bases: &mut SessionBases,
    runner: &dyn ProcessRunner,
) -> PublicationAttempt {
    let fail = |error: AppError| (error, ArtifactSet::default(), None);
    let (base, xml) = session_base(context, config, bases, builder, runner).map_err(fail)?;
    let inventory = SourceSetInventory::new(config);
    let source_set = inventory.named(&resolved.source_set_name).map_err(fail)?;
    // Пакет конфигурации — сам набор: база запомнит загруженным именно его. Основная
    // конфигурация под расширением — первый набор конфигурации проекта.
    let configuration = match resolved.extension.as_deref() {
        None => source_set,
        Some(_) => configuration_source_set(&inventory).map_err(fail)?,
    };
    let log_file = designer_log_file(
        config,
        base.provider(),
        &resolved.source_set_name,
        resolved.mode,
    )
    .map_err(fail)?;
    let package = match resolved.extension.as_deref() {
        Some(name) => Package::Extension(name),
        None => Package::Configuration,
    };

    // Конфигуратор загружает расширение поверх основной конфигурации.
    if matches!(package, Package::Extension(_)) {
        ensure_configuration_in(context, config, xml, base, runner)
            .map_err(|(error, log)| (error, ArtifactSet::default(), log))?;
    }
    let source_dir = sources_in_xml(context, config, xml, base, source_set).map_err(fail)?;

    let publication = StagedPublication::prepare_file(
        &resolved.output_path,
        &resolved.target_identity,
        ".artifacts-stage",
        resolved.mode.file_extension(),
    )
    .map_err(fail)?;
    let staging_file = publication.staging_path().to_path_buf();
    let cleanup_unmaterialized_stage = |error: AppError| {
        if staging_file.is_file() {
            error
        } else {
            publication.cleanup_failure(error)
        }
    };

    let build_result = base
        .build_package(
            context,
            runner,
            &configuration.name,
            &source_dir,
            package,
            &staging_file,
            log_file,
        )
        .map_err(|error| {
            (
                cleanup_unmaterialized_stage(error),
                ArtifactSet::default(),
                None,
            )
        })?;

    let mut artifacts = ArtifactSet::default();
    if staging_file.exists() {
        artifacts.push(
            ArtifactRef::new(
                ArtifactKind::Other("staged_artifact".to_owned()),
                &staging_file,
            )
            .with_role(ARTIFACT_ROLE_STAGE_FILE),
        );
    }
    if let Some(path) = build_result.platform_log_path.as_ref() {
        artifacts.push(
            ArtifactRef::new(ArtifactKind::PlatformLog, path).with_role(ARTIFACT_ROLE_PLATFORM_LOG),
        );
    }

    if let Err(error) = ensure_platform_success(&resolved.source_set_name, &build_result) {
        return Err((
            cleanup_unmaterialized_stage(error),
            artifacts,
            build_result.platform_log_path.clone(),
        ));
    }
    if !staging_file.is_file() {
        return Err((
            cleanup_unmaterialized_stage(AppError::Platform(format!(
                "{} did not produce artifact file '{}'",
                base.provider(),
                staging_file.display()
            ))),
            artifacts,
            build_result.platform_log_path.clone(),
        ));
    }

    if let Some(error) = refusal_before_publication(
        context,
        resolved,
        format!(
            "artifact publication for source-set '{}' and output '{}'",
            resolved.source_set_name,
            resolved.output_path.display()
        ),
    ) {
        return Err((error, artifacts, build_result.platform_log_path.clone()));
    }

    let publish_phase = publication
        .publish_file(context, "failed to publish staged artifact")
        .map_err(|error| {
            (
                error,
                artifacts.clone(),
                build_result.platform_log_path.clone(),
            )
        })?;

    let mut published_artifacts = ArtifactSet::default();
    published_artifacts.push(
        ArtifactRef::new(ArtifactKind::Package, &resolved.output_path)
            .with_role(ARTIFACT_ROLE_PACKAGE_FILE),
    );

    Ok((
        build_result,
        published_artifacts,
        publication_message(context, publish_phase),
    ))
}

/// Внешние обработки и отчёты: Конфигуратор собирает их в своей временной базе прогона.
fn run_external_build(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedArtifactsTarget,
    builder: Builder,
    session: &mut MakeSession,
) -> PublicationAttempt {
    if let Some(error) = interruption_before_safe_point(
        context,
        format!(
            "external artifact export for source-set '{}' and output '{}'",
            resolved.source_set_name,
            resolved.output_path.display()
        ),
    ) {
        return Err((error, ArtifactSet::default(), None));
    }

    let runner = match runner_of(config, &session.utilities, builder.provider) {
        Ok(runner) => runner,
        Err(error) => return Err((error, ArtifactSet::default(), None)),
    };
    build_external_in(
        context,
        config,
        resolved,
        builder,
        &mut session.bases,
        runner,
    )
}

/// Внешние обработки в базе Конфигуратора прогона: сперва основная конфигурация проекта —
/// как в базе проекта, на которой их собирали прежде, — затем каждая обработка.
fn build_external_in(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedArtifactsTarget,
    builder: Builder,
    bases: &mut SessionBases,
    runner: &dyn ProcessRunner,
) -> PublicationAttempt {
    let binary = builder.binary.clone();
    let (base, xml) = session_base(context, config, bases, builder, runner)
        .map_err(|error| (error, ArtifactSet::default(), None))?;
    ensure_configuration_in(context, config, xml, base, runner)
        .map_err(|(error, log)| (error, ArtifactSet::default(), log))?;

    let publication = StagedPublication::prepare_dir(
        &resolved.output_path,
        &resolved.target_identity,
        ".artifacts-stage",
    )
    .map_err(|error| (error, ArtifactSet::default(), None))?;
    let staging_dir = publication.staging_path().to_path_buf();

    let log_file = designer_log_file(
        config,
        Provider::Designer,
        &resolved.source_set_name,
        resolved.mode,
    )
    .map_err(|error| (error, ArtifactSet::default(), None))?;
    let dsl = DesignerDsl::new(
        binary,
        base.connection(),
        runner,
        log_file,
        context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
    );
    let descriptors = external_descriptors(context, config, resolved)
        .map_err(|error| (error, ArtifactSet::default(), None))?;
    let mut artifacts = ArtifactSet::default();
    let mut last_result = PlatformCommandResult {
        process: crate::platform::process::ProcessResult {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            interruption: None,
        },
        platform_log_path: None,
        platform_log: None,
        platform_log_read_error: None,
    };

    for descriptor in &descriptors {
        let publish_name = format!(
            "{}.{}",
            sanitize_file_stem(&descriptor.logical_name),
            resolved.mode.file_extension()
        );
        let staging_file = staging_dir.join(&publish_name);
        let published_file = resolved.output_path.join(&publish_name);
        write_temp_dir_metadata(
            &staging_file,
            TempDirKind::Stage,
            publication.run_id(),
            &published_file,
            &resolved.target_identity,
        )
        .map_err(|error| {
            (
                AppError::Runtime(format!("failed to write staging metadata: {error}")),
                artifacts.clone(),
                None,
            )
        })?;

        log_live_stage(
            "make: external export",
            "[Конфигуратор] exporting external artifact package",
        );
        let result = dsl
            .load_external_data_processor_or_report_from_files(
                &descriptor.descriptor_xml_path,
                &staging_file,
            )
            .map_err(|error| (AppError::from(error), artifacts.clone(), None))?;
        last_result = result.clone();
        if staging_file.exists() {
            artifacts.push(
                ArtifactRef::new(ArtifactKind::Package, &staging_file)
                    .with_role(ARTIFACT_ROLE_STAGE_FILE),
            );
        }
        if let Some(path) = result.platform_log_path.as_ref() {
            artifacts.push(
                ArtifactRef::new(ArtifactKind::PlatformLog, path)
                    .with_role(ARTIFACT_ROLE_PLATFORM_LOG),
            );
        }
        if let Err(error) = ensure_platform_success(&resolved.source_set_name, &result) {
            return Err((error, artifacts, result.platform_log_path.clone()));
        }
        if !staging_file.is_file() {
            return Err((
                AppError::Platform(format!(
                    "designer did not produce external artifact file '{}'",
                    staging_file.display()
                )),
                artifacts,
                result.platform_log_path.clone(),
            ));
        }

        log_live_stage(
            "make: external dump",
            "[Конфигуратор] dumping external artifact descriptor",
        );
        run_external_dump_designer(
            &dsl,
            &staging_file,
            &config
                .work_path
                .join("external-dump")
                .join(&resolved.source_set_name)
                .join(&descriptor.stable_id)
                .join(format!("{}.xml", descriptor.logical_name)),
            descriptor.artifact_type,
            &descriptor.logical_name,
        )
        .map_err(|(error, platform_log_path)| {
            (
                error,
                artifacts.clone(),
                platform_log_path.or_else(|| result.platform_log_path.clone()),
            )
        })?;
    }

    if let Some(error) = refusal_before_publication(
        context,
        resolved,
        format!(
            "external artifact publication for source-set '{}' and output '{}'",
            resolved.source_set_name,
            resolved.output_path.display()
        ),
    ) {
        return Err((error, artifacts, last_result.platform_log_path.clone()));
    }

    let publish_phase = publication
        .publish_dir(
            context,
            ARTIFACTS_BACKUP_PREFIX,
            "failed to publish staged external directory",
            // Путь вывода — место для порождённого, а не для чьей-то работы.
            &crate::use_cases::destruction_guard::DestructionConsent::RunnerOwned,
            &[],
        )
        .map_err(|error| {
            (
                error,
                artifacts.clone(),
                last_result.platform_log_path.clone(),
            )
        })?;

    for descriptor in &descriptors {
        let publish_name = format!(
            "{}.{}",
            sanitize_file_stem(&descriptor.logical_name),
            resolved.mode.file_extension()
        );
        let published_file = resolved.output_path.join(&publish_name);
        artifacts.push(
            ArtifactRef::new(ArtifactKind::Package, &published_file)
                .with_role(ARTIFACT_ROLE_PACKAGE_FILE),
        );
    }

    Ok((
        last_result,
        artifacts,
        publication_message(context, publish_phase),
    ))
}

fn requested_extension_name(extension: Option<&str>) -> Result<&str, AppError> {
    extension
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::Validation("artifacts cfe export requires non-empty --extension".to_owned())
        })
}

fn resolve_target(
    config: &AppConfig,
    args: &ArtifactsRequest,
) -> Result<ResolvedArtifactsTarget, AppError> {
    let output_path = validate_output_path(args)?;
    let inventory = SourceSetInventory::new(config);

    let (source_set, extension) = match args.mode {
        ArtifactsModeRequest::ConfigurationCf => {
            let source_set = match args.source_set.as_deref() {
                Some(name) => {
                    let source_set = inventory.named(name)?;
                    if source_set.purpose != SourceSetPurpose::Configuration {
                        return Err(AppError::Validation(format!(
                            "source-set '{name}' is not a configuration source-set"
                        )));
                    }
                    source_set
                }
                None => resolve_single_configuration_source_set(&inventory)?,
            };
            (source_set, None)
        }
        ArtifactsModeRequest::ExtensionCfe => {
            if let Some(source_set_name) = args.source_set.as_deref() {
                let source_set = inventory.named(source_set_name)?;
                if source_set.purpose != SourceSetPurpose::Extension {
                    return Err(AppError::Validation(format!(
                        "source-set '{source_set_name}' is not an extension source-set"
                    )));
                }
                let resolved_extension_name = platform_extension_name(source_set);
                // Набор расширения называет и само расширение: `--extension` лишь сверяется
                // с ним, когда назван.
                let requested_extension = match args.extension.as_deref() {
                    None => resolved_extension_name,
                    Some(extension) => requested_extension_name(Some(extension))?,
                };
                if resolved_extension_name != requested_extension {
                    return Err(AppError::Validation(format!(
                        "source-set '{source_set_name}' resolves to extension '{resolved_extension_name}', expected '{requested_extension}'"
                    )));
                }
                (source_set, Some(requested_extension.to_owned()))
            } else {
                let requested_extension = requested_extension_name(args.extension.as_deref())?;
                let candidates = inventory
                    .source_sets_with_purpose(SourceSetPurpose::Extension)
                    .into_iter()
                    .filter(|source_set| platform_extension_name(source_set) == requested_extension)
                    .collect::<Vec<_>>();
                if candidates.is_empty() {
                    let available = inventory
                        .source_sets_with_purpose(SourceSetPurpose::Extension)
                        .into_iter()
                        .map(|source_set| {
                            format!(
                                "{}=>{}",
                                source_set.name,
                                platform_extension_name(source_set)
                            )
                        })
                        .collect::<Vec<_>>();
                    return Err(AppError::Validation(format!(
                        "no extension source-set resolves to '{requested_extension}'; candidates [{}]",
                        available.join(", ")
                    )));
                }
                if candidates.len() != 1 {
                    let names = candidates
                        .iter()
                        .map(|source_set| source_set.name.as_str())
                        .collect::<Vec<_>>();
                    return Err(AppError::Validation(format!(
                        "extension '{requested_extension}' is ambiguous; matching source-sets [{}]",
                        names.join(", ")
                    )));
                }
                (candidates[0], Some(requested_extension.to_owned()))
            }
        }
        ArtifactsModeRequest::ExternalDataProcessorEpf
        | ArtifactsModeRequest::ExternalReportErf => {
            if args.extension.is_some() {
                return Err(AppError::Validation(
                    "external artifacts export does not support --extension".to_owned(),
                ));
            }
            let source_set_name = args.source_set.as_deref().ok_or_else(|| {
                AppError::Validation("external artifacts export requires <SET>".to_owned())
            })?;
            let source_set = inventory.named(source_set_name)?;
            let expected_purpose = match args.mode {
                ArtifactsModeRequest::ExternalDataProcessorEpf => {
                    SourceSetPurpose::ExternalDataProcessors
                }
                ArtifactsModeRequest::ExternalReportErf => SourceSetPurpose::ExternalReports,
                _ => unreachable!(),
            };
            if source_set.purpose != expected_purpose {
                return Err(AppError::Validation(format!(
                    "source-set '{source_set_name}' has incompatible type for requested external export"
                )));
            }
            (source_set, None)
        }
    };

    let _runtime_context = inventory
        .designer_context(&source_set.name)
        .ok_or_else(|| {
            AppError::Runtime(format!(
                "missing runtime context for source-set '{}'",
                source_set.name
            ))
        })?;

    let canonical_output_path = nearest_existing_canonical_path(&output_path).map_err(|error| {
        AppError::Runtime(format!("failed to canonicalize output path: {error}"))
    })?;
    let canonical_base_path =
        nearest_existing_canonical_path(&config.base_path).map_err(|error| {
            AppError::Runtime(format!("failed to canonicalize project base path: {error}"))
        })?;
    let canonical_work_path = nearest_existing_canonical_path(&config.work_path)
        .map_err(|error| AppError::Runtime(format!("failed to canonicalize workPath: {error}")))?;
    let target_identity = stable_path_identity(&canonical_output_path);
    let lock_path = hashed_lock_path(&canonical_output_path, "artifacts").map_err(|error| {
        AppError::Runtime(format!("failed to resolve artifacts lock path: {error}"))
    })?;

    Ok(ResolvedArtifactsTarget {
        mode: map_mode(args.mode),
        source_set_name: source_set.name.clone(),
        extension,
        output_path,
        source_path: inventory.source_path(source_set),
        is_directory_output: matches!(
            args.mode,
            ArtifactsModeRequest::ExternalDataProcessorEpf
                | ArtifactsModeRequest::ExternalReportErf
        ),
        canonical_output_path,
        canonical_base_path,
        canonical_work_path,
        target_identity,
        lock_path,
    })
}

/// Профиль сборки соответствует виду пакета. Исполнителя здесь не проверяют: строку
/// `make` матрицы держит валидация конфигурации, а готовность — выбор исполнителя.
fn validate_supported_matrix(args: &ArtifactsRequest) -> Option<AppError> {
    if args.execution.profile.backend_hint.as_deref() != Some("designer") {
        return Some(AppError::Validation(UNSUPPORTED_PROFILE_ERROR.to_owned()));
    }
    let expected_kind = match args.mode {
        ArtifactsModeRequest::ConfigurationCf => RunnerKind::Cf,
        ArtifactsModeRequest::ExtensionCfe => RunnerKind::Cfe,
        ArtifactsModeRequest::ExternalDataProcessorEpf => RunnerKind::Epf,
        ArtifactsModeRequest::ExternalReportErf => RunnerKind::Erf,
    };
    if args.execution.profile.kind != expected_kind {
        return Some(AppError::Validation(UNSUPPORTED_PROFILE_ERROR.to_owned()));
    }
    None
}

fn validate_output_path(args: &ArtifactsRequest) -> Result<PathBuf, AppError> {
    let output = args.output_path.trim();
    if output.is_empty() {
        return Err(AppError::Validation(
            "artifacts requires non-empty --output".to_owned(),
        ));
    }
    let output_path = PathBuf::from(output);
    match args.mode {
        ArtifactsModeRequest::ConfigurationCf | ArtifactsModeRequest::ExtensionCfe => {
            let expected_extension = match args.mode {
                ArtifactsModeRequest::ConfigurationCf => "cf",
                ArtifactsModeRequest::ExtensionCfe => "cfe",
                _ => unreachable!(),
            };
            if output_path.extension().and_then(|value| value.to_str()) != Some(expected_extension)
            {
                return Err(AppError::Validation(format!(
                    "artifacts output must end with .{expected_extension}"
                )));
            }
            if output_path.is_dir() {
                return Err(AppError::Validation(format!(
                    "artifacts output must be a file, got directory '{}'",
                    output_path.display()
                )));
            }
        }
        ArtifactsModeRequest::ExternalDataProcessorEpf
        | ArtifactsModeRequest::ExternalReportErf => {
            if !args.output_is_directory
                && output_path.extension().is_some()
                && !output_path.is_dir()
            {
                return Err(AppError::Validation(
                    "external artifacts output must be a directory".to_owned(),
                ));
            }
        }
    }
    Ok(output_path)
}

fn resolve_single_configuration_source_set<'a>(
    inventory: &SourceSetInventory<'a>,
) -> Result<&'a SourceSetConfig, AppError> {
    let configuration_source_sets =
        inventory.source_sets_with_purpose(SourceSetPurpose::Configuration);
    if configuration_source_sets.len() != 1 {
        let candidates = configuration_source_sets
            .iter()
            .map(|source_set| source_set.name.as_str())
            .collect::<Vec<_>>();
        return Err(AppError::Validation(format!(
            "artifacts cf export requires exactly one configuration source-set when <SET> is omitted; found [{}]",
            candidates.join(", ")
        )));
    }
    Ok(configuration_source_sets[0])
}

/// Последний шаг перед публикацией: прерывание и цель. Цель сверена при разрешении, но за
/// время работы исполнителя путь мог начать указывать в другое место — тогда публикация
/// останавливается, а промежуточная копия остаётся, как и при прерывании.
#[must_use = "a refusal must stop the publication"]
fn refusal_before_publication(
    context: &ExecutionContext,
    resolved: &ResolvedArtifactsTarget,
    safe_point: impl Into<String>,
) -> Option<AppError> {
    interruption_before_publish(context, safe_point)
        .or_else(|| validate_publish_target(resolved).err())
}

fn validate_publish_target(resolved: &ResolvedArtifactsTarget) -> Result<(), AppError> {
    if resolved.canonical_output_path
        != nearest_existing_canonical_path(&resolved.output_path).map_err(|error| {
            AppError::Runtime(format!("failed to re-canonicalize output path: {error}"))
        })?
    {
        return Err(AppError::Validation(format!(
            "output path changed since the target was resolved: {}",
            resolved.output_path.display()
        )));
    }
    if resolved.canonical_output_path == resolved.canonical_base_path {
        return Err(AppError::Validation(
            "artifacts output must not equal project base path".to_owned(),
        ));
    }
    if resolved.canonical_output_path == resolved.canonical_work_path {
        return Err(AppError::Validation(
            "artifacts output must not equal workPath".to_owned(),
        ));
    }
    if is_filesystem_root(&resolved.canonical_output_path) {
        return Err(AppError::Validation(
            "artifacts output must not equal filesystem root".to_owned(),
        ));
    }
    if !resolved.is_directory_output
        && resolved.output_path.exists()
        && resolved.output_path.is_dir()
    {
        return Err(AppError::Validation(format!(
            "artifacts output conflicts with existing directory '{}'",
            resolved.output_path.display()
        )));
    }
    Ok(())
}

fn cleanup_orphan_files(resolved: &ResolvedArtifactsTarget) -> Result<(), AppError> {
    let mut scan_roots = Vec::new();
    if let Some(parent) = resolved.output_path.parent() {
        scan_roots.push(parent.to_path_buf());
    }
    if resolved.is_directory_output {
        scan_roots.push(resolved.output_path.clone());
    }
    cleanup_owned_orphan_files(
        &scan_roots,
        &resolved.output_path,
        &resolved.target_identity,
        &[".artifacts-stage-"],
        &[ARTIFACTS_BACKUP_PREFIX],
        resolved.is_directory_output,
    )
}

/// Журнал `/Out` Конфигуратора у набора; `ibcmd` отвечает в свой вывод, журнала у него нет.
fn designer_log_file(
    config: &AppConfig,
    provider: Provider,
    source_set_name: &str,
    mode: ArtifactBuildMode,
) -> Result<Option<PathBuf>, AppError> {
    if provider != Provider::Designer {
        return Ok(None);
    }
    let log_dir = platform_logs_dir(&config.work_path).map_err(|error| {
        AppError::Runtime(format!("failed to create platform logs dir: {error}"))
    })?;
    let suffix = mode.file_extension();
    Ok(Some(
        log_dir.join(format!("artifacts-{source_set_name}-{suffix}.log")),
    ))
}

/// Набор основной конфигурации: на нём Конфигуратор собирает расширения. Валидация
/// конфигурации не пускает расширение без него.
fn configuration_source_set<'a>(
    inventory: &SourceSetInventory<'a>,
) -> Result<&'a SourceSetConfig, AppError> {
    inventory
        .source_sets_with_purpose(SourceSetPurpose::Configuration)
        .into_iter()
        .next()
        .ok_or_else(|| {
            AppError::Validation(
                "make requires a configuration source-set: an extension package is built on top of its configuration".to_owned(),
            )
        })
}

fn ensure_platform_success(
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

fn empty_result(
    mode: ArtifactBuildMode,
    started: Instant,
    source_set: Option<String>,
    extension: Option<String>,
    output_path: PathBuf,
    message: Option<String>,
) -> ArtifactsResult {
    let metadata = ArtifactBuildMetadata {
        artifact_type: mode,
        output_path: output_path.clone(),
        file_names: output_path
            .file_name()
            .map(|value| vec![value.to_string_lossy().into_owned()])
            .unwrap_or_default(),
        published: false,
    };
    let mut execution = ExecutionOutcome::new(ExecutionStatus::Failed).with_payload(metadata);
    if let Some(message) = message.clone() {
        execution = execution
            .with_diagnostics(vec![message.clone()])
            .with_errors(vec![ExecutionError::new("artifacts_failed", message)]);
    }
    ArtifactsResult {
        provider: None,
        // Ответ без исполнения: все шесть мест, которые его строят, — отказы раньше
        // запуска платформы: матрица, цель, цель выкладки, выбор исполнителя, замок и
        // чистка. Настоящий запуск строит ответ буквально и ставит признак сам.
        provider_dispatched: false,
        mode,
        source_set,
        extension,
        duration_ms: started.elapsed().as_millis() as u64,
        execution,
    }
}

fn merge_optional_messages(left: Option<String>, right: Option<String>) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => Some(format!("{left}; {right}")),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

/// What the publish phase has to say, and whether an interruption was deferred through it.
///
/// The flag used to be recovered by searching the message for the words "critical phase" — a
/// verdict taken from prose the runner itself had formatted (DEC.2026-09-12.TOOL-PROSE-NEVER-DECIDES). The phase knows the
/// fact, so the fact travels.
#[derive(Debug)]
struct PublicationOutcome {
    message: Option<String>,
    deferred_interruption: Option<crate::use_cases::context::ExecutionInterruption>,
}

/// Итог публикации берётся целиком: предупреждение уборки не может потеряться у места
/// вызова, и успех с неубранной копией не выглядит чистым. Разбор полный, без `..`:
/// новое поле итога не пройдёт мимо, пока здесь не решат, что с ним делать.
fn publication_message(
    context: &ExecutionContext,
    published: StagedPublicationOutcome,
) -> PublicationOutcome {
    let StagedPublicationOutcome {
        cleanup_warning,
        // Выход `make` — каталог раннера: сторож его не спрашивает, уничтоженного нет.
        discarded: _,
        deferred_interruption,
        previous_target_present: _,
    } = published;
    PublicationOutcome {
        message: merge_optional_messages(
            cleanup_warning,
            deferred_interruption
                .map(|interruption| publication_warning(context.command(), interruption)),
        ),
        deferred_interruption,
    }
}

/// Итог успешного `make`: сообщение публикации уходит в диагностику ответа, отложенное
/// прерывание — в прерывания. Здесь предупреждение уборки становится частью ответа.
fn published_execution(
    context: &ExecutionContext,
    artifacts: ArtifactSet,
    metadata: ArtifactBuildMetadata,
    publication: PublicationOutcome,
) -> ExecutionOutcome<ArtifactBuildMetadata> {
    let PublicationOutcome {
        message,
        deferred_interruption,
    } = publication;
    let execution = ExecutionOutcome::new(ExecutionStatus::Succeeded)
        .with_diagnostics(message.into_iter().collect())
        .with_artifacts(artifacts)
        .with_payload(metadata);
    match deferred_interruption {
        Some(interruption) => {
            execution.with_interruptions(vec![deferred_command_interruption_details(
                interruption,
                ExecutionInterruptionPhase::Publication,
                publication_warning(context.command(), interruption),
            )])
        }
        None => execution,
    }
}

fn publication_warning(
    command: crate::use_cases::context::CommandName,
    interruption: crate::use_cases::context::ExecutionInterruption,
) -> String {
    deferred_interruption_warning_for_command(
        "artifact publication completed",
        command,
        interruption,
    )
}

fn map_mode(mode: ArtifactsModeRequest) -> ArtifactBuildMode {
    match mode {
        ArtifactsModeRequest::ConfigurationCf => ArtifactBuildMode::ConfigurationCf,
        ArtifactsModeRequest::ExtensionCfe => ArtifactBuildMode::ExtensionCfe,
        ArtifactsModeRequest::ExternalDataProcessorEpf => {
            ArtifactBuildMode::ExternalDataProcessorEpf
        }
        ArtifactsModeRequest::ExternalReportErf => ArtifactBuildMode::ExternalReportErf,
    }
}

fn external_descriptors(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedArtifactsTarget,
) -> Result<Vec<ExternalArtifactDescriptor>, AppError> {
    let source_set = config
        .source_sets
        .iter()
        .find(|source_set| source_set.name == resolved.source_set_name)
        .ok_or_else(|| {
            AppError::Runtime(format!(
                "failed to resolve source-set '{}'",
                resolved.source_set_name
            ))
        })?;
    let expected_kind = source_set_external_kind(source_set).ok_or_else(|| {
        AppError::Validation(format!("source-set '{}' is not external", source_set.name))
    })?;
    match config.format {
        SourceFormat::Designer => discover_designer_external_artifacts(
            &resolved.source_set_name,
            &resolved.source_path,
            expected_kind,
        ),
        SourceFormat::Edt => {
            let mut utilities = PlatformUtilities::from_config(config);
            let location = utilities
                .locate(UtilityType::EdtCli)
                .map_err(AppError::from)?;
            let edt = crate::platform::edt::EdtDsl::new(
                location.path,
                config.work_path.join("edt-workspace"),
                utilities.runner_for(UtilityType::EdtCli),
                context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
            );
            prepare_edt_external_artifacts(config, source_set, &edt)
        }
    }
}

fn published_file_names(artifacts: &ArtifactSet) -> Vec<String> {
    artifacts
        .items
        .iter()
        .filter(|artifact| artifact.role.as_deref() == Some(ARTIFACT_ROLE_PACKAGE_FILE))
        .filter_map(|artifact| artifact.path.file_name())
        .map(|value| value.to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        build_external_in, build_package_in, cleanup_orphan_files, export_refusal,
        publication_message, publication_warning, published_execution, resolve_target,
        run_artifacts, validate_supported_matrix, MakeSession, ResolvedArtifactsTarget,
        SessionBases, StagedPublicationOutcome, ThrowawayInfobase,
    };
    use crate::config::model::{
        AppConfig, PlatformToolConfig, SourceFormat, SourceSetConfig, SourceSetPurpose,
        TestsConfig, ToolsConfig,
    };
    use crate::domain::artifact::{
        ArtifactSet, ARTIFACT_ROLE_PACKAGE_FILE, ARTIFACT_ROLE_PLATFORM_LOG,
        ARTIFACT_ROLE_STAGE_FILE,
    };
    use crate::domain::artifacts::{ArtifactBuildMetadata, ArtifactBuildMode, ArtifactsResult};
    use crate::domain::execution::{ExecutionInterruptionPhase, ExecutionStatus};
    use crate::platform::process::{
        ProcessError, ProcessExecutionPolicy, ProcessRequest, ProcessResult, ProcessRunner,
        SpawnResult,
    };
    use crate::support::error::AppError;
    use crate::support::fs::{
        metadata_sidecar_path, read_temp_dir_metadata, write_temp_dir_metadata, TempDirKind,
    };
    use crate::use_cases::context::{CommandName, ExecutionContext};
    use crate::use_cases::request::{ArtifactsModeRequest, ArtifactsRequest};
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}

    #[cfg(unix)]
    fn write_script(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        make_executable(path);
    }

    #[cfg(not(unix))]
    fn write_script(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(path, body).expect("write script");
        make_executable(path);
    }

    fn artifacts_payload(result: &ArtifactsResult) -> &ArtifactBuildMetadata {
        result.execution.payload.as_ref().expect("payload")
    }

    fn artifacts_set(result: &ArtifactsResult) -> &ArtifactSet {
        result.execution.artifacts.as_ref().expect("artifacts")
    }

    /// Подставной Конфигуратор: выгружает `.cf` и журнал, а затем делает то, что задал тест, —
    /// отменяет команду или подменяет цель — и выходит с заданным кодом.
    #[cfg(unix)]
    struct DumpThen<F> {
        then: F,
        exit_code: i32,
    }

    #[cfg(unix)]
    impl<F> DumpThen<F> {
        fn new(then: F) -> Self {
            Self { then, exit_code: 0 }
        }

        fn exiting(self, exit_code: i32) -> Self {
            Self { exit_code, ..self }
        }
    }

    #[cfg(unix)]
    impl<F: Fn(&ProcessExecutionPolicy)> ProcessRunner for DumpThen<F> {
        fn run_with_policy(
            &self,
            request: &ProcessRequest,
            policy: &ProcessExecutionPolicy,
        ) -> Result<ProcessResult, ProcessError> {
            // Как настоящий исполнитель, двойник отмечает работу, едва «запустил» процесс.
            policy.mark_started_for_test();
            let dumps = request.args.iter().any(|arg| arg == "/DumpCfg");
            let mut previous = "";
            for arg in &request.args {
                if previous == "/DumpCfg" {
                    fs::write(arg, "cf").map_err(|error| ProcessError::StdoutLogIo {
                        path: PathBuf::from(arg),
                        source: error,
                    })?;
                }
                if previous == "/Out" {
                    fs::write(arg, "designer log").map_err(|error| ProcessError::StdoutLogIo {
                        path: PathBuf::from(arg),
                        source: error,
                    })?;
                }
                previous = arg;
            }
            // Создание базы и загрузка исходников проходят чисто: задуманное случается на
            // выгрузке пакета.
            if !dumps {
                return Ok(ProcessResult {
                    exit_code: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                    interruption: None,
                });
            }
            (self.then)(policy);
            Ok(ProcessResult {
                exit_code: self.exit_code,
                stdout: String::new(),
                stderr: String::new(),
                interruption: None,
            })
        }

        fn spawn(
            &self,
            _request: &ProcessRequest,
            _work: &crate::platform::process::WorkGiven,
        ) -> Result<SpawnResult, ProcessError> {
            unreachable!("the export runs to its end and never spawns")
        }
    }

    fn fake_designer() -> crate::use_cases::throwaway_infobase::Builder {
        crate::use_cases::throwaway_infobase::Builder {
            provider: crate::domain::capability::Provider::Designer,
            binary: PathBuf::from("/tmp/fake-1cv8"),
        }
    }

    fn sample_config(
        base: &Path,
        work: &Path,
        platform_path: &Path,
        format: SourceFormat,
    ) -> AppConfig {
        AppConfig {
            base_path: base.to_path_buf(),
            work_path: work.to_path_buf(),
            format,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: vec![
                SourceSetConfig {
                    name: "configuration".to_owned(),
                    purpose: SourceSetPurpose::Configuration,
                    path: PathBuf::from("configuration"),
                },
                SourceSetConfig {
                    name: "ext-sales".to_owned(),
                    purpose: SourceSetPurpose::Extension,
                    path: PathBuf::from("extensions/ext-sales"),
                },
            ],
            tools: ToolsConfig {
                platform: PlatformToolConfig {
                    path: Some(platform_path.to_path_buf()),
                    strict: false,
                    version: None,
                },
                ..ToolsConfig::default()
            },
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    fn cf_request(output: &str) -> ArtifactsRequest {
        ArtifactsRequest {
            dry_run: false,
            output_is_directory: false,
            execution: ArtifactsRequest::default_execution(ArtifactsModeRequest::ConfigurationCf),
            mode: ArtifactsModeRequest::ConfigurationCf,
            output_path: output.to_owned(),
            source_set: None,
            extension: None,
        }
    }

    fn external_request(
        mode: ArtifactsModeRequest,
        output: &str,
        source_set: &str,
    ) -> ArtifactsRequest {
        ArtifactsRequest {
            dry_run: false,
            output_is_directory: false,
            execution: ArtifactsRequest::default_execution(mode),
            mode,
            output_path: output.to_owned(),
            source_set: Some(source_set.to_owned()),
            extension: None,
        }
    }

    fn add_external_source_set(config: &mut AppConfig, name: &str, purpose: SourceSetPurpose) {
        config.source_sets.push(SourceSetConfig {
            name: name.to_owned(),
            purpose,
            path: PathBuf::from(name),
        });
    }

    #[test]
    fn validate_supported_matrix_rejects_non_designer_profile() {
        let dir = tempdir().expect("tempdir");
        let mut request = cf_request("release.cf");
        request.execution.profile.backend_hint = Some("ibcmd".to_owned());
        let config = sample_config(
            dir.path(),
            dir.path(),
            Path::new("/tmp/1cv8"),
            SourceFormat::Designer,
        );

        let _ = config;
        let error = validate_supported_matrix(&request).expect("error");

        assert!(error.to_string().contains("runner profiles"));
    }

    #[test]
    fn resolve_target_uses_source_set_name_for_edt_extension_identity() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("extensions/ext-sales")).expect("extension dir");
        fs::write(
            dir.path().join("extensions/ext-sales/.project"),
            "<projectDescription><name>sales-project</name></projectDescription>",
        )
        .expect("project");
        let mut config = sample_config(
            dir.path(),
            dir.path(),
            Path::new("/tmp/1cv8"),
            SourceFormat::Edt,
        );
        config.source_sets[1].name = "SalesAddon".to_owned();
        let request = ArtifactsRequest {
            dry_run: false,
            output_is_directory: false,
            execution: ArtifactsRequest::default_execution(ArtifactsModeRequest::ExtensionCfe),
            mode: ArtifactsModeRequest::ExtensionCfe,
            output_path: "dist/sales.cfe".to_owned(),
            source_set: None,
            extension: Some("SalesAddon".to_owned()),
        };

        let resolved = resolve_target(&config, &request).expect("resolved");

        assert_eq!(resolved.source_set_name, "SalesAddon");
        assert_eq!(resolved.extension.as_deref(), Some("SalesAddon"));
        assert_eq!(resolved.mode, ArtifactBuildMode::ExtensionCfe);
    }

    #[test]
    fn resolve_target_rejects_blank_extension_for_cfe_mode() {
        let dir = tempdir().expect("tempdir");
        let config = sample_config(
            dir.path(),
            dir.path(),
            Path::new("/tmp/1cv8"),
            SourceFormat::Designer,
        );
        let request = ArtifactsRequest {
            dry_run: false,
            output_is_directory: false,
            execution: ArtifactsRequest::default_execution(ArtifactsModeRequest::ExtensionCfe),
            mode: ArtifactsModeRequest::ExtensionCfe,
            output_path: "dist/sales.cfe".to_owned(),
            source_set: Some("ext-sales".to_owned()),
            extension: Some("   ".to_owned()),
        };

        let error = resolve_target(&config, &request).expect_err("blank extension should fail");

        assert!(error.to_string().contains("non-empty --extension"));
    }

    /// Отказ называет позиционный `<SET>`: прежний ключ `--source-set` скрыт и в текстах
    /// отказов не звучит.
    #[test]
    fn resolve_target_refusals_name_the_positional_set() {
        let dir = tempdir().expect("tempdir");
        let mut config = sample_config(
            dir.path(),
            dir.path(),
            Path::new("/tmp/1cv8"),
            SourceFormat::Designer,
        );
        let mut external = cf_request("dist/external");
        external.mode = ArtifactsModeRequest::ExternalDataProcessorEpf;
        external.execution =
            ArtifactsRequest::default_execution(ArtifactsModeRequest::ExternalDataProcessorEpf);
        let without_set = resolve_target(&config, &external)
            .expect_err("external export without a set")
            .to_string();
        assert!(without_set.contains("requires <SET>"), "{without_set}");
        assert!(!without_set.contains("--source-set"), "{without_set}");

        config.source_sets.push(SourceSetConfig {
            name: "configuration-2".to_owned(),
            purpose: SourceSetPurpose::Configuration,
            path: PathBuf::from("configuration-2"),
        });
        let ambiguous = resolve_target(&config, &cf_request("dist/main.cf"))
            .expect_err("several configuration sets")
            .to_string();
        assert!(ambiguous.contains("when <SET> is omitted"), "{ambiguous}");
        assert!(!ambiguous.contains("--source-set"), "{ambiguous}");
    }

    #[cfg(unix)]
    #[test]
    fn run_artifacts_exports_cf_and_records_artifacts() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("configuration")).expect("config dir");
        let script = dir.path().join("1cv8");
        write_script(
            &script,
            "out=''\nprev=''\nfor arg in \"$@\"; do\n  if [ \"$prev\" = '/DumpCfg' ]; then printf 'cf' > \"$arg\"; fi\n  if [ \"$prev\" = '/Out' ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf 'designer log' > \"$out\"; fi\nexit 0",
        );
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        fs::create_dir_all(base.join("configuration")).expect("base config");
        fs::create_dir_all(&work).expect("work");
        let config = sample_config(&base, &work, &script, SourceFormat::Designer);
        let request = cf_request(&dir.path().join("dist/release.cf").display().to_string());

        let result = run_artifacts(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &request,
            &mut MakeSession::new(&config),
        )
        .expect("result");

        assert!(result.execution.is_ok());
        assert!(artifacts_payload(&result).output_path.is_file());
        assert_eq!(
            artifacts_set(&result).get_by_role(ARTIFACT_ROLE_PACKAGE_FILE),
            Some(artifacts_payload(&result).output_path.as_path())
        );
        assert!(artifacts_set(&result)
            .get_by_role(ARTIFACT_ROLE_PLATFORM_LOG)
            .is_some());
    }

    #[cfg(unix)]
    #[test]
    fn run_artifacts_honors_interruption_before_export_safe_point() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("configuration")).expect("config dir");
        let script = dir.path().join("1cv8");
        write_script(&script, "printf 'unexpected invocation\\n' >&2\nexit 1");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        fs::create_dir_all(base.join("configuration")).expect("base config");
        fs::create_dir_all(&work).expect("work");
        let config = sample_config(&base, &work, &script, SourceFormat::Designer);
        let request = cf_request(&dir.path().join("dist/release.cf").display().to_string());
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(CommandName::Artifacts).with_cancellation(cancellation);

        let failure = run_artifacts(&context, &config, &request, &mut MakeSession::new(&config))
            .expect_err("failure");
        let error_text = failure.error.to_string();
        let kind = failure.error.kind();
        let payload = failure.payload.expect("payload");

        assert!(error_text.contains("before entering artifact export"));
        assert_eq!(
            kind,
            crate::use_cases::result::UseCaseErrorKind::Cancelled(
                crate::support::error::CancelledAt::Boundary
            )
        );
        assert_eq!(payload.execution.status, ExecutionStatus::Cancelled);
        crate::use_cases::interruption::assert_stopped_at_a_safe_point(&payload.execution);
        let [interruption] = payload.execution.interruptions.as_slice() else {
            panic!(
                "one interruption expected: {:?}",
                payload.execution.interruptions
            );
        };
        // До экспорта ничего не выгружено: это безопасная точка команды.
        assert_eq!(
            interruption.phase,
            Some(ExecutionInterruptionPhase::CommandBoundary)
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_artifacts_platform_failure_without_stage_file_reports_no_stage_artifact() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("configuration")).expect("config dir");
        let script = dir.path().join("1cv8");
        write_script(
            &script,
            "out=''\nprev=''\nfor arg in \"$@\"; do\n  if [ \"$prev\" = '/Out' ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf 'designer log' > \"$out\"; fi\ncase \" $* \" in *' /DumpCfg '*) exit 12 ;; esac\nexit 0",
        );
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        fs::create_dir_all(base.join("configuration")).expect("base config");
        fs::create_dir_all(&work).expect("work");
        let config = sample_config(&base, &work, &script, SourceFormat::Designer);
        let request = cf_request(&dir.path().join("dist/release.cf").display().to_string());

        let failure = run_artifacts(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &request,
            &mut MakeSession::new(&config),
        )
        .expect_err("failure");
        let payload = failure.payload.expect("payload");
        let artifacts = artifacts_set(&payload);
        let dist_dir = dir.path().join("dist");

        assert!(artifacts.get_by_role(ARTIFACT_ROLE_STAGE_FILE).is_none());
        assert!(artifacts.get_by_role(ARTIFACT_ROLE_PACKAGE_FILE).is_none());
        assert!(artifacts.get_by_role(ARTIFACT_ROLE_PLATFORM_LOG).is_some());
        assert!(!dist_dir
            .read_dir()
            .expect("dist entries")
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".artifacts-stage-")));
    }

    #[cfg(unix)]
    #[test]
    fn designer_export_interruption_before_publish_retains_stage_artifact() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        fs::create_dir_all(base.join("configuration")).expect("base config");
        fs::create_dir_all(&work).expect("work");
        let config = sample_config(
            &base,
            &work,
            Path::new("/tmp/fake-1cv8"),
            SourceFormat::Designer,
        );
        let request = cf_request(&dir.path().join("dist/release.cf").display().to_string());
        let resolved = resolve_target(&config, &request).expect("resolved");
        let context = ExecutionContext::cli(CommandName::Artifacts);

        let failure = build_package_in(
            &context,
            &config,
            &resolved,
            fake_designer(),
            &mut SessionBases::default(),
            &DumpThen::new(|policy: &ProcessExecutionPolicy| policy.cancellation.cancel()),
        )
        .expect_err("interrupted before publish");
        let (error, artifacts, _platform_log_path) = failure;
        let stage_path = artifacts
            .get_by_role(ARTIFACT_ROLE_STAGE_FILE)
            .expect("stage artifact");

        assert!(error
            .to_string()
            .contains("before entering artifact publication"));
        assert!(stage_path.is_file());
        assert!(!resolved.output_path.exists());
    }

    /// Цель перепроверяется перед публикацией: путь, который за время работы исполнителя
    /// стал указывать в другое место, публикацию останавливает, цель не трогается, а
    /// промежуточная копия остаётся, как при прерывании.
    #[cfg(unix)]
    #[test]
    fn designer_export_rechecks_the_target_before_publishing() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        fs::create_dir_all(base.join("configuration")).expect("base config");
        fs::create_dir_all(&work).expect("work");
        let elsewhere = dir.path().join("elsewhere.cf");
        fs::write(&elsewhere, "someone else's package").expect("elsewhere");
        let config = sample_config(
            &base,
            &work,
            Path::new("/tmp/fake-1cv8"),
            SourceFormat::Designer,
        );
        let target = dir.path().join("dist/release.cf");
        let request = cf_request(&target.display().to_string());
        let resolved = resolve_target(&config, &request).expect("resolved");
        let context = ExecutionContext::cli(CommandName::Artifacts);
        // Пока исполнитель работает, на месте цели появляется ссылка в чужое место.
        let runner = DumpThen::new(|_: &ProcessExecutionPolicy| {
            std::os::unix::fs::symlink(&elsewhere, &target).expect("plant a link");
        });

        let (error, artifacts, _platform_log_path) = build_package_in(
            &context,
            &config,
            &resolved,
            fake_designer(),
            &mut SessionBases::default(),
            &runner,
        )
        .expect_err("a moved target must stop the publication");

        assert!(matches!(error, AppError::Validation(_)), "{error}");
        assert!(error.to_string().contains("output path changed"), "{error}");
        assert!(
            fs::symlink_metadata(&target)
                .expect("target")
                .file_type()
                .is_symlink(),
            "the planted link must stay in place"
        );
        assert_eq!(
            fs::read_to_string(&elsewhere).expect("elsewhere"),
            "someone else's package"
        );
        let stage_path = artifacts
            .get_by_role(ARTIFACT_ROLE_STAGE_FILE)
            .expect("stage artifact");
        assert!(stage_path.is_file());
    }

    /// Отказ, пришедший, когда отмена уже ожидает, остаётся отказом со своим кодом: сигнал
    /// без ошибки отмены прерыванием его не делает (#308).
    #[cfg(unix)]
    #[test]
    fn an_unrelated_failure_while_an_interruption_is_pending_stays_a_failure() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        fs::create_dir_all(base.join("configuration")).expect("base config");
        fs::create_dir_all(&work).expect("work");
        let config = sample_config(
            &base,
            &work,
            Path::new("/tmp/fake-1cv8"),
            SourceFormat::Designer,
        );
        let request = cf_request(&dir.path().join("dist/release.cf").display().to_string());
        let resolved = resolve_target(&config, &request).expect("resolved");
        let context = ExecutionContext::cli(CommandName::Artifacts);
        // Конфигуратор выходит с отказом, а оператор тем временем просит остановиться.
        let runner = DumpThen::new(|policy: &ProcessExecutionPolicy| policy.cancellation.cancel())
            .exiting(12);

        let (error, artifacts, platform_log_path) = build_package_in(
            &context,
            &config,
            &resolved,
            fake_designer(),
            &mut SessionBases::default(),
            &runner,
        )
        .expect_err("the export failed");
        assert!(
            context.interruption().is_some(),
            "the interruption is pending"
        );
        let failure = export_refusal(
            &resolved,
            std::time::Instant::now(),
            error,
            artifacts,
            platform_log_path,
        );

        assert_eq!(
            failure.error.kind(),
            crate::use_cases::result::UseCaseErrorKind::Platform
        );
        let payload = failure.payload.expect("payload");
        assert_eq!(payload.execution.status, ExecutionStatus::Failed);
        assert!(
            payload.execution.interruptions.is_empty(),
            "{:?}",
            payload.execution.interruptions
        );
        assert_eq!(payload.execution.errors[0].code, "designer_export_failed");
    }

    /// Конфигуратор, снятый отменой посреди выгрузки, — оборванная работа команды: фаза
    /// `provider_command`, род отказа — отмена (#308).
    #[cfg(unix)]
    #[test]
    fn a_designer_export_cancelled_after_its_start_is_a_cut_provider_command() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("configuration")).expect("config dir");
        let script = dir.path().join("1cv8");
        let started = dir.path().join("started");
        write_script(
            &script,
            &format!(
                ": > '{}'\nwaited=0\nwhile [ \"$waited\" -lt 300 ]; do sleep 0.1; waited=$((waited + 1)); done\nexit 0",
                started.display()
            ),
        );
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        fs::create_dir_all(base.join("configuration")).expect("base config");
        fs::create_dir_all(&work).expect("work");
        let config = sample_config(&base, &work, &script, SourceFormat::Designer);
        let request = cf_request(&dir.path().join("dist/release.cf").display().to_string());
        let cancellation = CancellationToken::new();
        let operator =
            crate::platform::process::cancel_when_started(&started, cancellation.clone());

        let failure = super::execute(
            &ExecutionContext::cli(CommandName::Artifacts).with_cancellation(cancellation),
            &config,
            &request,
        )
        .expect_err("the export was cancelled");
        assert!(
            operator.join().expect("operator"),
            "the command never marked its start"
        );

        assert_eq!(
            failure.error.kind(),
            crate::use_cases::result::UseCaseErrorKind::Cancelled(
                crate::support::error::CancelledAt::Work
            )
        );
        let payload = failure.payload.expect("payload");
        assert!(payload.provider_dispatched, "the export had started");
        assert_eq!(payload.execution.status, ExecutionStatus::Cancelled);
        let [interruption] = payload.execution.interruptions.as_slice() else {
            panic!(
                "one interruption expected: {:?}",
                payload.execution.interruptions
            );
        };
        assert_eq!(
            interruption.phase,
            Some(ExecutionInterruptionPhase::ProviderCommand)
        );
        assert!(!interruption.deferred);
        let [error] = payload.execution.errors.as_slice() else {
            panic!(
                "a cut export is one cancelled error: {:?}",
                payload.execution.errors
            );
        };
        assert_eq!(error.code, "cancelled");
        assert_eq!(
            interruption.message.as_deref(),
            Some(error.message.as_str())
        );
    }

    /// Подставной исполнитель, который записывает каждый вызов: пишет пакет по доводу
    /// `/DumpCfg` и по ключу `--out=`, а на шаге `cancel_on` просит остановиться.
    struct Recorder {
        calls: std::sync::Mutex<Vec<Vec<String>>>,
        cancel_on: Option<&'static str>,
    }

    impl Recorder {
        fn new() -> Self {
            Self {
                calls: std::sync::Mutex::new(Vec::new()),
                cancel_on: None,
            }
        }

        fn cancelling_on(step: &'static str) -> Self {
            Self {
                cancel_on: Some(step),
                ..Self::new()
            }
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().expect("calls").clone()
        }
    }

    impl ProcessRunner for Recorder {
        fn run_with_policy(
            &self,
            request: &ProcessRequest,
            policy: &ProcessExecutionPolicy,
        ) -> Result<ProcessResult, ProcessError> {
            policy.mark_started_for_test();
            self.calls.lock().expect("calls").push(request.args.clone());
            let external_load = after(&request.args, "/LoadExternalDataProcessorOrReportFromFiles")
                .and_then(|_| request.args.last());
            if let Some(binary) = external_load {
                fs::write(binary, "epf").expect("external package");
            }
            if let Some(xml) = after(&request.args, "/DumpExternalDataProcessorOrReportToFiles") {
                let name = Path::new(xml)
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default();
                fs::create_dir_all(Path::new(xml).parent().expect("parent")).expect("dump dir");
                fs::write(
                    xml,
                    format!(
                        "<ExternalDataProcessor><Properties><Name>{name}</Name></Properties></ExternalDataProcessor>"
                    ),
                )
                .expect("external descriptor");
            }
            let mut previous = "";
            for arg in &request.args {
                let package = if previous == "/DumpCfg" {
                    Some(arg.as_str())
                } else {
                    arg.strip_prefix("--out=")
                };
                if let Some(package) = package {
                    fs::write(package, "package").expect("package");
                }
                previous = arg;
            }
            if let Some(step) = self.cancel_on {
                if request.args.iter().any(|arg| arg == step) {
                    policy.cancellation.cancel();
                }
            }
            Ok(ProcessResult {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                interruption: None,
            })
        }

        fn spawn(
            &self,
            _request: &ProcessRequest,
            _work: &crate::platform::process::WorkGiven,
        ) -> Result<SpawnResult, ProcessError> {
            unreachable!("a package build runs every step to its end")
        }
    }

    fn throwaway_root(work: &Path) -> PathBuf {
        crate::use_cases::throwaway_infobase::throwaway_root(work).expect("root")
    }

    fn bases_left(work: &Path) -> Vec<PathBuf> {
        fs::read_dir(throwaway_root(work))
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default()
    }

    fn after<'a>(call: &'a [String], key: &str) -> Option<&'a str> {
        call.iter()
            .position(|arg| arg == key)
            .and_then(|index| call.get(index + 1))
            .map(String::as_str)
    }

    fn project(dir: &Path) -> (AppConfig, PathBuf) {
        let base = dir.join("base");
        let work = dir.join("work");
        fs::create_dir_all(base.join("configuration")).expect("configuration");
        fs::create_dir_all(base.join("extensions/ext-sales")).expect("extension");
        fs::create_dir_all(&work).expect("work");
        let config = sample_config(
            &base,
            &work,
            Path::new("/tmp/fake-1cv8"),
            SourceFormat::Designer,
        );
        (config, work)
    }

    /// Конфигуратор собирает `.cf` во временной базе под `workPath`: создаёт её, загружает
    /// исходники без файла версий и выгружает пакет — базу проекта не трогает.
    #[test]
    fn designer_builds_a_cf_from_the_sources_in_a_throwaway_base() {
        let dir = tempdir().expect("tempdir");
        let (config, work) = project(dir.path());
        let request = cf_request(&dir.path().join("dist/release.cf").display().to_string());
        let resolved = resolve_target(&config, &request).expect("resolved");
        let runner = Recorder::new();
        let mut base = SessionBases::default();

        build_package_in(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &resolved,
            fake_designer(),
            &mut base,
            &runner,
        )
        .expect("built");

        let calls = runner.calls();
        assert_eq!(calls.len(), 3, "{calls:?}");
        let root = throwaway_root(&work);
        let created = calls[0].iter().position(|arg| arg == "CREATEINFOBASE");
        let address = created
            .and_then(|index| calls[0].get(index + 1))
            .expect("address");
        assert!(address.contains(&root.display().to_string()), "{address}");
        assert!(!address.contains("/tmp/ib"), "{address}");
        let connection = after(&calls[1], "/IBConnectionString").expect("connection");
        assert!(
            connection.contains(&root.display().to_string()),
            "{connection}"
        );
        assert_eq!(
            after(&calls[1], "/LoadConfigFromFiles"),
            Some(
                config
                    .base_path
                    .join("configuration")
                    .display()
                    .to_string()
                    .as_str()
            )
        );
        assert!(
            !calls[1].iter().any(|arg| arg == "-updateConfigDumpInfo"),
            "{calls:?}"
        );
        assert!(
            !calls[1].iter().any(|arg| arg == "/UpdateDBCfg"),
            "{calls:?}"
        );
        assert!(after(&calls[2], "/DumpCfg").is_some(), "{calls:?}");
        assert!(resolved.output_path.is_file());

        let warnings = base
            .bases
            .into_iter()
            .flat_map(ThrowawayInfobase::close)
            .collect::<Vec<_>>();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(bases_left(&work).is_empty(), "{:?}", bases_left(&work));
    }

    /// Расширение Конфигуратор загружает поверх основной конфигурации: сперва она, затем
    /// расширение с `-Extension`, затем выгрузка расширения.
    #[test]
    fn designer_loads_the_configuration_before_the_extension() {
        let dir = tempdir().expect("tempdir");
        let (config, _work) = project(dir.path());
        let mut request = cf_request(&dir.path().join("dist/sales.cfe").display().to_string());
        request.mode = ArtifactsModeRequest::ExtensionCfe;
        request.execution = ArtifactsRequest::default_execution(ArtifactsModeRequest::ExtensionCfe);
        request.source_set = Some("ext-sales".to_owned());
        let resolved = resolve_target(&config, &request).expect("resolved");
        let extension = resolved.extension.clone().expect("extension name");
        let runner = Recorder::new();

        build_package_in(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &resolved,
            fake_designer(),
            &mut SessionBases::default(),
            &runner,
        )
        .expect("built");

        let calls = runner.calls();
        assert_eq!(calls.len(), 4, "{calls:?}");
        assert_eq!(
            after(&calls[1], "/LoadConfigFromFiles"),
            Some(
                config
                    .base_path
                    .join("configuration")
                    .display()
                    .to_string()
                    .as_str()
            )
        );
        assert_eq!(after(&calls[1], "-Extension"), None, "{calls:?}");
        assert_eq!(
            after(&calls[2], "/LoadConfigFromFiles"),
            Some(
                config
                    .base_path
                    .join("extensions/ext-sales")
                    .display()
                    .to_string()
                    .as_str()
            )
        );
        assert_eq!(after(&calls[2], "-Extension"), Some(extension.as_str()));
        assert!(after(&calls[3], "/DumpCfg").is_some(), "{calls:?}");
        assert_eq!(after(&calls[3], "-Extension"), Some(extension.as_str()));
    }

    /// Основная конфигурация попадает в базу прогона один раз: следующее расширение
    /// загружается поверх неё без повторной загрузки.
    #[test]
    fn one_base_serves_every_package_of_a_run() {
        let dir = tempdir().expect("tempdir");
        let (config, _work) = project(dir.path());
        let runner = Recorder::new();
        let context = ExecutionContext::cli(CommandName::Artifacts);
        let mut base = SessionBases::default();
        let cf = resolve_target(
            &config,
            &cf_request(&dir.path().join("dist/main.cf").display().to_string()),
        )
        .expect("cf");
        build_package_in(&context, &config, &cf, fake_designer(), &mut base, &runner)
            .expect("cf built");
        let mut request = cf_request(&dir.path().join("dist/sales.cfe").display().to_string());
        request.mode = ArtifactsModeRequest::ExtensionCfe;
        request.execution = ArtifactsRequest::default_execution(ArtifactsModeRequest::ExtensionCfe);
        request.source_set = Some("ext-sales".to_owned());
        let cfe = resolve_target(&config, &request).expect("cfe");
        build_package_in(&context, &config, &cfe, fake_designer(), &mut base, &runner)
            .expect("cfe built");

        let calls = runner.calls();
        let created = calls
            .iter()
            .filter(|call| call.iter().any(|arg| arg == "CREATEINFOBASE"))
            .count();
        let configuration_loads = calls
            .iter()
            .filter(|call| {
                call.iter().any(|arg| arg == "/LoadConfigFromFiles")
                    && !call.iter().any(|arg| arg == "-Extension")
            })
            .count();
        assert_eq!(created, 1, "{calls:?}");
        assert_eq!(configuration_loads, 1, "{calls:?}");
        assert_eq!(calls.len(), 5, "{calls:?}");
    }

    /// `ibcmd` создаёт базу со своим каталогом данных и собирает пакет `config import` с
    /// `--out`; основная конфигурация расширению не нужна, общий `workPath/ibcmd-data` не
    /// трогается.
    #[test]
    fn ibcmd_builds_with_out_and_its_own_data_directory() {
        let dir = tempdir().expect("tempdir");
        let (config, work) = project(dir.path());
        let mut request = cf_request(&dir.path().join("dist/sales.cfe").display().to_string());
        request.mode = ArtifactsModeRequest::ExtensionCfe;
        request.execution = ArtifactsRequest::default_execution(ArtifactsModeRequest::ExtensionCfe);
        request.source_set = Some("ext-sales".to_owned());
        let resolved = resolve_target(&config, &request).expect("resolved");
        let runner = Recorder::new();
        let ibcmd = crate::use_cases::throwaway_infobase::Builder {
            provider: crate::domain::capability::Provider::Ibcmd,
            binary: PathBuf::from("/tmp/fake-ibcmd"),
        };

        build_package_in(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &resolved,
            ibcmd,
            &mut SessionBases::default(),
            &runner,
        )
        .expect("built");

        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "{calls:?}");
        let root = throwaway_root(&work);
        for call in &calls {
            let data = after(call, "--data").expect("own data directory");
            assert!(data.starts_with(&root.display().to_string()), "{data}");
            assert_ne!(Path::new(data), work.join("ibcmd-data"));
            let db = after(call, "--db-path").expect("database path");
            assert!(db.starts_with(&root.display().to_string()), "{db}");
        }
        assert!(calls[0].iter().any(|arg| arg == "create"), "{calls:?}");
        let import = &calls[1];
        assert!(
            import
                .windows(2)
                .any(|pair| pair[0] == "config" && pair[1] == "import"),
            "{import:?}"
        );
        assert!(
            import.iter().any(|arg| arg.starts_with("--out=")),
            "a build without --out loads the sources into the base: {import:?}"
        );
        assert_eq!(
            import.last().map(String::as_str),
            Some(
                config
                    .base_path
                    .join("extensions/ext-sales")
                    .display()
                    .to_string()
                    .as_str()
            )
        );
        assert!(resolved.output_path.is_file());
    }

    /// Отмена, пришедшая во время создания базы, останавливает сборку на следующей
    /// безопасной точке — перед загрузкой; пакета нет, а база убирается.
    #[test]
    fn a_cancellation_during_creation_stops_before_the_load_and_the_base_is_removed() {
        let dir = tempdir().expect("tempdir");
        let (config, work) = project(dir.path());
        let request = cf_request(&dir.path().join("dist/release.cf").display().to_string());
        let resolved = resolve_target(&config, &request).expect("resolved");
        let runner = Recorder::cancelling_on("CREATEINFOBASE");
        let mut base = SessionBases::default();

        let (error, _, _) = build_package_in(
            &ExecutionContext::cli(CommandName::Artifacts)
                .with_cancellation(CancellationToken::new()),
            &config,
            &resolved,
            fake_designer(),
            &mut base,
            &runner,
        )
        .expect_err("cancelled");

        assert!(error.to_string().contains("sources load"), "{error}");
        assert!(error.cancellation().is_some(), "{error}");
        assert_eq!(runner.calls().len(), 1, "{:?}", runner.calls());
        assert!(!resolved.output_path.exists());
        drop(base);
        assert!(bases_left(&work).is_empty(), "{:?}", bases_left(&work));
    }

    /// Отмена во время загрузки останавливает сборку перед выгрузкой.
    #[test]
    fn a_cancellation_during_the_load_stops_before_the_dump() {
        let dir = tempdir().expect("tempdir");
        let (config, _work) = project(dir.path());
        let request = cf_request(&dir.path().join("dist/release.cf").display().to_string());
        let resolved = resolve_target(&config, &request).expect("resolved");
        let runner = Recorder::cancelling_on("/LoadConfigFromFiles");

        let (error, _, _) = build_package_in(
            &ExecutionContext::cli(CommandName::Artifacts)
                .with_cancellation(CancellationToken::new()),
            &config,
            &resolved,
            fake_designer(),
            &mut SessionBases::default(),
            &runner,
        )
        .expect_err("cancelled");

        assert!(error.to_string().contains("package dump"), "{error}");
        assert_eq!(runner.calls().len(), 2, "{:?}", runner.calls());
    }

    /// Формат EDT: исходники сперва переводит в XML `1cedtcli` — в каталог временной базы, —
    /// и Конфигуратор загружает уже их; перевод убирается вместе с базой.
    #[cfg(unix)]
    #[test]
    fn edt_sources_are_converted_to_xml_inside_the_throwaway_base_first() {
        let dir = tempdir().expect("tempdir");
        let (mut config, work) = project(dir.path());
        config.format = SourceFormat::Edt;
        fs::write(
            config.base_path.join("configuration/.project"),
            "<projectDescription><name>main-project</name></projectDescription>",
        )
        .expect("project");
        let edt = dir.path().join("1cedtcli");
        let edt_calls = dir.path().join("edt-calls");
        write_script(
            &edt,
            &format!(
                "target=''\nprev=''\nfor arg in \"$@\"; do\n  if [ \"$prev\" = '--configuration-files' ]; then target=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nmkdir -p \"$target\"\nprintf '<Configuration />' > \"$target/Configuration.xml\"\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0",
                edt_calls.display()
            ),
        );
        config.tools.edt_cli = crate::config::model::EdtCliConfig {
            path: Some(edt.clone()),
            auto_start: false,
            ..Default::default()
        };
        let request = cf_request(&dir.path().join("dist/release.cf").display().to_string());
        let resolved = resolve_target(&config, &request).expect("resolved");
        let runner = Recorder::new();
        let mut base = SessionBases::default();

        build_package_in(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &resolved,
            fake_designer(),
            &mut base,
            &runner,
        )
        .expect("built");

        let edt_calls = fs::read_to_string(&edt_calls).expect("edt calls");
        assert!(edt_calls.contains("main-project"), "{edt_calls}");
        let calls = runner.calls();
        let loaded = after(&calls[1], "/LoadConfigFromFiles").expect("load");
        assert!(
            loaded.starts_with(&throwaway_root(&work).display().to_string()),
            "{loaded}"
        );
        assert!(Path::new(loaded).join("Configuration.xml").is_file());
        drop(base);
        assert!(bases_left(&work).is_empty(), "{:?}", bases_left(&work));
    }

    fn external_project(dir: &Path) -> (AppConfig, PathBuf, ResolvedArtifactsTarget) {
        let (mut config, work) = project(dir);
        fs::create_dir_all(config.base_path.join("tools")).expect("tools");
        fs::write(
            config.base_path.join("tools/Tool.xml"),
            "<ExternalDataProcessor><Properties><Name>Tool</Name></Properties></ExternalDataProcessor>",
        )
        .expect("descriptor");
        add_external_source_set(
            &mut config,
            "tools",
            SourceSetPurpose::ExternalDataProcessors,
        );
        let request = external_request(
            ArtifactsModeRequest::ExternalDataProcessorEpf,
            &dir.join("dist/tools").display().to_string(),
            "tools",
        );
        let resolved = resolve_target(&config, &request).expect("resolved");
        (config, work, resolved)
    }

    fn position(calls: &[Vec<String>], step: &str) -> Vec<usize> {
        calls
            .iter()
            .enumerate()
            .filter(|(_, call)| call.iter().any(|arg| arg == step))
            .map(|(index, _)| index)
            .collect()
    }

    /// `make <EPF_SET>` собирает обработку в базе Конфигуратора, куда сперва загружена
    /// основная конфигурация проекта — без файла версий и без `/UpdateDBCfg`.
    #[test]
    fn an_external_set_is_built_on_top_of_the_configuration() {
        let dir = tempdir().expect("tempdir");
        let (config, _work, resolved) = external_project(dir.path());
        let runner = Recorder::new();

        build_external_in(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &resolved,
            fake_designer(),
            &mut SessionBases::default(),
            &runner,
        )
        .expect("built");

        let calls = runner.calls();
        assert_eq!(position(&calls, "CREATEINFOBASE"), [0], "{calls:?}");
        assert_eq!(position(&calls, "/LoadConfigFromFiles"), [1], "{calls:?}");
        assert_eq!(
            after(&calls[1], "/LoadConfigFromFiles"),
            Some(
                config
                    .base_path
                    .join("configuration")
                    .display()
                    .to_string()
                    .as_str()
            )
        );
        for flag in ["-updateConfigDumpInfo", "/UpdateDBCfg", "-Extension"] {
            assert!(!calls[1].iter().any(|arg| arg == flag), "{calls:?}");
        }
        // Загрузка основной конфигурации пишет свой журнал: журнал набора его не затирает.
        let configuration_log = after(&calls[1], "/Out").expect("configuration log");
        assert!(
            configuration_log.ends_with("-configuration.log"),
            "{configuration_log}"
        );
        let external_log = after(&calls[2], "/Out").expect("external log");
        assert_ne!(configuration_log, external_log);
        assert_eq!(
            position(&calls, "/LoadExternalDataProcessorOrReportFromFiles"),
            [2],
            "{calls:?}"
        );
        assert!(resolved.output_path.join("Tool.epf").is_file());
    }

    /// Обход Конфигуратором собирает внешние обработки в той же базе: основная конфигурация в
    /// ней уже есть, и второй раз она не загружается.
    #[test]
    fn a_designer_walk_builds_externals_in_its_own_base() {
        let dir = tempdir().expect("tempdir");
        let (config, _work, external) = external_project(dir.path());
        let runner = Recorder::new();
        let context = ExecutionContext::cli(CommandName::Artifacts);
        let mut bases = SessionBases::default();
        let cf = resolve_target(
            &config,
            &cf_request(&dir.path().join("dist/main.cf").display().to_string()),
        )
        .expect("cf");
        build_package_in(&context, &config, &cf, fake_designer(), &mut bases, &runner)
            .expect("cf built");
        build_external_in(
            &context,
            &config,
            &external,
            fake_designer(),
            &mut bases,
            &runner,
        )
        .expect("external built");

        let calls = runner.calls();
        assert_eq!(bases.bases.len(), 1);
        assert_eq!(position(&calls, "CREATEINFOBASE").len(), 1, "{calls:?}");
        assert_eq!(
            position(&calls, "/LoadConfigFromFiles").len(),
            1,
            "{calls:?}"
        );
    }

    /// Второй набор конфигурации, собранный в общей базе Конфигуратора, запоминается под своим
    /// именем: расширение после него загружает основную конфигурацию заново, а не собирается
    /// поверх чужой.
    #[test]
    fn an_extension_after_another_configuration_set_reloads_the_main_one() {
        let dir = tempdir().expect("tempdir");
        let (mut config, _work) = project(dir.path());
        fs::create_dir_all(config.base_path.join("second")).expect("second");
        config.source_sets.push(SourceSetConfig {
            name: "second".to_owned(),
            purpose: SourceSetPurpose::Configuration,
            path: PathBuf::from("second"),
        });
        let runner = Recorder::new();
        let context = ExecutionContext::cli(CommandName::Artifacts);
        let mut bases = SessionBases::default();
        let mut second = cf_request(&dir.path().join("dist/second.cf").display().to_string());
        second.source_set = Some("second".to_owned());
        let second = resolve_target(&config, &second).expect("second");
        let extension = ArtifactsRequest {
            dry_run: false,
            output_is_directory: false,
            execution: ArtifactsRequest::default_execution(ArtifactsModeRequest::ExtensionCfe),
            mode: ArtifactsModeRequest::ExtensionCfe,
            output_path: dir.path().join("dist/sales.cfe").display().to_string(),
            source_set: Some("ext-sales".to_owned()),
            extension: None,
        };
        let extension = resolve_target(&config, &extension).expect("extension");

        build_package_in(
            &context,
            &config,
            &second,
            fake_designer(),
            &mut bases,
            &runner,
        )
        .expect("second built");
        build_package_in(
            &context,
            &config,
            &extension,
            fake_designer(),
            &mut bases,
            &runner,
        )
        .expect("extension built");

        let calls = runner.calls();
        let loads = position(&calls, "/LoadConfigFromFiles");
        assert_eq!(loads.len(), 3, "{calls:?}");
        let main_root = config.base_path.join("configuration").display().to_string();
        assert!(
            calls[loads[1]].iter().any(|arg| arg == &main_root),
            "the main configuration is loaded again before the extension: {calls:?}"
        );
    }

    /// Обход, где пакеты собирал `ibcmd`, даёт внешним обработкам свою базу Конфигуратора:
    /// базу `ibcmd` Конфигуратор не открывает, а основную конфигурацию загружает в свою.
    #[test]
    fn an_ibcmd_walk_gives_externals_a_designer_base_of_their_own() {
        let dir = tempdir().expect("tempdir");
        let (config, _work, external) = external_project(dir.path());
        let runner = Recorder::new();
        let context = ExecutionContext::cli(CommandName::Artifacts);
        let mut bases = SessionBases::default();
        let cf = resolve_target(
            &config,
            &cf_request(&dir.path().join("dist/main.cf").display().to_string()),
        )
        .expect("cf");
        let ibcmd = crate::use_cases::throwaway_infobase::Builder {
            provider: crate::domain::capability::Provider::Ibcmd,
            binary: PathBuf::from("/tmp/fake-ibcmd"),
        };
        build_package_in(&context, &config, &cf, ibcmd, &mut bases, &runner).expect("cf built");
        build_external_in(
            &context,
            &config,
            &external,
            fake_designer(),
            &mut bases,
            &runner,
        )
        .expect("external built");

        let calls = runner.calls();
        assert_eq!(bases.bases.len(), 2);
        let ibcmd_base = after(&calls[0], "--db-path")
            .expect("ibcmd base")
            .to_owned();
        let designer_calls = calls
            .iter()
            .filter(|call| {
                call.iter()
                    .any(|arg| arg == "CREATEINFOBASE" || arg == "/IBConnectionString")
            })
            .collect::<Vec<_>>();
        assert!(!designer_calls.is_empty(), "{calls:?}");
        for call in designer_calls {
            assert!(
                !call.iter().any(|arg| arg.contains(&ibcmd_base)),
                "Designer opened the ibcmd base: {call:?}"
            );
        }
        let created = position(&calls, "CREATEINFOBASE");
        let loaded = position(&calls, "/LoadConfigFromFiles");
        let external_load = position(&calls, "/LoadExternalDataProcessorOrReportFromFiles");
        assert_eq!(created.len(), 1, "{calls:?}");
        assert_eq!(loaded.len(), 1, "{calls:?}");
        assert!(
            created[0] < loaded[0] && loaded[0] < external_load[0],
            "{calls:?}"
        );
    }

    /// Прогон, у которого выгрузка отказала, свою временную базу тоже убирает.
    #[cfg(unix)]
    #[test]
    fn a_failed_run_removes_its_throwaway_base() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        write_script(
            &script,
            "case \" $* \" in *' /DumpCfg '*) exit 12 ;; esac\nexit 0",
        );
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        fs::create_dir_all(base.join("configuration")).expect("base config");
        fs::create_dir_all(&work).expect("work");
        let config = sample_config(&base, &work, &script, SourceFormat::Designer);
        let request = cf_request(&dir.path().join("dist/release.cf").display().to_string());

        super::execute(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &request,
        )
        .expect_err("the dump failed");

        assert!(bases_left(&work).is_empty(), "{:?}", bases_left(&work));
    }

    /// Превью проекта EDT ищет `1cedtcli`: без него превью отказывает, как отказал бы прогон.
    #[cfg(unix)]
    #[test]
    fn an_edt_preview_without_the_edt_cli_is_refused() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        write_script(&script, "exit 0");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        fs::create_dir_all(base.join("configuration")).expect("base config");
        fs::create_dir_all(&work).expect("work");
        let mut config = sample_config(&base, &work, &script, SourceFormat::Edt);
        config.tools.edt_cli = crate::config::model::EdtCliConfig {
            path: Some(dir.path().join("missing-1cedtcli")),
            auto_start: false,
            ..Default::default()
        };
        let mut request = cf_request(&dir.path().join("dist/release.cf").display().to_string());
        request.dry_run = true;

        let failure = run_artifacts(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &request,
            &mut MakeSession::new(&config),
        )
        .expect_err("no EDT CLI");

        let payload = failure.payload.expect("payload");
        assert!(!payload.execution.is_ok());
        assert!(!dir.path().join("dist").exists());
    }

    /// Ключ `providers.make` внешних наборов не касается: их собирает Конфигуратор, и
    /// квитанция называет его умолчанием и при назначенном `ibcmd`.
    #[cfg(unix)]
    #[test]
    fn the_make_key_does_not_apply_to_an_external_set() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        write_script(&script, "exit 0");
        let (mut config, _work, _) = external_project(dir.path());
        config.tools.platform.path = Some(script);
        config.providers = [(
            crate::domain::capability::Operation::Make,
            crate::domain::capability::Provider::Ibcmd,
        )]
        .into();
        let mut request = external_request(
            ArtifactsModeRequest::ExternalDataProcessorEpf,
            &dir.path().join("dist/tools").display().to_string(),
            "tools",
        );
        request.dry_run = true;

        let result = run_artifacts(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &request,
            &mut MakeSession::new(&config),
        )
        .expect("preview");

        let receipt = result.provider.expect("receipt");
        assert_eq!(
            receipt.selected,
            Some(crate::domain::capability::Provider::Designer)
        );
        assert_eq!(
            receipt.origin,
            crate::domain::capability::ProviderOrigin::Default
        );
    }

    /// Перевод набора EDT в XML делается один раз за прогон: база `ibcmd` и база
    /// Конфигуратора берут один и тот же каталог.
    #[cfg(unix)]
    #[test]
    fn an_edt_set_is_converted_once_per_run() {
        let dir = tempdir().expect("tempdir");
        let (mut config, _work) = project(dir.path());
        config.format = SourceFormat::Edt;
        fs::write(
            config.base_path.join("configuration/.project"),
            "<projectDescription><name>main-project</name></projectDescription>",
        )
        .expect("project");
        let edt = dir.path().join("1cedtcli");
        let edt_calls = dir.path().join("edt-calls");
        write_script(
            &edt,
            &format!(
                "target=''\nprev=''\nfor arg in \"$@\"; do\n  if [ \"$prev\" = '--configuration-files' ]; then target=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nmkdir -p \"$target\"\nprintf '<Configuration />' > \"$target/Configuration.xml\"\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0",
                edt_calls.display()
            ),
        );
        config.tools.edt_cli = crate::config::model::EdtCliConfig {
            path: Some(edt),
            auto_start: false,
            ..Default::default()
        };
        let context = ExecutionContext::cli(CommandName::Artifacts);
        let runner = Recorder::new();
        let mut bases = SessionBases::default();
        let configuration = config.source_sets[0].clone();
        let ibcmd = crate::use_cases::throwaway_infobase::Builder {
            provider: crate::domain::capability::Provider::Ibcmd,
            binary: PathBuf::from("/tmp/fake-ibcmd"),
        };

        let (ibcmd_base, xml) =
            super::session_base(&context, &config, &mut bases, ibcmd, &runner).expect("ibcmd");
        let first = super::sources_in_xml(&context, &config, xml, ibcmd_base, &configuration)
            .expect("converted");
        let (designer_base, xml) =
            super::session_base(&context, &config, &mut bases, fake_designer(), &runner)
                .expect("designer");
        let second = super::sources_in_xml(&context, &config, xml, designer_base, &configuration)
            .expect("reused");

        assert_eq!(first, second);
        let edt_calls = fs::read_to_string(&edt_calls).expect("edt calls");
        assert_eq!(edt_calls.lines().count(), 1, "{edt_calls}");
    }

    /// `make <SET>` убирает свою временную базу и после успеха.
    #[cfg(unix)]
    #[test]
    fn execute_removes_its_throwaway_base() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("1cv8");
        write_script(
            &script,
            "prev=''\nfor arg in \"$@\"; do\n  if [ \"$prev\" = '/DumpCfg' ]; then printf 'cf' > \"$arg\"; fi\n  prev=\"$arg\"\ndone\nexit 0",
        );
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        fs::create_dir_all(base.join("configuration")).expect("base config");
        fs::create_dir_all(&work).expect("work");
        let config = sample_config(&base, &work, &script, SourceFormat::Designer);
        let request = cf_request(&dir.path().join("dist/release.cf").display().to_string());

        let result = super::execute(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &request,
        )
        .expect("built");

        assert!(artifacts_payload(&result).output_path.is_file());
        assert!(bases_left(&work).is_empty(), "{:?}", bases_left(&work));
    }

    #[test]
    fn publication_warning_reports_an_interrupted_context() {
        let warning = publication_warning(
            CommandName::Artifacts,
            crate::use_cases::context::ExecutionInterruption::Cancelled,
        );

        assert!(warning.contains("cancel"));
        assert!(warning.contains("critical phase"));
    }

    #[test]
    fn publication_message_keeps_cleanup_warning_in_result_contract() {
        let context = ExecutionContext::cli(CommandName::Artifacts);
        let publication = publication_message(
            &context,
            StagedPublicationOutcome {
                cleanup_warning: Some("cleanup warning".to_owned()),
                discarded: Default::default(),
                deferred_interruption: Some(
                    crate::use_cases::context::ExecutionInterruption::Cancelled,
                ),
                previous_target_present: true,
            },
        );
        let metadata = ArtifactBuildMetadata {
            artifact_type: ArtifactBuildMode::ConfigurationCf,
            output_path: PathBuf::from("dist/main.cf"),
            file_names: vec!["main.cf".to_owned()],
            published: true,
        };

        let execution =
            published_execution(&context, ArtifactSet::default(), metadata, publication);

        // Предупреждение уборки доходит до ответа, и успех чистым не выглядит.
        assert_eq!(execution.status, ExecutionStatus::Succeeded);
        let [message] = execution.diagnostics.as_slice() else {
            panic!("one diagnostic expected: {:?}", execution.diagnostics);
        };
        assert!(message.contains("cleanup warning"), "{message}");
        assert!(message.contains("cancellation request"), "{message}");
        // The fact travels beside the text, so nobody has to read the text to recover it.
        let [interruption] = execution.interruptions.as_slice() else {
            panic!("one interruption expected: {:?}", execution.interruptions);
        };
        assert!(interruption.deferred);
        assert_eq!(
            interruption.phase,
            Some(ExecutionInterruptionPhase::Publication)
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_artifacts_exports_external_processors_and_records_all_published_files() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("base/external-processors")).expect("external dir");
        fs::write(
            dir.path().join("base/external-processors/alpha.xml"),
            "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
        )
        .expect("alpha descriptor");
        fs::write(
            dir.path().join("base/external-processors/beta.xml"),
            "<ExternalDataProcessor><Properties><Name>Beta</Name></Properties></ExternalDataProcessor>",
        )
        .expect("beta descriptor");
        let script = dir.path().join("1cv8");
        write_script(
            &script,
            "out=''\nprev=''\nload_state=0\ndump_state=0\nname=''\nfor arg in \"$@\"; do\n  if [ \"$prev\" = '/LoadExternalDataProcessorOrReportFromFiles' ]; then load_state=1; prev=\"$arg\"; continue; fi\n  if [ \"$load_state\" = 1 ]; then printf 'external' > \"$arg\"; load_state=0; fi\n  if [ \"$prev\" = '/DumpExternalDataProcessorOrReportToFiles' ]; then dump_state=1; fi\n  if [ \"$dump_state\" = 1 ]; then case \"$arg\" in *Alpha.xml) name='Alpha' ;; *Beta.xml) name='Beta' ;; *) name='Unknown' ;; esac; printf '<ExternalDataProcessor><Properties><Name>%s</Name></Properties></ExternalDataProcessor>' \"$name\" > \"$arg\"; dump_state=0; fi\n  if [ \"$prev\" = '/Out' ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf 'designer log' > \"$out\"; fi\nexit 0",
        );
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        fs::create_dir_all(&base).expect("base dir");
        fs::create_dir_all(&work).expect("work");
        let mut config = sample_config(&base, &work, &script, SourceFormat::Designer);
        add_external_source_set(
            &mut config,
            "external-processors",
            SourceSetPurpose::ExternalDataProcessors,
        );
        let request = external_request(
            ArtifactsModeRequest::ExternalDataProcessorEpf,
            &dir.path().join("dist/external").display().to_string(),
            "external-processors",
        );

        let result = run_artifacts(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &request,
            &mut MakeSession::new(&config),
        )
        .expect("result");
        let mut file_names = result
            .execution
            .payload
            .as_ref()
            .expect("payload")
            .file_names
            .clone();
        file_names.sort();

        assert!(result.execution.is_ok());
        assert!(artifacts_payload(&result).output_path.is_dir());
        assert_eq!(
            file_names,
            vec!["Alpha.epf".to_owned(), "Beta.epf".to_owned()]
        );
        assert!(artifacts_set(&result)
            .get_by_role(ARTIFACT_ROLE_PLATFORM_LOG)
            .is_some());
        assert!(artifacts_payload(&result)
            .output_path
            .join("Alpha.epf")
            .is_file());
        assert!(artifacts_payload(&result)
            .output_path
            .join("Beta.epf")
            .is_file());
    }

    #[cfg(unix)]
    #[test]
    fn run_artifacts_replaces_stale_external_packages_atomically() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("base/external-processors")).expect("external dir");
        fs::write(
            dir.path().join("base/external-processors/alpha.xml"),
            "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
        )
        .expect("alpha descriptor");
        let script = dir.path().join("1cv8");
        write_script(
            &script,
            "out=''\nprev=''\nload_state=0\ndump_state=0\nname=''\nfor arg in \"$@\"; do\n  if [ \"$prev\" = '/LoadExternalDataProcessorOrReportFromFiles' ]; then load_state=1; prev=\"$arg\"; continue; fi\n  if [ \"$load_state\" = 1 ]; then printf 'external' > \"$arg\"; load_state=0; fi\n  if [ \"$prev\" = '/DumpExternalDataProcessorOrReportToFiles' ]; then dump_state=1; fi\n  if [ \"$dump_state\" = 1 ]; then case \"$arg\" in *Alpha.xml) name='Alpha' ;; *) name='Unknown' ;; esac; printf '<ExternalDataProcessor><Properties><Name>%s</Name></Properties></ExternalDataProcessor>' \"$name\" > \"$arg\"; dump_state=0; fi\n  if [ \"$prev\" = '/Out' ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf 'designer log' > \"$out\"; fi\nexit 0",
        );
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        fs::create_dir_all(&base).expect("base dir");
        fs::create_dir_all(&work).expect("work");
        let output = dir.path().join("dist/external");
        fs::create_dir_all(&output).expect("output");
        fs::write(output.join("stale.epf"), "stale").expect("stale file");
        let mut config = sample_config(&base, &work, &script, SourceFormat::Designer);
        add_external_source_set(
            &mut config,
            "external-processors",
            SourceSetPurpose::ExternalDataProcessors,
        );
        let request = external_request(
            ArtifactsModeRequest::ExternalDataProcessorEpf,
            &output.display().to_string(),
            "external-processors",
        );

        let result = run_artifacts(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &request,
            &mut MakeSession::new(&config),
        )
        .expect("result");
        let mut file_names = result
            .execution
            .payload
            .as_ref()
            .expect("payload")
            .file_names
            .clone();
        file_names.sort();

        assert!(result.execution.is_ok());
        assert_eq!(file_names, vec!["Alpha.epf".to_owned()]);
        assert!(!output.join("stale.epf").exists());
        assert!(output.join("Alpha.epf").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn run_artifacts_keeps_existing_target_when_mid_batch_publish_fails() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("base/external-processors")).expect("external dir");
        fs::write(
            dir.path().join("base/external-processors/alpha.xml"),
            "<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
        )
        .expect("alpha descriptor");
        fs::write(
            dir.path().join("base/external-processors/beta.xml"),
            "<ExternalDataProcessor><Properties><Name>Beta</Name></Properties></ExternalDataProcessor>",
        )
        .expect("beta descriptor");
        let script = dir.path().join("1cv8");
        write_script(
            &script,
            "out=''\nload_state=0\nfor arg in \"$@\"; do\n  if [ \"$prev\" = '/LoadExternalDataProcessorOrReportFromFiles' ]; then load_state=1; prev=\"$arg\"; continue; fi\n  if [ \"$load_state\" = 1 ]; then case \"$arg\" in *Beta.epf) printf 'boom' > \"$arg\"; exit 12 ;; *) printf 'external' > \"$arg\" ;; esac; load_state=0; fi\n  if [ \"$prev\" = '/Out' ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf 'platform fail' > \"$out\"; fi\nexit 0",
        );
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        fs::create_dir_all(&base).expect("base dir");
        fs::create_dir_all(&work).expect("work");
        let output = dir.path().join("dist/external");
        fs::create_dir_all(&output).expect("output");
        fs::write(output.join("stale.epf"), "stale").expect("stale file");
        let mut config = sample_config(&base, &work, &script, SourceFormat::Designer);
        add_external_source_set(
            &mut config,
            "external-processors",
            SourceSetPurpose::ExternalDataProcessors,
        );
        let request = external_request(
            ArtifactsModeRequest::ExternalDataProcessorEpf,
            &output.display().to_string(),
            "external-processors",
        );

        let failure = run_artifacts(
            &ExecutionContext::cli(CommandName::Artifacts),
            &config,
            &request,
            &mut MakeSession::new(&config),
        )
        .expect_err("failure");
        let payload = failure.payload.expect("payload");

        assert_eq!(
            fs::read_to_string(output.join("stale.epf")).expect("stale"),
            "stale"
        );
        assert!(!output.join("Alpha.epf").exists());
        assert!(!output.join("Beta.epf").exists());
        assert!(!payload.execution.is_ok());
        assert!(!payload.execution.errors[0].message.is_empty());
    }

    /// Каталог внешних обработок `output` в `dir` — цель, которую уборка сверяет со следами.
    fn external_output_target(dir: &Path, output: &Path) -> ResolvedArtifactsTarget {
        ResolvedArtifactsTarget {
            mode: ArtifactBuildMode::ExternalDataProcessorEpf,
            source_set_name: "external".to_owned(),
            extension: None,
            output_path: output.to_path_buf(),
            source_path: dir.join("external"),
            is_directory_output: true,
            canonical_output_path: output.to_path_buf(),
            canonical_base_path: dir.to_path_buf(),
            canonical_work_path: dir.to_path_buf(),
            target_identity: "identity".to_owned(),
            lock_path: dir.join("lock"),
        }
    }

    #[test]
    fn cleanup_orphan_files_ignores_malformed_metadata() {
        let dir = tempdir().expect("tempdir");
        let output = dir.path().join("dist/external");
        fs::create_dir_all(&output).expect("output");
        let backup_dir = output
            .parent()
            .expect("parent")
            .join(".artifacts-backup-run-1");
        fs::create_dir_all(&backup_dir).expect("backup");
        let meta_path = metadata_sidecar_path(&backup_dir);
        fs::write(&meta_path, b"not json").expect("metadata");
        let resolved = external_output_target(dir.path(), &output);

        cleanup_orphan_files(&resolved).expect("cleanup");

        assert!(backup_dir.exists());
        assert!(meta_path.exists());
    }

    #[test]
    fn cleanup_orphan_files_scans_directory_output_root() {
        let dir = tempdir().expect("tempdir");
        let output = dir.path().join("dist/external");
        fs::create_dir_all(&output).expect("output");
        let stale = output.join(".artifacts-stage-run-1.epf");
        fs::create_dir_all(&stale).expect("stale");
        write_temp_dir_metadata(
            &stale,
            TempDirKind::Stage,
            "run-1",
            &output.join("published.epf"),
            "identity",
        )
        .expect("metadata");
        let mut metadata = read_temp_dir_metadata(&stale).expect("read metadata");
        metadata.created_at -= chrono::Duration::days(2);
        fs::write(
            metadata_sidecar_path(&stale),
            serde_json::to_vec_pretty(&metadata).expect("json"),
        )
        .expect("rewrite metadata");
        let resolved = external_output_target(dir.path(), &output);

        cleanup_orphan_files(&resolved).expect("cleanup");

        assert!(!stale.exists());
    }

    #[test]
    fn cleanup_orphan_files_removes_old_stage_directory_cleanup_unit() {
        let dir = tempdir().expect("tempdir");
        let output = dir.path().join("dist/external");
        fs::create_dir_all(&output).expect("output");
        let stage_dir = output
            .parent()
            .expect("parent")
            .join(".artifacts-stage-run-1");
        fs::create_dir_all(&stage_dir).expect("stage");
        write_temp_dir_metadata(&stage_dir, TempDirKind::Stage, "run-1", &output, "identity")
            .expect("metadata");
        let meta_path = metadata_sidecar_path(&stage_dir);
        let mut metadata = read_temp_dir_metadata(&stage_dir).expect("read metadata");
        metadata.created_at -= chrono::Duration::days(2);
        fs::write(
            &meta_path,
            serde_json::to_vec_pretty(&metadata).expect("json"),
        )
        .expect("rewrite metadata");
        let resolved = external_output_target(dir.path(), &output);

        cleanup_orphan_files(&resolved).expect("cleanup");

        assert!(!stage_dir.exists());
        assert!(!meta_path.exists());
    }

    #[test]
    fn cleanup_orphan_files_removes_old_stage_metadata_sidecar_without_stage_file() {
        let dir = tempdir().expect("tempdir");
        let output = dir.path().join("dist/external");
        fs::create_dir_all(&output).expect("output");
        let stage_file = output
            .parent()
            .expect("parent")
            .join(".artifacts-stage-run-1.cf");
        write_temp_dir_metadata(
            &stage_file,
            TempDirKind::Stage,
            "run-1",
            &output,
            "identity",
        )
        .expect("metadata");
        let meta_path = metadata_sidecar_path(&stage_file);
        let mut metadata = read_temp_dir_metadata(&stage_file).expect("read metadata");
        metadata.created_at -= chrono::Duration::days(2);
        fs::write(
            &meta_path,
            serde_json::to_vec_pretty(&metadata).expect("json"),
        )
        .expect("rewrite metadata");
        let resolved = external_output_target(dir.path(), &output);

        cleanup_orphan_files(&resolved).expect("cleanup");

        assert!(!stage_file.exists());
        assert!(!meta_path.exists());
    }

    #[test]
    fn cleanup_orphan_files_removes_old_backup_directory_cleanup_unit() {
        let dir = tempdir().expect("tempdir");
        let output = dir.path().join("dist/external");
        fs::create_dir_all(&output).expect("output");
        let backup_dir = output
            .parent()
            .expect("parent")
            .join(".artifacts-backup-run-1");
        fs::create_dir_all(&backup_dir).expect("backup");
        write_temp_dir_metadata(
            &backup_dir,
            TempDirKind::Backup,
            "run-1",
            &output,
            "identity",
        )
        .expect("metadata");
        let meta_path = metadata_sidecar_path(&backup_dir);
        let mut metadata = read_temp_dir_metadata(&backup_dir).expect("read metadata");
        metadata.created_at -= chrono::Duration::days(2);
        fs::write(
            &meta_path,
            serde_json::to_vec_pretty(&metadata).expect("json"),
        )
        .expect("rewrite metadata");
        let resolved = external_output_target(dir.path(), &output);

        cleanup_orphan_files(&resolved).expect("cleanup");

        assert!(!backup_dir.exists());
        assert!(!meta_path.exists());
    }

    #[test]
    fn cleanup_orphan_files_ignores_recent_metadata() {
        let dir = tempdir().expect("tempdir");
        let output = dir.path().join("dist/external");
        fs::create_dir_all(&output).expect("output");
        let recent = output
            .parent()
            .expect("parent")
            .join(".artifacts-stage-run-1");
        fs::create_dir_all(&recent).expect("stage");
        write_temp_dir_metadata(&recent, TempDirKind::Stage, "run-1", &output, "identity")
            .expect("metadata");
        let resolved = external_output_target(dir.path(), &output);

        cleanup_orphan_files(&resolved).expect("cleanup");

        assert!(recent.exists());
        assert!(metadata_sidecar_path(&recent).exists());
    }

    #[test]
    fn cleanup_orphan_files_ignores_foreign_metadata() {
        let dir = tempdir().expect("tempdir");
        let output = dir.path().join("dist/external");
        fs::create_dir_all(&output).expect("output");
        let foreign = output
            .parent()
            .expect("parent")
            .join(".artifacts-backup-run-1");
        fs::create_dir_all(&foreign).expect("backup");
        write_temp_dir_metadata(&foreign, TempDirKind::Backup, "run-1", &output, "identity")
            .expect("metadata");
        let meta_path = metadata_sidecar_path(&foreign);
        let mut metadata = read_temp_dir_metadata(&foreign).expect("read metadata");
        metadata.tool = "foreign-tool".to_owned();
        metadata.created_at -= chrono::Duration::days(2);
        fs::write(
            &meta_path,
            serde_json::to_vec_pretty(&metadata).expect("json"),
        )
        .expect("rewrite metadata");
        let resolved = external_output_target(dir.path(), &output);

        cleanup_orphan_files(&resolved).expect("cleanup");

        assert!(foreign.exists());
        assert!(meta_path.exists());
    }
}
