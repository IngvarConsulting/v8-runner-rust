use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use tracing::debug;

use crate::config::model::{
    declared_name, AppConfig, MissingDbmsField, SourceFormat, SourceSetConfig,
};
use crate::domain::capability::{Operation, Provider, TargetKind};
use crate::domain::init::{InitResult, InitStep, InitStepStatus};
use crate::platform::connection::ClusterInfobaseCreation;
use crate::platform::designer::DesignerDsl;
use crate::platform::edt::EdtDsl;
use crate::platform::edt_session::{EdtSessionHostOptions, EdtSessionManager};
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl};
use crate::platform::locator::UtilityType;
use crate::platform::result::PlatformCommandResult;
use crate::platform::secrets::mask_text;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::{AppError, CapabilityReason};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::exchange_guard::{remember_created_base, AssembledMemory, EdtSourceMemory};
use crate::use_cases::ibcmd_diagnostics::format_failure_evidence;
use crate::use_cases::interruption::{self, append_warnings, collecting_deferrals, Deferrals};
use crate::use_cases::progress::{log_live_stage, log_live_stage_status, LiveStageStatus};
use crate::use_cases::request::InitRequest;
use crate::use_cases::result::{stamp_dispatch, UseCaseError, UseCaseFailure, UseCaseResult};
use crate::use_cases::source_inventory::SourceSetInventory;
use crate::use_cases::throwaway_infobase::{edt_sources_to_xml, EdtConversion};
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
    // Общая сессия EDT одна на команду: импорт рабочей области и перевод исходников в XML
    // идут через неё, как у `push`, и одноразовый `1cedtcli` не упирается в рабочую область,
    // которую держит сессия. Сессия закрывается вместе с командой.
    let mut shared_edt: Option<EdtDsl<'static>> = None;

    // Исходники EDT переводятся в XML из рабочей области: у формата EDT она заводится до базы.
    if config.format == SourceFormat::Edt {
        record_step(
            &mut steps,
            &mut first_error,
            ensure_edt_workspace(context, config, &mut utilities, &mut shared_edt, dry_run),
        );
    }
    // Файловую базу проекта EDT собирают из перевода, а перевод — из рабочей области: без
    // неё база не создаётся, и повтор команды начинает с чистого места.
    let workspace_failed = steps
        .last()
        .is_some_and(|step: &InitStep| step.status == InitStepStatus::Failed);
    let infobase = if workspace_failed && config.target_kind() == TargetKind::File {
        StepOutcome::failed(
            "infobase",
            "create",
            Instant::now(),
            AppError::Runtime(if dry_run {
                "the infobase of an EDT project is assembled from its sources converted to XML in the EDT workspace, and the workspace step above cannot run: the infobase would not be created".to_owned()
            } else {
                "the infobase of an EDT project is assembled from its sources converted to XML in the EDT workspace, and the workspace was not initialized: the infobase is not created; run infobase create again once the workspace import succeeds".to_owned()
            }),
        )
    } else {
        ensure_infobase(
            context,
            config,
            &mut utilities,
            provider,
            shared_edt.as_ref(),
            dry_run,
        )
    };
    record_step(&mut steps, &mut first_error, infobase);
    if config.format != SourceFormat::Edt {
        record_step(
            &mut steps,
            &mut first_error,
            ensure_edt_workspace(context, config, &mut utilities, &mut shared_edt, dry_run),
        );
    }

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
    shared_edt: Option<&EdtDsl<'static>>,
    dry_run: bool,
) -> StepOutcome {
    let started = Instant::now();
    match config.target_kind() {
        // Базу автономного сервера создают до его запуска, на его машине: раннер к серверу
        // только подключается (`INV.CLI.INFOBASE-CREATE-FOLLOWS-THE-TARGET-KIND`).
        TargetKind::Standalone => {
            StepOutcome::failed("infobase", "create", started, standalone_refusal())
        }
        TargetKind::Cluster => ensure_cluster_infobase(context, config, utilities, dry_run),
        TargetKind::File => match config.v8_connection().file_path().map(PathBuf::from) {
            Some(infobase_dir) => ensure_file_infobase(
                context,
                config,
                utilities,
                provider,
                &infobase_dir,
                shared_edt,
                dry_run,
            ),
            None => StepOutcome::failed(
                "infobase",
                "create",
                started,
                AppError::Runtime("a file target names no infobase path".to_owned()),
            ),
        },
    }
}

/// Отказ автономной цели с рецептом создания базы на машине сервера.
fn standalone_refusal() -> AppError {
    AppError::capability_for(
        CapabilityReason::Target,
        "infobase create does not create the infobase of a standalone server: it is created on the server machine before the server starts — `ibcmd server config init`, then `ibcmd infobase create` (with --import, --load or --restore for the configuration); the runner attaches to the running gate named by infobase.standalone",
    )
}

/// Существующая файловая база — отказ, как у подъёма из снимка с созданием: команда
/// создаёт новую базу и чужую не трогает.
fn existing_file_infobase(infobase_dir: &Path) -> AppError {
    AppError::Validation(format!(
        "the file infobase '{}' already exists: infobase create creates a new infobase and leaves an existing one untouched; push loads the sources into it (push --force when the runner has no memory of it), or remove the directory and run infobase create again",
        infobase_dir.display()
    ))
}

fn ensure_file_infobase(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    provider: Provider,
    infobase_dir: &Path,
    shared_edt: Option<&EdtDsl<'static>>,
    dry_run: bool,
) -> StepOutcome {
    let started = Instant::now();
    let marker = infobase_marker_path(infobase_dir);
    debug!("[Инфобаза] Подготовка: {}", infobase_dir.display());
    if marker.exists() {
        return StepOutcome::failed(
            "infobase",
            "create",
            started,
            existing_file_infobase(infobase_dir),
        );
    }
    let assembled = assembled_configuration(config);
    let contents = match assembled {
        Some(set) => format!(" with the main configuration of source-set '{}'", set.name),
        None => " empty".to_owned(),
    };

    if dry_run {
        // The platform is located here so an absent one refuses during the preview; the
        // parent directory below is the first thing this step would create. A set of an EDT
        // project is converted to XML first, so EDT CLI is located as well.
        let converter = match assembled.filter(|_| config.format == SourceFormat::Edt) {
            Some(_) => match utilities.locate(UtilityType::EdtCli) {
                Ok(location) => format!(" converted to XML by {}", location.path.display()),
                Err(error) => {
                    return StepOutcome::failed(
                        "infobase",
                        "create",
                        started,
                        AppError::from(error),
                    )
                }
            },
            None => String::new(),
        };
        return match locate_infobase_creator(provider, utilities) {
            Ok(binary) => StepOutcome::planned(
                "infobase",
                "create",
                started,
                format!(
                    "would create a file infobase at '{}'{contents}{converter} via {}",
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

    let assembly = match assembled.map(|set| prepare_assembly(context, config, set, shared_edt)) {
        None => None,
        Some(Ok(assembly)) => Some(assembly),
        Some(Err(error)) => return StepOutcome::failed("infobase", "create", started, error),
    };
    let import = assembly.as_ref().map(|assembly| assembly.import.as_path());
    let memory = assembly.as_ref().map(|assembly| &assembly.memory);
    let export_warnings: &[String] = assembly
        .as_ref()
        .map_or(&[], |assembly| assembly.warnings.as_slice());

    log_live_stage("init: infobase create", "[Platform] creating infobase");
    let settled = collecting_deferrals(|deferrals| {
        let created = create_file_infobase(
            context, config, utilities, provider, import, &marker, deferrals,
        )?;
        if !marker.exists() {
            return Err(missing_infobase_marker_error(
                "infobase creation did not produce marker file",
                &marker,
                &created,
            ));
        }
        // Созданную раннером базу он помнит с рождения: собранный набор — его деревом,
        // остальные — пустой памятью (`INV.USE-CASES.WHAT-COUNTS-AS-MEMORY-OF-THE-BASE`).
        Ok(StepOutcome::ok(
            "infobase",
            "create",
            started,
            format!("infobase created{contents}: {}", marker.display()),
        )
        .with_warnings(export_warnings)
        .with_warnings(remember_created_base(config, memory).as_slice()))
    });
    match settled {
        Ok((step, warnings)) => step.with_warnings(&warnings),
        Err(error) => StepOutcome::failed("infobase", "create", started, error),
    }
}

/// Набор, из которого файловая база собирается при создании: основная конфигурация проекта.
fn assembled_configuration(config: &AppConfig) -> Option<&SourceSetConfig> {
    SourceSetInventory::new(config).main_configuration()
}

/// Сборка файловой базы: каталог XML для `--import`, память о наборе и предупреждения
/// перевода.
struct Assembly {
    memory: AssembledMemory,
    import: PathBuf,
    warnings: Vec<String>,
}

/// Общая сессия EDT команды над её рабочей областью.
fn shared_edt_session(
    context: &ExecutionContext,
    config: &AppConfig,
    binary: PathBuf,
) -> Result<EdtDsl<'static>, AppError> {
    let manager =
        EdtSessionManager::for_config(config, EdtSessionHostOptions::for_cli_command(config))
            .map_err(AppError::from)?;
    EdtDsl::new_shared_session(
        binary,
        config.work_path.join("edt-workspace"),
        Arc::new(manager),
        Duration::from_millis(config.tools.edt_cli.startup_timeout_ms),
        Duration::from_millis(config.tools.edt_cli.command_timeout_ms),
        context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
    )
    .map_err(AppError::from)
}

/// Готовит сборку файловой базы из набора `set`: каталог XML для `--import`, память о
/// наборе и предупреждения перевода. Дерево исходников снимается до сборки: правка,
/// сделанная во время неё, останется изменением для первой отправки. Исходники EDT
/// переводит в XML единственный перевод ([`edt_sources_to_xml`]) в тот же каталог, куда их
/// переводит `push`, — и память о переводе у первой отправки та же, что после неё
/// (`INV.CLI.A-FILE-BASE-OF-AN-EDT-PROJECT-IS-ASSEMBLED-FROM-ITS-SOURCES`).
fn prepare_assembly(
    context: &ExecutionContext,
    config: &AppConfig,
    set: &SourceSetConfig,
    shared_edt: Option<&EdtDsl<'static>>,
) -> Result<Assembly, AppError> {
    match config.format {
        SourceFormat::Designer => Ok(Assembly {
            memory: AssembledMemory::prepare(config, set)?,
            import: set.root_in(&config.base_path),
            warnings: Vec::new(),
        }),
        SourceFormat::Edt => {
            let source = EdtSourceMemory::prepare(config, set)?;
            let target = SourceSetInventory::new(config)
                .designer_context(&set.name)
                .map(|designer| designer.path().to_path_buf())
                .ok_or_else(|| {
                    AppError::Runtime(format!(
                        "missing change-detection context for source-set '{}'",
                        set.name
                    ))
                })?;
            let warnings = edt_sources_to_xml(
                context,
                config,
                set,
                &target,
                shared_edt.map_or(
                    EdtConversion::OneShot { timeout: None },
                    EdtConversion::Session,
                ),
            )?;
            Ok(Assembly {
                memory: AssembledMemory::prepare(config, set)?.with_edt_source(source),
                import: target,
                warnings,
            })
        }
    }
}

/// Создаёт файловую базу исполнителем; с `import` — сразу с конфигурацией из этого
/// каталога. Возвращает итог создания самой базы.
fn create_file_infobase(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    provider: Provider,
    import: Option<&Path>,
    marker: &Path,
    deferrals: &mut Deferrals,
) -> Result<PlatformCommandResult, AppError> {
    let policy = context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None);
    match provider {
        Provider::Ibcmd => {
            let binary = utilities
                .locate(UtilityType::Ibcmd)
                .map_err(AppError::from)?
                .path;
            let connection =
                IbcmdConnection::from_infobase(&config.infobase).map_err(AppError::from)?;
            let created = IbcmdDsl::new(
                binary,
                connection,
                utilities.runner_for(UtilityType::Ibcmd),
                policy,
            )
            .infobase_create(import)
            .map_err(AppError::from)?;
            deferrals.note_result(INFOBASE_CREATE, &created);
            ensure_created(&created, marker)?;
            Ok(created)
        }
        Provider::Designer => {
            let binary = utilities
                .locate(UtilityType::V8)
                .map_err(AppError::from)?
                .path;
            let runner = utilities.runner_for(UtilityType::V8);
            let created = DesignerDsl::new(
                binary.clone(),
                config.v8_connection(),
                runner,
                None,
                policy.clone(),
            )
            .create_infobase()
            .map_err(AppError::from)?;
            deferrals.note_result(INFOBASE_CREATE, &created);
            ensure_created(&created, marker)?;
            let Some(import) = import else {
                return Ok(created);
            };
            // Запасной исполнитель собирает базу теми же шагами, что отправка: загрузка
            // исходников без записи файла версий в каталог, затем обновление базы данных.
            // Сборка, которая не дошла до конца — отказ, отмена, — оставляет базу созданной
            // пустой: память говорит это, и первая отправка полная.
            let assembled = assemble_with_designer(
                DesignerDsl::new(
                    binary,
                    config.v8_connection(),
                    runner,
                    Some(designer_log_file(config)?),
                    policy,
                ),
                import,
                deferrals,
            );
            match assembled {
                Ok(()) => Ok(created),
                Err(error) => {
                    let memory = remember_created_base(config, None)
                        .map(|failure| format!(" ({failure})"))
                        .unwrap_or_default();
                    Err(error.with_context(format!(
                        "the infobase was created empty and its main configuration was not loaded{memory}; the first push loads every source-set in full"
                    )))
                }
            }
        }
        other => Err(crate::use_cases::unimplemented_provider(
            Operation::Init,
            other,
        )),
    }
}

/// Сборка созданной Конфигуратором базы: загрузка основной конфигурации без записи файла
/// версий в каталог исходников, затем обновление базы данных.
fn assemble_with_designer(
    designer: DesignerDsl<'_>,
    import: &Path,
    deferrals: &mut Deferrals,
) -> Result<(), AppError> {
    let loaded = designer
        .load_config_from_files_untouched(import, None)
        .map_err(AppError::from)?;
    deferrals.note_result(INFOBASE_CREATE, &loaded);
    ensure_platform_success("load the main configuration", "infobase", &loaded)?;
    let updated = designer.update_db_cfg(None).map_err(AppError::from)?;
    deferrals.note_result(INFOBASE_CREATE, &updated);
    ensure_platform_success("update the database configuration", "infobase", &updated)
}

/// Исход создания по коду выхода. Неудача, после которой файл базы всё же появился, оставила
/// каталог с базой неизвестного вида: повтор `infobase create` на ней отказывает, а память о
/// ней не записана, — отказ называет оба выхода.
fn ensure_created(result: &PlatformCommandResult, marker: &Path) -> Result<(), AppError> {
    result.process.outcome().map_err(|_code| {
        let error = failed_create(result);
        if !marker.exists() {
            return error;
        }
        let dir = marker.parent().unwrap_or(marker);
        error.with_context(format!(
            "the directory '{}' is left with a partly created infobase that infobase create refuses as existing and the runner has no memory of: remove the directory and run infobase create again, or load the sources over it with push --force",
            dir.display()
        ))
    })
}

/// Журнал `/Out` Конфигуратора, собирающего созданную базу.
fn designer_log_file(config: &AppConfig) -> Result<PathBuf, AppError> {
    crate::support::temp::platform_logs_dir(&config.work_path)
        .map(|dir| dir.join("infobase-create-designer.log"))
        .map_err(|error| AppError::Runtime(format!("failed to create platform logs dir: {error}")))
}

/// Базу в кластере Конфигуратор создаёт одной командой — регистрация в кластере и база
/// данных в СУБД, — пустой: память знает только, что база есть, и первая отправка полная.
fn ensure_cluster_infobase(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    dry_run: bool,
) -> StepOutcome {
    let started = Instant::now();
    // Строка подключения из конфигурации бывает с `Usr=`/`Pwd=`: базу называет
    // `describe_target`, а не строка как есть (INV.CLI.SECRETS-NEVER-REACH-THE-OUTPUT).
    let target = config.v8_connection().describe_target();
    let creation = match cluster_creation(config) {
        Ok(creation) => creation,
        Err(error) => return StepOutcome::failed("infobase", "create", started, error),
    };
    let database = format!(
        "the database '{}' on '{}'",
        creation.database_name, creation.database_server
    );
    if dry_run {
        // Есть ли база уже, без действия не узнать: CREATEINFOBASE отвечает на это кодом,
        // которым отвечает и на любой другой отказ, а вопроса `rac` к кластеру ещё нет (#213).
        return match locate_infobase_creator(Provider::Designer, utilities) {
            Ok(binary) => StepOutcome::planned(
                "infobase",
                "create",
                started,
                format!(
                    "would create {target} in the cluster with {database} via {}; whether it already exists is not observable before the creation; CrSQLDB=Y silently takes an existing database of that name, even one holding another infobase, and a failed creation may leave the database abandoned in the DBMS",
                    binary.display()
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
    log_live_stage(
        "init: infobase create",
        "[Конфигуратор] creating the infobase in the cluster",
    );
    let settled = collecting_deferrals(|deferrals| {
        let binary = utilities
            .locate(UtilityType::V8)
            .map_err(AppError::from)?
            .path;
        let created = DesignerDsl::new(
            binary,
            config.v8_connection(),
            utilities.runner_for(UtilityType::V8),
            None,
            context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None),
        )
        .create_cluster_infobase(&creation)
        .map_err(AppError::from)?;
        deferrals.note_result(INFOBASE_CREATE, &created);
        if created.process.outcome().is_err() {
            return Err(cluster_create_failure(&creation, &created, &database));
        }
        Ok(StepOutcome::ok(
            "infobase",
            "create",
            started,
            format!("{target} created in the cluster with {database}; the first push loads every source-set in full"),
        )
        .with_warnings(remember_created_base(config, None).as_slice()))
    });
    match settled {
        Ok((step, warnings)) => step.with_warnings(&warnings),
        Err(error) => StepOutcome::failed("infobase", "create", started, error),
    }
}

/// Реквизиты создания базы в кластере: доступ к СУБД и национальные настройки читает
/// единственный владелец контракта `infobase.dbms` (`InfobaseConfig::dbms_access`,
/// `dbms_locale`), администратора кластера — секция `cluster`. Нехватка обязательного поля —
/// отказ до запуска платформы с именем ключа.
fn cluster_creation(config: &AppConfig) -> Result<ClusterInfobaseCreation<'_>, AppError> {
    let refused = |missing: MissingDbmsField| {
        AppError::Validation(format!("infobase create in a cluster: {missing}"))
    };
    let access = config.infobase.dbms_access().map_err(refused)?;
    let locale = config.infobase.dbms_locale().map_err(refused)?;
    let cluster = config.infobase.cluster.as_ref();
    Ok(ClusterInfobaseCreation {
        dbms: access.kind,
        database_server: access.server,
        database_name: access.name,
        database_user: access.user,
        database_password: access.password,
        locale,
        cluster_user: declared_name(cluster.and_then(|cluster| cluster.user.as_deref())),
        cluster_password: cluster
            .and_then(|cluster| cluster.password.as_deref())
            .filter(|password| !password.is_empty()),
    })
}

/// Отказ создания базы в кластере. Причину по прозе платформы раннер не угадывает
/// (`INV.PLATFORM.PROSE-DEBT-ONLY-SHRINKS`), поэтому отказ называет то, что известно без неё:
/// без администратора кластера — этот уровень и его ключи; и что неудача может оставить базу
/// данных в СУБД. Пароли в выводе платформы скрыты.
fn cluster_create_failure(
    creation: &ClusterInfobaseCreation<'_>,
    result: &PlatformCommandResult,
    database: &str,
) -> AppError {
    let secrets: Vec<&str> = [creation.database_password, creation.cluster_password]
        .into_iter()
        .flatten()
        .collect();
    let mut message = format_failure_evidence(
        format!(
            "create infobase failed for 'infobase' with exit code {}",
            result.process.exit_code
        ),
        &mask_text(&result.process.stdout, &secrets),
        &mask_text(&result.process.stderr, &secrets),
        None,
        None,
    );
    if creation.cluster_user.is_none() {
        message.push_str(
            "; no cluster administrator is declared: a cluster with administrators admits the creation only for one — declare infobase.cluster.user and infobase.cluster.password (cluster administrator level) in v8project.local.yaml",
        );
    }
    message.push_str(&format!(
        "; a failed creation may leave {database} in the DBMS, and a retry over it registers the infobase on that database — check the DBMS before retrying"
    ));
    AppError::Platform(message)
}

fn ensure_edt_workspace(
    context: &ExecutionContext,
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    shared_edt: &mut Option<EdtDsl<'static>>,
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

    let one_shot;
    let dsl: &EdtDsl<'_> = if config.tools.edt_cli.interactive_mode {
        match shared_edt {
            Some(session) => session,
            None => match shared_edt_session(context, config, binary) {
                Ok(session) => shared_edt.insert(session),
                Err(error) => {
                    return StepOutcome::failed("edt_workspace", "import", started, error)
                }
            },
        }
    } else {
        one_shot = EdtDsl::new(
            binary,
            workspace.clone(),
            utilities.runner_for(UtilityType::EdtCli),
            context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
        );
        &one_shot
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

/// Locates the utility that would create the infobase, without creating it.
///
/// Mirrors the dispatch of [`create_file_infobase`] so a preview refuses on the same
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
    crate::use_cases::source_inventory::ordered_by_purpose(&config.source_sets)
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
    result
        .process
        .outcome()
        .map_err(|_code| AppError::Platform(failure_details(action, target, result)))
}

/// Создание базы не удалось.
fn failed_create(result: &PlatformCommandResult) -> AppError {
    AppError::Platform(failure_details("create infobase", "infobase", result))
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
        AppConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig,
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

    /// Рабочая область — шаг за шагом базы: тесты рабочей области берут ответ команды и тогда,
    /// когда шаг базы на серверной цели без реквизитов СУБД отказывает.
    fn workspace_step_result(
        result: crate::use_cases::result::UseCaseResult<crate::domain::init::InitResult>,
    ) -> crate::domain::init::InitResult {
        match result {
            Ok(result) => result,
            Err(failure) => failure.payload.expect("payload"),
        }
    }

    fn workspace_step(result: &crate::domain::init::InitResult) -> &crate::domain::init::InitStep {
        result
            .steps
            .iter()
            .find(|step| step.target == "edt_workspace")
            .expect("workspace step")
    }

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
                "#!/bin/sh\nset -eu\nprompt() {{ printf '1C:EDT>'; }}\ncurrent_dir=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"-data\" ]; then current_dir=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nsleep {}\nprintf 'START\\n' >> '{}'\ntrap 'printf \"EXIT\\\\n\" >> \"{}\"' EXIT\nprompt\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> '{}'\n  eval \"set -- $line\"\n  cmd=\"${{1:-}}\"\n  if [ \"$#\" -gt 0 ]; then shift; fi\n  case \"$cmd\" in\n    cd)\n      if [ \"$#\" -eq 0 ]; then\n        printf '%s\\n' \"$current_dir\"\n      else\n        current_dir=\"$1\"\n      fi\n      prompt\n      ;;\n    import)\n      prompt\n      ;;\n    export)\n      target=\"\"\n      while [ \"$#\" -gt 0 ]; do\n        if [ \"$1\" = \"--configuration-files\" ]; then shift; target=\"$1\"; fi\n        shift\n      done\n      mkdir -p \"$target\"\n      printf '<Configuration />\\n' > \"$target/Configuration.xml\"\n      prompt\n      ;;\n    *)\n      prompt\n      ;;\n  esac\ndone\n",
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

    /// База в кластере без реквизитов СУБД — отказ шага базы до запуска платформы; рабочая
    /// область формата Конфигуратора пропускается.
    #[test]
    fn init_refuses_a_cluster_base_without_the_dbms_section() {
        let mut config = sample_config();
        config.format = SourceFormat::Designer;
        config.infobase.connection = "Srvr=server;Ref=demo".to_owned();

        let failure = super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        )
        .expect_err("a cluster base needs the dbms section");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        assert!(failure.error.message().contains("infobase.dbms.kind"));
        let result = failure.payload.expect("payload");
        assert_eq!(result.steps.len(), 2);
        assert_eq!(result.steps[0].status, InitStepStatus::Failed);
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
        for set in ["main", "ext"] {
            fs::create_dir_all(config.base_path.join(set)).expect("sources");
        }
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
        let mut config = config_with_platform(
            root,
            InfobaseConfig::file(format!("File={}", infobase.display())),
            &platform,
            Default::default(),
        );
        // Без набора основной конфигурации создание — один процесс: отсрочку называет он.
        config
            .source_sets
            .retain(|set| set.purpose != SourceSetPurpose::Configuration);
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
        assert!(message.starts_with("infobase created empty:"), "{message}");
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
        // Рабочая область после базы — у проекта формата Конфигуратора с расширением-
        // инструментом формата EDT: безопасная точка её импорта идёт за созданием.
        let tool_dir = dir.path().join("tool-client-mcp");
        fs::create_dir_all(&tool_dir).expect("tool dir");
        config.tools.client_mcp.extension = Some(ToolExtensionConfig {
            name: "client_mcp".to_owned(),
            input: ToolExtensionInput::Source(ToolExtensionSourceConfig {
                path: tool_dir,
                format: Some(SourceFormat::Edt),
            }),
        });

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

    /// ibcmd: создание файловой базы, отложившее отмену и вернувшее 255, — отказ по коду
    /// выхода; второго вопроса к базе нет, а текст называет отсрочку первой.
    #[cfg(unix)]
    #[test]
    fn an_ibcmd_creation_that_failed_after_a_deferred_cancellation_names_it() {
        let dir = tempdir().expect("tempdir");
        let ibcmd = dir.path().join("ibcmd");
        let calls = dir.path().join("calls.log");
        let held = HeldCommand::in_dir(dir.path());
        write_utility(&ibcmd, &calls, None, &held.script_branch("create", 255));
        let config = config_with_platform(
            dir.path(),
            InfobaseConfig::file(format!("File={}", dir.path().join("ib").display())),
            &ibcmd,
            crate::domain::capability::ibcmd_for_every_choice(),
        );

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
        assert!(message.contains("with exit code 255"), "{message}");
        let payload = failure.payload.expect("payload");
        assert!(payload.provider_dispatched);
        let calls = fs::read_to_string(&calls).expect("calls");
        assert!(calls.contains("--import="), "{calls}");
        assert!(!calls.contains("generation-id"), "{calls}");
    }

    /// Конфигуратор создал базу, отложив отмену: сборку основной конфигурации отмена
    /// останавливает, база остаётся пустой, и память говорит это — первая отправка полная.
    #[cfg(unix)]
    #[test]
    fn a_cancellation_deferred_by_the_creation_leaves_an_empty_remembered_base() {
        let dir = tempdir().expect("tempdir");
        let (mut config, held) = held_designer_create(dir.path(), 0, InfobaseFile::Laid);
        config.source_sets = sample_config().source_sets;
        fs::write(
            config.base_path.join("main").join("Configuration.xml"),
            "<Configuration/>",
        )
        .expect("main source");

        let failure =
            create_interrupted_while_held(&config, &held).expect_err("the assembly is stopped");

        assert!(matches!(
            failure.error.kind(),
            UseCaseErrorKind::Cancelled(_)
        ));
        let message = failure.error.message();
        assert!(
            message.contains("the infobase was created empty")
                && message.contains("the first push loads every source-set in full"),
            "{message}"
        );
        let calls = fs::read_to_string(dir.path().join("calls.log")).expect("calls");
        assert!(!calls.contains("/UpdateDBCfg"), "{calls}");
        let contexts = crate::change_detection::source_sets::SourceSetsService::new(&config)
            .designer_contexts();
        crate::use_cases::exchange_guard::require_memory(
            &ExecutionContext::cli(CommandName::Build),
            &config,
            &contexts,
            None,
        )
        .expect("the empty base is remembered");
        let main = contexts
            .iter()
            .find(|context| context.name() == "main")
            .expect("main");
        let analysis = crate::change_detection::analyzer::analyze_context(main, &config.work_path);
        assert!(
            matches!(
                analysis.outcome,
                Ok(crate::change_detection::analyzer::AnalysisOutcome::Changes { .. })
            ),
            "the main configuration is not remembered as loaded: {:?}",
            analysis.outcome
        );
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

        let result = workspace_step_result(super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        ));

        let edt_calls_text = fs::read_to_string(&edt_calls).expect("edt calls");
        assert_eq!(workspace_step(&result).status, InitStepStatus::Ok);
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

        let result = workspace_step_result(super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        ));

        let edt_calls_text = fs::read_to_string(&edt_calls).expect("edt calls");
        assert_eq!(workspace_step(&result).status, InitStepStatus::Ok);
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

        let result = workspace_step_result(super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        ));

        let edt_calls_text = fs::read_to_string(&edt_calls).expect("edt calls");
        assert_eq!(workspace_step(&result).status, InitStepStatus::Ok);
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

        let result = workspace_step_result(super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        ));

        let edt_calls_text = fs::read_to_string(&edt_calls).expect("edt calls");
        assert_eq!(workspace_step(&result).status, InitStepStatus::Ok);
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

        let result = workspace_step_result(super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        ));

        assert_eq!(workspace_step(&result).status, InitStepStatus::Ok);
        assert!(
            !edt_calls.exists()
                || fs::read_to_string(&edt_calls)
                    .expect("edt calls")
                    .trim()
                    .is_empty()
        );
        assert!(edt_workspace_marker_path(&work.join("edt-workspace")).exists());
    }

    /// Проект EDT при `interactive_mode`: импорт рабочей области и перевод основного набора
    /// в XML идут через одну общую сессию EDT команды — второй процесс `1cedtcli` не
    /// стартует и в рабочую область, которую держит сессия, не упирается; `ibcmd` собирает
    /// базу из перевода.
    #[cfg(unix)]
    #[test]
    fn an_edt_file_base_is_converted_through_the_shared_session_of_the_command() {
        let dir = tempdir().expect("tempdir");
        let edt_script = dir.path().join("edt").join("1cedtcli");
        let edt_calls = dir.path().join("edt-calls.log");
        write_interactive_edt_script(&edt_script, &edt_calls);
        let ibcmd = dir.path().join("platform").join("ibcmd");
        fs::create_dir_all(ibcmd.parent().expect("platform dir")).expect("platform dir");
        let ibcmd_calls = dir.path().join("ibcmd-calls.log");
        let infobase = dir.path().join("ib");
        write_utility(
            &ibcmd,
            &ibcmd_calls,
            None,
            &format!(
                "case \"$args\" in *create*) mkdir -p '{0}' && : > '{0}/1Cv8.1CD';; esac\n",
                infobase.display()
            ),
        );
        let mut config = config_with_platform(
            dir.path(),
            InfobaseConfig::file(format!("File={}", infobase.display())),
            &ibcmd,
            crate::domain::capability::ibcmd_for_every_choice(),
        );
        config.format = SourceFormat::Edt;
        for set in ["main", "ext"] {
            fs::write(
                config.base_path.join(set).join(".project"),
                format!("<projectDescription><name>{set}</name></projectDescription>"),
            )
            .expect("source");
        }
        config.tools.edt_cli.path = Some(edt_script);
        config.tools.edt_cli.interactive_mode = true;

        let result = super::run_init(&ExecutionContext::cli(CommandName::Init), &config, false);

        let edt_calls_text = fs::read_to_string(&edt_calls).expect("edt calls");
        let result = result.unwrap_or_else(|failure| panic!("{failure:?}\nEDT: {edt_calls_text}"));
        assert_eq!(result.steps[0].target, "edt_workspace");
        assert_eq!(result.steps[1].status, InitStepStatus::Ok, "{result:?}");
        assert_eq!(
            edt_calls_text.matches("START").count(),
            1,
            "{edt_calls_text}"
        );
        assert!(
            edt_calls_text.contains("export --project-name main"),
            "{edt_calls_text}"
        );
        let ibcmd_calls = fs::read_to_string(&ibcmd_calls).expect("ibcmd calls");
        assert!(ibcmd_calls.contains("--import="), "{ibcmd_calls}");
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

        let result = workspace_step_result(super::run_init(
            &crate::use_cases::context::ExecutionContext::cli(
                crate::use_cases::context::CommandName::Init,
            ),
            &config,
            false,
        ));

        let edt_calls_text = fs::read_to_string(&edt_calls).expect("edt calls");
        assert_eq!(workspace_step(&result).status, InitStepStatus::Ok);
        assert_eq!(edt_calls_text.matches("START").count(), 1);
        assert_eq!(edt_calls_text.matches("import --project").count(), 2);
    }
}
