//! `reset`: основная конфигурация (или расширение) возвращается к конфигурации базы данных —
//! обратный ход `apply`, ремонтный шаг; саму базу данных команда не трогает
//! (`INV.CLI.RESET-DISCARDS-THE-UNAPPLIED`). Своё и чужое непринятое не различаются: решение о
//! сбросе принимает человек.
//!
//! Ход: признак непринятого тем же чтением, что у `status --deep` (нет — отката нет; не
//! получен — отказ до отката), поколение до отката инструментом записи набора, затем своя
//! хеш-память набора заменяется пустой, затем откат ([`act`]), затем запись журнала поколений
//! переписывает инструмент, который её сделал, — если база не ушла от неё.

pub(crate) mod act;

use std::path::PathBuf;
use std::time::Instant;

use self::act::RollingBack;
use crate::config::model::{AppConfig, SourceSetConfig};
use crate::domain::capability::{Operation, Provider};
use crate::domain::reset::{HashMemoryFate, ResetOutcome, ResetResult};
use crate::domain::source_set::SourceSetPurpose;
use crate::domain::status::{GenerationAfter, GenerationRecordFate};
use crate::platform::locator::UtilityType;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::generation_reader::{
    designer_log_file, read_by_record_tool, read_unapplied, GenerationProcess, LocatedTool,
};
use crate::use_cases::generation_record::{self, RecordStep};
use crate::use_cases::interruption::{append_warnings, collecting_deferrals};
use crate::use_cases::request::ResetRequest;
use crate::use_cases::result::{stamp_dispatch, UseCaseFailure, UseCaseResult};
use crate::use_cases::source_inventory::SourceSetInventory;

type ResetFailure = UseCaseFailure<ResetResult>;

/// Caller must ensure exclusive ownership of `config.work_path` and of the file infobase.
pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ResetRequest,
) -> UseCaseResult<ResetResult> {
    stamp_dispatch(
        crate::use_cases::provider_selection::stamp_session(run(context, config, request), context),
        context.work(),
    )
}

fn run(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ResetRequest,
) -> UseCaseResult<ResetResult> {
    let started = Instant::now();
    let inventory = SourceSetInventory::new(config);
    let set = target(&inventory, request).map_err(ResetFailure::without_payload)?;
    let mut result = ResetResult {
        provider: None,
        ok: false,
        provider_dispatched: false,
        source_set: set.name.clone(),
        purpose: set.purpose,
        outcome: ResetOutcome::Failed,
        hash_memory: None,
        generation: None,
        message: None,
        duration_ms: 0,
    };
    let mut utilities = PlatformUtilities::from_config(config);
    let selected = match crate::use_cases::provider_selection::select(
        config,
        &mut utilities,
        Operation::Reset,
    ) {
        Ok(selected) => selected,
        Err((error, receipt)) => {
            result.message = Some(error.to_string());
            result.duration_ms = started.elapsed().as_millis() as u64;
            return crate::use_cases::provider_selection::attach(
                Err(ResetFailure::with_payload(error, result)),
                &receipt,
            );
        }
    };
    let receipt = selected.receipt.clone();
    let executor = Executor {
        provider: selected.provider,
        binary: selected.location.map(|location| location.path),
        utilities,
    };
    let extension = (set.purpose == SourceSetPurpose::Extension).then_some(set.name.as_str());
    let outcome = if request.dry_run {
        result.outcome = ResetOutcome::Planned;
        result.message = Some(format!(
            "would discard the unapplied changes of {}; planned, {} not dispatched",
            subject(set, extension),
            executor.provider
        ));
        Ok(())
    } else {
        discard(
            context,
            config,
            &inventory,
            set,
            extension,
            executor,
            &mut result,
        )
    };
    result.duration_ms = started.elapsed().as_millis() as u64;
    let outcome = match outcome {
        Ok(()) => {
            result.ok = true;
            Ok(result)
        }
        Err(error) => {
            result.outcome = ResetOutcome::Failed;
            result.message = Some(error.to_string());
            Err(ResetFailure::with_payload(error, result))
        }
    };
    crate::use_cases::provider_selection::attach(outcome, &receipt)
}

/// Цель команды: с набором — он, если он идёт в базу; без набора — основная конфигурация.
/// Расширение откатывается только названным набором.
fn target<'a>(
    inventory: &SourceSetInventory<'a>,
    request: &ResetRequest,
) -> Result<&'a SourceSetConfig, AppError> {
    let set = match request.source_set.as_deref().map(str::trim) {
        Some("") => {
            return Err(AppError::Validation(
                "reset source-set requires a non-empty name".to_owned(),
            ))
        }
        Some(name) => inventory.named(name)?,
        None => match inventory.main_configuration() {
            Some(set) => set,
            None => return Err(no_main_configuration()),
        },
    };
    if set.purpose.is_external() {
        return Err(AppError::Validation(format!(
            "source-set '{}' holds external files: they are not loaded into the infobase, so reset has nothing to discard",
            set.name
        )));
    }
    Ok(set)
}

/// Отказ проекта без набора основной конфигурации: у него одни внешние наборы — набор
/// расширения без основной конфигурации проверка проекта не пропускает, — и откатывать
/// нечего.
fn no_main_configuration() -> AppError {
    AppError::Validation(
        "the project declares no CONFIGURATION source-set, only external files that are not loaded into the infobase: reset has nothing to discard".to_owned(),
    )
}

/// Как называется цель в текстах.
fn subject(set: &SourceSetConfig, extension: Option<&str>) -> String {
    match extension {
        Some(extension) => format!("extension '{extension}' of source-set '{}'", set.name),
        None => format!("the main configuration of source-set '{}'", set.name),
    }
}

/// Признак, память, откат и запись поколения — по порядку; что сделано, ответ называет и
/// при отказе.
fn discard(
    context: &ExecutionContext,
    config: &AppConfig,
    inventory: &SourceSetInventory<'_>,
    set: &SourceSetConfig,
    extension: Option<&str>,
    mut executor: Executor,
    result: &mut ResetResult,
) -> Result<(), AppError> {
    if extension.is_some() {
        executor.require_installed(context, config, inventory, set)?;
    }
    let unapplied = executor
        .unapplied(context, config, extension)
        .map_err(|error| {
            if error.cancellation().is_some() {
                error
            } else {
                error.with_context(format!(
                    "the unapplied state of {} is not known, so nothing was rolled back",
                    subject(set, extension)
                ))
            }
        })?;
    if !unapplied {
        result.outcome = ResetOutcome::NothingToDiscard;
        result.message = Some(format!(
            "{} has nothing unapplied: it equals the database configuration, and nothing was rolled back",
            subject(set, extension)
        ));
        return Ok(());
    }
    let memory = inventory.designer_context(&set.name);
    let record = memory.and_then(|memory| {
        crate::use_cases::exchange_guard::recorded_generation(memory, &config.work_path)
            .map(|record| (memory, record))
    });
    // Поколение до отката сверяется с записью, как у `apply`: запись переписывается, только
    // когда база не ушла от неё, — иначе откат спрятал бы чужую загрузку от проверки
    // `non_fast_forward` следующей отправки.
    let record = match record {
        Some((memory, record)) => {
            let before = if record.after == GenerationAfter::FailedBuild {
                Before::FailedLoad
            } else {
                match executor.read_generation(context, config, record.tool, set, extension) {
                    Ok(Some(token)) => Before::Token(token),
                    Ok(None) => Before::NoAnswer,
                    Err(error) if error.cancellation().is_some() => return Err(error),
                    Err(error) => {
                        tracing::debug!(%error, "the generation before the reset is not known");
                        Before::NoAnswer
                    }
                }
            };
            Some((memory, record, before))
        }
        None => None,
    };
    // Пришедшая отмена останавливает до памяти: пустая память без отката заставила бы
    // следующую отправку грузить набор целиком зря.
    if let Some(error) = crate::use_cases::interruption::pending_interruption_error(
        context,
        format!("reset for source-set '{}'", set.name),
    ) {
        return Err(error);
    }
    result.hash_memory = Some(match memory {
        Some(memory) => crate::use_cases::exchange_guard::empty_hash_memory_before_reset(
            memory,
            &config.work_path,
        )?,
        None => HashMemoryFate::Absent,
    });
    let memory_emptied = result.hash_memory == Some(HashMemoryFate::Replaced);
    let ((), deferrals) = collecting_deferrals(|deferrals| {
        executor.roll_back(context, config, set, extension, deferrals)
    })
    .map_err(|error| {
        // Память набора уже пуста: следующая отправка грузит его целиком — это видно и в
        // тексте, а не только в поле `hash_memory`.
        if memory_emptied {
            error.with_context(format!(
                "the source memory of source-set '{}' was emptied before the rollback, so the next push loads it in full",
                set.name
            ))
        } else {
            error
        }
    })?;
    let (generation, note) = match record {
        None => (GenerationRecordFate::Unchecked, None),
        Some((memory, record, before)) => match before {
            // Запись перед неудачной загрузкой остаётся: что та загрузка сделала с базой,
            // неизвестно, и её сверку делает следующая отправка.
            Before::FailedLoad => (
                GenerationRecordFate::Kept,
                Some(format!(
                    "the generation record of source-set '{}' was made before a failed load and is kept, so the next push compares the infobase with it",
                    set.name
                )),
            ),
            Before::Token(token) if token == record.token => {
                let after = executor.read_generation(context, config, record.tool, set, extension);
                generation_record::carry_record(
                    RecordStep::Reset,
                    memory,
                    &config.work_path,
                    &record,
                    after,
                )?
            }
            // После отката поколение может вернуться к записи (правили в Конфигураторе, не
            // применив) — тогда отправка пройдёт; иначе она откажет.
            Before::Token(token) => (
                GenerationRecordFate::Kept,
                Some(format!(
                    "the infobase moved away from the record of source-set '{}' before the reset: its configuration generation was {token}, the record after the last {} is {}; the record is kept, so the next push that loads is refused if the infobase still differs from the record",
                    set.name, record.after, record.token
                )),
            ),
            Before::NoAnswer => generation_record::erase_record(
                RecordStep::Reset,
                memory,
                &config.work_path,
                &format!(
                    "could not be read by {}, the tool of its record",
                    record.tool
                ),
            ),
        },
    };
    result.generation = Some(generation);
    result.outcome = ResetOutcome::Discarded;
    let cancelled = crate::use_cases::apply::cancellation_after(context, &deferrals, "reset");
    let warnings: Vec<String> = deferrals.into_iter().chain(note).chain(cancelled).collect();
    result.message = Some(append_warnings(
        format!(
            "rolled {} back to the database configuration",
            subject(set, extension)
        ),
        &warnings,
    ));
    Ok(())
}

/// Поколение базы до отката, сверяемое с записью набора.
enum Before {
    /// Запись сделана перед неудачной загрузкой: поколение не спрашивается.
    FailedLoad,
    /// Инструмент записи не ответил.
    NoAnswer,
    Token(String),
}

/// Исполнитель команды: процесс платформы на каждое действие.
struct Executor {
    provider: Provider,
    binary: Option<PathBuf>,
    utilities: PlatformUtilities,
}

impl Executor {
    /// Исполнитель и его утилита — в том виде, что знает общий читатель поколения.
    fn tool(&self) -> LocatedTool<'_> {
        LocatedTool {
            provider: self.provider,
            binary: self.binary.as_deref(),
        }
    }

    /// Расширение откатывается, только когда оно есть в базе: состав базы читает и
    /// сопоставляет с наборами то же место, что у `pull --all` и `apply`
    /// (`INV.USE-CASES.INSTALLED-EXTENSIONS-ARE-MATCHED-IN-ONE-PLACE`).
    fn require_installed(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        inventory: &SourceSetInventory<'_>,
        set: &SourceSetConfig,
    ) -> Result<(), AppError> {
        let installed = crate::use_cases::installed_extensions::read_installed_extensions(
            context,
            config,
            Operation::Reset,
            self.provider,
            self.binary.as_deref(),
            &self.utilities,
        )?;
        let packages = inventory.installed_packages(&installed)?;
        if packages
            .not_installed
            .iter()
            .any(|missing| missing.name == set.name)
        {
            return Err(AppError::Validation(format!(
                "{} is not installed in the infobase: reset has nothing to discard",
                subject(set, Some(&set.name))
            )));
        }
        Ok(())
    }

    /// Признак непринятого — тем же чтением, что у `status --deep`.
    fn unapplied(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        extension: Option<&str>,
    ) -> Result<bool, AppError> {
        let binary = self.tool().path(context)?;
        match self.provider {
            Provider::Designer => read_unapplied(
                context,
                config,
                || {
                    Ok(GenerationProcess::Designer {
                        binary,
                        runner: self.utilities.runner_for(UtilityType::V8),
                        log_file: designer_log_file(config, "reset-unapplied")?,
                    })
                },
                extension,
            ),
            Provider::Ibcmd => read_unapplied(
                context,
                config,
                || {
                    Ok(GenerationProcess::Ibcmd {
                        binary,
                        runner: self.utilities.runner_for(UtilityType::Ibcmd),
                        data_path: None,
                    })
                },
                extension,
            ),
            other @ (Provider::Agent | Provider::IbcmdRs | Provider::Webinst) => Err(
                crate::use_cases::unimplemented_provider(Operation::Reset, other),
            ),
        }
    }

    fn roll_back(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        set: &SourceSetConfig,
        extension: Option<&str>,
        deferrals: &mut crate::use_cases::interruption::Deferrals,
    ) -> Result<(), AppError> {
        let binary = self.tool().path(context)?;
        let tool = match self.provider {
            Provider::Designer => RollingBack::Designer {
                binary,
                runner: self.utilities.runner_for(UtilityType::V8),
                log_file: designer_log_file(config, &format!("reset-{}", set.name))?,
            },
            Provider::Ibcmd => RollingBack::Ibcmd {
                binary,
                runner: self.utilities.runner_for(UtilityType::Ibcmd),
            },
            other @ (Provider::Agent | Provider::IbcmdRs | Provider::Webinst) => {
                return Err(crate::use_cases::unimplemented_provider(
                    Operation::Reset,
                    other,
                ))
            }
        };
        act::roll_back(context, config, tool, &set.name, extension, deferrals)
    }

    /// Поколение до и после отката — инструментом записи набора.
    fn read_generation(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        tool: Provider,
        set: &SourceSetConfig,
        extension: Option<&str>,
    ) -> Result<Option<String>, AppError> {
        read_by_record_tool(
            context,
            config,
            tool,
            LocatedTool {
                provider: self.provider,
                binary: self.binary.as_deref(),
            },
            &mut self.utilities,
            &format!("reset-{}-generation", set.name),
            extension,
        )
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    use super::execute;
    use crate::config::model::{
        AppConfig, PlatformToolConfig, SourceFormat, SourceSetConfig, SourceSetPurpose,
        TestsConfig, ToolsConfig,
    };
    use crate::domain::capability::{Operation, Provider};
    use crate::domain::reset::ResetOutcome;
    use crate::platform::process::HeldCommand;
    use crate::support::error::CancelledAt;
    use crate::use_cases::context::{CommandName, ExecutionContext};
    use crate::use_cases::request::ResetRequest;
    use crate::use_cases::result::UseCaseErrorKind;

    /// Поддельная утилита: журнал вызовов; сохранения основной конфигурации и конфигурации
    /// базы данных — содержимым файлов `main` и `db` (по умолчанию разным: непринятое есть),
    /// и ветка `branch` перед ними.
    fn write_tool(dir: &Path, name: &str, branch: &str) -> PathBuf {
        let path = dir.join(name);
        let body = format!(
            "#!/bin/sh\nargs=\"$*\"\nprintf '%s\\n' \"$args\" >> '{calls}'\nlast=''\ntarget=''\nprev=''\nfor arg in \"$@\"; do\n  if [ \"$prev\" = '/DumpCfg' ] || [ \"$prev\" = '/DumpDBCfg' ]; then target=\"$arg\"; fi\n  prev=\"$arg\"\n  last=\"$arg\"\ndone\n{branch}\ncase \"$args\" in\n  *'config save --db'*|*'/DumpDBCfg'*) printf 'db' > \"${{target:-$last}}\" ;;\n  *'config save'*|*'/DumpCfg'*) printf 'main' > \"${{target:-$last}}\" ;;\nesac\nexit 0\n",
            calls = dir.join("calls.log").display(),
        );
        fs::write(&path, body).expect("tool script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("executable");
        path
    }

    fn config(dir: &Path, tool: &Path, reset: Provider) -> AppConfig {
        let base = dir.join("base");
        fs::create_dir_all(base.join("main")).expect("main");
        fs::write(base.join("main").join("Configuration.xml"), "<x/>").expect("marker");
        AppConfig {
            base_path: base,
            work_path: dir.join("work"),
            format: SourceFormat::Designer,
            providers: [(Operation::Reset, reset)].into(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: Some("origin".to_owned()),
            source_sets: vec![SourceSetConfig {
                name: "main".to_owned(),
                purpose: SourceSetPurpose::Configuration,
                path: PathBuf::from("main"),
            }],
            tools: ToolsConfig {
                platform: PlatformToolConfig {
                    path: Some(tool.to_path_buf()),
                    strict: false,
                    version: None,
                },
                ..ToolsConfig::default()
            },
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    fn main_configuration() -> ResetRequest {
        ResetRequest {
            source_set: None,
            dry_run: false,
        }
    }

    fn calls(dir: &Path) -> String {
        fs::read_to_string(dir.join("calls.log")).unwrap_or_default()
    }

    /// Отмена до точки безопасности отката останавливает его до записи в базу.
    #[test]
    fn a_rollback_stopped_at_its_safe_point_writes_nothing() {
        let dir = tempdir().expect("tempdir");
        let tool = write_tool(dir.path(), "1cv8", "");
        let config = config(dir.path(), &tool, Provider::Designer);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let context = ExecutionContext::cli(CommandName::Reset).with_cancellation(cancellation);
        let utilities = crate::platform::utilities::PlatformUtilities::from_config(&config);

        let error = crate::use_cases::interruption::collecting_deferrals(|deferrals| {
            super::act::roll_back(
                &context,
                &config,
                super::act::RollingBack::Designer {
                    binary: &tool,
                    runner: utilities.runner_for(crate::platform::locator::UtilityType::V8),
                    log_file: dir.path().join("rollback.log"),
                },
                "main",
                None,
                deferrals,
            )
        })
        .expect_err("cancelled");

        assert_eq!(
            crate::use_cases::result::UseCaseErrorKind::of(&error),
            UseCaseErrorKind::Cancelled(CancelledAt::Boundary)
        );
        assert!(
            error
                .to_string()
                .contains("before entering reset for source-set 'main' safe point"),
            "{error}"
        );
        assert!(calls(dir.path()).is_empty(), "{}", calls(dir.path()));
    }

    /// Отмена, пришедшая до признака непринятого, ничего не трогает: ни сохранений, ни
    /// памяти, ни отката.
    #[test]
    fn a_cancellation_before_the_unapplied_check_touches_nothing() {
        let dir = tempdir().expect("tempdir");
        let tool = write_tool(dir.path(), "1cv8", "");
        let config = config(dir.path(), &tool, Provider::Designer);
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let failure = execute(
            &ExecutionContext::cli(CommandName::Reset).with_cancellation(cancellation),
            &config,
            &main_configuration(),
        )
        .expect_err("cancelled");

        assert!(
            matches!(failure.error.kind(), UseCaseErrorKind::Cancelled(_)),
            "{}",
            failure.error
        );
        let result = failure.payload.expect("payload");
        assert_eq!(result.outcome, ResetOutcome::Failed);
        assert_eq!(result.hash_memory, None);
        assert!(calls(dir.path()).is_empty(), "{}", calls(dir.path()));
    }

    /// Отмену, которую отложил сам откат, ответ называет один раз; откат состоялся.
    #[test]
    fn a_cancellation_deferred_by_the_rollback_is_named_once() {
        let dir = tempdir().expect("tempdir");
        let held = HeldCommand::in_dir(dir.path());
        let tool = write_tool(dir.path(), "ibcmd", &held.script_branch("config reset", 0));
        let config = config(dir.path(), &tool, Provider::Ibcmd);
        let cancellation = CancellationToken::new();

        let result = held
            .interrupt_during(cancellation.clone(), || {
                execute(
                    &ExecutionContext::cli(CommandName::Reset).with_cancellation(cancellation),
                    &config,
                    &main_configuration(),
                )
            })
            .expect("the rollback ran to its end");

        assert_eq!(result.outcome, ResetOutcome::Discarded);
        let message = result.message.as_deref().unwrap_or_default();
        assert_eq!(
            message
                .matches("unsafe interruption was not performed")
                .count(),
            1,
            "{message}"
        );
    }

    /// Откат, отложивший отмену и потом не удавшийся, остаётся отказом платформы, называет
    /// отложенную отмену и подсказку об открытом Конфигураторе
    /// (`INV.USE-CASES.A-DEFERRED-CANCELLATION-OUTLIVES-A-LATER-FAILURE`).
    #[test]
    fn a_rollback_that_fails_after_a_deferred_cancellation_names_it() {
        let dir = tempdir().expect("tempdir");
        let held = HeldCommand::in_dir(dir.path());
        let tool = write_tool(
            dir.path(),
            "ibcmd",
            &held.script_branch("config reset", 255),
        );
        let config = config(dir.path(), &tool, Provider::Ibcmd);
        let cancellation = CancellationToken::new();

        let failure = held
            .interrupt_during(cancellation.clone(), || {
                execute(
                    &ExecutionContext::cli(CommandName::Reset).with_cancellation(cancellation),
                    &config,
                    &main_configuration(),
                )
            })
            .expect_err("the rollback failed");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Platform);
        let message = failure.error.message();
        assert!(
            message
                .starts_with("config reset ended after cancellation request during critical phase"),
            "{message}"
        );
        assert!(
            message.contains("close it and run reset again"),
            "{message}"
        );
        assert_eq!(
            failure.payload.expect("payload").outcome,
            ResetOutcome::Failed
        );
    }
}
