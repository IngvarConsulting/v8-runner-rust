//! Выгрузка через агентский shell Конфигуратора.
//!
//! Агент читает и пишет только внутри своего `AgentBaseDir`, поэтому цель выставляется
//! ему символической ссылкой: инкрементальная и частичная выгрузка обновляют цель на
//! месте, как у пакетного Конфигуратора, а полная идёт в каталог пользователя агента
//! и оттуда переносится той же ступенчатой публикацией. Перед
//! инкрементальной выгрузкой агента спрашивают о поколении конфигурации: если оно
//! не изменилось с последней удачной загрузки или выгрузки, выгружать нечего.

use super::*;
use crate::domain::capability::Provider;
use crate::platform::agent::WaitPolicy;
use crate::platform::locator::UtilityLocation;
use crate::platform::process::ProcessResult;
use crate::support::fs::move_dir;
use crate::use_cases::agent_session::{
    argument, collect_dir, collect_file, collect_into_dir, connect, expose_dir, generation_id,
    make_output_dir, run_id, stage_file, tidy, transcript_log, wait_policy, withdraw_dir,
    write_text, AgentHandle, Exchange, GenerationLedger, Recorded,
};

/// Выгрузка одного плана через одну сессию. Выгрузка по изменившемуся без годного файла
/// версий (`OverDirectory::Whole`) идёт полной поверх каталога, без `--update`.
pub(super) fn run_dump_agent(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    plan: &DumpPlan,
    objects: Option<&[PartialDumpSelector]>,
    location: Option<&UtilityLocation>,
    utilities: &mut PlatformUtilities,
) -> Result<(PlatformCommandResult, DumpNotes, bool), AppError> {
    let wait = wait_policy(context);
    let transcript = transcript_log(config, &format!("dump-{}", resolved.source_set_name))?;

    log_live_stage("dump: agent", "[агент] opening the Designer agent session");
    let mut handle = connect(
        context,
        config,
        utilities,
        location.map(|found| found.path.as_path()),
        transcript.clone(),
        &wait,
    )?;
    let outcome = dump_through(context, config, resolved, plan, objects, &mut handle, &wait);
    handle.finish(&wait);
    let (reply_transcript, notes, up_to_date) = outcome?;

    Ok((
        PlatformCommandResult {
            process: ProcessResult {
                exit_code: 0,
                stdout: reply_transcript,
                stderr: String::new(),
                interruption: None,
            },
            platform_log_path: Some(transcript),
            platform_log: None,
            platform_log_read_error: None,
        },
        notes,
        up_to_date,
    ))
}

fn dump_through(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    plan: &DumpPlan,
    objects: Option<&[PartialDumpSelector]>,
    handle: &mut AgentHandle,
    wait: &WaitPolicy,
) -> Result<(String, DumpNotes, bool), AppError> {
    let exchange = handle.exchange(config)?;
    let extension = resolved.extension.as_deref();
    let inventory = SourceSetInventory::new(config);
    let set = inventory.designer_context(&resolved.source_set_name);
    let ledger = set.and_then(|source| GenerationLedger::of(source, &config.work_path));

    // Поколение спрашивается до выгрузки: сравнение на равенство с записью после
    // последней удачной операции и говорит, есть ли что выгружать.
    let generation = generation_id(handle.session(), extension, wait)?;
    let recorded = ledger
        .as_ref()
        .map_or(Recorded::Nothing, GenerationLedger::read);
    let foreign_note = match &recorded {
        Recorded::Foreign { identity } => Some(format!(
            "the recorded configuration generation belongs to {identity}, not to this base and directory; it was not used"
        )),
        Recorded::Nothing | Recorded::Ours(_) => None,
    };
    // Пропуск по поколению — только у выгрузки по изменившемуся от годного файла версий:
    // полная поверх каталога его пишет, и каталог без него не годится как «уже выгружено».
    if matches!(plan, DumpPlan::OverDirectory(OverDirectory::ByVersionFile)) && objects.is_none() {
        if let Some(unchanged) = set.and_then(|set| {
            crate::use_cases::exchange_guard::unchanged_since_the_record(
                set,
                &config.work_path,
                Provider::Agent,
                &generation,
            )
        }) {
            return Ok((String::new(), DumpNotes::message(Some(unchanged)), true));
        }
    }

    let run = run_id();
    let stage = match plan.mode() {
        DumpMode::Full => "dump: full",
        DumpMode::Incremental => "dump: incremental",
        DumpMode::Partial => "dump: partial",
    };
    let (transcript, cleanup) = match plan {
        DumpPlan::Full => {
            let out_relative = format!("dump/{run}");
            make_output_dir(handle, &exchange, &out_relative)?;
            let command = with_extension(
                format!(
                    "config dump-config-to-files --dir={}",
                    argument(&out_relative)
                ),
                extension,
            );
            log_live_stage(stage, "[агент] exporting configuration files");
            let transcript = run_command(handle, &command, wait);
            // Результат забирается к раннеру под workPath и уже оттуда публикуется
            // ступенчато; на стороне точки входа следа не остаётся.
            let produced = config.work_path.join("agent").join("exchange").join(&run);
            let collected = transcript
                .as_ref()
                .ok()
                .map(|_| collect_dir(handle, &exchange, &out_relative, &produced));
            tidy(handle, &exchange, &out_relative);
            let transcript = transcript?;
            collected.transpose()?;
            let cleanup = publish_full(context, config, resolved, &produced);
            let _ = std::fs::remove_dir(produced.parent().unwrap_or(&produced));
            (transcript, cleanup?)
        }
        DumpPlan::OverDirectory(how) => {
            let update = match how {
                OverDirectory::ByVersionFile => " --update",
                OverDirectory::Whole(_) => "",
            };
            ensure_dir(&resolved.platform_target_path).map_err(|error| {
                AppError::Runtime(format!("failed to create target dir: {error}"))
            })?;
            let target_relative = format!("target/{run}");
            match &exchange {
                // Каталог точки входа виден раннеру: цель выставляется ссылкой и
                // обновляется на месте, как у пакетного Конфигуратора.
                Exchange::Dir(user_dir) => {
                    let target =
                        expose_dir(user_dir, &target_relative, &resolved.platform_target_path)?;
                    let forecast = forecast_for(how, || {
                        forecast_through(handle, &exchange, &target, extension, wait, config, &run)
                    });
                    let command = with_extension(
                        format!(
                            "config dump-config-to-files --dir={}{update}",
                            argument(&target)
                        ),
                        extension,
                    );
                    log_live_stage(stage, "[агент] exporting configuration files");
                    let outcome = run_command(handle, &command, wait);
                    withdraw_dir(user_dir, &target);
                    (
                        outcome?,
                        DumpNotes {
                            forecast,
                            ..DumpNotes::default()
                        },
                    )
                }
                // По сети цель целиком не возится: точке входа хватает описи выгрузки
                // (`ConfigDumpInfo.xml`), чтобы выгрузить только изменённое; обратно
                // приходят изменённые файлы и новая опись, поверх локальной цели.
                Exchange::Sftp => {
                    make_output_dir(handle, &exchange, &target_relative)?;
                    let dump_info = resolved.platform_target_path.join("ConfigDumpInfo.xml");
                    if dump_info.is_file() {
                        stage_file(
                            handle,
                            &exchange,
                            &format!("{target_relative}/ConfigDumpInfo.xml"),
                            &dump_info,
                        )?;
                    }
                    let forecast = forecast_for(how, || {
                        forecast_through(
                            handle,
                            &exchange,
                            &target_relative,
                            extension,
                            wait,
                            config,
                            &run,
                        )
                    });
                    let command = with_extension(
                        format!(
                            "config dump-config-to-files --dir={}{update}",
                            argument(&target_relative)
                        ),
                        extension,
                    );
                    log_live_stage(stage, "[агент] exporting changed configuration files");
                    let outcome = run_command(handle, &command, wait);
                    let collected = outcome.as_ref().ok().map(|_| {
                        collect_into_dir(
                            handle,
                            &exchange,
                            &target_relative,
                            &resolved.platform_target_path,
                        )
                    });
                    tidy(handle, &exchange, &target_relative);
                    let transcript = outcome?;
                    collected.transpose()?;
                    (
                        transcript,
                        DumpNotes {
                            forecast,
                            ..DumpNotes::default()
                        },
                    )
                }
            }
        }
        DumpPlan::Partial => {
            let objects = objects.ok_or_else(|| {
                AppError::Runtime(
                    "partial dump objects were not validated before execution".to_owned(),
                )
            })?;
            ensure_dir(&resolved.platform_target_path).map_err(|error| {
                AppError::Runtime(format!("failed to create target dir: {error}"))
            })?;
            let list_relative = format!("dump-lists/{run}.txt");
            let list = objects
                .iter()
                .map(|object| format!("{}\n", object.normalized()))
                .collect::<String>();
            write_text(handle, &exchange, &list_relative, &list)?;
            let target_relative = format!("target/{run}");
            let outcome = match &exchange {
                Exchange::Dir(user_dir) => {
                    let target =
                        expose_dir(user_dir, &target_relative, &resolved.platform_target_path)?;
                    let command = with_extension(
                        format!(
                            "config dump-config-to-files --dir={} --list-file={}",
                            argument(&target),
                            argument(&list_relative)
                        ),
                        extension,
                    );
                    log_live_stage(stage, "[агент] exporting selected configuration objects");
                    let outcome = run_command(handle, &command, wait);
                    withdraw_dir(user_dir, &target);
                    outcome
                }
                Exchange::Sftp => {
                    make_output_dir(handle, &exchange, &target_relative)?;
                    let command = with_extension(
                        format!(
                            "config dump-config-to-files --dir={} --list-file={}",
                            argument(&target_relative),
                            argument(&list_relative)
                        ),
                        extension,
                    );
                    log_live_stage(stage, "[агент] exporting selected configuration objects");
                    let outcome = run_command(handle, &command, wait);
                    let collected = outcome.as_ref().ok().map(|_| {
                        collect_into_dir(
                            handle,
                            &exchange,
                            &target_relative,
                            &resolved.platform_target_path,
                        )
                    });
                    tidy(handle, &exchange, &target_relative);
                    collected.transpose().and(outcome)
                }
            };
            tidy(handle, &exchange, &list_relative);
            (outcome?, DumpNotes::default())
        }
    };
    // Поколение спрашивается и после выгрузки: то же — память записывается, другое — базу
    // правили во время выгрузки, и ответ это называет. Выборка объектов каталог с базой не
    // сводит и поколения не пишет; после отмены поколение не спрашивается.
    let changed_note = match (set, plan) {
        (Some(set), DumpPlan::Full | DumpPlan::OverDirectory(_))
            if crate::use_cases::interruption::pending_interruption_error(
                context,
                "the configuration generation",
            )
            .is_none() =>
        {
            let after = generation_id(handle.session(), extension, wait).ok();
            crate::use_cases::exchange_guard::record_after_dump(
                set,
                &config.work_path,
                Provider::Agent,
                Some(&generation),
                after.as_deref(),
            )
        }
        _ => None,
    };
    let mut notes = cleanup.after(foreign_note);
    notes.message = merge_optional_messages(notes.message, changed_note);
    Ok((transcript, notes, false))
}

/// Прогноз нужен только выгрузке по изменившемуся.
fn forecast_for(how: &OverDirectory, ask: impl FnOnce() -> DumpForecast) -> Option<DumpForecast> {
    match how {
        OverDirectory::ByVersionFile => Some(ask()),
        OverDirectory::Whole(_) => None,
    }
}

/// Прогноз агента: `--get-changes` в той же сессии перед `--update` (замер #166). Список
/// агент пишет в свой каталог пользователя, раннер его забирает. Отказ агента и ответ вне
/// словаря — «неизвестен»: выгрузка идёт как просили.
fn forecast_through(
    handle: &mut AgentHandle,
    exchange: &Exchange,
    target: &str,
    extension: Option<&str>,
    wait: &WaitPolicy,
    config: &AppConfig,
    run: &str,
) -> DumpForecast {
    let list_relative = format!("dump-forecast/{run}.txt");
    let command = with_extension(
        format!(
            "config dump-config-to-files --dir={} --update --get-changes={}",
            argument(target),
            argument(&list_relative)
        ),
        extension,
    );
    log_live_stage("dump: forecast", "[агент] asking what the dump will do");
    let Ok(local) = crate::support::temp::dump_forecast_file(&config.work_path) else {
        return DumpForecast::Unknown;
    };
    let read = run_command(handle, &command, wait)
        .and_then(|_| collect_file(handle, exchange, &list_relative, local.path()))
        .ok()
        .and_then(|()| std::fs::read(local.path()).ok());
    tidy(handle, exchange, &list_relative);
    read.map_or(DumpForecast::Unknown, |bytes| {
        crate::platform::dump_forecast::read_changes_list(&bytes)
    })
}

fn with_extension(mut command: String, extension: Option<&str>) -> String {
    if let Some(extension) = extension {
        command.push_str(&format!(" --extension={}", argument(extension)));
    }
    command
}

fn run_command(
    handle: &mut AgentHandle,
    command: &str,
    wait: &WaitPolicy,
) -> Result<String, AppError> {
    let reply = handle
        .session()
        .run(command, wait)
        .map_err(AppError::from)?;
    reply.outcome().map_err(AppError::from)?;
    Ok(reply.transcript())
}

/// Перенос выгрузки в цель той же ступенчатой публикацией, что у Конфигуратора.
fn publish_full(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    produced: &Path,
) -> Result<DumpNotes, AppError> {
    let publication = StagedPublication::prepare_dir(
        &resolved.platform_target_path,
        &resolved.platform_target_identity,
        ".dump-stage",
    )?;
    let staging_dir = publication.staging_path().to_path_buf();
    // `prepare_dir` создаёт пустой каталог стадии; результат агента занимает его место.
    if let Err(error) = std::fs::remove_dir(&staging_dir)
        .and_then(|_| move_dir(produced, &staging_dir))
        .map_err(|error| AppError::Runtime(format!("failed to stage agent dump: {error}")))
    {
        return Err(publication.cleanup_failure(error));
    }
    publish_full_dump(context, config, resolved, &publication)
}
