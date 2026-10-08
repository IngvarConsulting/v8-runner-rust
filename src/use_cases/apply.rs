//! `apply`: основная конфигурация становится конфигурацией базы данных — отдельным шагом
//! после `push --no-apply` или после отправки, у которой не удалось только применение
//! (`INV.CLI.APPLY-IS-A-SEPARATE-STEP`). Здесь же владелец самого акта применения
//! ([`act`]), которым применяют и `push`, и расширение-инструмент.

pub(crate) mod act;
pub(crate) mod record;

use std::path::PathBuf;
use std::time::Instant;

use self::act::{Applier, Subject};
use crate::config::model::{AppConfig, SourceSetConfig};
use crate::domain::apply::{ApplyGeneration, ApplyOutcome, ApplyResult, ApplyStep};
use crate::domain::capability::{Operation, Provider};
use crate::domain::next_step::NextStep;
use crate::domain::source_set::{SourceSetContext, SourceSetPurpose};
use crate::domain::status::GenerationAfter;
use crate::platform::agent::WaitPolicy;
use crate::platform::locator::UtilityType;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::agent_session::{self, AgentHandle};
use crate::use_cases::context::{shell_word, ExecutionContext};
use crate::use_cases::extension_identity::extension_name_key;
use crate::use_cases::generation_reader::{designer_log_file, read_generation, GenerationProcess};
use crate::use_cases::interruption::{append_warnings, collecting_deferrals};
use crate::use_cases::request::ApplyRequest;
use crate::use_cases::result::{stamp_dispatch, UseCaseError, UseCaseFailure, UseCaseResult};
use crate::use_cases::source_inventory::SourceSetInventory;

/// Отказ отправки, у которой загрузка удалась, а применение нет: загруженное запомнено как
/// у `push --no-apply`, и выход — `apply` этого набора
/// (`INV.USE-CASES.A-PUSH-WHOSE-APPLY-FAILED-KEEPS-THE-LOAD`). Отмена остаётся отменой:
/// её род и место назовёт транспорт, а текст скажет, что загрузка сохранена.
pub(crate) fn load_kept_unapplied(
    context: &ExecutionContext,
    set: &str,
    error: AppError,
) -> AppError {
    let advice = context.advised_command(&format!("apply {}", shell_word(set)));
    let kept = format!(
        "source-set '{set}' is loaded into the main configuration but not applied to the database configuration; apply it with {advice}"
    );
    if error.cancellation().is_some() {
        return error.with_context(kept);
    }
    AppError::Refused(Box::new(
        UseCaseError::from(error)
            .followed_by(&kept)
            .with_next(NextStep::command("apply").for_source_set(set)),
    ))
}

type ApplyFailure = UseCaseFailure<ApplyResult>;

/// Caller must ensure exclusive ownership of `config.work_path` and of the file infobase.
pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ApplyRequest,
) -> UseCaseResult<ApplyResult> {
    stamp_dispatch(
        crate::use_cases::provider_selection::stamp_session(run(context, config, request), context),
        context.work(),
    )
}

fn empty(started: Instant) -> ApplyResult {
    ApplyResult {
        provider: None,
        ok: false,
        provider_dispatched: false,
        steps: Vec::new(),
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

fn run(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ApplyRequest,
) -> UseCaseResult<ApplyResult> {
    let started = Instant::now();
    let mut utilities = PlatformUtilities::from_config(config);
    let (selected, receipt) = match crate::use_cases::provider_selection::select(
        config,
        &mut utilities,
        Operation::Apply,
    ) {
        Ok(selected) => {
            let receipt = selected.receipt.clone();
            (selected, receipt)
        }
        Err((error, receipt)) => {
            return crate::use_cases::provider_selection::attach(
                Err(ApplyFailure::with_payload(error, empty(started))),
                &receipt,
            )
        }
    };
    let mut executor = Executor::new(
        selected.provider,
        selected.location.map(|location| location.path),
        utilities,
    );
    let outcome = walk(context, config, request, &mut executor, started);
    executor.finish();
    crate::use_cases::provider_selection::attach(outcome, &receipt)
}

/// Что применяется: набор проекта или расширение-инструмент.
struct Target<'a> {
    name: String,
    purpose: SourceSetPurpose,
    extension: Option<&'a str>,
    /// Контекст памяти набора; у расширения-инструмента записи поколения нет.
    memory: Option<&'a SourceSetContext>,
    /// Почему шаг пропускается, если пропускается.
    skip: Option<String>,
}

fn walk(
    context: &ExecutionContext,
    config: &AppConfig,
    request: &ApplyRequest,
    executor: &mut Executor,
    started: Instant,
) -> UseCaseResult<ApplyResult> {
    let inventory = SourceSetInventory::new(config);
    let targets = match targets(context, config, request, &inventory, executor) {
        Ok(targets) => targets,
        Err(error) => return Err(ApplyFailure::with_payload(error, empty(started))),
    };
    let mut result = empty(started);
    let mut failure = None;
    // Отмены, которые отложило последнее применение: их уже назвал его шаг.
    let mut last_deferrals: Vec<String> = Vec::new();
    for (index, target) in targets.iter().enumerate() {
        let step_started = Instant::now();
        let mut step = ApplyStep {
            source_set: target.name.clone(),
            purpose: target.purpose,
            outcome: ApplyOutcome::NotRun,
            message: None,
            duration_ms: 0,
            generation: None,
        };
        if failure.is_some() {
            step.message = Some("not run after the previous failure".to_owned());
        } else if let Some(reason) = &target.skip {
            step.outcome = ApplyOutcome::Skipped;
            step.message = Some(reason.clone());
        } else if request.dry_run {
            step.outcome = ApplyOutcome::Planned;
            step.message = Some(format!(
                "would apply the main configuration to the database configuration; planned, {} not dispatched",
                executor.provider()
            ));
        } else {
            match apply_target(context, config, executor, target, index) {
                Ok(done) => {
                    step.outcome = ApplyOutcome::Applied;
                    step.generation = done.generation;
                    let warnings: Vec<String> =
                        done.deferrals.iter().cloned().chain(done.notes).collect();
                    last_deferrals = done.deferrals;
                    step.message = Some(append_warnings(
                        "applied the main configuration to the database configuration".to_owned(),
                        &warnings,
                    ));
                }
                Err(error) => {
                    step.outcome = ApplyOutcome::Failed;
                    step.message = Some(error.to_string());
                    failure = Some(error);
                }
            }
            step.duration_ms = step_started.elapsed().as_millis() as u64;
        }
        crate::use_cases::build_progress::log_timeline_stage(
            &step.source_set,
            "apply",
            step.message.as_deref().unwrap_or_default(),
            if step.outcome == ApplyOutcome::Failed {
                crate::use_cases::build_progress::TimelineStageStatus::Failed
            } else {
                crate::use_cases::build_progress::TimelineStageStatus::Succeeded
            },
        );
        result.steps.push(step);
    }
    result.duration_ms = started.elapsed().as_millis() as u64;
    match failure {
        None => {
            // Отмена, пришедшая после последнего применения, исход не меняет — применение
            // состоялось, — но ответ её называет
            // (`INV.USE-CASES.AN-INTERRUPTION-STATUS-MEANS-A-TERMINAL-OUTCOME`).
            if let Some(warning) = cancellation_after_apply(context, &last_deferrals) {
                if let Some(step) = result
                    .steps
                    .iter_mut()
                    .rev()
                    .find(|step| step.outcome == ApplyOutcome::Applied)
                {
                    let message = step.message.take().unwrap_or_default();
                    step.message = Some(append_warnings(message, &[warning]));
                }
            }
            result.ok = true;
            Ok(result)
        }
        Some(error) => Err(ApplyFailure::with_payload(error, result)),
    }
}

/// Шаги команды по порядку наборов: с набором — он один; без набора — все наборы, а за
/// ними расширение-инструмент. Внешние наборы в базу не идут. Без набора расширения, которых
/// в базе нет, пропускаются: состав базы читает и сопоставляет с наборами то же место, что у
/// `pull --all` (`INV.USE-CASES.INSTALLED-EXTENSIONS-ARE-MATCHED-IN-ONE-PLACE`). Превью
/// состав базы не читает.
fn targets<'a>(
    context: &ExecutionContext,
    config: &'a AppConfig,
    request: &ApplyRequest,
    inventory: &'a SourceSetInventory<'a>,
    executor: &Executor,
) -> Result<Vec<Target<'a>>, AppError> {
    let sets: Vec<&SourceSetConfig> = match request.source_set.as_deref().map(str::trim) {
        Some("") => {
            return Err(AppError::Validation(
                "apply source-set requires a non-empty name".to_owned(),
            ))
        }
        Some(name) => vec![inventory.named(name)?],
        None => inventory.ordered_source_sets(),
    };
    let tool_extension = request
        .source_set
        .is_none()
        .then(|| crate::use_cases::tool_extension::client_mcp_extension(config))
        .flatten();
    let reads_installed = request.source_set.is_none()
        && !request.dry_run
        && (tool_extension.is_some()
            || sets
                .iter()
                .any(|set| set.purpose == SourceSetPurpose::Extension));
    let (not_installed, installed): (Vec<&str>, Vec<String>) = if reads_installed {
        let installed = crate::use_cases::installed_extensions::read_installed_extensions(
            context,
            config,
            Operation::Apply,
            executor.provider(),
            executor.binary(),
            &executor.utilities,
        )?;
        let packages = inventory.installed_packages(&installed)?;
        (
            packages
                .not_installed
                .iter()
                .map(|set| set.name.as_str())
                .collect(),
            installed,
        )
    } else {
        (Vec::new(), Vec::new())
    };
    let mut targets: Vec<Target<'a>> = sets
        .into_iter()
        .map(|set| Target {
            name: set.name.clone(),
            purpose: set.purpose,
            extension: (set.purpose == SourceSetPurpose::Extension).then_some(set.name.as_str()),
            memory: inventory.designer_context(&set.name),
            skip: if set.purpose.is_external() {
                Some("external files are not loaded into the infobase".to_owned())
            } else if not_installed.contains(&set.name.as_str()) {
                Some("the extension is not installed in the infobase".to_owned())
            } else {
                None
            },
        })
        .collect();
    if let Some(extension) = tool_extension {
        let key = extension_name_key(&extension.name);
        targets.push(Target {
            name: format!("tool:{}", extension.name),
            purpose: SourceSetPurpose::Extension,
            extension: Some(extension.name.as_str()),
            memory: None,
            skip: (reads_installed
                && !installed.iter().any(|name| extension_name_key(name) == key))
            .then(|| "the extension is not installed in the infobase".to_owned()),
        });
    }
    Ok(targets)
}

/// Применение одного набора под записью поколения
/// (`INV.USE-CASES.A-LOAD-WITHOUT-APPLY-IS-REMEMBERED-AS-UNAPPLIED`). Поколение до и после
/// читает инструмент, которым сделана запись: токены разных инструментов несравнимы. Совпало
/// с записью — запись переносится на ответ после применения; разошлось — применение идёт,
/// запись остаётся, и следующая отправка это назовёт; ответа нет — запись после применения
/// стирается. Запись перед неудачной загрузкой — отказ до применения при любом поколении.
fn apply_target(
    context: &ExecutionContext,
    config: &AppConfig,
    executor: &mut Executor,
    target: &Target<'_>,
    index: usize,
) -> Result<TargetApplied, AppError> {
    let record = target.memory.and_then(|set| {
        crate::use_cases::exchange_guard::recorded_generation(set, &config.work_path)
    });
    let (Some(set), Some(record)) = (target.memory, record) else {
        let deferrals = applied(context, config, executor, target, index)?;
        return Ok(TargetApplied {
            generation: target.memory.map(|_| ApplyGeneration::Unchecked),
            deferrals,
            notes: Vec::new(),
        });
    };
    // Запись перед неудачной загрузкой не доказывает ничего и при равном поколении:
    // Конфигуратор читает поколение применённого расширения, и наполовину загруженное оно не
    // показывает (замер 8.3.27.2074). Отмена, которая уже пришла, отвечает отменой, а не
    // отказом (`INV.USE-CASES.AN-APPLY-AFTER-A-FAILED-LOAD-IS-REFUSED`).
    if record.after == GenerationAfter::FailedBuild {
        if let Some(error) = crate::use_cases::interruption::pending_interruption_error(
            context,
            format!("apply for source-set '{}'", target.name),
        ) {
            return Err(error);
        }
        // Поколение здесь нужно только тексту отказа: сбой чтения отказ не подменяет.
        let before = executor
            .read_generation(context, config, record.tool, target, index)
            .ok()
            .flatten();
        return Err(crate::use_cases::exchange_guard::apply_after_failed_load(
            context,
            config,
            &target.name,
            before.as_deref(),
            &record,
        ));
    }
    let before = executor.read_generation(context, config, record.tool, target, index)?;
    match before {
        Some(token) if token == record.token => {
            let deferrals = applied(context, config, executor, target, index)?;
            let after = executor.read_generation(context, config, record.tool, target, index);
            let (generation, note) =
                record::carry_after_apply(set, &config.work_path, &record, after)?;
            Ok(TargetApplied {
                generation: Some(generation),
                deferrals,
                notes: note.into_iter().collect(),
            })
        }
        Some(token) => {
            let deferrals = applied(context, config, executor, target, index)?;
            Ok(TargetApplied {
                generation: Some(ApplyGeneration::Kept),
                deferrals,
                notes: vec![format!(
                    "the infobase moved ahead of the record of source-set '{}' before the apply: its configuration generation was {token}, the record after the last {} is {}; the record is kept, so the next push that loads is refused until the infobase is pulled or overwritten",
                    target.name, record.after, record.token
                )],
            })
        }
        None => {
            let deferrals = applied(context, config, executor, target, index)?;
            let (generation, note) = record::erase_after_apply(
                set,
                &config.work_path,
                &format!(
                    "could not be read by {}, the tool of its record",
                    record.tool
                ),
            );
            Ok(TargetApplied {
                generation: Some(generation),
                deferrals,
                notes: note.into_iter().collect(),
            })
        }
    }
}

/// Применённый шаг: что стало с записью, отмены, которые отложил акт, и прочие
/// предупреждения — порознь, чтобы отмену после применения называть ровно раз.
struct TargetApplied {
    generation: Option<ApplyGeneration>,
    deferrals: Vec<String>,
    notes: Vec<String>,
}

/// Отмена, пришедшая после применения, которое её не отложило: исход она не меняет —
/// применение состоялось, — но ответ её называет. Отложенную самим актом уже назвал его шаг
/// (`deferrals`), и второй раз она не называется. Одно правило у `apply` и у отправки без
/// изменений.
pub(crate) fn cancellation_after_apply(
    context: &ExecutionContext,
    deferrals: &[String],
) -> Option<String> {
    if !deferrals.is_empty() {
        return None;
    }
    crate::use_cases::interruption::deferred_interruption_warning_after(context, "apply")
}

/// Сам акт применения: отмену, которую он отложил, называет и удача, и отказ.
fn applied(
    context: &ExecutionContext,
    config: &AppConfig,
    executor: &mut Executor,
    target: &Target<'_>,
    index: usize,
) -> Result<Vec<String>, AppError> {
    let subject = Subject {
        kind: if target.memory.is_some() {
            act::SubjectKind::SourceSet
        } else {
            act::SubjectKind::ToolExtension
        },
        // Набор называется своим именем, расширение-инструмент — именем расширения.
        name: match (target.memory, target.extension) {
            (None, Some(extension)) => extension,
            (Some(_), _) | (None, None) => target.name.as_str(),
        },
        extension: target.extension,
        timeline: &target.name,
    };
    collecting_deferrals(|deferrals| executor.apply(context, config, &subject, index, deferrals))
        .map(|((), warnings)| warnings)
}

/// Исполнитель команды: процесс платформы на каждое действие или одна сессия агента.
struct Executor {
    provider: Provider,
    binary: Option<PathBuf>,
    utilities: PlatformUtilities,
    /// Сессия агента открывается перед первым действием и живёт до конца команды.
    session: Option<(AgentHandle, WaitPolicy)>,
}

impl Executor {
    fn new(provider: Provider, binary: Option<PathBuf>, utilities: PlatformUtilities) -> Self {
        Self {
            provider,
            binary,
            utilities,
            session: None,
        }
    }

    const fn provider(&self) -> Provider {
        self.provider
    }

    fn binary(&self) -> Option<&std::path::Path> {
        self.binary.as_deref()
    }

    fn located(&self) -> Result<&std::path::Path, AppError> {
        self.binary.as_deref().ok_or_else(|| {
            AppError::Runtime(format!(
                "{} was not located before the apply",
                self.provider
            ))
        })
    }

    fn session(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
    ) -> Result<(&mut AgentHandle, WaitPolicy), AppError> {
        if self.session.is_none() {
            let wait = agent_session::wait_policy(context);
            let transcript = agent_session::transcript_log(config, "apply")?;
            let handle = agent_session::connect(
                context,
                config,
                &mut self.utilities,
                self.binary.as_deref(),
                transcript,
                &wait,
            )?;
            self.session = Some((handle, wait));
        }
        let (handle, wait) = self
            .session
            .as_mut()
            .expect("agent session was just opened");
        Ok((handle, wait.clone()))
    }

    fn apply(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        subject: &Subject<'_>,
        index: usize,
        deferrals: &mut crate::use_cases::interruption::Deferrals,
    ) -> Result<(), AppError> {
        match self.provider {
            Provider::Designer => {
                let binary = self.located()?.to_path_buf();
                let log_file = designer_log_file(
                    config,
                    &format!("apply-{index:02}-{}", subject.timeline.replace(':', "-")),
                )?;
                act::apply(
                    context,
                    config,
                    Applier::Designer {
                        binary: &binary,
                        runner: self.utilities.runner_for(UtilityType::V8),
                        log_file,
                    },
                    subject,
                    deferrals,
                )
            }
            Provider::Ibcmd => {
                let binary = self.located()?.to_path_buf();
                act::apply(
                    context,
                    config,
                    Applier::Ibcmd {
                        binary: &binary,
                        runner: self.utilities.runner_for(UtilityType::Ibcmd),
                    },
                    subject,
                    deferrals,
                )
            }
            Provider::Agent => {
                let (handle, wait) = self.session(context, config)?;
                act::apply(
                    context,
                    config,
                    Applier::Agent {
                        handle,
                        wait: &wait,
                    },
                    subject,
                    deferrals,
                )
            }
            other @ (Provider::IbcmdRs | Provider::Webinst) => Err(
                crate::use_cases::unimplemented_provider(Operation::Apply, other),
            ),
        }
    }

    /// Поколение инструментом `tool` — тем, что записал запись набора. Свой инструмент
    /// спрашивает сам; Конфигуратор и `ibcmd` ищутся, если исполнитель другой; агент другого
    /// исполнителя не спрашивается. `None` — ответа нет.
    fn read_generation(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        tool: Provider,
        target: &Target<'_>,
        index: usize,
    ) -> Result<Option<String>, AppError> {
        if tool == Provider::Agent {
            if self.provider != Provider::Agent {
                return Ok(None);
            }
            if crate::use_cases::interruption::pending_interruption_error(
                context,
                "the configuration generation",
            )
            .is_some()
            {
                return Ok(None);
            }
            let (handle, wait) = self.session(context, config)?;
            return agent_session::generation_id(handle.session(), target.extension, &wait)
                .map(Some);
        }
        let utility = match tool {
            Provider::Designer => UtilityType::V8,
            Provider::Ibcmd => UtilityType::Ibcmd,
            Provider::Agent | Provider::IbcmdRs | Provider::Webinst => return Ok(None),
        };
        let binary = if tool == self.provider {
            self.located()?.to_path_buf()
        } else {
            match self.utilities.locate(utility) {
                Ok(location) => location.path,
                Err(error) => {
                    tracing::debug!(%error, %tool, "the tool of the generation record is not found");
                    return Ok(None);
                }
            }
        };
        let runner = self.utilities.runner_for(utility);
        let name = format!("apply-{index:02}-{}-generation", target.name);
        read_generation(
            context,
            config,
            || {
                Ok(match utility {
                    UtilityType::Ibcmd => GenerationProcess::Ibcmd {
                        binary: &binary,
                        runner,
                        data_path: None,
                    },
                    UtilityType::V8 => GenerationProcess::Designer {
                        binary: &binary,
                        runner,
                        log_file: designer_log_file(config, &name)?,
                    },
                    other => {
                        return Err(AppError::Runtime(format!(
                            "{other:?} does not read the configuration generation"
                        )))
                    }
                })
            },
            target.extension,
        )
    }

    fn finish(&mut self) {
        if let Some((handle, wait)) = self.session.take() {
            handle.finish(&wait);
        }
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
    use crate::change_detection::source_sets::SourceSetsService;
    use crate::config::model::{
        AppConfig, PlatformToolConfig, SourceFormat, SourceSetConfig, SourceSetPurpose,
        TestsConfig, ToolsConfig,
    };
    use crate::domain::apply::{ApplyGeneration, ApplyOutcome};
    use crate::domain::capability::{Operation, Provider};
    use crate::domain::status::GenerationAfter;
    use crate::platform::process::HeldCommand;
    use crate::support::error::CancelledAt;
    use crate::use_cases::agent_session::{ApplyMark, GenerationLedger, Recorded};
    use crate::use_cases::context::{CommandName, ExecutionContext};
    use crate::use_cases::request::ApplyRequest;
    use crate::use_cases::result::UseCaseErrorKind;

    const FIRST: &str = "1111111111111111111111111111111111111111";

    /// Поддельная утилита: журнал вызовов, поколение Конфигуратора из файла `token` в `/Out`,
    /// и ветка `branch` перед удачным выходом.
    fn write_tool(path: &Path, calls: &Path, token: &Path, branch: &str) {
        let body = format!(
            "#!/bin/sh\nargs=\"$*\"\nprintf '%s\\n' \"$args\" >> '{calls}'\nout=''\nprev=''\nfor arg in \"$@\"; do\n  if [ \"$prev\" = '/Out' ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\n{branch}\ncase \"$args\" in *GetConfigGenerationID*) if [ -f '{token}' ]; then cat '{token}' > \"$out\"; fi; exit 0 ;; esac\nif [ -n \"$out\" ]; then : > \"$out\"; fi\nexit 0\n",
            calls = calls.display(),
            token = token.display(),
        );
        fs::write(path, body).expect("tool script");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable");
    }

    fn config(dir: &Path, tool: &Path, apply: Provider) -> AppConfig {
        let base = dir.join("base");
        fs::create_dir_all(base.join("main")).expect("main");
        fs::write(base.join("main").join("Configuration.xml"), "<x/>").expect("marker");
        fs::create_dir_all(base.join("epf")).expect("epf");
        AppConfig {
            base_path: base,
            work_path: dir.join("work"),
            format: SourceFormat::Designer,
            providers: [(Operation::Apply, apply)].into(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: Some("origin".to_owned()),
            source_sets: vec![
                SourceSetConfig {
                    name: "epf".to_owned(),
                    purpose: SourceSetPurpose::ExternalDataProcessors,
                    path: PathBuf::from("epf"),
                },
                SourceSetConfig {
                    name: "main".to_owned(),
                    purpose: SourceSetPurpose::Configuration,
                    path: PathBuf::from("main"),
                },
            ],
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

    fn ledger(config: &AppConfig) -> GenerationLedger {
        let set = SourceSetsService::new(config)
            .designer_contexts()
            .into_iter()
            .find(|set| set.name() == "main")
            .expect("main");
        GenerationLedger::of(&set, &config.work_path).expect("ledger")
    }

    fn all_sets() -> ApplyRequest {
        ApplyRequest {
            source_set: None,
            dry_run: false,
        }
    }

    /// Без набора `apply` идёт порядком наборов: основная конфигурация применяется, внешний
    /// набор в базу не идёт и пропускается.
    #[test]
    fn an_apply_walks_the_sets_in_order_and_skips_external_files() {
        let dir = tempdir().expect("tempdir");
        let (tool, calls) = (dir.path().join("1cv8"), dir.path().join("calls.log"));
        write_tool(&tool, &calls, &dir.path().join("token"), "");
        let config = config(dir.path(), &tool, Provider::Designer);

        let result = execute(
            &ExecutionContext::cli(CommandName::Apply),
            &config,
            &all_sets(),
        )
        .expect("apply");

        let outcomes: Vec<_> = result
            .steps
            .iter()
            .map(|step| (step.source_set.as_str(), step.outcome))
            .collect();
        assert_eq!(
            outcomes,
            [
                ("main", ApplyOutcome::Applied),
                ("epf", ApplyOutcome::Skipped)
            ]
        );
        assert_eq!(result.steps[0].generation, Some(ApplyGeneration::Unchecked));
        let calls = fs::read_to_string(&calls).expect("calls");
        assert_eq!(calls.matches("/UpdateDBCfg").count(), 1, "{calls}");
    }

    /// Отмена до точки безопасности останавливает команду до записи в базу.
    #[test]
    fn an_apply_stopped_at_its_safe_point_writes_nothing() {
        let dir = tempdir().expect("tempdir");
        let (tool, calls) = (dir.path().join("1cv8"), dir.path().join("calls.log"));
        write_tool(&tool, &calls, &dir.path().join("token"), "");
        let config = config(dir.path(), &tool, Provider::Designer);
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let failure = execute(
            &ExecutionContext::cli(CommandName::Apply).with_cancellation(cancellation),
            &config,
            &all_sets(),
        )
        .expect_err("cancelled");

        assert_eq!(
            failure.error.kind(),
            UseCaseErrorKind::Cancelled(CancelledAt::Boundary)
        );
        assert!(
            failure
                .error
                .message()
                .contains("before entering update_db_cfg for source-set 'main' safe point"),
            "{}",
            failure.error
        );
        let calls = fs::read_to_string(&calls).unwrap_or_default();
        assert!(!calls.contains("/UpdateDBCfg"), "{calls}");
    }

    /// Применение `ibcmd`, отложившее отмену и потом не удавшееся, остаётся отказом, но
    /// отложенную отмену называет (`INV.USE-CASES.A-DEFERRED-CANCELLATION-OUTLIVES-A-LATER-FAILURE`).
    #[test]
    fn an_apply_that_fails_after_a_deferred_cancellation_names_it() {
        let dir = tempdir().expect("tempdir");
        let (tool, calls) = (dir.path().join("ibcmd"), dir.path().join("calls.log"));
        let held = HeldCommand::in_dir(dir.path());
        write_tool(
            &tool,
            &calls,
            &dir.path().join("token"),
            &held.script_branch("config apply", 17),
        );
        let config = config(dir.path(), &tool, Provider::Ibcmd);
        let cancellation = CancellationToken::new();

        let failure = held
            .interrupt_during(cancellation.clone(), || {
                execute(
                    &ExecutionContext::cli(CommandName::Apply).with_cancellation(cancellation),
                    &config,
                    &all_sets(),
                )
            })
            .expect_err("the apply failed");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::Platform);
        let message = failure.error.message();
        assert!(
            message.starts_with("apply ended after cancellation request during critical phase"),
            "{message}"
        );
        let result = failure.payload.expect("payload");
        assert_eq!(result.steps[0].outcome, ApplyOutcome::Failed);
        assert_eq!(result.steps[1].outcome, ApplyOutcome::NotRun);
    }

    /// Отмену, которую отложило само применение, шаг называет один раз: предупреждение
    /// команды о пришедшей отмене к нему не добавляется.
    #[test]
    fn a_cancellation_deferred_by_the_last_apply_is_named_once() {
        let dir = tempdir().expect("tempdir");
        let (tool, calls) = (dir.path().join("ibcmd"), dir.path().join("calls.log"));
        let held = HeldCommand::in_dir(dir.path());
        write_tool(
            &tool,
            &calls,
            &dir.path().join("token"),
            &held.script_branch("config apply", 0),
        );
        let config = config(dir.path(), &tool, Provider::Ibcmd);
        let cancellation = CancellationToken::new();

        let result = held
            .interrupt_during(cancellation.clone(), || {
                execute(
                    &ExecutionContext::cli(CommandName::Apply).with_cancellation(cancellation),
                    &config,
                    &all_sets(),
                )
            })
            .expect("the apply ran to its end");

        let message = result.steps[0].message.as_deref().unwrap_or_default();
        assert_eq!(result.steps[0].outcome, ApplyOutcome::Applied);
        assert_eq!(
            message
                .matches("unsafe interruption was not performed")
                .count(),
            1,
            "{message}"
        );
    }

    /// Инструмент записи не ответил поколением: применение идёт, запись стирается, и ответ
    /// это называет (`INV.USE-CASES.A-LOAD-WITHOUT-APPLY-IS-REMEMBERED-AS-UNAPPLIED`).
    #[test]
    fn an_apply_without_an_answer_of_the_record_tool_erases_the_record() {
        let dir = tempdir().expect("tempdir");
        let (tool, calls) = (dir.path().join("1cv8"), dir.path().join("calls.log"));
        write_tool(&tool, &calls, &dir.path().join("token"), "");
        let config = config(dir.path(), &tool, Provider::Designer);
        ledger(&config)
            .record_as(
                Provider::Designer,
                FIRST,
                GenerationAfter::Build,
                ApplyMark::Unapplied,
            )
            .expect("record");

        let result = execute(
            &ExecutionContext::cli(CommandName::Apply),
            &config,
            &all_sets(),
        )
        .expect("apply");

        assert_eq!(result.steps[0].generation, Some(ApplyGeneration::Erased));
        assert!(result.steps[0]
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("its record is erased"));
        assert_eq!(ledger(&config).read(), Recorded::Nothing);
    }

    /// Запись помечена неудачной загрузкой, а инструмент записи поколением не ответил:
    /// расхождения не исключить, и `apply` отказывает до применения
    /// (`INV.USE-CASES.AN-APPLY-AFTER-A-FAILED-LOAD-IS-REFUSED`).
    #[test]
    fn an_apply_after_a_failed_load_without_an_answer_is_refused() {
        let dir = tempdir().expect("tempdir");
        let (tool, calls) = (dir.path().join("1cv8"), dir.path().join("calls.log"));
        write_tool(&tool, &calls, &dir.path().join("token"), "");
        let config = config(dir.path(), &tool, Provider::Designer);
        ledger(&config)
            .record_as(
                Provider::Designer,
                FIRST,
                GenerationAfter::FailedBuild,
                ApplyMark::Applied,
            )
            .expect("record");

        let failure = execute(
            &ExecutionContext::cli(CommandName::Apply),
            &config,
            &all_sets(),
        )
        .expect_err("refused");

        assert_eq!(failure.error.kind(), UseCaseErrorKind::NonFastForward);
        assert_eq!(
            failure.error.next().map(|next| next.command.as_str()),
            Some("push")
        );
        let calls = fs::read_to_string(&calls).expect("calls");
        assert!(!calls.contains("/UpdateDBCfg"), "{calls}");
    }

    /// Пришедшая отмена отвечает отменой, а не отказом `non_fast_forward`, и у записи перед
    /// неудачной загрузкой.
    #[test]
    fn a_pending_cancellation_before_a_failed_load_refusal_answers_the_cancellation() {
        let dir = tempdir().expect("tempdir");
        let (tool, calls) = (dir.path().join("1cv8"), dir.path().join("calls.log"));
        write_tool(&tool, &calls, &dir.path().join("token"), "");
        let config = config(dir.path(), &tool, Provider::Designer);
        ledger(&config)
            .record_as(
                Provider::Designer,
                FIRST,
                GenerationAfter::FailedBuild,
                ApplyMark::Applied,
            )
            .expect("record");
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let failure = execute(
            &ExecutionContext::cli(CommandName::Apply).with_cancellation(cancellation),
            &config,
            &all_sets(),
        )
        .expect_err("cancelled");

        assert!(
            matches!(failure.error.kind(), UseCaseErrorKind::Cancelled(_)),
            "{}",
            failure.error
        );
        assert!(fs::read_to_string(&calls).unwrap_or_default().is_empty());
    }

    /// Запись сделана другим инструментом: поколение до и после читает он, а применяет
    /// исполнитель `apply`; совпавшая запись переносится с его ответом и снятым признаком.
    #[test]
    fn an_apply_reads_the_generation_with_the_tool_of_the_record() {
        let dir = tempdir().expect("tempdir");
        let calls = dir.path().join("calls.log");
        let token = dir.path().join("token");
        fs::write(&token, format!("{FIRST}\r\n")).expect("token");
        write_tool(&dir.path().join("1cv8"), &calls, &token, "");
        write_tool(&dir.path().join("ibcmd"), &calls, &token, "");
        let config = config(dir.path(), &dir.path().join("1cv8"), Provider::Ibcmd);
        ledger(&config)
            .record_as(
                Provider::Designer,
                FIRST,
                GenerationAfter::Build,
                ApplyMark::Unapplied,
            )
            .expect("record");

        let result = execute(
            &ExecutionContext::cli(CommandName::Apply),
            &config,
            &all_sets(),
        )
        .expect("apply");

        assert_eq!(result.steps[0].generation, Some(ApplyGeneration::Recorded));
        let Recorded::Ours(record) = ledger(&config).read() else {
            panic!("the record is carried over");
        };
        assert_eq!(record.tool, Provider::Designer);
        assert_eq!(record.token, FIRST);
        assert!(record.applied);
        let calls = fs::read_to_string(&calls).expect("calls");
        assert_eq!(
            calls.matches("/GetConfigGenerationID").count(),
            2,
            "{calls}"
        );
        assert!(calls.contains("config apply"), "{calls}");
        assert!(!calls.contains("/UpdateDBCfg"), "{calls}");
    }
}
