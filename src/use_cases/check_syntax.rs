use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::config::model::{AppConfig, SourceFormat, SourceSetConfig};
use crate::domain::capability::{Operation, Provider};
use crate::domain::issue::{EdtIssue, Issue, IssueSeverity, ObjectIssue};
use crate::domain::syntax::{CheckName, SyntaxCheckResult, SyntaxCheckStatus, SyntaxIssueSummary};
use crate::parsers::designer_validation;
use crate::parsers::edt_validation;
use crate::platform::designer::DesignerDsl;
use crate::platform::edt::EdtDsl;
use crate::platform::edt_session::{EdtSessionHostOptions, EdtSessionManager};
use crate::platform::locator::UtilityType;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::{AppError, CapabilityReason};
use crate::support::temp::platform_logs_dir;
#[cfg(test)]
use crate::use_cases::context::CommandName;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::progress::log_live_stage;
use crate::use_cases::request::{
    DesignerClientScope, DesignerConfigCheck,
    DesignerConfigSyntaxRequest as DesignerConfigSyntaxArgs, ExtendedModulesPolicy,
    SyntaxExtensionScope, SyntaxRequest as SyntaxArgs, SyntaxTargetRequest as SyntaxTarget,
};
use crate::use_cases::result::{stamp_dispatch, UseCaseFailure, UseCaseResult};
use crate::use_cases::source_inventory::SourceSetInventory;
use tracing::debug;

const SUPPORTED_DESIGNER_SYNTAX_ERROR: &str =
    "check currently supports only the Designer provider and format=DESIGNER";
const SUPPORTED_EDT_SYNTAX_ERROR: &str =
    "check edt currently supports only the Designer provider and format=EDT";
static LOG_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &SyntaxArgs,
) -> UseCaseResult<SyntaxCheckResult> {
    debug!(
        command = context.command().as_str(),
        transport = ?context.transport(),
        "executing syntax use case"
    );
    stamp_dispatch(run_syntax_branch(context, config, args), context.work())
}

type SyntaxExecutionFailure = UseCaseFailure<SyntaxCheckResult>;

#[cfg(test)]
fn run_syntax(config: &AppConfig, args: &SyntaxArgs) -> UseCaseResult<SyntaxCheckResult> {
    let context = ExecutionContext::cli(CommandName::Syntax);
    execute(&context, config, args)
}

fn run_syntax_branch(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &SyntaxArgs,
) -> UseCaseResult<SyntaxCheckResult> {
    let started = Instant::now();
    // Ветка выбирается раньше всего остального: иначе отказ уже отменённой проверки EDT
    // назвался бы именем проверки конфигурации. У ветки EDT своя такая же проверка.
    if let SyntaxTarget::Edt { projects } = &args.target {
        return run_edt_syntax(context, config, projects, args.dry_run, started);
    }
    if let Some(failure) =
        interrupted_syntax_failure(context, CheckName::DesignerConfig, started, None)
    {
        return Err(failure);
    }
    // Отказ по предмету спрашивается на ветке платформы: проверку проекта EDT внешние
    // наборы переживают — её выполняет EDT CLI, и предмет у неё свой.
    if let Some(failure) = external_subject_refusal(config, started) {
        return Err(failure);
    }

    // Ветка одна: проверку конфигурации выполняет `/CheckConfig`, а проверку проекта EDT
    // — свой путь выше. Нормализация теперь только раскладывает режимы в argv.
    let flags = normalize_config_flags(match &args.target {
        SyntaxTarget::DesignerConfig(config_args) => config_args,
        SyntaxTarget::Edt { .. } => unreachable!("EDT syntax is handled before normalization"),
    });

    if let Some(error) = validate_designer_supported_matrix(config) {
        let error_message = error.to_string();
        return Err(SyntaxExecutionFailure::with_payload(
            error,
            failed_result(
                CheckName::DesignerConfig,
                SyntaxCheckStatus::ToolFailed,
                -1,
                started,
                vec![],
                None,
                Some(error_message),
                None,
            ),
        ));
    }

    // Превью отвечает раньше `platform_logs_dir`: каталог журналов платформы — первая
    // собственная запись этой команды, и превью её не делает. Боевой порядок при этом
    // остаётся прежним, поэтому первым отказом у нечитаемого рабочего каталога
    // по-прежнему приходит отказ журнала, а не отказ поиска платформы.
    if args.dry_run {
        return preview_designer_config(config, &flags, started);
    }

    debug!(
        check = CheckName::DesignerConfig.as_str(),
        flags = ?flags,
        "starting syntax check"
    );
    let log_dir = match platform_logs_dir(&config.work_path) {
        Ok(dir) => dir,
        Err(error) => {
            let app_error = AppError::Runtime(format!(
                "failed to prepare syntax platform logs directory '{}': {error}",
                config.work_path.display()
            ));
            let error_message = app_error.to_string();
            return Err(SyntaxExecutionFailure::with_payload(
                app_error,
                failed_result(
                    CheckName::DesignerConfig,
                    SyntaxCheckStatus::ToolFailed,
                    -1,
                    started,
                    vec![],
                    None,
                    Some(error_message),
                    None,
                ),
            ));
        }
    };

    let log_path = unique_log_path(&log_dir, CheckName::DesignerConfig.as_str());
    debug!(path = %log_path.display(), "syntax platform log reserved");

    let SelectedDesigner {
        utilities,
        receipt,
        location,
    } = select_designer(config, started)?;

    let runner = utilities.runner_for(UtilityType::V8);
    let dsl = DesignerDsl::new(
        location.path,
        config.v8_connection(),
        runner,
        Some(log_path.clone()),
        context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
    );

    let flags: Vec<&str> = flags.iter().map(String::as_str).collect();
    let stage_label = "check: designer-config";
    log_live_stage(stage_label, "[Конфигуратор] running syntax check");
    let platform_result = dsl.check_config(&flags);

    let platform_result = match platform_result {
        Ok(result) => result,
        Err(error) => {
            let app_error = AppError::from(error);
            let message = app_error.to_string();
            let mut result = failed_result(
                CheckName::DesignerConfig,
                SyntaxCheckStatus::ToolFailed,
                -1,
                started,
                vec![],
                None,
                Some(message),
                Some(log_path),
            );
            result.provider = Some(receipt);
            return Err(SyntaxExecutionFailure::with_payload(app_error, result));
        }
    };

    let mut result = build_result(CheckName::DesignerConfig, platform_result, started);
    result.provider = Some(receipt);
    match result.status {
        // Превью сюда не доходит — оно возвращается раньше запуска, — но исход у него
        // тот же: отказом становятся только приговоры конфигурации.
        SyntaxCheckStatus::Clean | SyntaxCheckStatus::Planned => Ok(result),
        SyntaxCheckStatus::IssuesFound | SyntaxCheckStatus::ToolFailed => {
            Err(SyntaxExecutionFailure::with_payload(
                AppError::Runtime(format!(
                    "syntax check '{}' finished with status {:?} (exit code {})",
                    result.check_name, result.status, result.exit_code
                )),
                result,
            ))
        }
    }
}

/// Исполнитель, выбранный для проверки конфигурации: та же квитанция и тот же путь, что
/// получает боевой прогон. Превью доходит ровно сюда и дальше не идёт.
struct SelectedDesigner {
    utilities: PlatformUtilities,
    receipt: crate::domain::capability::ProviderReceipt,
    location: crate::platform::locator::UtilityLocation,
}

fn select_designer(
    config: &AppConfig,
    started: Instant,
) -> Result<SelectedDesigner, SyntaxExecutionFailure> {
    let mut utilities = PlatformUtilities::from_config(config);
    let selected = match crate::use_cases::provider_selection::select(
        config,
        &mut utilities,
        crate::domain::capability::Operation::Syntax,
    ) {
        Ok(selected) => selected,
        Err((error, receipt)) => {
            let message = error.to_string();
            let mut result = failed_result(
                CheckName::DesignerConfig,
                SyntaxCheckStatus::ToolFailed,
                -1,
                started,
                vec![],
                None,
                Some(message),
                None,
            );
            result.provider = Some(receipt);
            return Err(SyntaxExecutionFailure::with_payload(error, result));
        }
    };
    let receipt = selected.receipt;
    let Some(location) = selected.location else {
        return Err(SyntaxExecutionFailure::without_payload(
            crate::use_cases::unimplemented_provider(
                crate::domain::capability::Operation::Syntax,
                selected.provider,
            ),
        ));
    };
    Ok(SelectedDesigner {
        utilities,
        receipt,
        location,
    })
}

/// Превью проверки конфигурации: та же проверка запроса, тот же поиск утилиты, и возврат
/// раньше собственных записей команды. Каталог журналов платформы не создаётся, поэтому
/// путь журнала превью не называет — файла не будет. Строку в журнале действий превью
/// всё же оставляет: оно не прячется.
fn preview_designer_config(
    config: &AppConfig,
    flags: &[String],
    started: Instant,
) -> UseCaseResult<SyntaxCheckResult> {
    let selected = select_designer(config, started)?;
    log_live_stage(
        "check: preview",
        "[Конфигуратор] preview only, configuration not checked",
    );
    let mut result = planned_result(CheckName::DesignerConfig, started);
    result.provider = Some(selected.receipt);
    result.message = Some(format!(
        "would run `/CheckConfig {}` via {}; configuration not checked",
        flags.join(" "),
        selected.location.path.display()
    ));
    Ok(result)
}

/// Ответ превью: приговора конфигурации нет, потому что конфигурацию не смотрели.
fn planned_result(check_name: CheckName, started: Instant) -> SyntaxCheckResult {
    SyntaxCheckResult {
        provider: None,
        provider_dispatched: false,
        message: None,
        status: SyntaxCheckStatus::Planned,
        // Кода выхода не наблюдалось: платформа не запускалась.
        exit_code: -1,
        check_name,
        summary: summarize_issues(&[]),
        issues: vec![],
        duration_ms: elapsed_millis(started),
        platform_log_path: None,
        stderr: None,
        log_read_warning: None,
    }
}

/// Проверка внешних обработок и отчётов платформой не описана: `/CheckConfig` проверяет
/// конфигурацию базы, а не внешний файл. Проект, где других наборов нет, получил бы ответ
/// «чисто», ничего не проверив, поэтому отказ — по предмету, и он не изменится со временем
/// (`DEC.2026-09-21.CHECK-IS-CHECKCONFIG`).
fn external_subject_refusal(
    config: &AppConfig,
    started: Instant,
) -> Option<SyntaxExecutionFailure> {
    let sets = &config.source_sets;
    if sets.is_empty() || !sets.iter().all(|set| set.purpose.is_external()) {
        return None;
    }
    let error = AppError::capability_for(
        CapabilityReason::Subject,
        "the project declares only external data processors and reports, and the platform describes no check for them: `check` checks the configuration",
    );
    let message = error.to_string();
    Some(SyntaxExecutionFailure::with_payload(
        error,
        failed_result(
            CheckName::DesignerConfig,
            SyntaxCheckStatus::ToolFailed,
            -1,
            started,
            vec![],
            None,
            Some(message),
            None,
        ),
    ))
}

fn normalize_config_flags(args: &DesignerConfigSyntaxArgs) -> Vec<String> {
    let mut flags = Vec::new();
    push_config_check(&mut flags, args, DesignerConfigCheck::ConfigLogIntegrity);
    push_config_check(&mut flags, args, DesignerConfigCheck::IncorrectReferences);
    push_client_scope(&mut flags, args, DesignerClientScope::ThinClient);
    push_client_scope(&mut flags, args, DesignerClientScope::WebClient);
    push_client_scope(&mut flags, args, DesignerClientScope::MobileClient);
    push_client_scope(&mut flags, args, DesignerClientScope::Server);
    push_client_scope(&mut flags, args, DesignerClientScope::ExternalConnection);
    push_client_scope(
        &mut flags,
        args,
        DesignerClientScope::ExternalConnectionServer,
    );
    push_client_scope(&mut flags, args, DesignerClientScope::MobileAppClient);
    push_client_scope(&mut flags, args, DesignerClientScope::MobileAppServer);
    push_client_scope(
        &mut flags,
        args,
        DesignerClientScope::ThickClientManagedApplication,
    );
    push_client_scope(
        &mut flags,
        args,
        DesignerClientScope::ThickClientServerManagedApplication,
    );
    push_client_scope(
        &mut flags,
        args,
        DesignerClientScope::ThickClientOrdinaryApplication,
    );
    push_client_scope(
        &mut flags,
        args,
        DesignerClientScope::ThickClientServerOrdinaryApplication,
    );
    push_config_check(&mut flags, args, DesignerConfigCheck::MobileClientDigiSign);
    push_config_check(&mut flags, args, DesignerConfigCheck::DistributiveModules);
    push_config_check(&mut flags, args, DesignerConfigCheck::UnreferenceProcedures);
    push_config_check(&mut flags, args, DesignerConfigCheck::HandlersExistence);
    push_config_check(&mut flags, args, DesignerConfigCheck::EmptyHandlers);
    push_extended_modules_policy(&mut flags, args.extended_modules());
    push_config_check(&mut flags, args, DesignerConfigCheck::UnsupportedFunctional);
    push_extension_scope(&mut flags, args.extension_scope());
    flags
}

fn push_flag(flags: &mut Vec<String>, enabled: bool, flag: &str) {
    if enabled {
        flags.push(flag.to_owned());
    }
}

fn push_config_check(
    flags: &mut Vec<String>,
    args: &DesignerConfigSyntaxArgs,
    check: DesignerConfigCheck,
) {
    push_flag(flags, args.has_check(check), check.flag());
}

fn push_client_scope<T>(flags: &mut Vec<String>, args: &T, scope: DesignerClientScope)
where
    T: HasClientScopes,
{
    push_flag(flags, args.has_client_scope(scope), scope.flag());
}

fn push_extended_modules_policy(flags: &mut Vec<String>, policy: ExtendedModulesPolicy) {
    push_flag(flags, policy.is_enabled(), "-ExtendedModulesCheck");
    push_flag(
        flags,
        policy.checks_synchronous_calls(),
        "-CheckUseSynchronousCalls",
    );
    push_flag(flags, policy.checks_modality(), "-CheckUseModality");
}

fn push_extension_scope(flags: &mut Vec<String>, scope: &SyntaxExtensionScope) {
    if let Some(extension) = scope.extension() {
        flags.push("-Extension".to_owned());
        flags.push(extension.to_owned());
    }
    if scope.includes_all_extensions() {
        flags.push("-AllExtensions".to_owned());
    }
}

trait HasClientScopes {
    fn has_client_scope(&self, scope: DesignerClientScope) -> bool;
}

impl HasClientScopes for DesignerConfigSyntaxArgs {
    fn has_client_scope(&self, scope: DesignerClientScope) -> bool {
        DesignerConfigSyntaxArgs::has_client_scope(self, scope)
    }
}

fn validate_designer_supported_matrix(config: &AppConfig) -> Option<AppError> {
    if config.default_provider(Operation::Syntax) != Some(Provider::Designer)
        || config.format != SourceFormat::Designer
    {
        Some(AppError::Validation(
            SUPPORTED_DESIGNER_SYNTAX_ERROR.to_owned(),
        ))
    } else {
        None
    }
}

fn validate_edt_supported_matrix(config: &AppConfig) -> Option<AppError> {
    if config.default_provider(Operation::Syntax) != Some(Provider::Designer)
        || config.format != SourceFormat::Edt
    {
        Some(AppError::Validation(SUPPORTED_EDT_SYNTAX_ERROR.to_owned()))
    } else {
        None
    }
}

/// Утилита EDT CLI, найденная для проверки проекта. Ветка EDT ищет её напрямую и
/// квитанции о выборе исполнителя не имеет: выбирать не из чего.
fn locate_edt(
    config: &AppConfig,
    started: Instant,
) -> Result<(PlatformUtilities, crate::platform::locator::UtilityLocation), SyntaxExecutionFailure>
{
    let mut utilities = PlatformUtilities::from_config(config);
    match utilities.locate(UtilityType::EdtCli) {
        Ok(location) => Ok((utilities, location)),
        Err(error) => {
            let message = error.to_string();
            let app_error = AppError::from(error);
            Err(SyntaxExecutionFailure::with_payload(
                app_error,
                failed_result(
                    CheckName::Edt,
                    SyntaxCheckStatus::ToolFailed,
                    -1,
                    started,
                    vec![],
                    None,
                    Some(message),
                    None,
                ),
            ))
        }
    }
}

/// Превью проверки проекта EDT. Квитанции здесь нет — её не имеет и боевой прогон.
fn preview_edt(
    config: &AppConfig,
    source_sets: &[&SourceSetConfig],
    started: Instant,
) -> UseCaseResult<SyntaxCheckResult> {
    let (_utilities, location) = locate_edt(config, started)?;
    log_live_stage("check: preview", "[EDT] preview only, project not checked");
    let mut result = planned_result(CheckName::Edt, started);
    let names: Vec<&str> = source_sets
        .iter()
        .map(|source_set| source_set.name.as_str())
        .collect();
    result.message = Some(format!(
        "would check {} by {}; project not checked",
        names.join(", "),
        location.path.display()
    ));
    Ok(result)
}

fn run_edt_syntax(
    context: &ExecutionContext,
    config: &AppConfig,
    projects: &[String],
    dry_run: bool,
    started: Instant,
) -> UseCaseResult<SyntaxCheckResult> {
    if let Some(failure) = interrupted_syntax_failure(context, CheckName::Edt, started, None) {
        return Err(failure);
    }
    if let Some(error) = validate_edt_supported_matrix(config) {
        let error_message = error.to_string();
        return Err(SyntaxExecutionFailure::with_payload(
            error,
            failed_result(
                CheckName::Edt,
                SyntaxCheckStatus::ToolFailed,
                -1,
                started,
                vec![],
                None,
                Some(error_message),
                None,
            ),
        ));
    }

    let inventory = SourceSetInventory::new(config);
    let source_sets = match resolve_edt_source_sets(&inventory, projects) {
        Ok(source_sets) => source_sets,
        Err(error) => {
            let error_message = error.to_string();
            return Err(SyntaxExecutionFailure::with_payload(
                error,
                failed_result(
                    CheckName::Edt,
                    SyntaxCheckStatus::ToolFailed,
                    -1,
                    started,
                    vec![],
                    None,
                    Some(error_message),
                    None,
                ),
            ));
        }
    };

    // Та же остановка, что и у ветки Конфигуратора: раньше первой записи на диск.
    if dry_run {
        return preview_edt(config, &source_sets, started);
    }

    let log_dir = match platform_logs_dir(&config.work_path) {
        Ok(dir) => dir,
        Err(error) => {
            let app_error = AppError::Runtime(format!(
                "failed to prepare syntax platform logs directory '{}': {error}",
                config.work_path.display()
            ));
            let error_message = app_error.to_string();
            return Err(SyntaxExecutionFailure::with_payload(
                app_error,
                failed_result(
                    CheckName::Edt,
                    SyntaxCheckStatus::ToolFailed,
                    -1,
                    started,
                    vec![],
                    None,
                    Some(error_message),
                    None,
                ),
            ));
        }
    };

    let (utilities, location) = locate_edt(config, started)?;

    let edt_binary = location.path;
    let interactive_dsl = if config.tools.edt_cli.interactive_mode {
        match EdtSessionManager::for_config(config, EdtSessionHostOptions::for_cli_command(config))
        {
            Ok(manager) => match EdtDsl::new_shared_session(
                edt_binary.clone(),
                config.work_path.join("edt-workspace"),
                Arc::new(manager),
                Duration::from_millis(config.tools.edt_cli.startup_timeout_ms),
                Duration::from_millis(config.tools.edt_cli.command_timeout_ms),
                context.process_policy(
                    InterruptionSafetyClass::GracefulThenKill,
                    context.edt_timeout(),
                ),
            ) {
                Ok(dsl) => Some(dsl.with_timeout(context.edt_timeout())),
                Err(error) => {
                    let app_error = AppError::from(error);
                    let message = app_error.to_string();
                    return Err(SyntaxExecutionFailure::with_payload(
                        app_error,
                        failed_result(
                            CheckName::Edt,
                            SyntaxCheckStatus::ToolFailed,
                            -1,
                            started,
                            vec![],
                            None,
                            Some(message),
                            None,
                        ),
                    ));
                }
            },
            Err(error) => {
                let app_error = AppError::from(error);
                let message = app_error.to_string();
                return Err(SyntaxExecutionFailure::with_payload(
                    app_error,
                    failed_result(
                        CheckName::Edt,
                        SyntaxCheckStatus::ToolFailed,
                        -1,
                        started,
                        vec![],
                        None,
                        Some(message),
                        None,
                    ),
                ));
            }
        }
    } else {
        None
    };
    let mut issues = Vec::new();
    let mut status = SyntaxCheckStatus::Clean;
    let mut exit_code = 0;
    let mut stderr_lines = Vec::new();
    let mut log_warnings = Vec::new();
    let mut single_platform_log_path = None;
    let single_source_set = source_sets.len() == 1;

    for source_set in source_sets {
        let source_path = inventory.source_path(source_set);
        let log_path = unique_log_path(
            &log_dir,
            &format!("edt_{}", source_set.name.replace(' ', "_")),
        );
        if let Some(failure) =
            interrupted_syntax_failure(context, CheckName::Edt, started, Some(log_path.clone()))
        {
            return Err(failure);
        }
        log_live_stage("check: edt", "[EDT] validating project");
        let result = match if let Some(dsl) = interactive_dsl.as_ref() {
            dsl.validate_project(&source_path, &log_path)
        } else {
            EdtDsl::new(
                edt_binary.clone(),
                config.work_path.join("edt-workspace"),
                utilities.runner_for(UtilityType::EdtCli),
                context.process_policy(
                    InterruptionSafetyClass::GracefulThenKill,
                    context.edt_timeout(),
                ),
            )
            .with_timeout(context.edt_timeout())
            .validate_project(&source_path, &log_path)
        } {
            Ok(result) => result,
            Err(error) => {
                let app_error = AppError::from(error);
                let message = app_error.to_string();
                return Err(SyntaxExecutionFailure::with_payload(
                    app_error,
                    failed_result(
                        CheckName::Edt,
                        SyntaxCheckStatus::ToolFailed,
                        -1,
                        started,
                        vec![],
                        None,
                        Some(message),
                        Some(log_path),
                    ),
                ));
            }
        };

        if single_source_set {
            single_platform_log_path = Some(log_path);
        }

        if !result.process.stderr.trim().is_empty() {
            stderr_lines.push(format!(
                "{}: {}",
                source_set.name,
                result.process.stderr.trim()
            ));
        }
        if let Some(log_warning) = &result.platform_log_read_error {
            log_warnings.push(format!("{}: {log_warning}", source_set.name));
        }

        let project_issues = result
            .platform_log
            .as_deref()
            .map(edt_validation::parse)
            .unwrap_or_default();
        let project_status = edt_status_from_result(
            result.process.exit_code,
            &project_issues,
            result.platform_log_read_error.is_some(),
        );
        status = combine_status(status, project_status);

        if result.process.exit_code != 0
            && (project_status == SyntaxCheckStatus::ToolFailed || exit_code == 0)
        {
            exit_code = result.process.exit_code;
        }

        if result.process.exit_code != 0 && project_issues.is_empty() {
            issues.push(fallback_edt_issue(
                &source_set.name,
                result.process.exit_code,
                if result.process.stderr.trim().is_empty() {
                    None
                } else {
                    Some(result.process.stderr.as_str())
                },
                result.platform_log_read_error.as_deref(),
                result.platform_log_path.as_deref(),
            ));
        } else {
            issues.extend(project_issues);
        }
    }

    let stderr = (!stderr_lines.is_empty()).then_some(stderr_lines.join("\n"));
    let log_read_warning = (!log_warnings.is_empty()).then_some(log_warnings.join("\n"));
    let result = SyntaxCheckResult {
        provider: None,
        provider_dispatched: false,
        message: None,
        status,
        exit_code,
        check_name: CheckName::Edt,
        summary: summarize_issues(&issues),
        issues,
        duration_ms: elapsed_millis(started),
        platform_log_path: single_platform_log_path,
        stderr,
        log_read_warning,
    };

    match result.status {
        // Превью сюда не доходит — оно возвращается раньше запуска, — но исход у него
        // тот же: отказом становятся только приговоры конфигурации.
        SyntaxCheckStatus::Clean | SyntaxCheckStatus::Planned => Ok(result),
        SyntaxCheckStatus::IssuesFound | SyntaxCheckStatus::ToolFailed => {
            Err(SyntaxExecutionFailure::with_payload(
                AppError::Runtime(format!(
                    "syntax check '{}' finished with status {:?} (exit code {})",
                    result.check_name, result.status, result.exit_code
                )),
                result,
            ))
        }
    }
}

fn interrupted_syntax_failure(
    context: &ExecutionContext,
    check_name: CheckName,
    started: Instant,
    platform_log_path: Option<PathBuf>,
) -> Option<SyntaxExecutionFailure> {
    let interruption = context.interruption()?;
    let message =
        crate::use_cases::interruption::command_interruption_message(context, interruption);
    Some(SyntaxExecutionFailure::with_payload(
        AppError::Runtime(message.clone()),
        failed_result(
            check_name,
            SyntaxCheckStatus::ToolFailed,
            -1,
            started,
            vec![],
            None,
            Some(message),
            platform_log_path,
        ),
    ))
}

fn resolve_edt_source_sets<'a>(
    inventory: &SourceSetInventory<'a>,
    projects: &[String],
) -> Result<Vec<&'a SourceSetConfig>, AppError> {
    if !inventory.has_edt_contexts() {
        return Err(AppError::Validation(
            "check requires at least one source-set".to_owned(),
        ));
    }

    if projects.is_empty() {
        return Ok(inventory.source_sets());
    }

    let mut selected = Vec::new();
    let mut unknown = Vec::new();

    for project in projects {
        if let Some(source_set) = inventory.source_set(project) {
            selected.push(source_set);
        } else {
            unknown.push(project.clone());
        }
    }

    if !unknown.is_empty() {
        return Err(AppError::Validation(format!(
            "unknown EDT project(s): {}",
            unknown.join(", ")
        )));
    }

    Ok(selected)
}

fn edt_status_from_result(
    exit_code: i32,
    issues: &[Issue],
    log_unreadable: bool,
) -> SyntaxCheckStatus {
    if log_unreadable && exit_code == 0 && issues.is_empty() {
        return SyntaxCheckStatus::ToolFailed;
    }
    if exit_code == 0 && issues.is_empty() {
        SyntaxCheckStatus::Clean
    } else if !issues.is_empty() {
        SyntaxCheckStatus::IssuesFound
    } else {
        SyntaxCheckStatus::ToolFailed
    }
}

fn combine_status(current: SyntaxCheckStatus, next: SyntaxCheckStatus) -> SyntaxCheckStatus {
    match (current, next) {
        (SyntaxCheckStatus::ToolFailed, _) | (_, SyntaxCheckStatus::ToolFailed) => {
            SyntaxCheckStatus::ToolFailed
        }
        (SyntaxCheckStatus::IssuesFound, _) | (_, SyntaxCheckStatus::IssuesFound) => {
            SyntaxCheckStatus::IssuesFound
        }
        _ => SyntaxCheckStatus::Clean,
    }
}

fn unique_log_path(dir: &Path, check_name: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sequence = LOG_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(
        "syntax_{}_{}_{}_{}.log",
        check_name,
        timestamp,
        std::process::id(),
        sequence
    ))
}

fn build_result(
    check_name: CheckName,
    platform_result: PlatformCommandResult,
    started: Instant,
) -> SyntaxCheckResult {
    let PlatformCommandResult {
        process,
        platform_log_path,
        platform_log,
        platform_log_read_error,
    } = platform_result;
    let exit_code = process.exit_code;
    let stderr = (!process.stderr.trim().is_empty()).then_some(process.stderr);
    let mut issues = platform_log
        .as_deref()
        .map(designer_validation::parse)
        .unwrap_or_default();
    let log_read_warning = platform_log_read_error;
    let status = verdict(exit_code, log_read_warning.is_some());

    if status != SyntaxCheckStatus::Clean && issues.is_empty() {
        issues.push(fallback_issue(
            exit_code,
            stderr.as_deref(),
            log_read_warning.as_deref(),
            platform_log_path.as_deref(),
        ));
    }

    SyntaxCheckResult {
        provider: None,
        provider_dispatched: false,
        message: None,
        status,
        exit_code,
        check_name,
        summary: summarize_issues(&issues),
        issues,
        duration_ms: elapsed_millis(started),
        platform_log_path,
        stderr,
        log_read_warning,
    }
}

fn failed_result(
    check_name: CheckName,
    status: SyntaxCheckStatus,
    exit_code: i32,
    started: Instant,
    issues: Vec<Issue>,
    log_read_warning: Option<String>,
    stderr: Option<String>,
    platform_log_path: Option<PathBuf>,
) -> SyntaxCheckResult {
    SyntaxCheckResult {
        provider: None,
        provider_dispatched: false,
        message: None,
        status,
        exit_code,
        check_name,
        summary: summarize_issues(&issues),
        issues,
        duration_ms: elapsed_millis(started),
        platform_log_path,
        stderr,
        log_read_warning,
    }
}

/// Вердикт проверки: код выхода инструмента и то, удалось ли прочитать его журнал.
///
/// Журнал, которого ждали и не прочитали, оставляет вердикт неизвестным, а неизвестность
/// называется отдельным значением, а не сводится к чистоте: проверка, чьи замечания никто
/// не прочитал, чистой не является, и зелёный CI на ней — худший из возможных ответов.
fn verdict(exit_code: i32, log_unreadable: bool) -> SyntaxCheckStatus {
    let status = status_from_exit_code(exit_code);
    // Помета только ужесточает: непрочитанный журнал превращает чистоту в сбой, но уже
    // известный вердикт не переписывает — про найденные замечания инструмент сказал
    // кодом выхода, и это знание не пропадает оттого, что подробностей не видно.
    if log_unreadable && status == SyntaxCheckStatus::Clean {
        return SyntaxCheckStatus::ToolFailed;
    }
    status
}

fn status_from_exit_code(exit_code: i32) -> SyntaxCheckStatus {
    match exit_code {
        0 => SyntaxCheckStatus::Clean,
        101 => SyntaxCheckStatus::IssuesFound,
        _ => SyntaxCheckStatus::ToolFailed,
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn summarize_issues(issues: &[Issue]) -> SyntaxIssueSummary {
    let mut summary = SyntaxIssueSummary {
        errors: 0,
        warnings: 0,
        info: 0,
    };

    for issue in issues {
        match issue_severity(issue) {
            IssueSeverity::Error => summary.errors += 1,
            IssueSeverity::Warning => summary.warnings += 1,
            IssueSeverity::Info => summary.info += 1,
        }
    }

    summary
}

fn issue_severity(issue: &Issue) -> &IssueSeverity {
    match issue {
        Issue::Module(issue) => &issue.severity,
        Issue::Object(issue) => &issue.severity,
        Issue::Edt(issue) => &issue.severity,
    }
}

fn fallback_issue(
    exit_code: i32,
    stderr: Option<&str>,
    log_read_warning: Option<&str>,
    platform_log_path: Option<&Path>,
) -> Issue {
    let message = if let Some(log_read_warning) = log_read_warning {
        format!(
            "Designer exited with code {exit_code}; no parseable issues found; /Out log unreadable: {log_read_warning}"
        )
    } else if let Some(stderr) = stderr.filter(|stderr| !stderr.trim().is_empty()) {
        format!(
            "Designer exited with code {exit_code}; no parseable issues found; stderr: {}",
            stderr.trim()
        )
    } else if let Some(path) = platform_log_path {
        format!(
            "Designer exited with code {exit_code}; no parseable issues found in /Out log '{}'",
            path.display()
        )
    } else {
        format!("Designer exited with code {exit_code}; no parseable issues found")
    };

    Issue::Object(ObjectIssue {
        object: "Designer".to_owned(),
        message,
        severity: IssueSeverity::Error,
    })
}

fn fallback_edt_issue(
    project_name: &str,
    exit_code: i32,
    stderr: Option<&str>,
    log_read_warning: Option<&str>,
    platform_log_path: Option<&Path>,
) -> Issue {
    let message = if let Some(log_read_warning) = log_read_warning {
        format!(
            "EDT check for project '{project_name}' exited with code {exit_code}; no parseable issues found; --file log unreadable: {log_read_warning}"
        )
    } else if let Some(stderr) = stderr.filter(|stderr| !stderr.trim().is_empty()) {
        format!(
            "EDT check for project '{project_name}' exited with code {exit_code}; no parseable issues found; stderr: {}",
            stderr.trim()
        )
    } else if let Some(path) = platform_log_path {
        format!(
            "EDT check for project '{project_name}' exited with code {exit_code}; no parseable issues found in --file log '{}'",
            path.display()
        )
    } else {
        format!(
            "EDT check for project '{project_name}' exited with code {exit_code}; no parseable issues found"
        )
    };

    Issue::Edt(EdtIssue {
        path: project_name.to_owned(),
        line: None,
        column: None,
        message,
        severity: IssueSeverity::Error,
        check: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        edt_status_from_result, execute, normalize_config_flags, run_syntax, status_from_exit_code,
    };
    use crate::config::model::{
        AppConfig, BuildConfig, SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig,
        ToolsConfig,
    };
    use crate::domain::issue::{Issue, IssueSeverity};
    use crate::domain::syntax::{CheckName, SyntaxCheckStatus};
    use crate::use_cases::context::{CommandName, ExecutionContext};
    use crate::use_cases::request::{
        DesignerClientScope, DesignerClientScopes, DesignerConfigChecks,
        DesignerConfigSyntaxRequest as DesignerConfigSyntaxArgs, ExtendedModulesPolicy,
        SyntaxExtensionScope, SyntaxRequest as SyntaxArgs, SyntaxTargetRequest as SyntaxTarget,
    };
    use crate::use_cases::result::UseCaseErrorKind;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tempfile::tempdir;

    /// DEC.2026-09-12.A-LABEL-MAY-ONLY-MAKE-A-VERDICT-STRICTER admits prose as a *label* on a finding, never as a verdict, and that admission
    /// rests on three properties. Two of them are proven here; the third — that the verdict comes
    /// from the exit code — is `status_from_exit_code` having no other input.
    #[test]
    fn labels_can_only_make_a_verdict_stricter() {
        // Designer: the verdict is the exit code and nothing else. No text reaches it, so no
        // wording can turn a failure into a pass.
        assert_eq!(status_from_exit_code(0), SyntaxCheckStatus::Clean);
        assert_eq!(status_from_exit_code(101), SyntaxCheckStatus::IssuesFound);
        assert_eq!(status_from_exit_code(1), SyntaxCheckStatus::ToolFailed);
        assert_eq!(status_from_exit_code(-1), SyntaxCheckStatus::ToolFailed);

        // EDT: findings may only tighten the answer. Recognising nothing keeps the exit code's
        // verdict; recognising something can add `IssuesFound` but never `Clean`.
        let finding = vec![Issue::Object(crate::domain::issue::ObjectIssue {
            object: "Catalogs.Items".to_owned(),
            message: "unreadable wording".to_owned(),
            severity: IssueSeverity::Error,
        })];
        assert_eq!(
            edt_status_from_result(0, &[], false),
            SyntaxCheckStatus::Clean,
            "nothing recognised and the tool is happy: the exit code decides"
        );
        assert_eq!(
            edt_status_from_result(0, &finding, false),
            SyntaxCheckStatus::IssuesFound,
            "a recognised finding may only tighten the verdict"
        );
        assert_eq!(
            edt_status_from_result(7, &[], false),
            SyntaxCheckStatus::ToolFailed,
            "nothing recognised and the tool failed: still a failure, never a pass"
        );
        assert_eq!(
            edt_status_from_result(7, &finding, false),
            SyntaxCheckStatus::IssuesFound
        );
    }

    /// The unsafe side is the default: a line whose severity nobody recognises is an error.
    #[test]
    fn an_unrecognised_severity_is_an_error_not_a_warning() {
        let issues = crate::parsers::designer_validation::parse(
            "Catalogs.Items Ein unbekannter Fehlertext ohne bekannte Marker\n",
        );
        for issue in &issues {
            let severity = match issue {
                Issue::Module(issue) => &issue.severity,
                Issue::Object(issue) => &issue.severity,
                Issue::Edt(issue) => &issue.severity,
            };
            assert_eq!(
                severity,
                &IssueSeverity::Error,
                "an unreadable label must fall to the unsafe side"
            );
        }
    }

    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut perms = fs::metadata(path).expect("metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).expect("chmod");
        }

        #[cfg(not(unix))]
        let _ = path;
    }

    fn write_script(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write");
        make_executable(path);
    }

    fn utility_path(dir: &Path, name: &str) -> PathBuf {
        if cfg!(windows) {
            dir.join(format!("{name}.exe"))
        } else {
            dir.join(name)
        }
    }

    fn write_designer_script(
        path: &Path,
        log_body: Option<&str>,
        stderr: Option<&str>,
        exit_code: i32,
    ) {
        let log_branch = log_body
            .map(|body| format!("if [ -n \"$out\" ]; then cat <<'LOG' > \"$out\"\n{body}\nLOG\nfi"))
            .unwrap_or_default();
        let stderr_branch = stderr
            .map(|stderr| format!("printf '%s\\n' '{}' >&2", stderr.replace('\'', "'\\''")))
            .unwrap_or_default();
        let body = format!(
            "out=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\n{log_branch}\n{stderr_branch}\nexit {exit_code}"
        );
        write_script(path, &body);
    }

    fn write_edt_script(
        path: &Path,
        check_log_body: Option<&str>,
        stderr: Option<&str>,
        exit_code: i32,
    ) {
        let log_branch = check_log_body
            .map(|body| format!("if [ -n \"$out\" ]; then cat <<'LOG' > \"$out\"\n{body}\nLOG\nfi"))
            .unwrap_or_default();
        let stderr_branch = stderr
            .map(|stderr| format!("printf '%s\\n' '{}' >&2", stderr.replace('\'', "'\\''")))
            .unwrap_or_default();
        let body = format!(
            "out=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"--file\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\n{log_branch}\n{stderr_branch}\nexit {exit_code}"
        );
        write_script(path, &body);
    }

    fn write_edt_script_with_calls(path: &Path, calls_log: &Path) {
        let body = format!(
            "out=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"--file\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nprintf '%s\\n' \"$*\" >> '{}'\nif [ -n \"$out\" ]; then : > \"$out\"; fi\nexit 0",
            calls_log.display()
        );
        write_script(path, &body);
    }

    #[cfg(unix)]
    fn write_interactive_edt_script_with_calls(path: &Path, calls_log: &Path) {
        let body = format!(
            "set -eu\n\
             prompt() {{ printf '1C:EDT>'; }}\n\
             current_dir=\"\"\n\
             prev=\"\"\n\
             for arg in \"$@\"; do\n\
               if [ \"$prev\" = \"-data\" ]; then current_dir=\"$arg\"; fi\n\
               prev=\"$arg\"\n\
             done\n\
             printf 'START\\n' >> '{}'\n\
             trap 'printf \"EXIT\\\\n\" >> \"{}\"' EXIT\n\
             prompt\n\
             while IFS= read -r line; do\n\
               printf '%s\\n' \"$line\" >> '{}'\n\
               eval \"set -- $line\"\n\
               cmd=\"${{1:-}}\"\n\
               if [ \"$#\" -gt 0 ]; then shift; fi\n\
               case \"$cmd\" in\n\
                 cd)\n\
                   if [ \"$#\" -eq 0 ]; then\n\
                     printf '%s\\n' \"$current_dir\"\n\
                   else\n\
                     current_dir=\"$1\"\n\
                   fi\n\
                   prompt\n\
                   ;;\n\
                 validate)\n\
                   out=\"\"\n\
                   prev=\"\"\n\
                   for arg in \"$@\"; do\n\
                     if [ \"$prev\" = \"--file\" ]; then out=\"$arg\"; fi\n\
                     prev=\"$arg\"\n\
                   done\n\
                   if [ -n \"$out\" ]; then : > \"$out\"; fi\n\
                   prompt\n\
                   ;;\n\
                 *)\n\
                   prompt\n\
                   ;;\n\
               esac\n\
             done\n",
            calls_log.display(),
            calls_log.display(),
            calls_log.display()
        );
        write_script(path, &body);
    }

    fn sample_config(base_path: &Path, work_path: &Path, platform_path: &Path) -> AppConfig {
        AppConfig {
            base_path: base_path.to_path_buf(),
            work_path: work_path.to_path_buf(),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: vec![SourceSetConfig {
                name: "main".to_owned(),
                purpose: SourceSetPurpose::Configuration,
                path: Path::new(".").to_path_buf(),
            }],
            build: BuildConfig::default(),
            tools: ToolsConfig {
                platform: crate::config::model::PlatformToolConfig {
                    path: Some(platform_path.to_path_buf()),
                    strict: false,
                    version: None,
                },
                enterprise: Default::default(),
                edt_cli: Default::default(),
                ..Default::default()
            },
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    fn sample_edt_config(base_path: &Path, work_path: &Path, edt_cli_path: &Path) -> AppConfig {
        AppConfig {
            base_path: base_path.to_path_buf(),
            work_path: work_path.to_path_buf(),
            format: SourceFormat::Edt,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            source_sets: vec![
                SourceSetConfig {
                    name: "main".to_owned(),
                    purpose: SourceSetPurpose::Configuration,
                    path: Path::new("main-edt").to_path_buf(),
                },
                SourceSetConfig {
                    name: "ext".to_owned(),
                    purpose: SourceSetPurpose::Extension,
                    path: Path::new("ext-edt").to_path_buf(),
                },
            ],
            build: BuildConfig::default(),
            tools: ToolsConfig {
                platform: Default::default(),
                enterprise: Default::default(),
                edt_cli: crate::config::model::EdtCliConfig {
                    path: Some(edt_cli_path.to_path_buf()),
                    auto_start: false,
                    ..Default::default()
                },
                ..Default::default()
            },
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    #[test]
    fn status_mapping_matches_designer_exit_codes() {
        assert_eq!(status_from_exit_code(0), SyntaxCheckStatus::Clean);
        assert_eq!(status_from_exit_code(101), SyntaxCheckStatus::IssuesFound);
        assert_eq!(status_from_exit_code(1), SyntaxCheckStatus::ToolFailed);
    }

    #[test]
    fn normalizes_config_flags() {
        let args = DesignerConfigSyntaxArgs::new(
            DesignerConfigChecks::default(),
            DesignerClientScopes::new([
                DesignerClientScope::ThinClient,
                DesignerClientScope::Server,
            ]),
            ExtendedModulesPolicy::basic(false),
            SyntaxExtensionScope::SingleExtension {
                name: "Ext".to_owned(),
            },
        );
        let flags = normalize_config_flags(&args);

        assert_eq!(flags, vec!["-ThinClient", "-Server", "-Extension", "Ext"]);
    }

    /// Режимы проверки модулей выполняет та же `/CheckConfig`: проверок конфигурации в
    /// таком запросе нет, а сам набор режимов доезжает до платформы прежним.
    #[test]
    fn module_modes_are_checked_by_check_config() {
        let args = DesignerConfigSyntaxArgs::new(
            DesignerConfigChecks::new([]),
            DesignerClientScopes::new([DesignerClientScope::Server]),
            ExtendedModulesPolicy::basic(true),
            SyntaxExtensionScope::AllExtensions,
        );
        let flags = normalize_config_flags(&args);

        assert_eq!(
            flags,
            vec!["-Server", "-ExtendedModulesCheck", "-AllExtensions"]
        );
    }

    /// Пустой запрос выполняет профиль по умолчанию: пустая `/CheckConfig` не проверяет
    /// ничего и отвечает «чисто», а команда обещает проверку.
    #[test]
    fn a_request_without_modes_runs_the_default_profile() {
        let args = DesignerConfigSyntaxArgs::new(
            DesignerConfigChecks::new([]),
            DesignerClientScopes::default(),
            ExtendedModulesPolicy::basic(false),
            SyntaxExtensionScope::MainConfiguration,
        );
        assert!(args.names_no_mode());

        let profile =
            DesignerConfigSyntaxArgs::default_profile(SyntaxExtensionScope::MainConfiguration);
        let flags = normalize_config_flags(&profile);

        assert_eq!(
            flags,
            vec![
                "-ThinClient",
                "-Server",
                "-UnreferenceProcedures",
                "-HandlersExistence",
                "-EmptyHandlers",
                "-ExtendedModulesCheck"
            ]
        );
    }

    #[test]
    fn unsupported_matrix_returns_validation_failure_without_fake_issue() {
        let dir = tempdir().expect("tempdir");
        let mut config = sample_config(dir.path(), dir.path(), dir.path());
        config.format = SourceFormat::Edt;
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::DesignerConfig(default_config_args()),
        };

        let error = run_syntax(&config, &args).expect_err("expected failure");
        let kind = error.error.kind();
        let result = error
            .payload
            .expect("syntax validation failures should preserve a structured payload");

        assert_eq!(kind, UseCaseErrorKind::Validation);
        assert!(result.issues.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn clean_exit_returns_clean_status() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let binary = utility_path(&dir.path().join("platform").join("bin"), "1cv8");
        fs::create_dir_all(&base).expect("base");
        fs::create_dir_all(&work).expect("work");
        // Чистый прогон Конфигуратора журнал всё-таки пишет — пустым. Фейк без журнала
        // изображал бы не чистоту, а потерю вердикта, и с 2026-09-17 это сбой, а не успех.
        write_designer_script(&binary, Some(""), None, 0);
        let config = sample_config(&base, &work, &dir.path().join("platform"));
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::DesignerConfig(default_config_args()),
        };

        let result = run_syntax(&config, &args).expect("clean run");

        assert_eq!(result.status, SyntaxCheckStatus::Clean);
        assert_eq!(result.exit_code, 0);
        assert!(result.log_read_warning.is_none());
    }

    /// Инструмент вышел нулём, но журнал, в который он пишет замечания, прочитать не
    /// удалось. Вердикта нет — и чистотой он не становится: иначе CI зеленел бы на
    /// проверке, чьих замечаний никто не видел.
    #[cfg(unix)]
    #[test]
    fn a_clean_exit_with_an_unreadable_log_is_not_clean() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let binary = utility_path(&dir.path().join("platform").join("bin"), "1cv8");
        fs::create_dir_all(&base).expect("base");
        fs::create_dir_all(&work).expect("work");
        write_designer_script(&binary, None, None, 0);
        let config = sample_config(&base, &work, &dir.path().join("platform"));
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::DesignerConfig(default_config_args()),
        };

        let failure = run_syntax(&config, &args).expect_err("an unread verdict is not a success");
        let result = failure
            .payload
            .expect("syntax failures should preserve a structured payload");

        assert_eq!(result.status, SyntaxCheckStatus::ToolFailed);
        assert!(result.log_read_warning.is_some());
        assert_eq!(
            result.issues.len(),
            1,
            "the refusal must name why the verdict is unknown"
        );
    }

    #[cfg(unix)]
    #[test]
    fn validation_exit_preserves_parsed_issues() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let binary = utility_path(&dir.path().join("platform").join("bin"), "1cv8");
        fs::create_dir_all(&base).expect("base");
        fs::create_dir_all(&work).expect("work");
        write_designer_script(
            &binary,
            Some("{CommonModules.TestModule(7,2)}: Ошибка компиляции\n{1}: context"),
            None,
            101,
        );
        let config = sample_config(&base, &work, &dir.path().join("platform"));
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::DesignerConfig(DesignerConfigSyntaxArgs::new(
                DesignerConfigChecks::new([]),
                DesignerClientScopes::new([DesignerClientScope::Server]),
                ExtendedModulesPolicy::basic(false),
                SyntaxExtensionScope::MainConfiguration,
            )),
        };

        let failure = run_syntax(&config, &args).expect_err("expected validation failure");
        let result = failure
            .payload
            .expect("syntax validation failures should preserve a structured payload");

        assert_eq!(result.status, SyntaxCheckStatus::IssuesFound);
        assert_eq!(result.exit_code, 101);
        assert_eq!(result.issues.len(), 1);
        match &result.issues[0] {
            Issue::Module(issue) => assert_eq!(issue.path, "CommonModules.TestModule"),
            _ => panic!("expected module issue"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn tool_failure_preserves_stderr_and_fallback_issue() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let binary = utility_path(&dir.path().join("platform").join("bin"), "1cv8");
        fs::create_dir_all(&base).expect("base");
        fs::create_dir_all(&work).expect("work");
        write_designer_script(&binary, None, Some("license error"), 1);
        let config = sample_config(&base, &work, &dir.path().join("platform"));
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::DesignerConfig(DesignerConfigSyntaxArgs::new(
                DesignerConfigChecks::new([]),
                DesignerClientScopes::new([DesignerClientScope::Server]),
                ExtendedModulesPolicy::basic(false),
                SyntaxExtensionScope::MainConfiguration,
            )),
        };

        let failure = run_syntax(&config, &args).expect_err("expected tool failure");
        let result = failure
            .payload
            .expect("syntax tool failures should preserve a structured payload");

        assert_eq!(result.status, SyntaxCheckStatus::ToolFailed);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.issues.len(), 1);
        assert!(result
            .stderr
            .as_deref()
            .expect("stderr")
            .contains("license error"));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_out_log_keeps_structured_failure() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let binary = utility_path(&dir.path().join("platform").join("bin"), "1cv8");
        fs::create_dir_all(&base).expect("base");
        fs::create_dir_all(&work).expect("work");
        write_script(&binary, "exit 101");
        let config = sample_config(&base, &work, &dir.path().join("platform"));
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::DesignerConfig(DesignerConfigSyntaxArgs::new(
                DesignerConfigChecks::new([]),
                DesignerClientScopes::new([DesignerClientScope::Server]),
                ExtendedModulesPolicy::basic(false),
                SyntaxExtensionScope::MainConfiguration,
            )),
        };

        let failure = run_syntax(&config, &args).expect_err("expected failure");
        let result = failure
            .payload
            .expect("syntax failures should preserve a structured payload");

        assert_eq!(result.status, SyntaxCheckStatus::IssuesFound);
        assert!(result.log_read_warning.is_some());
        assert_eq!(result.issues.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn syntax_edt_runs_all_source_sets_when_projects_not_specified() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let main_dir = base.join("main-edt");
        let ext_dir = base.join("ext-edt");
        let binary = utility_path(&dir.path().join("edt"), "1cedtcli");
        fs::create_dir_all(&work).expect("work");
        fs::create_dir_all(&main_dir).expect("main");
        fs::create_dir_all(&ext_dir).expect("ext");
        write_edt_script(
            &binary,
            Some("ERROR\tCommonModules.Test\t1\t1\tCheck\tmessage"),
            None,
            1,
        );
        let config = sample_edt_config(&base, &work, &binary);
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::Edt { projects: vec![] },
        };

        let failure = run_syntax(&config, &args).expect_err("expected issues");
        let result = failure
            .payload
            .expect("syntax EDT failures should preserve a structured payload");

        assert_eq!(result.check_name, CheckName::Edt);
        assert_eq!(result.status, SyntaxCheckStatus::IssuesFound);
        assert_eq!(result.summary.errors, 2);
        assert!(result.platform_log_path.is_none());
    }

    #[test]
    fn syntax_edt_rejects_unknown_project_names() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let main_dir = base.join("main-edt");
        let ext_dir = base.join("ext-edt");
        let binary = utility_path(&dir.path().join("edt"), "1cedtcli");
        fs::create_dir_all(&work).expect("work");
        fs::create_dir_all(&main_dir).expect("main");
        fs::create_dir_all(&ext_dir).expect("ext");
        write_edt_script(&binary, None, None, 0);
        let config = sample_edt_config(&base, &work, &binary);
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::Edt {
                projects: vec!["unknown".to_owned()],
            },
        };

        let failure = run_syntax(&config, &args).expect_err("expected validation failure");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Validation);
        assert!(failure
            .error
            .to_string()
            .contains("unknown EDT project(s): unknown"));
    }

    #[cfg(unix)]
    #[test]
    fn syntax_edt_prefers_tool_failed_exit_code_in_aggregate() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let main_dir = base.join("main-edt");
        let ext_dir = base.join("ext-edt");
        let binary = utility_path(&dir.path().join("edt"), "1cedtcli");
        fs::create_dir_all(&work).expect("work");
        fs::create_dir_all(&main_dir).expect("main");
        fs::create_dir_all(&ext_dir).expect("ext");
        write_script(
            &binary,
            "out=\"\"\nargs=\"$*\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"--file\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif printf '%s' \"$args\" | grep -q -- 'main-edt'; then\n  if [ -n \"$out\" ]; then printf 'ERROR\\tCatalogs.Items\\t1\\t1\\tRule\\tmsg\\n' > \"$out\"; fi\n  exit 1\nfi\nexit 17",
        );
        let config = sample_edt_config(&base, &work, &binary);
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::Edt { projects: vec![] },
        };

        let failure = run_syntax(&config, &args).expect_err("expected failure");
        let result = failure
            .payload
            .expect("syntax EDT failures should preserve a structured payload");

        assert_eq!(result.status, SyntaxCheckStatus::ToolFailed);
        assert_eq!(result.exit_code, 17);
    }

    #[cfg(unix)]
    #[test]
    fn syntax_edt_uses_mcp_timeout_budget_for_subprocess() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let main_dir = base.join("main-edt");
        let ext_dir = base.join("ext-edt");
        let binary = utility_path(&dir.path().join("edt"), "1cedtcli");
        fs::create_dir_all(&work).expect("work");
        fs::create_dir_all(&main_dir).expect("main");
        fs::create_dir_all(&ext_dir).expect("ext");
        write_script(&binary, "sleep 1\nexit 0");
        let mut config = sample_edt_config(&base, &work, &binary);
        config.tools.edt_cli.command_timeout_ms = 20;
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::Edt {
                projects: vec!["main".to_owned()],
            },
        };
        let context = ExecutionContext::mcp_stdio(CommandName::Syntax)
            .with_edt_timeout(Some(Duration::from_millis(20)));

        let failure = execute(&context, &config, &args).expect_err("expected timeout");
        let message = failure.error.to_string();
        let payload = failure
            .payload
            .expect("syntax EDT failures should preserve a structured payload");

        assert!(message.contains("timed out"));
        assert_eq!(payload.status, SyntaxCheckStatus::ToolFailed);
        assert_eq!(payload.exit_code, -1);
    }

    #[cfg(unix)]
    #[test]
    fn syntax_edt_bounds_each_one_shot_project_by_the_edt_step_cap() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let main_dir = base.join("main-edt");
        let ext_dir = base.join("ext-edt");
        let binary = utility_path(&dir.path().join("edt"), "1cedtcli");
        fs::create_dir_all(&work).expect("work");
        fs::create_dir_all(&main_dir).expect("main");
        fs::create_dir_all(&ext_dir).expect("ext");
        write_script(&binary, "sleep 0.06\nexit 0");
        let config = sample_edt_config(&base, &work, &binary);
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::Edt { projects: vec![] },
        };
        // Запас нарочно большой: предел шага здесь свой у каждого проекта и ни от чего
        // не убывает, поэтому 20 мс против sleep 0.06 срабатывают детерминированно.
        let context = ExecutionContext::mcp_stdio(CommandName::Syntax)
            .with_edt_timeout(Some(Duration::from_millis(20)));

        let failure = execute(&context, &config, &args).expect_err("expected timeout");
        let message = failure.error.to_string();
        let payload = failure
            .payload
            .expect("syntax EDT failures should preserve a structured payload");

        assert!(message.contains("timed out") || message.contains("timeout expired"));
        assert_eq!(payload.status, SyntaxCheckStatus::ToolFailed);
        assert_eq!(payload.exit_code, -1);
    }

    #[cfg(unix)]
    #[test]
    fn syntax_edt_uses_one_shot_execution_when_interactive_mode_is_disabled() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let main_dir = base.join("main-edt");
        let ext_dir = base.join("ext-edt");
        let binary = utility_path(&dir.path().join("edt"), "1cedtcli");
        let calls_log = dir.path().join("edt-calls.log");
        fs::create_dir_all(&work).expect("work");
        fs::create_dir_all(&main_dir).expect("main");
        fs::create_dir_all(&ext_dir).expect("ext");
        write_edt_script_with_calls(&binary, &calls_log);
        let mut config = sample_edt_config(&base, &work, &binary);
        config.tools.edt_cli.interactive_mode = false;
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::Edt {
                projects: vec!["main".to_owned()],
            },
        };

        let result = run_syntax(&config, &args).expect("syntax");
        let calls = fs::read_to_string(&calls_log).expect("calls log");

        assert_eq!(result.status, SyntaxCheckStatus::Clean);
        assert!(calls.contains("-command validate"));
        assert!(!calls.contains("START"));
    }

    #[cfg(unix)]
    #[test]
    fn syntax_edt_uses_shared_session_execution_when_interactive_mode_is_enabled() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work = dir.path().join("work");
        let main_dir = base.join("main-edt");
        let ext_dir = base.join("ext-edt");
        let binary = utility_path(&dir.path().join("edt"), "1cedtcli");
        let calls_log = dir.path().join("edt-calls.log");
        fs::create_dir_all(&work).expect("work");
        fs::create_dir_all(&main_dir).expect("main");
        fs::create_dir_all(&ext_dir).expect("ext");
        write_interactive_edt_script_with_calls(&binary, &calls_log);
        let mut config = sample_edt_config(&base, &work, &binary);
        config.tools.edt_cli.interactive_mode = true;
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::Edt {
                projects: vec!["main".to_owned()],
            },
        };

        let result = run_syntax(&config, &args).expect("syntax");
        let calls = fs::read_to_string(&calls_log).expect("calls log");

        assert_eq!(result.status, SyntaxCheckStatus::Clean);
        assert_eq!(calls.matches("START").count(), 1);
        assert_eq!(calls.matches("EXIT").count(), 1);
        assert!(calls.contains("validate"));
    }

    #[test]
    fn log_directory_creation_failure_is_reported_before_spawn() {
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("base");
        let work_file = dir.path().join("work-file");
        let binary = utility_path(&dir.path().join("platform").join("bin"), "1cv8");
        fs::create_dir_all(&base).expect("base");
        fs::write(&work_file, "not a directory").expect("work file");
        write_designer_script(&binary, None, None, 0);
        let config = sample_config(&base, &work_file, &dir.path().join("platform"));
        let args = SyntaxArgs {
            dry_run: false,
            target: SyntaxTarget::DesignerConfig(default_config_args()),
        };

        let failure = run_syntax(&config, &args).expect_err("expected failure");
        let message = failure.error.to_string();
        let result = failure
            .payload
            .expect("syntax failures should preserve a structured payload");

        assert_eq!(result.status, SyntaxCheckStatus::ToolFailed);
        assert!(message.contains("failed to prepare syntax platform logs directory"));
    }

    fn default_config_args() -> DesignerConfigSyntaxArgs {
        DesignerConfigSyntaxArgs::new(
            DesignerConfigChecks::default(),
            DesignerClientScopes::default(),
            ExtendedModulesPolicy::basic(false),
            SyntaxExtensionScope::MainConfiguration,
        )
    }
}
