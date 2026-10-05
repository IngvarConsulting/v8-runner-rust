use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use tracing::debug;

use crate::config::model::{AppConfig, SourceFormat, SourceSetConfig, SourceSetPurpose};
use crate::domain::capability::{Operation, Provider};
use crate::domain::init::{InitResult, InitStep, InitStepStatus};
use crate::platform::designer::DesignerDsl;
use crate::platform::edt::EdtDsl;
use crate::platform::edt_session::{EdtSessionHostOptions, EdtSessionManager};
use crate::platform::ibcmd::{
    IbcmdConnection, IbcmdDsl, IbcmdInfobaseCreateOutcome, IbcmdInfobaseCreateStatus,
};
use crate::platform::locator::UtilityType;
use crate::platform::process::ProcessError;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::ibcmd_diagnostics::format_failure_evidence;
use crate::use_cases::interruption::{self, append_warnings, collecting_deferrals};
use crate::use_cases::progress::{log_live_stage, log_live_stage_status, LiveStageStatus};
use crate::use_cases::request::InitRequest;
use crate::use_cases::result::{stamp_dispatch, UseCaseError, UseCaseFailure, UseCaseResult};
use crate::use_cases::tool_extension;

pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &InitRequest,
) -> UseCaseResult<InitResult> {
    debug!(
        command = context.command().as_str(),
        transport = ?context.transport(),
        "executing init use case"
    );
    stamp_dispatch(run_init(context, config, args.dry_run), context.work())
}

pub(crate) type InitExecutionFailure = UseCaseFailure<InitResult>;
const EDT_WORKSPACE_MARKER: &str = ".v8tr-initialized";
/// Действие, которым учёт называет отмену, отложенную созданием базы.
const INFOBASE_CREATE: &str = "infobase create";

fn run_init(
    context: &ExecutionContext,
    config: &AppConfig,
    dry_run: bool,
) -> UseCaseResult<InitResult> {
    let started = Instant::now();
    let mut utilities = PlatformUtilities::from_config(config);
    // Исполнитель нужен только шагу создания базы, и тот сам сообщает об отсутствии
    // утилиты своим статусом: отказ выбора здесь не прерывает команду — у серверного
    // подключения и у чисто EDT-проекта этот шаг может и не понадобиться. Квитанция
    // при этом остаётся честной: никто не готов, пропущенные названы.
    let (provider, receipt) =
        match crate::use_cases::provider_selection::select(config, &mut utilities, Operation::Init)
        {
            Ok(selected) => (selected.provider, selected.receipt),
            // Без строки в матрице (автономный сервер) исполнителя нет вовсе; шаг
            // создания базы назовёт это сам.
            Err((_error, receipt)) => (
                config
                    .default_provider(Operation::Init)
                    .unwrap_or(crate::domain::capability::Provider::Designer),
                receipt,
            ),
        };
    let mut steps = Vec::new();
    let mut first_error: Option<UseCaseError> = None;

    record_step(
        &mut steps,
        &mut first_error,
        ensure_infobase(context, config, &mut utilities, provider, dry_run),
    );
    record_step(
        &mut steps,
        &mut first_error,
        ensure_edt_workspace(context, config, &mut utilities, dry_run),
    );

    let mut result = init_result(started, steps, first_error.is_none());
    if dry_run {
        // Строка о ходе остаётся в выводе, хотя ни база, ни рабочее пространство не
        // тронуты: запись о вызове несёт конверт, журнала превью не ведёт.
        log_live_stage("init: preview", "[Init] preview only, nothing created");
    }
    result.provider = Some(receipt);

    match first_error {
        Some(error) => Err(InitExecutionFailure::with_payload(error, result)),
        None => Ok(result),
    }
}

fn init_result(started: Instant, steps: Vec<InitStep>, ok: bool) -> InitResult {
    InitResult {
        provider: None,
        ok,
        provider_dispatched: false,
        steps,
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

fn record_step(
    steps: &mut Vec<InitStep>,
    first_error: &mut Option<UseCaseError>,
    outcome: StepOutcome,
) {
    if first_error.is_none() {
        *first_error = outcome.error.clone();
    }
    log_step_status(&outcome.step);
    steps.push(outcome.step);
}

fn log_step_status(step: &InitStep) {
    let Some(label) = live_step_label(step) else {
        return;
    };
    let Some((status, marker)) = live_status_marker(&step.status) else {
        return;
    };
    log_live_stage_status(
        label,
        status,
        &format!("{marker} {}: {}", step.target, step.action),
    );
}

fn live_step_label(step: &InitStep) -> Option<&'static str> {
    match (step.target.as_str(), step.action.as_str()) {
        ("infobase", "create") => Some("init: infobase create"),
        ("edt_workspace", "import") if !matches!(step.status, InitStepStatus::Skipped) => {
            Some("init: edt import")
        }
        _ => None,
    }
}

fn live_status_marker(status: &InitStepStatus) -> Option<(LiveStageStatus, &'static str)> {
    match status {
        InitStepStatus::Ok => Some((LiveStageStatus::Succeeded, "✓")),
        InitStepStatus::Failed => Some((LiveStageStatus::Failed, "✗")),
        InitStepStatus::Skipped | InitStepStatus::Planned => None,
    }
}

#[derive(Debug, Clone)]
struct StepOutcome {
    step: InitStep,
    error: Option<UseCaseError>,
}

impl StepOutcome {
    fn ok(target: &str, action: &str, started: Instant, message: impl Into<String>) -> Self {
        Self {
            step: InitStep {
                target: target.to_owned(),
                action: action.to_owned(),
                status: InitStepStatus::Ok,
                message: Some(message.into()),
                duration_ms: started.elapsed().as_millis() as u64,
            },
            error: None,
        }
    }

    fn skipped(target: &str, action: &str, started: Instant, message: impl Into<String>) -> Self {
        Self {
            step: InitStep {
                target: target.to_owned(),
                action: action.to_owned(),
                status: InitStepStatus::Skipped,
                message: Some(message.into()),
                duration_ms: started.elapsed().as_millis() as u64,
            },
            error: None,
        }
    }

    /// Сообщение шага, за которым идут предупреждения об отложенных отменах.
    fn with_warnings(mut self, warnings: &[String]) -> Self {
        if !warnings.is_empty() {
            let message = self.step.message.take().unwrap_or_default();
            self.step.message = Some(append_warnings(message, warnings));
        }
        self
    }

    /// Records a step that was decided but deliberately not performed.
    fn planned(target: &str, action: &str, started: Instant, message: impl Into<String>) -> Self {
        Self {
            step: InitStep {
                target: target.to_owned(),
                action: action.to_owned(),
                status: InitStepStatus::Planned,
                message: Some(message.into()),
                duration_ms: started.elapsed().as_millis() as u64,
            },
            error: None,
        }
    }

    fn failed(
        target: &str,
        action: &str,
        started: Instant,
        error: impl Into<UseCaseError>,
    ) -> Self {
        let error = error.into();
        Self {
            step: InitStep {
                target: target.to_owned(),
                action: action.to_owned(),
                status: InitStepStatus::Failed,
                message: Some(error.message().to_owned()),
                duration_ms: started.elapsed().as_millis() as u64,
            },
            error: Some(error),
        }
    }
}

fn ensure_infobase(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    provider: Provider,
    dry_run: bool,
) -> StepOutcome {
    // Автономный сервер поднимает человек: раннер его не создаёт и не запускает.
    if config.infobase.standalone.is_some() {
        return StepOutcome::skipped(
            "infobase",
            "create",
            Instant::now(),
            "a standalone server is started by hand and is never created by the runner: infobase.standalone names an existing gate".to_owned(),
        );
    }
    let Some(infobase_dir) = config.v8_connection().file_path().map(PathBuf::from) else {
        return match provider {
            Provider::Ibcmd => {
                ensure_server_infobase(context, config, utilities, provider, dry_run)
            }
            // Конфигуратор серверную базу не создаёт: шаг пропускается, как и раньше,
            // а выбрать ibcmd можно ключом providers.init.
            other => StepOutcome::skipped(
                "infobase",
                "create",
                Instant::now(),
                format!(
                    "server infobase connection detected; automatic creation is not supported by the {other} provider, set providers.init: ibcmd"
                ),
            ),
        };
    };

    ensure_file_infobase(context, config, utilities, provider, &infobase_dir, dry_run)
}

fn ensure_file_infobase(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    provider: Provider,
    infobase_dir: &Path,
    dry_run: bool,
) -> StepOutcome {
    let started = Instant::now();
    let marker = infobase_marker_path(infobase_dir);
    debug!("[Инфобаза] Подготовка: {}", infobase_dir.display());
    if marker.exists() {
        return StepOutcome::skipped(
            "infobase",
            "create",
            started,
            format!("infobase already exists: {}", marker.display()),
        );
    }

    if dry_run {
        // The platform is located here so an absent one refuses during the preview; the
        // parent directory below is the first thing this step would create.
        return match locate_infobase_creator(provider, utilities) {
            Ok(binary) => StepOutcome::planned(
                "infobase",
                "create",
                started,
                format!(
                    "would create a file infobase at '{}' via {}",
                    infobase_dir.display(),
                    binary.display()
                ),
            ),
            Err(error) => StepOutcome::failed("infobase", "create", started, error),
        };
    }

    if let Err(error) = prepare_infobase_parent(infobase_dir) {
        return StepOutcome::failed("infobase", "create", started, error);
    }

    if let Some(outcome) =
        interruption_step_outcome(context, "infobase", "create", started, "infobase create")
    {
        return outcome;
    }

    log_live_stage("init: infobase create", "[Platform] creating infobase");
    infobase_create_step(
        context,
        config,
        utilities,
        provider,
        started,
        |created| match created.status {
            IbcmdInfobaseCreateStatus::Created if marker.exists() => Ok(StepOutcome::ok(
                "infobase",
                "create",
                started,
                format!("infobase created: {}", marker.display()),
            )),
            IbcmdInfobaseCreateStatus::Created => Err(missing_infobase_marker_error(
                "infobase creation did not produce marker file",
                &marker,
                &created.result,
            )),
            IbcmdInfobaseCreateStatus::AlreadyExists if marker.exists() => {
                Ok(StepOutcome::skipped(
                    "infobase",
                    "create",
                    started,
                    format!("infobase already exists: {}", marker.display()),
                ))
            }
            IbcmdInfobaseCreateStatus::AlreadyExists => Err(missing_infobase_marker_error(
                "infobase create reported an existing file infobase but marker file is missing",
                &marker,
                &created.result,
            )),
            IbcmdInfobaseCreateStatus::Failed => Err(failed_create(&created.result)),
            IbcmdInfobaseCreateStatus::Unconfirmed(error) => {
                Err(unconfirmed_create(error, &created.result))
            }
        },
    )
}

fn ensure_server_infobase(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    provider: Provider,
    dry_run: bool,
) -> StepOutcome {
    let started = Instant::now();
    // Строка подключения из конфигурации бывает с `Usr=`/`Pwd=`: базу называет
    // `describe_target`, а не строка как есть (INV.CLI.SECRETS-NEVER-REACH-THE-OUTPUT).
    let target = config.v8_connection().describe_target();
    if dry_run {
        // A server infobase cannot be observed without acting: `ibcmd infobase create`
        // is what distinguishes created from already-present. The preview therefore names
        // the target and the binary and stops short of that distinction.
        return match locate_infobase_creator(provider, utilities) {
            Ok(binary) => StepOutcome::planned(
                "infobase",
                "create",
                started,
                format!(
                    "would ensure {target} via {binary}; whether it already exists is not observable without creating it",
                    binary = binary.display()
                ),
            ),
            Err(error) => StepOutcome::failed("infobase", "create", started, error),
        };
    }
    if let Some(outcome) =
        interruption_step_outcome(context, "infobase", "create", started, "infobase create")
    {
        return outcome;
    }
    log_live_stage("init: infobase create", "[ibcmd] ensuring server infobase");
    infobase_create_step(
        context,
        config,
        utilities,
        provider,
        started,
        |created| match created.status {
            IbcmdInfobaseCreateStatus::Created => Ok(StepOutcome::ok(
                "infobase",
                "create",
                started,
                format!("{target} ensured via ibcmd"),
            )),
            IbcmdInfobaseCreateStatus::AlreadyExists => Ok(StepOutcome::skipped(
                "infobase",
                "create",
                started,
                format!("{target} already exists"),
            )),
            IbcmdInfobaseCreateStatus::Failed => Err(failed_create(&created.result)),
            IbcmdInfobaseCreateStatus::Unconfirmed(error) => {
                Err(unconfirmed_create(error, &created.result))
            }
        },
    )
}

/// Шаг создания базы. Создание и учёт отмены, которую оно отложило, у любой базы идут
/// здесь; `settle` решает, что исход создания значит для этой базы. Удача несёт отложенную
/// отмену в сообщении шага, отказ открывает ею свой текст.
fn infobase_create_step(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    provider: Provider,
    started: Instant,
    settle: impl FnOnce(IbcmdInfobaseCreateOutcome) -> Result<StepOutcome, AppError>,
) -> StepOutcome {
    let settled = collecting_deferrals(|deferrals| {
        let created = create_infobase(context, config, utilities, provider)?;
        deferrals.note_result(INFOBASE_CREATE, &created.result);
        settle(created)
    });
    match settled {
        Ok((step, warnings)) => step.with_warnings(&warnings),
        Err(error) => StepOutcome::failed("infobase", "create", started, error),
    }
}

fn ensure_edt_workspace(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    dry_run: bool,
) -> StepOutcome {
    let started = Instant::now();
    let tool_extension_path = tool_extension::client_mcp_edt_source_path(config);
    if config.format != SourceFormat::Edt && tool_extension_path.is_none() {
        return StepOutcome::skipped(
            "edt_workspace",
            "import",
            started,
            "EDT workspace initialization is not applicable for format=DESIGNER",
        );
    }

    let workspace = config.work_path.join("edt-workspace");
    let marker = edt_workspace_marker_path(&workspace);
    let project_source_import = if config.format == SourceFormat::Edt && !marker.exists() {
        ProjectSourceImport::Include
    } else {
        ProjectSourceImport::Skip
    };
    let projects = edt_import_projects(config, project_source_import, tool_extension_path);
    if workspace.exists() && !workspace.is_dir() {
        return StepOutcome::failed(
            "edt_workspace",
            "import",
            started,
            AppError::Runtime(format!(
                "EDT workspace path exists but is not a directory: {}",
                workspace.display()
            )),
        );
    }
    if workspace.exists() && marker.exists() && projects.is_empty() {
        return StepOutcome::skipped(
            "edt_workspace",
            "import",
            started,
            format!("workspace already initialized: {}", workspace.display()),
        );
    }

    if dry_run {
        return match utilities.locate(UtilityType::EdtCli) {
            Ok(location) => StepOutcome::planned(
                "edt_workspace",
                "import",
                started,
                format!(
                    "would import {} project(s) into '{}' via {}",
                    projects.len(),
                    workspace.display(),
                    location.path.display()
                ),
            ),
            Err(error) => {
                StepOutcome::failed("edt_workspace", "import", started, AppError::from(error))
            }
        };
    }

    if let Err(error) = std::fs::create_dir_all(&workspace) {
        return StepOutcome::failed(
            "edt_workspace",
            "import",
            started,
            AppError::Runtime(format!(
                "failed to create EDT workspace '{}': {error}",
                workspace.display()
            )),
        );
    }

    if let Some(outcome) = interruption_step_outcome(
        context,
        "edt_workspace",
        "import",
        started,
        "EDT workspace import",
    ) {
        return outcome;
    }

    let binary = match utilities.locate(UtilityType::EdtCli) {
        Ok(location) => location.path,
        Err(error) => {
            return StepOutcome::failed("edt_workspace", "import", started, AppError::from(error))
        }
    };

    let dsl = if config.tools.edt_cli.interactive_mode {
        match EdtSessionManager::for_config(config, EdtSessionHostOptions::for_cli_command(config))
        {
            Ok(manager) => match EdtDsl::new_shared_session(
                binary,
                workspace.clone(),
                Arc::new(manager),
                Duration::from_millis(config.tools.edt_cli.startup_timeout_ms),
                Duration::from_millis(config.tools.edt_cli.command_timeout_ms),
                context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
            ) {
                Ok(dsl) => dsl,
                Err(error) => {
                    return StepOutcome::failed(
                        "edt_workspace",
                        "import",
                        started,
                        AppError::from(error),
                    )
                }
            },
            Err(error) => {
                return StepOutcome::failed(
                    "edt_workspace",
                    "import",
                    started,
                    AppError::from(error),
                )
            }
        }
    } else {
        EdtDsl::new(
            binary,
            workspace.clone(),
            utilities.runner_for(UtilityType::EdtCli),
            context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
        )
    };
    debug!("[EDT] Инициализация workspace: {}", workspace.display());
    let mut imported_projects = Vec::new();
    for project in projects {
        if let Some(outcome) = interruption_step_outcome(
            context,
            "edt_workspace",
            "import",
            started,
            "EDT project import",
        ) {
            return outcome;
        }
        debug!("[EDT] Импорт проекта: {}", project.name);
        log_live_stage(
            "init: edt import",
            &format!("[EDT] importing source-set project '{}'", project.name),
        );
        match dsl.import_project(&project.path) {
            Ok(result) => {
                if let Err(error) =
                    ensure_platform_success("import EDT project", &project.name, &result)
                {
                    return StepOutcome::failed("edt_workspace", "import", started, error);
                }
            }
            Err(error) => {
                return StepOutcome::failed(
                    "edt_workspace",
                    "import",
                    started,
                    AppError::from(error),
                )
            }
        }
        imported_projects.push(project.name);
    }

    if let Err(error) = std::fs::write(&marker, b"initialized\n") {
        return StepOutcome::failed(
            "edt_workspace",
            "import",
            started,
            AppError::Runtime(format!(
                "failed to persist EDT workspace marker '{}': {error}",
                marker.display()
            )),
        );
    }

    StepOutcome::ok(
        "edt_workspace",
        "import",
        started,
        append_warnings(
            edt_workspace_initialized_message(&workspace, &imported_projects),
            context_deferred_warning(context).as_slice(),
        ),
    )
}

fn edt_workspace_initialized_message(workspace: &Path, imported_projects: &[String]) -> String {
    let mut message = format!("workspace initialized: {}", workspace.display());
    if !imported_projects.is_empty() {
        message.push_str("; imported EDT projects: ");
        message.push_str(&imported_projects.join(", "));
    }
    message
}

fn create_infobase_via_designer(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
) -> Result<IbcmdInfobaseCreateOutcome, AppError> {
    let binary = utilities
        .locate(UtilityType::V8)
        .map_err(AppError::from)?
        .path;
    DesignerDsl::new(
        binary,
        config.v8_connection(),
        utilities.runner_for(UtilityType::V8),
        None,
        context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None),
    )
    .create_infobase()
    .map(|result| IbcmdInfobaseCreateOutcome {
        status: if result.process.exit_code == 0 {
            IbcmdInfobaseCreateStatus::Created
        } else {
            IbcmdInfobaseCreateStatus::Failed
        },
        result,
    })
    .map_err(AppError::from)
}

fn create_infobase_via_ibcmd(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
) -> Result<IbcmdInfobaseCreateOutcome, AppError> {
    let binary = utilities
        .locate(UtilityType::Ibcmd)
        .map_err(AppError::from)?
        .path;
    let connection = IbcmdConnection::from_infobase(&config.infobase).map_err(AppError::from)?;
    IbcmdDsl::new(
        binary,
        connection,
        utilities.runner_for(UtilityType::Ibcmd),
        context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None),
    )
    .ensure_infobase_create()
    .map_err(AppError::from)
}

/// Locates the utility that would create the infobase, without creating it.
///
/// Mirrors the `builder` dispatch of [`create_infobase`] so a preview refuses on the same
/// missing platform the apply would.
fn locate_infobase_creator(
    provider: Provider,
    utilities: &mut PlatformUtilities,
) -> Result<PathBuf, AppError> {
    let utility = match provider {
        Provider::Designer => UtilityType::V8,
        Provider::Ibcmd => UtilityType::Ibcmd,
        other => {
            return Err(crate::use_cases::unimplemented_provider(
                Operation::Init,
                other,
            ))
        }
    };
    utilities
        .locate(utility)
        .map(|location| location.path)
        .map_err(AppError::from)
}

fn create_infobase(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    provider: Provider,
) -> Result<IbcmdInfobaseCreateOutcome, AppError> {
    match provider {
        Provider::Designer => create_infobase_via_designer(context, config, utilities),
        Provider::Ibcmd => create_infobase_via_ibcmd(context, config, utilities),
        other => Err(crate::use_cases::unimplemented_provider(
            Operation::Init,
            other,
        )),
    }
}

fn interruption_step_outcome(
    context: &ExecutionContext,
    target: &str,
    action: &str,
    started: Instant,
    safe_point: &str,
) -> Option<StepOutcome> {
    interruption::interruption_before_safe_point(context, safe_point)
        .map(|error| StepOutcome::failed(target, action, started, error))
}

fn context_deferred_warning(context: &ExecutionContext) -> Option<String> {
    interruption::deferred_interruption_warning_after(context, "operation completed successfully")
}

fn prepare_infobase_parent(path: &Path) -> Result<(), AppError> {
    let Some(parent) = path.parent() else {
        return Err(AppError::Runtime(format!(
            "infobase path '{}' has no parent directory",
            path.display()
        )));
    };
    std::fs::create_dir_all(parent).map_err(|error| {
        AppError::Runtime(format!(
            "failed to prepare infobase parent '{}': {error}",
            parent.display()
        ))
    })
}

fn infobase_marker_path(path: &Path) -> PathBuf {
    path.join("1Cv8.1CD")
}

fn edt_workspace_marker_path(path: &Path) -> PathBuf {
    path.join(EDT_WORKSPACE_MARKER)
}

fn ordered_source_sets(config: &AppConfig) -> Vec<&SourceSetConfig> {
    let mut configuration = Vec::new();
    let mut extensions = Vec::new();
    let mut external_processors = Vec::new();
    let mut external_reports = Vec::new();

    for source_set in &config.source_sets {
        match source_set.purpose {
            SourceSetPurpose::Configuration => configuration.push(source_set),
            SourceSetPurpose::Extension => extensions.push(source_set),
            SourceSetPurpose::ExternalDataProcessors => external_processors.push(source_set),
            SourceSetPurpose::ExternalReports => external_reports.push(source_set),
        }
    }

    configuration.extend(extensions);
    configuration.extend(external_processors);
    configuration.extend(external_reports);
    configuration
}

#[derive(Debug)]
struct EdtImportProject {
    name: String,
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectSourceImport {
    Include,
    Skip,
}

fn edt_import_projects(
    config: &AppConfig,
    project_source_import: ProjectSourceImport,
    tool_extension_path: Option<PathBuf>,
) -> Vec<EdtImportProject> {
    let mut projects = Vec::new();
    if project_source_import == ProjectSourceImport::Include {
        projects.extend(ordered_source_sets(config).into_iter().map(|source_set| {
            EdtImportProject {
                name: source_set.name.clone(),
                path: source_set.root_in(&config.base_path),
            }
        }));
    }

    if let Some(path) = tool_extension_path {
        projects.push(EdtImportProject {
            name: "tool:client_mcp".to_owned(),
            path,
        });
    }

    projects
}

fn ensure_platform_success(
    action: &str,
    target: &str,
    result: &PlatformCommandResult,
) -> Result<(), AppError> {
    if result.process.exit_code == 0 {
        return Ok(());
    }
    Err(AppError::Platform(failure_details(action, target, result)))
}

/// Создание базы не удалось.
fn failed_create(result: &PlatformCommandResult) -> AppError {
    AppError::Platform(failure_details("create infobase", "infobase", result))
}

/// Создание не удалось, а вопрос, есть ли база уже, остался без ответа. Род ответа — у
/// вопроса: отмена остаётся отменой; улики создания идут рядом.
fn unconfirmed_create(error: ProcessError, result: &PlatformCommandResult) -> AppError {
    AppError::from(error).with_context(format!(
        "{}; whether the infobase already existed went unanswered",
        failure_details("create infobase", "infobase", result)
    ))
}

/// Что не удалось и с каким кодом; вывод и журнал за ним пишет владелец улик.
fn failure_details(action: &str, target: &str, result: &PlatformCommandResult) -> String {
    format_failure_evidence(
        format!(
            "{action} failed for '{target}' with exit code {}",
            result.process.exit_code
        ),
        &result.process.stdout,
        &result.process.stderr,
        result.platform_log.as_deref(),
        result.platform_log_path.as_deref(),
    )
}

fn missing_infobase_marker_error(
    reason: &str,
    marker: &Path,
    result: &PlatformCommandResult,
) -> AppError {
    let mut details = vec![format!("{reason} '{}'", marker.display())];
    if !result.process.stdout.trim().is_empty() {
        details.push(format!("stdout: {}", result.process.stdout.trim()));
    }
    if !result.process.stderr.trim().is_empty() {
        details.push(format!("stderr: {}", result.process.stderr.trim()));
    }
    AppError::Runtime(details.join("; "))
}

#[cfg(test)]
mod tests {
    use super::{
        edt_workspace_marker_path, infobase_marker_path, ordered_source_sets, InitStepStatus,
    };
    #[cfg(unix)]
    use crate::config::model::InfobaseConfig;
    use crate::config::model::{
        AppConfig, BuildConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig,
        ToolExtensionConfig, ToolExtensionInput, ToolExtensionSourceConfig, ToolsConfig,
    };
    #[cfg(unix)]
    use crate::platform::process::HeldCommand;
    #[cfg(unix)]
    use crate::support::error::CancelledAt;
    #[cfg(unix)]
    use crate::use_cases::context::{CommandName, ExecutionContext};
    #[cfg(unix)]
    use crate::use_cases::result::UseCaseErrorKind;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    fn sample_config() -> AppConfig {
        AppConfig {
            base_path: PathBuf::from("/tmp/base"),
            work_path: PathBuf::from("/tmp/work"),
            format: SourceFormat::Edt,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: vec![
                SourceSetConfig {
                    name: "ext".to_owned(),
                    purpose: SourceSetPurpose::Extension,
                    path: PathBuf::from("ext"),
                },
                SourceSetConfig {
                    name: "main".to_owned(),
                    purpose: SourceSetPurpose::Configuration,
                    path: PathBuf::from("main"),
                },
            ],
            build: BuildConfig::default(),
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("chmod");
    }

    #[cfg(unix)]
    fn write_one_shot_edt_script(path: &Path, calls_log: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(
            path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
                calls_log.display()
            ),
        )
        .expect("write edt script");
        make_executable(path);
    }

    #[cfg(unix)]
    fn write_interactive_edt_script(path: &Path, calls_log: &Path) {
        write_interactive_edt_script_with_startup_delay(path, calls_log, 0);
    }

    #[cfg(unix)]
    fn write_interactive_edt_script_with_startup_delay(
        path: &Path,
        calls_log: &Path,
        startup_delay_ms: u64,
    ) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(
            path,
            format!(
                "#!/bin/sh\nset -eu\nprompt() {{ printf '1C:EDT>'; }}\ncurrent_dir=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"-data\" ]; then current_dir=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nsleep {}\nprintf 'START\\n' >> '{}'\ntrap 'printf \"EXIT\\\\n\" >> \"{}\"' EXIT\nprompt\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> '{}'\n  eval \"set -- $line\"\n  cmd=\"${{1:-}}\"\n  if [ \"$#\" -gt 0 ]; then shift; fi\n  case \"$cmd\" in\n    cd)\n      if [ \"$#\" -eq 0 ]; then\n        printf '%s\\n' \"$current_dir\"\n      else\n        current_dir=\"$1\"\n      fi\n      prompt\n      ;;\n    import)\n      prompt\n      ;;\n    *)\n      prompt\n      ;;\n  esac\ndone\n",
                startup_delay_ms as f64 / 1000.0,
                calls_log.display(),
                calls_log.display(),
                calls_log.display()
            ),
        )
        .expect("write interactive edt script");
        make_executable(path);
    }

    #[test]
    fn infobase_marker_uses_1cv8_1cd_file() {
        assert_eq!(
            infobase_marker_path(Path::new("/tmp/ib")),
            PathBuf::from("/tmp/ib/1Cv8.1CD")
        );
    }

    #[test]
    fn edt_workspace_marker_uses_internal_file_name() {
        assert_eq!(
            edt_workspace_marker_path(Path::new("/tmp/ws")),
            PathBuf::from("/tmp/ws/.v8tr-initialized")
        );
    }

    #[test]
    fn ordered_source_sets_puts_configuration_before_extensions() {
        let config = sample_config();
        let ordered = ordered_source_sets(&config);
        assert_eq!(ordered[0].name, "main");
        assert_eq!(ordered[1].name, "ext");
    }

    #[test]
    fn init_skips_infobase_creation_for_server_connection() {
        let mut config = sample_config();
        config.format = SourceFormat::Designer;
        config.infobase.connection = "Srvr=server;Ref=demo".to_owned();

        let result = super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        )
        .expect("server init should skip infobase create");

        assert!(result.ok);
        assert_eq!(result.steps.len(), 2);
        assert_eq!(result.steps[0].target, "infobase");
        assert_eq!(result.steps[0].action, "create");
        assert_eq!(result.steps[0].status, InitStepStatus::Skipped);
        assert_eq!(
            result.steps[0].message.as_deref(),
            Some(
                "server infobase connection detected; automatic creation is not supported by the designer provider, set providers.init: ibcmd"
            )
        );
        assert_eq!(result.steps[1].target, "edt_workspace");
        assert_eq!(result.steps[1].status, InitStepStatus::Skipped);
    }

    #[test]
    fn init_honors_interruption_before_infobase_create_safe_point() {
        let dir = tempdir().expect("tempdir");
        let mut config = sample_config();
        config.base_path = dir.path().join("base");
        config.work_path = dir.path().join("work");
        config.format = SourceFormat::Designer;
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let failure = super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            )
            .with_cancellation(cancellation),
            &config,
            false,
        )
        .expect_err("interrupted init");
        let payload = failure.payload.expect("payload");

        assert_eq!(payload.steps[0].status, InitStepStatus::Failed);
        assert!(payload.steps[0]
            .message
            .as_deref()
            .expect("message")
            .contains("before entering infobase create safe point"));
    }

    /// Проект с временной базой и подставной утилитой вместо платформы.
    #[cfg(unix)]
    fn config_with_platform(
        root: &Path,
        infobase: InfobaseConfig,
        platform: &Path,
        providers: std::collections::BTreeMap<
            crate::domain::capability::Operation,
            crate::domain::capability::Provider,
        >,
    ) -> AppConfig {
        let mut config = sample_config();
        config.base_path = root.join("base");
        config.work_path = root.join("work");
        config.format = SourceFormat::Designer;
        config.infobase = infobase;
        config.providers = providers;
        config.tools.platform.path = Some(platform.to_path_buf());
        config
    }

    /// Подставная утилита платформы: пишет вызовы в `calls`, по желанию кладёт на
    /// CREATEINFOBASE файл базы `marker` и держит команду из `branch`, пока тест её не
    /// отпустит.
    #[cfg(unix)]
    fn write_utility(path: &Path, calls: &Path, marker: Option<&Path>, branch: &str) {
        let marker = marker
            .map(|marker| {
                format!(
                    "if [ \"$1\" = \"CREATEINFOBASE\" ]; then mkdir -p '{}' && : > '{}'; fi\n",
                    marker.parent().expect("infobase dir").display(),
                    marker.display()
                )
            })
            .unwrap_or_default();
        fs::write(
            path,
            format!(
                "#!/bin/sh\nargs=\"$*\"\nprintf '%s\\n' \"$args\" >> '{}'\n{marker}{branch}exit 0\n",
                calls.display()
            ),
        )
        .expect("utility script");
        make_executable(path);
    }

    /// Кладёт ли подставной Конфигуратор файл базы, создавая её.
    #[cfg(unix)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum InfobaseFile {
        Laid,
        Missing,
    }

    /// Проект с файловой базой в `root`, чьё создание подставной Конфигуратор держит и
    /// кончает кодом `exit_code`.
    #[cfg(unix)]
    fn held_designer_create(
        root: &Path,
        exit_code: i32,
        file: InfobaseFile,
    ) -> (AppConfig, HeldCommand) {
        let infobase = root.join("ib");
        let platform = root.join("1cv8");
        let held = HeldCommand::in_dir(root);
        write_utility(
            &platform,
            &root.join("calls.log"),
            (file == InfobaseFile::Laid)
                .then(|| infobase.join("1Cv8.1CD"))
                .as_deref(),
            &held.script_branch("CREATEINFOBASE", exit_code),
        );
        let config = config_with_platform(
            root,
            InfobaseConfig::file(format!("File={}", infobase.display())),
            &platform,
            Default::default(),
        );
        (config, held)
    }

    /// `infobase create`, который оператор отменяет, пока утилита держит создание базы.
    #[cfg(unix)]
    #[track_caller]
    fn create_interrupted_while_held(
        config: &AppConfig,
        held: &HeldCommand,
    ) -> crate::use_cases::result::UseCaseResult<crate::domain::init::InitResult> {
        let cancellation = CancellationToken::new();
        held.interrupt_during(cancellation.clone(), || {
            super::execute(
                &ExecutionContext::cli(CommandName::Init).with_cancellation(cancellation),
                config,
                &crate::use_cases::request::InitRequest { dry_run: false },
            )
        })
    }

    /// Созданная база называет отмену, которую создание отложило: шаг успешен и говорит об
    /// этом словами учёта.
    #[cfg(unix)]
    #[test]
    fn a_created_infobase_names_the_cancellation_its_creation_deferred() {
        let dir = tempdir().expect("tempdir");
        let (config, held) = held_designer_create(dir.path(), 0, InfobaseFile::Laid);

        let result =
            create_interrupted_while_held(&config, &held).expect("the infobase is created");

        assert!(result.provider_dispatched);
        assert_eq!(result.steps[0].status, InitStepStatus::Ok);
        let message = result.steps[0].message.as_deref().expect("message");
        assert!(message.starts_with("infobase created:"), "{message}");
        assert!(
            message.contains(
                "infobase create completed successfully after cancellation request during critical phase"
            ),
            "{message}"
        );
    }

    /// Создание отложило отмену, и команда остановилась на следующей безопасной точке —
    /// перед рабочим пространством EDT. Ответ — отмена, а шаг создания называет отсрочку.
    #[cfg(unix)]
    #[test]
    fn a_stop_after_the_creation_leaves_its_deferred_cancellation_in_the_step() {
        let dir = tempdir().expect("tempdir");
        let (mut config, held) = held_designer_create(dir.path(), 0, InfobaseFile::Laid);
        config.format = SourceFormat::Edt;

        let failure =
            create_interrupted_while_held(&config, &held).expect_err("stopped at the safe point");

        assert_eq!(
            failure.error.kind(),
            UseCaseErrorKind::Cancelled(CancelledAt::Boundary)
        );
        let message = failure.error.message();
        assert!(
            message.contains("before entering EDT workspace import safe point"),
            "{message}"
        );
        let payload = failure.payload.expect("payload");
        assert_eq!(payload.steps[0].status, InitStepStatus::Ok);
        let created = payload.steps[0].message.as_deref().expect("message");
        assert!(
            created.contains(
                "infobase create completed successfully after cancellation request during critical phase"
            ),
            "{created}"
        );
        assert_eq!(payload.steps[1].status, InitStepStatus::Failed);
    }

    /// Создание Конфигуратором, отложившее отмену и отказавшее, остаётся отказом, а
    /// отложенную отмену его текст называет первой.
    #[cfg(unix)]
    #[test]
    fn a_failed_creation_after_a_deferred_cancellation_names_it() {
        let dir = tempdir().expect("tempdir");
        let (config, held) = held_designer_create(dir.path(), 5, InfobaseFile::Missing);

        let failure =
            create_interrupted_while_held(&config, &held).expect_err("the creation failed");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Platform);
        let message = failure.error.message();
        assert!(
            message.starts_with(
                "infobase create ended after cancellation request during critical phase"
            ),
            "{message}"
        );
        assert!(
            message.contains("create infobase failed for 'infobase' with exit code 5"),
            "{message}"
        );
        let payload = failure.payload.expect("payload");
        assert!(payload.provider_dispatched);
        assert_eq!(payload.steps[0].status, InitStepStatus::Failed);
    }

    /// Создание прошло, а файла базы нет: отказ, и отложенную отмену он называет первой.
    #[cfg(unix)]
    #[test]
    fn a_creation_without_its_marker_after_a_deferred_cancellation_names_it() {
        let dir = tempdir().expect("tempdir");
        let (config, held) = held_designer_create(dir.path(), 0, InfobaseFile::Missing);

        let failure = create_interrupted_while_held(&config, &held).expect_err("no marker file");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Runtime);
        let message = failure.error.message();
        assert!(
            message
                .starts_with("infobase create completed successfully after cancellation request"),
            "{message}"
        );
        assert!(
            message.contains("infobase creation did not produce marker file"),
            "{message}"
        );
    }

    /// ibcmd: создание, отложившее отмену и вернувшее 255, оставляет вопрос о базе без
    /// ответа — после отмены его уже не запускают. Ответ — отмена, и он называет и отсрочку,
    /// и код создания.
    #[cfg(unix)]
    #[test]
    fn an_ibcmd_creation_whose_question_went_unanswered_names_the_deferred_cancellation() {
        for (case, infobase) in [
            ("file", None),
            (
                "server",
                Some(InfobaseConfig::server(
                    "Srvr=srv;Ref=demo",
                    crate::config::model::InfobaseDbmsConfig::new(
                        "PostgreSQL",
                        "localhost",
                        "demo",
                    ),
                )),
            ),
        ] {
            let dir = tempdir().expect("tempdir");
            let ibcmd = dir.path().join("ibcmd");
            let calls = dir.path().join("calls.log");
            let held = HeldCommand::in_dir(dir.path());
            write_utility(&ibcmd, &calls, None, &held.script_branch("create", 255));
            let infobase = infobase.unwrap_or_else(|| {
                InfobaseConfig::file(format!("File={}", dir.path().join("ib").display()))
            });
            let config = config_with_platform(
                dir.path(),
                infobase,
                &ibcmd,
                crate::domain::capability::ibcmd_for_every_choice(),
            );

            let failure = create_interrupted_while_held(&config, &held)
                .expect_err("the command is cancelled");

            assert_eq!(
                failure.error.kind(),
                UseCaseErrorKind::Cancelled(CancelledAt::Boundary),
                "{case}"
            );
            let message = failure.error.message();
            assert!(
                message.starts_with(
                    "infobase create ended after cancellation request during critical phase"
                ),
                "{case}: {message}"
            );
            assert!(message.contains("with exit code 255"), "{case}: {message}");
            assert!(
                message.contains("whether the infobase already existed went unanswered"),
                "{case}: {message}"
            );
            let payload = failure.payload.expect("payload");
            assert!(payload.provider_dispatched, "{case}");
            let calls = fs::read_to_string(&calls).expect("calls");
            assert!(!calls.contains("generation-id"), "{case}: {calls}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn init_uses_one_shot_edt_when_interactive_mode_is_disabled() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let main_dir = base.join("main");
        let ext_dir = base.join("ext");
        let edt_script = dir.path().join("edt").join("1cedtcli");
        let edt_calls = dir.path().join("edt-calls.log");
        fs::create_dir_all(&main_dir).expect("main dir");
        fs::create_dir_all(&ext_dir).expect("ext dir");
        write_one_shot_edt_script(&edt_script, &edt_calls);

        let mut config = sample_config();
        config.base_path = base;
        config.work_path = work.clone();
        config.infobase.connection = "Srvr=server;Ref=demo".to_owned();
        config.tools.edt_cli.path = Some(edt_script);
        config.tools.edt_cli.interactive_mode = false;

        let result = super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        )
        .expect("init");

        let edt_calls_text = fs::read_to_string(&edt_calls).expect("edt calls");
        assert!(result.ok);
        assert!(edt_calls_text.contains("-command import --project"));
        assert_eq!(
            edt_calls_text.matches("-command import --project").count(),
            2
        );
        assert!(!edt_calls_text.contains("START"));
        assert!(edt_workspace_marker_path(&work.join("edt-workspace")).exists());
    }

    #[cfg(unix)]
    #[test]
    fn init_imports_edt_client_mcp_tool_extension_source_project() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let main_dir = base.join("main");
        let tool_dir = base.join("tool-client-mcp");
        let edt_script = dir.path().join("edt").join("1cedtcli");
        let edt_calls = dir.path().join("edt-calls.log");
        fs::create_dir_all(&main_dir).expect("main dir");
        fs::create_dir_all(&tool_dir).expect("tool dir");
        write_one_shot_edt_script(&edt_script, &edt_calls);

        let mut config = sample_config();
        config.base_path = base;
        config.work_path = work.clone();
        config.infobase.connection = "Srvr=server;Ref=demo".to_owned();
        config.source_sets = vec![SourceSetConfig {
            name: "main".to_owned(),
            purpose: SourceSetPurpose::Configuration,
            path: PathBuf::from("main"),
        }];
        config.tools.edt_cli.path = Some(edt_script);
        config.tools.edt_cli.interactive_mode = false;
        config.tools.client_mcp.extension = Some(ToolExtensionConfig {
            name: "client_mcp".to_owned(),
            input: ToolExtensionInput::Source(ToolExtensionSourceConfig {
                path: tool_dir.clone(),
                format: Some(SourceFormat::Edt),
            }),
        });

        let result = super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        )
        .expect("init");

        let edt_calls_text = fs::read_to_string(&edt_calls).expect("edt calls");
        assert!(result.ok);
        assert_eq!(
            edt_calls_text.matches("-command import --project").count(),
            2
        );
        assert!(edt_calls_text.contains(main_dir.display().to_string().as_str()));
        assert!(edt_calls_text.contains(tool_dir.display().to_string().as_str()));
        assert!(edt_workspace_marker_path(&work.join("edt-workspace")).exists());
    }

    #[cfg(unix)]
    #[test]
    fn init_imports_edt_client_mcp_tool_extension_for_designer_project() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let main_dir = base.join("main");
        let tool_dir = base.join("tool-client-mcp");
        let edt_script = dir.path().join("edt").join("1cedtcli");
        let edt_calls = dir.path().join("edt-calls.log");
        fs::create_dir_all(&main_dir).expect("main dir");
        fs::create_dir_all(&tool_dir).expect("tool dir");
        write_one_shot_edt_script(&edt_script, &edt_calls);

        let mut config = sample_config();
        config.base_path = base;
        config.work_path = work.clone();
        config.format = SourceFormat::Designer;
        config.infobase.connection = "Srvr=server;Ref=demo".to_owned();
        config.source_sets = vec![SourceSetConfig {
            name: "main".to_owned(),
            purpose: SourceSetPurpose::Configuration,
            path: PathBuf::from("main"),
        }];
        config.tools.edt_cli.path = Some(edt_script);
        config.tools.edt_cli.interactive_mode = false;
        config.tools.client_mcp.extension = Some(ToolExtensionConfig {
            name: "client_mcp".to_owned(),
            input: ToolExtensionInput::Source(ToolExtensionSourceConfig {
                path: tool_dir.clone(),
                format: Some(SourceFormat::Edt),
            }),
        });

        let result = super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        )
        .expect("init");

        let edt_calls_text = fs::read_to_string(&edt_calls).expect("edt calls");
        assert!(result.ok);
        assert_eq!(
            edt_calls_text.matches("-command import --project").count(),
            1
        );
        assert!(!edt_calls_text.contains(main_dir.display().to_string().as_str()));
        assert!(edt_calls_text.contains(tool_dir.display().to_string().as_str()));
        assert!(edt_workspace_marker_path(&work.join("edt-workspace")).exists());
    }

    #[cfg(unix)]
    #[test]
    fn init_imports_edt_client_mcp_tool_extension_into_existing_workspace() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let main_dir = base.join("main");
        let tool_dir = base.join("tool-client-mcp");
        let edt_script = dir.path().join("edt").join("1cedtcli");
        let edt_calls = dir.path().join("edt-calls.log");
        fs::create_dir_all(&main_dir).expect("main dir");
        fs::create_dir_all(&tool_dir).expect("tool dir");
        let workspace = work.join("edt-workspace");
        fs::create_dir_all(&workspace).expect("workspace");
        fs::write(edt_workspace_marker_path(&workspace), "initialized\n").expect("marker");
        write_one_shot_edt_script(&edt_script, &edt_calls);

        let mut config = sample_config();
        config.base_path = base;
        config.work_path = work.clone();
        config.infobase.connection = "Srvr=server;Ref=demo".to_owned();
        config.source_sets = vec![SourceSetConfig {
            name: "main".to_owned(),
            purpose: SourceSetPurpose::Configuration,
            path: PathBuf::from("main"),
        }];
        config.tools.edt_cli.path = Some(edt_script);
        config.tools.edt_cli.interactive_mode = false;
        config.tools.client_mcp.extension = Some(ToolExtensionConfig {
            name: "client_mcp".to_owned(),
            input: ToolExtensionInput::Source(ToolExtensionSourceConfig {
                path: tool_dir.clone(),
                format: Some(SourceFormat::Edt),
            }),
        });

        let result = super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        )
        .expect("init");

        let edt_calls_text = fs::read_to_string(&edt_calls).expect("edt calls");
        assert!(result.ok);
        assert_eq!(
            edt_calls_text.matches("-command import --project").count(),
            1
        );
        assert!(!edt_calls_text.contains(main_dir.display().to_string().as_str()));
        assert!(edt_calls_text.contains(tool_dir.display().to_string().as_str()));
        assert!(edt_workspace_marker_path(&work.join("edt-workspace")).exists());
    }

    #[cfg(unix)]
    #[test]
    fn init_cli_interactive_auto_start_remains_lazy_without_edt_commands() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let edt_script = dir.path().join("edt").join("1cedtcli");
        let edt_calls = dir.path().join("edt-calls.log");
        fs::create_dir_all(&base).expect("base dir");
        write_interactive_edt_script(&edt_script, &edt_calls);

        let mut config = sample_config();
        config.base_path = base;
        config.work_path = work.clone();
        config.infobase.connection = "Srvr=server;Ref=demo".to_owned();
        config.source_sets = vec![];
        config.tools.edt_cli.path = Some(edt_script);
        config.tools.edt_cli.interactive_mode = true;
        config.tools.edt_cli.auto_start = true;

        let result = super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        )
        .expect("init");

        assert!(result.ok);
        assert!(
            !edt_calls.exists()
                || fs::read_to_string(&edt_calls)
                    .expect("edt calls")
                    .trim()
                    .is_empty()
        );
        assert!(edt_workspace_marker_path(&work.join("edt-workspace")).exists());
    }

    #[cfg(unix)]
    #[test]
    fn init_cli_shared_session_does_not_charge_startup_against_first_command_timeout() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let main_dir = base.join("main");
        let ext_dir = base.join("ext");
        let edt_script = dir.path().join("edt").join("1cedtcli");
        let edt_calls = dir.path().join("edt-calls.log");
        fs::create_dir_all(&main_dir).expect("main dir");
        fs::create_dir_all(&ext_dir).expect("ext dir");
        write_interactive_edt_script_with_startup_delay(&edt_script, &edt_calls, 3_000);

        let mut config = sample_config();
        config.base_path = base;
        config.work_path = work.clone();
        config.infobase.connection = "Srvr=server;Ref=demo".to_owned();
        config.tools.edt_cli.path = Some(edt_script);
        config.tools.edt_cli.interactive_mode = true;
        config.tools.edt_cli.startup_timeout_ms = 30_000;
        config.tools.edt_cli.command_timeout_ms = 2_000;

        let result = super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        )
        .expect("init");

        let edt_calls_text = fs::read_to_string(&edt_calls).expect("edt calls");
        assert!(result.ok);
        assert_eq!(edt_calls_text.matches("START").count(), 1);
        assert_eq!(edt_calls_text.matches("import --project").count(), 2);
    }
}
