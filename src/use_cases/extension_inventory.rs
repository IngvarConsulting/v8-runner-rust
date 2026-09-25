//! Reads and changes the extension composition of the configured infobase.
//!
//! Provider selection chooses the standalone agent or IBCMD. Designer has no
//! batch key for installed extensions, so it cannot serve this family.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use crate::config::model::AppConfig;
use crate::domain::capability::Provider;
use crate::domain::extensions::{
    ExtensionInventoryResult, ExtensionsResult, ExtensionsStep, InstalledExtension,
    RequestedInventory,
};
use crate::platform::extension_inventory::{
    parse_extension_inventory, read_applied_extension_descriptor,
};
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl, IbcmdError};
use crate::platform::locator::UtilityType;
use crate::platform::process::ProcessError;
use crate::platform::result::PlatformCommandResult;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::extension_agent::ExtensionAgent;
use crate::use_cases::request::{ExtensionInventoryRequest, ExtensionInventoryScope};
use crate::use_cases::result::{UseCaseError, UseCaseFailure, UseCaseResult};
use tracing::debug;

pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExtensionInventoryRequest,
) -> UseCaseResult<ExtensionInventoryResult> {
    debug!(
        command = context.command().as_str(),
        transport = ?context.transport(),
        "executing extension inventory use case"
    );
    let started = Instant::now();
    // Имя проверяется до подключения и до запуска утилиты: иначе пустое имя доходило
    // до платформы и возвращалось жалобой на перечень, в котором его нет.
    if let ExtensionInventoryScope::Named { name } = &request.scope {
        if !valid_extension_name(name) {
            return Err(UseCaseFailure::without_payload(AppError::Validation(
                "--name must be a non-empty 1C identifier".to_owned(),
            )));
        }
    }
    let mut utilities = PlatformUtilities::from_config(config);
    let selected = crate::use_cases::provider_selection::select(
        config,
        &mut utilities,
        crate::domain::capability::Operation::Extensions,
    )
    .map_err(|(error, _receipt)| UseCaseFailure::without_payload(error))?;
    let receipt = selected.receipt;
    let executor = Executor::of(selected.provider, selected.location, config)
        .map_err(UseCaseFailure::without_payload)?;
    if request.dry_run {
        // Reading the composition starts the platform, authenticates and leaves a journal
        // trace, so the read is previewed like any change: the target and the account are
        // named, and nothing is asked of the platform yet.
        return Ok(ExtensionInventoryResult {
            provider: Some(receipt),
            ok: true,
            provider_dispatched: false,
            requested: requested(&request.scope),
            plan: Some(format!(
                "would read {} of {} via {}",
                match &request.scope {
                    ExtensionInventoryScope::All => "every installed extension".to_owned(),
                    ExtensionInventoryScope::Named { name } => format!("extension '{name}'"),
                },
                executor.target_label(config),
                executor.label()
            )),
            extensions: Vec::new(),
            duration_ms: started.elapsed().as_millis() as u64,
        });
    }

    let extensions = match executor {
        Executor::Agent { v8 } => {
            let mut agent = ExtensionAgent::open(context, config, v8.as_deref())
                .map_err(UseCaseFailure::without_payload)?;
            let inventory = agent.inventory(match &request.scope {
                ExtensionInventoryScope::All => None,
                ExtensionInventoryScope::Named { name } => Some(name.as_str()),
            });
            agent.close();
            let extensions = inventory.map_err(UseCaseFailure::without_payload)?;
            ensure_requested_record(&extensions, request)
                .map_err(UseCaseFailure::without_payload)?;
            extensions
        }
        Executor::Ibcmd { binary, connection } => {
            let dsl = IbcmdDsl::new(
                binary,
                connection,
                utilities.runner_for(UtilityType::Ibcmd),
                context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
            );
            let subject = inventory_subject(&request.scope);

            let platform_result = match &request.scope {
                ExtensionInventoryScope::All => dsl.infobase_extension_list(),
                ExtensionInventoryScope::Named { name } => dsl.infobase_extension_info(name),
            }
            .map_err(|error| {
                UseCaseFailure::without_payload(snapshot_dispatch_error(
                    context, error, "read", &subject,
                ))
            })?;

            validate_snapshot_step(&platform_result, "read", &subject)
                .map_err(UseCaseFailure::without_payload)?;
            if context.cancellation().is_cancelled() {
                return Err(UseCaseFailure::without_payload(AppError::Cancelled(
                    "extension inventory cancelled".to_owned(),
                )));
            }
            let mut extensions = read_inventory(&platform_result, request)
                .map_err(UseCaseFailure::without_payload)?;
            attest_applied_prefixes(context, config, request, &dsl, &mut extensions)
                .map_err(UseCaseFailure::without_payload)?;
            extensions
        }
    };

    Ok(ExtensionInventoryResult {
        provider: Some(receipt),
        ok: true,
        provider_dispatched: true,
        requested: requested(&request.scope),
        plan: None,
        extensions,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// The list/info command omits NamePrefix. Save the *applied database* CFE for
/// every returned record, export its descriptor, then verify the list has not
/// changed while the slower snapshots were read. Never substitute a working
/// configuration export: upload without apply makes that a different state.
fn attest_applied_prefixes(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExtensionInventoryRequest,
    dsl: &IbcmdDsl<'_>,
    extensions: &mut [InstalledExtension],
) -> Result<(), AppError> {
    if extensions.is_empty() {
        return Ok(());
    }
    let temp = crate::support::temp::private_temp_dir(&config.work_path)
        .map_err(|error| AppError::Runtime(format!("cannot create inventory temp: {error}")))?;

    let result = (|| -> Result<(), AppError> {
        for (index, extension) in extensions.iter_mut().enumerate() {
            let subject = format!("extension '{}'", extension.name);
            if context.cancellation().is_cancelled() {
                return Err(AppError::Cancelled(
                    "extension inventory cancelled".to_owned(),
                ));
            }
            let entry = temp.path().join(index.to_string());
            let xml = entry.join("xml");
            fs::create_dir_all(&xml).map_err(|error| {
                AppError::Runtime(format!("cannot prepare extension snapshot: {error}"))
            })?;
            let saved = entry.join("applied.cfe");
            let save = dsl
                .config_save(&saved, true, Some(&extension.name))
                .map_err(|error| snapshot_dispatch_error(context, error, "save", &subject))?;
            validate_snapshot_step(&save, "save", &subject)?;
            if context.cancellation().is_cancelled() {
                return Err(AppError::Cancelled(
                    "extension inventory cancelled".to_owned(),
                ));
            }
            let exported = dsl
                .config_export_file(&saved, &xml)
                .map_err(|error| snapshot_dispatch_error(context, error, "export", &subject))?;
            validate_snapshot_step(&exported, "export", &subject)?;
            if context.cancellation().is_cancelled() {
                return Err(AppError::Cancelled(
                    "extension inventory cancelled".to_owned(),
                ));
            }
            let descriptor = read_applied_extension_descriptor(&xml.join("Configuration.xml"))
                .map_err(|error| {
                    AppError::InvalidOutput(format!(
                        "invalid applied extension descriptor for '{}': {error}",
                        extension.name
                    ))
                })?;
            if !descriptor.name.eq_ignore_ascii_case(&extension.name)
                || descriptor.version != extension.version
                || descriptor.purpose != extension.purpose
            {
                return Err(AppError::InvalidOutput(format!(
                    "applied extension '{}' does not agree with the platform inventory",
                    extension.name
                )));
            }
            extension.name_prefix = Some(descriptor.name_prefix);
        }

        let subject = inventory_subject(&request.scope);
        let verified = match &request.scope {
            ExtensionInventoryScope::All => dsl.infobase_extension_list(),
            ExtensionInventoryScope::Named { name } => dsl.infobase_extension_info(name),
        }
        .map_err(|error| snapshot_dispatch_error(context, error, "re-read inventory", &subject))?;
        validate_snapshot_step(&verified, "re-read inventory", &subject)?;
        if context.cancellation().is_cancelled() {
            return Err(AppError::Cancelled(
                "extension inventory cancelled".to_owned(),
            ));
        }
        let verified = read_inventory(&verified, request)?;
        if inventory_identity(extensions)? != inventory_identity(&verified)? {
            return Err(AppError::InvalidOutput(
                "extension inventory changed while reading applied prefixes".to_owned(),
            ));
        }
        Ok(())
    })();
    let cleanup = temp.close().map_err(|_| {
        AppError::Runtime("could not remove the private database extension snapshot".to_owned())
    });
    match (result, cleanup) {
        (_, Err(error)) => Err(error),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn inventory_subject(scope: &ExtensionInventoryScope) -> String {
    match scope {
        ExtensionInventoryScope::All => "all installed extensions".to_owned(),
        ExtensionInventoryScope::Named { name } => format!("extension '{name}'"),
    }
}

fn snapshot_dispatch_error(
    context: &ExecutionContext,
    error: IbcmdError,
    step: &str,
    subject: &str,
) -> AppError {
    if context.cancellation().is_cancelled()
        || matches!(&error, IbcmdError::Spawn(ProcessError::Cancelled { .. }))
    {
        AppError::Cancelled("extension inventory cancelled".to_owned())
    } else if let IbcmdError::Spawn(ProcessError::TimedOut { timeout_ms, .. }) = error {
        AppError::TimedOut(format!(
            "extension inventory {step} timed out after {timeout_ms} ms"
        ))
    } else {
        AppError::Platform(format!("extension inventory {step} failed for {subject}"))
    }
}

fn validate_snapshot_step(
    result: &PlatformCommandResult,
    step: &str,
    subject: &str,
) -> Result<(), AppError> {
    if result.process.exit_code == 0 {
        Ok(())
    } else {
        // A platform diagnostic may echo connection arguments and passwords.
        Err(AppError::Platform(format!(
            "extension inventory {step} failed for {subject} with exit code {}",
            result.process.exit_code
        )))
    }
}

fn inventory_identity(
    extensions: &[InstalledExtension],
) -> Result<BTreeMap<String, InstalledExtension>, AppError> {
    let mut by_name = BTreeMap::new();
    for extension in extensions {
        let mut without_prefix = extension.clone();
        without_prefix.name_prefix = None;
        if by_name
            .insert(extension.name.to_lowercase(), without_prefix)
            .is_some()
        {
            return Err(AppError::InvalidOutput(
                "platform extension inventory has duplicate names".to_owned(),
            ));
        }
    }
    Ok(by_name)
}

fn requested(scope: &ExtensionInventoryScope) -> RequestedInventory {
    match scope {
        ExtensionInventoryScope::All => RequestedInventory::All,
        ExtensionInventoryScope::Named { name } => RequestedInventory::Named { name: name.clone() },
    }
}

fn validate_success(result: &PlatformCommandResult) -> Result<(), AppError> {
    if result.process.exit_code == 0 {
        return Ok(());
    }
    let mut details = vec![format!(
        "platform extension read failed with exit code {}",
        result.process.exit_code
    )];
    for (label, value) in [
        ("stdout", result.process.stdout.as_str()),
        ("stderr", result.process.stderr.as_str()),
    ] {
        let value = value.trim();
        if !value.is_empty() {
            details.push(format!("{label}: {value}"));
        }
    }
    Err(AppError::Platform(details.join("; ")))
}

/// Reads the inventory, and for a named request proves the record is the one asked for.
///
/// Имя расширения — идентификатор 1С: буква или подчёркивание в начале, дальше буквы,
/// цифры и подчёркивания. Пустая строка и пробелы именем не являются.
fn valid_extension_name(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_alphanumeric())
}

/// The platform answers `info --name` with the same record shape as `list`, so without
/// this check a renamed or substituted record would be reported as the requested one.
fn read_inventory(
    result: &PlatformCommandResult,
    request: &ExtensionInventoryRequest,
) -> Result<Vec<InstalledExtension>, AppError> {
    let extensions = parse_extension_inventory(&result.process.stdout).map_err(|error| {
        AppError::InvalidOutput(format!(
            "cannot read the platform extension inventory: {error}"
        ))
    })?;
    ensure_requested_record(&extensions, request)?;
    Ok(extensions)
}

fn ensure_requested_record(
    extensions: &[InstalledExtension],
    request: &ExtensionInventoryRequest,
) -> Result<(), AppError> {
    if let ExtensionInventoryScope::Named { name } = &request.scope {
        if extensions.len() != 1 || !extensions[0].name.eq_ignore_ascii_case(name) {
            return Err(AppError::InvalidOutput(format!(
                "platform did not report exactly the requested extension '{name}'"
            )));
        }
    }
    Ok(())
}

/// Исполнитель семейства `extensions`: `ibcmd` — утилитой над своим подключением, агент —
/// сессией (утилита нужна только управляемому агенту, чтобы его запустить).
pub(crate) enum Executor {
    Ibcmd {
        binary: PathBuf,
        connection: IbcmdConnection,
    },
    /// `v8` — утилита, которой раннер запускает своего агента; у подключённого агента и
    /// шлюза её нет.
    Agent { v8: Option<PathBuf> },
}

impl Executor {
    /// Исполнитель по итогу выбора. Подключение `ibcmd` строится здесь и только для
    /// `ibcmd`: с ним приходит требование секции `infobase.dbms`, а агенту, который в СУБД
    /// не ходит, оно не нужно.
    pub(crate) fn of(
        provider: Provider,
        location: Option<crate::platform::locator::UtilityLocation>,
        config: &AppConfig,
    ) -> Result<Self, AppError> {
        match (provider, location) {
            (Provider::Agent, location) => Ok(Self::Agent {
                v8: location.map(|l| l.path),
            }),
            (Provider::Ibcmd, Some(location)) => {
                // У автономного сервера строки подключения нет: туда ходит шлюз, и
                // жалоба на секцию `dbms` назвала бы не ту причину.
                if config.infobase.standalone.is_some() {
                    return Err(AppError::Runtime(
                        "ibcmd was selected for a target without a connection string".to_owned(),
                    ));
                }
                Ok(Self::Ibcmd {
                    binary: location.path,
                    connection: IbcmdConnection::from_infobase(&config.infobase)?,
                })
            }
            (provider @ Provider::Ibcmd, None)
            | (provider @ (Provider::Designer | Provider::IbcmdRs | Provider::Webinst), _) => {
                Err(crate::use_cases::unimplemented_provider(
                    crate::domain::capability::Operation::Extensions,
                    provider,
                ))
            }
        }
    }

    pub(crate) fn label(&self) -> String {
        match self {
            Self::Ibcmd { binary, .. } => binary.display().to_string(),
            Self::Agent { .. } => "the designer agent".to_owned(),
        }
    }

    /// Цель для превью без секретов: `ibcmd` называет базу так, как пойдёт к ней сам, —
    /// файлом или базой в СУБД; агент — базу, к которой подключится, или шлюз
    /// автономного сервера.
    pub(crate) fn target_label(&self, config: &AppConfig) -> String {
        match self {
            Self::Ibcmd { connection, .. } => connection.describe_target(),
            Self::Agent { .. } => match config.infobase.standalone.as_ref() {
                Some(standalone) => format!("standalone server at {}", standalone.gate),
                None => config.v8_connection().describe_target(),
            },
        }
    }
}

/// Change to the extension composition of the infobase.
///
/// Each variant is one platform command; there is no combined "ensure" shape, because a
/// caller that means create and a caller that means update must fail differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionChangeRequest {
    Create {
        name: String,
        name_prefix: String,
        synonym: Option<String>,
        purpose: Option<String>,
    },
    Delete {
        name: String,
    },
    SetActive {
        name: String,
        active: bool,
    },
}

impl ExtensionChangeRequest {
    fn target(&self) -> &str {
        match self {
            Self::Create { name, .. } | Self::Delete { name } | Self::SetActive { name, .. } => {
                name
            }
        }
    }

    fn action(&self) -> &'static str {
        match self {
            Self::Create { .. } => "create",
            Self::Delete { .. } => "delete",
            Self::SetActive { active: true, .. } => "activate",
            Self::SetActive { active: false, .. } => "deactivate",
        }
    }
}

/// Applies one change to the extension composition of the configured infobase.
pub fn change(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ExtensionChangeRequest,
    dry_run: bool,
) -> UseCaseResult<ExtensionsResult> {
    debug!(
        command = context.command().as_str(),
        action = request.action(),
        "executing extension composition change"
    );
    let started = Instant::now();
    let mut utilities = PlatformUtilities::from_config(config);
    let selected = crate::use_cases::provider_selection::select(
        config,
        &mut utilities,
        crate::domain::capability::Operation::Extensions,
    )
    .map_err(|(error, _receipt)| UseCaseFailure::without_payload(error))?;
    let receipt = selected.receipt;
    let executor = Executor::of(selected.provider, selected.location, config)
        .map_err(UseCaseFailure::without_payload)?;
    if dry_run {
        return Ok(ExtensionsResult {
            provider: Some(receipt),
            ok: true,
            provider_dispatched: false,
            steps: vec![ExtensionsStep {
                target: request.target().to_owned(),
                action: request.action().to_owned(),
                ok: true,
                message: Some(format!(
                    "would {} '{}' in {} via {}",
                    request.action(),
                    request.target(),
                    executor.target_label(config),
                    executor.label()
                )),
                duration_ms: 0,
            }],
            duration_ms: started.elapsed().as_millis() as u64,
        });
    }

    let outcome = match executor {
        Executor::Agent { v8 } => {
            ExtensionAgent::open(context, config, v8.as_deref()).and_then(|mut agent| {
                let outcome = match request {
                    ExtensionChangeRequest::Create {
                        name,
                        name_prefix,
                        synonym,
                        purpose,
                    } => agent.create(name, name_prefix, synonym.as_deref(), purpose.as_deref()),
                    ExtensionChangeRequest::Delete { name } => agent.delete(name),
                    ExtensionChangeRequest::SetActive { name, active } => {
                        agent.set_active(name, *active)
                    }
                };
                agent.close();
                outcome
            })
        }
        Executor::Ibcmd { binary, connection } => {
            let dsl = IbcmdDsl::new(
                binary,
                connection,
                utilities.runner_for(UtilityType::Ibcmd),
                context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None),
            );
            let platform_result = match request {
                ExtensionChangeRequest::Create {
                    name,
                    name_prefix,
                    synonym,
                    purpose,
                } => dsl.infobase_extension_create(
                    name,
                    name_prefix,
                    synonym.as_deref(),
                    purpose.as_deref(),
                ),
                ExtensionChangeRequest::Delete { name } => dsl.infobase_extension_delete(name),
                ExtensionChangeRequest::SetActive { name, active } => {
                    dsl.infobase_extension_set_active(name, *active)
                }
            };
            platform_result
                .map_err(AppError::from)
                .and_then(|result| validate_success(&result))
        }
    };

    let step_duration = started.elapsed().as_millis() as u64;
    match outcome {
        Ok(()) => Ok(ExtensionsResult {
            provider: Some(receipt.clone()),
            ok: true,
            provider_dispatched: true,
            steps: vec![ExtensionsStep {
                target: request.target().to_owned(),
                action: request.action().to_owned(),
                ok: true,
                message: None,
                duration_ms: step_duration,
            }],
            duration_ms: started.elapsed().as_millis() as u64,
        }),
        Err(error) => {
            let payload = ExtensionsResult {
                provider: Some(receipt.clone()),
                ok: false,
                provider_dispatched: true,
                steps: vec![ExtensionsStep {
                    target: request.target().to_owned(),
                    action: request.action().to_owned(),
                    ok: false,
                    message: Some(error.to_string()),
                    duration_ms: step_duration,
                }],
                duration_ms: started.elapsed().as_millis() as u64,
            };
            Err(UseCaseFailure::with_payload(
                UseCaseError::from(error),
                payload,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::read_inventory;
    use crate::platform::process::ProcessResult;
    use crate::platform::result::PlatformCommandResult;
    use crate::use_cases::request::{ExtensionInventoryRequest, ExtensionInventoryScope};

    fn platform_result(stdout: &str) -> PlatformCommandResult {
        PlatformCommandResult {
            process: ProcessResult {
                exit_code: 0,
                stdout: stdout.to_owned(),
                stderr: String::new(),
                interruption: None,
            },
            platform_log_path: None,
            platform_log: None,
            platform_log_read_error: None,
        }
    }

    const ONE: &str = "name                         : \"Проба\"\nversion                      : \nactive                       : yes\npurpose                      : add-on\nsafe-mode                    : yes\nsecurity-profile-name        : \nunsafe-action-protection     : yes\nused-in-distributed-infobase : no\nscope                        : infobase\nhash-sum                     : \"9hfFb6YVX2OwLKZaL1L69Eq0Vrg=\"\n";

    #[test]
    fn a_named_read_refuses_a_record_that_is_not_the_requested_one() {
        let request = ExtensionInventoryRequest {
            dry_run: false,
            scope: ExtensionInventoryScope::Named {
                name: "Другая".to_owned(),
            },
        };

        let error = read_inventory(&platform_result(ONE), &request).expect_err("refusal");

        assert!(error.to_string().contains("Другая"), "{error}");
    }

    #[test]
    fn a_named_read_accepts_the_requested_record() {
        let request = ExtensionInventoryRequest {
            dry_run: false,
            scope: ExtensionInventoryScope::Named {
                name: "Проба".to_owned(),
            },
        };

        let extensions = read_inventory(&platform_result(ONE), &request).expect("inventory");

        assert_eq!(extensions.len(), 1);
        assert_eq!(extensions[0].name, "Проба");
    }

    #[test]
    fn a_named_read_refuses_a_second_unrequested_record() {
        let request = ExtensionInventoryRequest {
            dry_run: false,
            scope: ExtensionInventoryScope::Named {
                name: "Проба".to_owned(),
            },
        };
        let extra = ONE.replace("Проба", "Другая");
        let error = read_inventory(&platform_result(&format!("{ONE}\n{extra}")), &request)
            .expect_err("named reply must have exactly one record");
        assert!(error.to_string().contains("exactly"), "{error}");
    }

    #[test]
    fn an_empty_infobase_reads_as_an_empty_inventory() {
        let request = ExtensionInventoryRequest {
            dry_run: false,
            scope: ExtensionInventoryScope::All,
        };

        let extensions = read_inventory(&platform_result(""), &request).expect("inventory");

        assert!(extensions.is_empty());
    }
}
