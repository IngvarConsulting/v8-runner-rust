use std::time::Instant;

use crate::config::model::{AppConfig, SourceSetPurpose};
use crate::domain::capability::Provider;
use crate::domain::extensions::{ExtensionsResult, ExtensionsStep};
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl, IbcmdError};
use crate::platform::locator::UtilityType;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::extension_agent::ExtensionAgent;
use crate::use_cases::extension_identity::platform_extension_name;
use crate::use_cases::ibcmd_diagnostics::format_ibcmd_failure_details;
use crate::use_cases::interruption;
use crate::use_cases::progress::log_live_stage;
use crate::use_cases::request::ConfigureExtensionsRequest;
use crate::use_cases::result::{UseCaseFailure, UseCaseResult};
use tracing::{debug, info};

const DISABLE_SAFETY_ACTION: &str = "disable_safety";
const EXTENSIONS_SUCCESS_LABEL: &str = "Extension properties updated successfully";
const EXTENSIONS_FAILURE_LABEL: &str = "Extension property update failed";

pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &ConfigureExtensionsRequest,
) -> UseCaseResult<ExtensionsResult> {
    debug!(
        command = context.command().as_str(),
        transport = ?context.transport(),
        "executing configure extensions use case"
    );
    let started = Instant::now();
    let targets = match resolve_targets(config, args) {
        Ok(targets) => targets,
        Err(error) => {
            return Err(UseCaseFailure::without_payload(error));
        }
    };

    // Подключение `ibcmd` строится только для него: у автономного сервера строки
    // подключения нет, туда ходит шлюз.
    let connection = if config.infobase.standalone.is_some() {
        None
    } else {
        match IbcmdConnection::from_infobase(&config.infobase) {
            Ok(connection) => Some(connection),
            Err(error) => {
                return Err(UseCaseFailure::without_payload(AppError::from(error)));
            }
        }
    };

    let mut utilities = PlatformUtilities::from_config(config);
    let selected = match crate::use_cases::provider_selection::select(
        config,
        &mut utilities,
        crate::domain::capability::Operation::Extensions,
    ) {
        Ok(selected) => selected,
        Err((error, receipt)) => {
            let mut result = ExtensionsResult {
                provider: None,
                ok: false,
                provider_dispatched: false,
                steps: Vec::new(),
                duration_ms: started.elapsed().as_millis() as u64,
            };
            result.provider = Some(receipt);
            return Err(UseCaseFailure::with_payload(error, result));
        }
    };
    let receipt = selected.receipt;
    // Превью называет цели и исполнителя и ничего не трогает: установленное состояние
    // расширения не опрашивается, потому что опрос — это уже запуск платформы.
    if args.dry_run {
        let executor = selected
            .location
            .as_ref()
            .map(|location| location.path.display().to_string())
            .unwrap_or_else(|| "the designer agent".to_owned());
        let target_name = match connection.as_ref() {
            Some(connection) => connection.describe_target(),
            None => "the infobase".to_owned(),
        };
        return Ok(ExtensionsResult {
            provider: Some(receipt),
            provider_dispatched: false,
            ok: true,
            steps: targets
                .into_iter()
                .map(|target| ExtensionsStep {
                    message: Some(format!(
                        "would disable safe mode and unsafe action protection for '{target}' in {target_name} via {executor}; installed state is not probed"
                    )),
                    target,
                    action: DISABLE_SAFETY_ACTION.to_owned(),
                    ok: true,
                    duration_ms: 0,
                })
                .collect(),
            duration_ms: started.elapsed().as_millis() as u64,
        });
    }

    let mut setter = match (selected.provider, selected.location) {
        (Provider::Agent, location) => {
            match ExtensionAgent::open(context, config, location.map(|l| l.path).as_deref()) {
                Ok(agent) => SafetySetter::Agent(Box::new(agent)),
                Err(error) => return Err(UseCaseFailure::without_payload(error)),
            }
        }
        (_, Some(location)) => SafetySetter::Ibcmd(Box::new(
            IbcmdDsl::new(
                location.path,
                match connection {
                    Some(connection) => connection,
                    None => {
                        return Err(UseCaseFailure::without_payload(AppError::Runtime(
                            "ibcmd was selected for a target without a connection string"
                                .to_owned(),
                        )))
                    }
                },
                utilities.runner_for(UtilityType::Ibcmd),
            )
            .with_execution_policy(
                context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None),
            ),
        )),
        (provider, None) => {
            return Err(UseCaseFailure::without_payload(
                crate::use_cases::unimplemented_provider(
                    crate::domain::capability::Operation::Extensions,
                    provider,
                ),
            ));
        }
    };

    let outcome = disable_safety_for(context, &mut setter, targets, started);
    setter.close();
    outcome.map(|steps| ExtensionsResult {
        provider: Some(receipt),
        provider_dispatched: true,
        ok: true,
        duration_ms: started.elapsed().as_millis() as u64,
        steps,
    })
}

/// Исполнитель одного и того же действия: `ibcmd` — процессом на цель, агент — одной
/// сессией на команду.
// Обе ветки тяжёлые — сессия агента и DSL ibcmd, — и без упаковки размер большей
// платил бы каждый экземпляр независимо от того, какой исполнитель выбран.
enum SafetySetter<'a> {
    Ibcmd(Box<IbcmdDsl<'a>>),
    Agent(Box<ExtensionAgent>),
}

impl SafetySetter<'_> {
    /// Итог шага: удача с сообщением, отказ платформы с уликой или ошибка раннера.
    fn disable(&mut self, target: &str) -> Result<String, StepFailure> {
        match self {
            Self::Ibcmd(dsl) => {
                match dsl.infobase_extension_update_properties(target, false, false) {
                    Ok(result) if result.process.exit_code == 0 => {
                        let mut message =
                            "безопасный режим и защита от опасных действий отключены".to_owned();
                        if let Some(warning) = deferred_interruption_warning(&result) {
                            message.push_str("; ");
                            message.push_str(&warning);
                        }
                        Ok(message)
                    }
                    Ok(result) => Err(StepFailure::Platform(format_ibcmd_failure_details(
                        "extension update",
                        "extension",
                        target,
                        result.process.exit_code,
                        &result.process.stdout,
                        &result.process.stderr,
                        None,
                        None,
                    ))),
                    Err(error) => Err(StepFailure::Error(map_extension_update_error(
                        target, error,
                    ))),
                }
            }
            Self::Agent(agent) => agent
                .disable_safety(target)
                .map(|()| "безопасный режим и защита от опасных действий отключены".to_owned())
                .map_err(StepFailure::Error),
        }
    }

    fn close(self) {
        if let Self::Agent(agent) = self {
            agent.close();
        }
    }
}

enum StepFailure {
    Platform(String),
    Error(AppError),
}

fn disable_safety_for(
    context: &ExecutionContext,
    setter: &mut SafetySetter<'_>,
    targets: Vec<String>,
    started: Instant,
) -> Result<Vec<ExtensionsStep>, UseCaseFailure<ExtensionsResult>> {
    let mut steps = Vec::new();
    for target in targets {
        if let Some(interruption) = context.interruption() {
            let message = interruption::interruption_before_safe_point_message(
                context,
                interruption,
                "extension update",
            );
            let payload = ExtensionsResult {
                provider: None,
                provider_dispatched: true,
                ok: false,
                steps,
                duration_ms: started.elapsed().as_millis() as u64,
            };
            return Err(UseCaseFailure::with_payload(
                AppError::Runtime(message),
                payload,
            ));
        }
        let step_started = Instant::now();
        debug!(
            target = target.as_str(),
            "configuring extension safety flags"
        );
        log_extension_progress(
            &target,
            DISABLE_SAFETY_ACTION,
            "running",
            "updating extension properties",
        );
        match setter.disable(&target) {
            Ok(message) => {
                let step = ExtensionsStep {
                    target,
                    action: DISABLE_SAFETY_ACTION.to_owned(),
                    ok: true,
                    message: Some(message),
                    duration_ms: step_started.elapsed().as_millis() as u64,
                };
                log_extension_step(&step);
                steps.push(step);
            }
            Err(StepFailure::Platform(message)) => {
                let step = ExtensionsStep {
                    target: target.clone(),
                    action: DISABLE_SAFETY_ACTION.to_owned(),
                    ok: false,
                    message: Some(message.clone()),
                    duration_ms: step_started.elapsed().as_millis() as u64,
                };
                log_extension_step(&step);
                steps.push(step);
                log_extensions_summary(false);
                let payload = ExtensionsResult {
                    provider: None,
                    provider_dispatched: true,
                    ok: false,
                    steps,
                    duration_ms: started.elapsed().as_millis() as u64,
                };
                return Err(UseCaseFailure::with_payload(
                    AppError::Platform(message),
                    payload,
                ));
            }
            Err(StepFailure::Error(app_error)) => {
                let message = app_error.to_string();
                let step = ExtensionsStep {
                    target: target.clone(),
                    action: DISABLE_SAFETY_ACTION.to_owned(),
                    ok: false,
                    message: Some(message.clone()),
                    duration_ms: step_started.elapsed().as_millis() as u64,
                };
                log_extension_step(&step);
                steps.push(step);
                log_extensions_summary(false);
                let payload = ExtensionsResult {
                    provider: None,
                    provider_dispatched: true,
                    ok: false,
                    steps,
                    duration_ms: started.elapsed().as_millis() as u64,
                };
                return Err(UseCaseFailure::with_payload(app_error, payload));
            }
        }
    }

    log_extensions_summary(true);
    Ok(steps)
}

fn log_extension_step(step: &ExtensionsStep) {
    log_extension_progress(
        &step.target,
        &step.action,
        if step.ok { "succeeded" } else { "failed" },
        step.message.as_deref().unwrap_or("ok"),
    );
}

fn log_extension_progress(target: &str, action: &str, status: &str, detail: &str) {
    let label = format!("{target}: {action}");
    if status == "running" {
        log_live_stage(&label, detail);
        return;
    }

    info!(
        timeline_status = status,
        timeline_label = label.as_str(),
        timeline_detail = detail
    );
}

fn log_extensions_summary(ok: bool) {
    info!(
        timeline_status = if ok { "succeeded" } else { "failed" },
        timeline_label = if ok {
            EXTENSIONS_SUCCESS_LABEL
        } else {
            EXTENSIONS_FAILURE_LABEL
        },
    );
}

fn deferred_interruption_warning(
    result: &crate::platform::result::PlatformCommandResult,
) -> Option<String> {
    interruption::deferred_process_interruption_warning(
        "extension properties updated successfully",
        result,
    )
}

fn map_extension_update_error(target: &str, error: IbcmdError) -> AppError {
    AppError::from(error).with_context(format!(
        "ibcmd extension update failed for extension '{target}'"
    ))
}

/// Resolves and validates the entire selection before lock/cleanup or platform dispatch.
/// The execution boundary repeats this pure check for non-CLI callers.
pub(crate) fn resolve_targets(
    config: &AppConfig,
    args: &ConfigureExtensionsRequest,
) -> Result<Vec<String>, AppError> {
    let available = config
        .source_sets
        .iter()
        .filter(|source_set| source_set.purpose == SourceSetPurpose::Extension)
        .map(|source_set| {
            (
                source_set.name.as_str(),
                platform_extension_name(source_set).to_owned(),
            )
        })
        .collect::<Vec<_>>();

    if args.names.is_empty() && args.installed_names.is_empty() {
        return Ok(available.into_iter().map(|(_, name)| name).collect());
    }

    let mut targets = Vec::new();
    for requested in &args.names {
        let Some((_, resolved)) = available
            .iter()
            .find(|(name, _)| *name == requested.as_str())
        else {
            return Err(AppError::Validation(format!(
                "unknown extension source-set '{requested}'"
            )));
        };
        if !targets.contains(resolved) {
            targets.push(resolved.clone());
        }
    }
    for name in &args.installed_names {
        if name.trim().is_empty() || name.chars().any(char::is_control) || name.starts_with('-') {
            return Err(AppError::Validation(
                "installed extension name must be nonblank, contain no control characters, and not start with '-'".to_owned(),
            ));
        }
        if !targets.contains(name) {
            targets.push(name.clone());
        }
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::{execute, map_extension_update_error, resolve_targets};
    use crate::config::model::{
        AppConfig, BuildConfig, PlatformToolConfig, SourceFormat, SourceSetConfig,
        SourceSetPurpose, TestsConfig, ToolsConfig,
    };
    use crate::platform::ibcmd::IbcmdError;
    use crate::platform::process::ProcessError;
    use crate::support::error::AppError;
    use crate::use_cases::context::{CommandName, ExecutionContext};
    use crate::use_cases::request::ConfigureExtensionsRequest;
    use crate::use_cases::result::UseCaseErrorKind;
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

    #[cfg(unix)]
    fn write_script(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        make_executable(path);
    }

    fn sample_config(base: &Path, work: &Path, ibcmd_path: &Path) -> AppConfig {
        AppConfig {
            base_path: base.to_path_buf(),
            work_path: work.to_path_buf(),
            format: SourceFormat::Edt,
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
                    name: "client_mcp".to_owned(),
                    purpose: SourceSetPurpose::Extension,
                    path: PathBuf::from("exts/client-mcp"),
                },
            ],
            build: BuildConfig::default(),
            tools: ToolsConfig {
                platform: PlatformToolConfig {
                    path: Some(ibcmd_path.to_path_buf()),
                    strict: false,
                    version: None,
                },
                ..ToolsConfig::default()
            },
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    #[test]
    fn resolve_targets_uses_source_set_name_for_edt_extension_identity() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("exts").join("client-mcp")).expect("ext dir");
        fs::write(
            dir.path().join("exts").join("client-mcp").join(".project"),
            "<projectDescription><name>client-mcp-project</name></projectDescription>",
        )
        .expect("project file");
        let config = sample_config(dir.path(), dir.path(), Path::new("/tmp/ibcmd"));

        let targets =
            resolve_targets(&config, &ConfigureExtensionsRequest::default()).expect("targets");

        assert_eq!(targets, vec!["client_mcp"]);
    }

    #[test]
    fn installed_targets_do_not_require_source_sets_or_normalize_names() {
        let dir = tempdir().expect("tempdir");
        let mut config = sample_config(dir.path(), dir.path(), Path::new("/tmp/ibcmd"));
        config.source_sets.clear();
        let names = vec![
            "YAXUNIT".to_owned(),
            " Проба ".to_owned(),
            "yaxunit".to_owned(),
        ];
        let request = ConfigureExtensionsRequest {
            installed_names: names.clone(),
            ..Default::default()
        };
        assert_eq!(resolve_targets(&config, &request).expect("targets"), names);
    }

    #[test]
    fn explicit_selection_is_ordered_union_without_implicit_all() {
        let dir = tempdir().expect("tempdir");
        let config = sample_config(dir.path(), dir.path(), Path::new("/tmp/ibcmd"));
        let mut request = ConfigureExtensionsRequest {
            installed_names: vec!["YAXUNIT".to_owned()],
            ..Default::default()
        };
        assert_eq!(
            resolve_targets(&config, &request).expect("direct"),
            ["YAXUNIT"]
        );
        request.names = vec!["client_mcp".to_owned(), "client_mcp".to_owned()];
        request
            .installed_names
            .extend(["client_mcp".to_owned(), "YAXUNIT".to_owned()]);
        assert_eq!(
            resolve_targets(&config, &request).expect("mixed"),
            ["client_mcp", "YAXUNIT"]
        );
    }

    #[test]
    fn invalid_direct_names_and_unknown_source_sets_fail_validation() {
        let dir = tempdir().expect("tempdir");
        let config = sample_config(dir.path(), dir.path(), Path::new("/tmp/ibcmd"));
        for name in ["", "  ", "\t", "YAX\nUNIT", "YAX\0UNIT", "--all"] {
            let request = ConfigureExtensionsRequest {
                installed_names: vec!["YAXUNIT".to_owned(), name.to_owned()],
                ..Default::default()
            };
            assert!(matches!(
                resolve_targets(&config, &request),
                Err(AppError::Validation(_))
            ));
        }
        for name in ["YAXUNIT", "configuration"] {
            let request = ConfigureExtensionsRequest {
                names: vec![name.to_owned()],
                installed_names: vec![name.to_owned()],
                ..Default::default()
            };
            let error = resolve_targets(&config, &request).expect_err("unknown source-set");
            assert!(error.to_string().contains("unknown extension source-set"));
        }
    }

    #[test]
    fn extension_update_spawn_failure_preserves_typed_process_context() {
        let error = map_extension_update_error(
            "client_mcp",
            IbcmdError::Spawn(ProcessError::SpawnFailed {
                cmd: "ibcmd extension update".to_owned(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing ibcmd"),
            }),
        );

        assert!(error
            .to_string()
            .contains("ibcmd extension update failed for extension 'client_mcp'"));
        assert!(matches!(
            error,
            AppError::PlatformProcessContext {
                source: ProcessError::SpawnFailed { .. },
                ..
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn execute_updates_extension_properties_via_ibcmd() {
        let dir = tempdir().expect("tempdir");
        let calls = dir.path().join("ibcmd.calls.log");
        let ibcmd = dir.path().join("ibcmd");
        fs::create_dir_all(dir.path().join("exts").join("client-mcp")).expect("ext dir");
        fs::write(
            dir.path().join("exts").join("client-mcp").join(".project"),
            "<projectDescription><name>client_mcp</name></projectDescription>",
        )
        .expect("project file");
        write_script(
            &ibcmd,
            &format!("printf '%s\\n' \"$*\" >> '{}'\nexit 0", calls.display()),
        );
        let config = sample_config(dir.path(), dir.path(), &ibcmd);

        let result = execute(
            &ExecutionContext::cli(CommandName::Extensions),
            &config,
            &ConfigureExtensionsRequest::default(),
        )
        .expect("execute");

        assert!(result.ok);
        let calls_text = fs::read_to_string(calls).expect("calls");
        assert!(calls_text.contains("extension update"));
        assert!(calls_text.contains("--name client_mcp"));
        assert!(calls_text.contains("--safe-mode no"));
        assert!(calls_text.contains("--unsafe-action-protection no"));
    }

    #[cfg(unix)]
    #[test]
    fn execute_updates_extension_properties_via_ibcmd_server_contract() {
        let dir = tempdir().expect("tempdir");
        let calls = dir.path().join("ibcmd.calls.log");
        let ibcmd = dir.path().join("ibcmd");
        fs::create_dir_all(dir.path().join("exts").join("client-mcp")).expect("ext dir");
        fs::write(
            dir.path().join("exts").join("client-mcp").join(".project"),
            "<projectDescription><name>client_mcp</name></projectDescription>",
        )
        .expect("project file");
        write_script(
            &ibcmd,
            &format!("printf '%s\\n' \"$*\" >> '{}'\nexit 0", calls.display()),
        );
        let mut config = sample_config(dir.path(), dir.path(), &ibcmd);
        config.infobase = crate::config::model::InfobaseConfig::server(
            "Srvr=cluster:1541;Ref=demo",
            crate::config::model::InfobaseDbmsConfig::new("PostgreSQL", "localhost", "demo")
                .with_credentials(Some("postgres".to_owned()), Some("pg-secret".to_owned())),
        )
        .with_credentials(Some("Admin".to_owned()), Some("secret".to_owned()));

        let result = execute(
            &ExecutionContext::cli(CommandName::Extensions),
            &config,
            &ConfigureExtensionsRequest::default(),
        )
        .expect("execute");

        assert!(result.ok);
        let calls_text = fs::read_to_string(calls).expect("calls");
        assert!(calls_text.contains("--dbms PostgreSQL"));
        assert!(calls_text.contains("--database-server localhost"));
        assert!(calls_text.contains("--database-name demo"));
        assert!(calls_text.contains("--database-user postgres"));
        assert!(calls_text.contains("--database-password pg-secret"));
        assert!(calls_text.contains("--user Admin"));
        assert!(calls_text.contains("--password secret"));
    }

    #[cfg(unix)]
    #[test]
    fn execute_extension_non_zero_exit_reports_operation_target_and_exit_code() {
        let dir = tempdir().expect("tempdir");
        let ibcmd = dir.path().join("ibcmd");
        fs::create_dir_all(dir.path().join("exts").join("client-mcp")).expect("ext dir");
        fs::write(
            dir.path().join("exts").join("client-mcp").join(".project"),
            "<projectDescription><name>client_mcp</name></projectDescription>",
        )
        .expect("project file");
        write_script(&ibcmd, "echo 'bad extension state' >&2\nexit 17");
        let config = sample_config(dir.path(), dir.path(), &ibcmd);

        let failure = execute(
            &ExecutionContext::cli(CommandName::Extensions),
            &config,
            &ConfigureExtensionsRequest::default(),
        )
        .expect_err("failure");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Platform);
        assert!(failure
            .error
            .message()
            .contains("extension update failed for extension 'client_mcp' with exit code 17"));
        assert!(failure
            .error
            .message()
            .contains("stderr: bad extension state"));
    }

    #[cfg(unix)]
    #[test]
    fn execute_honors_interruption_before_extension_update_safe_point() {
        let dir = tempdir().expect("tempdir");
        let calls = dir.path().join("ibcmd.calls.log");
        let ibcmd = dir.path().join("ibcmd");
        fs::create_dir_all(dir.path().join("exts").join("client-mcp")).expect("ext dir");
        fs::write(
            dir.path().join("exts").join("client-mcp").join(".project"),
            "<projectDescription><name>client_mcp</name></projectDescription>",
        )
        .expect("project file");
        write_script(
            &ibcmd,
            &format!("printf '%s\\n' \"$*\" >> '{}'\nexit 0", calls.display()),
        );
        let config = sample_config(dir.path(), dir.path(), &ibcmd);
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let failure = execute(
            &ExecutionContext::cli(CommandName::Extensions).with_cancellation(cancellation),
            &config,
            &ConfigureExtensionsRequest::default(),
        )
        .expect_err("interrupted execution");
        let payload = failure.payload.expect("payload");

        assert!(failure
            .error
            .message()
            .contains("before entering extension update safe point"));
        assert!(payload.steps.is_empty());
        assert!(!calls.exists() || fs::read_to_string(&calls).expect("calls").trim().is_empty());
    }
}
