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
    argument, collect_dir, collect_into_dir, connect, expose_output_dir, generation_id,
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
                config,
                set,
                Provider::Agent,
                &generation,
            )
        }) {
            return Ok((String::new(), DumpNotes::message(Some(unchanged)), true));
        }
    }

    // Агент прогноза не спрашивает: начало выгрузки по изменившемуся отмечается в каталоге,
    // куда она ляжет, и режим узнаётся по факту.
    let by_fact = mark_start(context, resolved, plan, objects);
    let watch = dump_watch(config, set, by_fact);
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
            let produced = context
                .external_full_pull()
                .map(|external| external.stage().to_path_buf())
                .unwrap_or_else(|| config.work_path.join("agent").join("exchange").join(&run));
            let collected = transcript.as_ref().ok().map(|_| {
                if context.external_full_pull().is_some() {
                    collect_into_dir(handle, &exchange, &out_relative, &produced)
                } else {
                    collect_dir(handle, &exchange, &out_relative, &produced)
                }
            });
            tidy(handle, &exchange, &out_relative);
            let transcript = transcript?;
            collected.transpose()?;
            if let Some(external) = context.external_full_pull() {
                external.generation_before(Provider::Agent, Some(generation.clone()));
                external.generation_after(generation_id(handle.session(), extension, wait).ok());
            }
            let cleanup = publish_full(context, config, resolved, &produced);
            if context.external_full_pull().is_none() {
                let _ = std::fs::remove_dir(produced.parent().unwrap_or(&produced));
            }
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
                    let target = expose_output_dir(
                        user_dir,
                        &target_relative,
                        &resolved.platform_target_path,
                    )?;
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
                    (outcome?, DumpNotes::default())
                }
                // По сети цель целиком не возится: точке входа хватает описи выгрузки
                // (`ConfigDumpInfo.xml`), чтобы выгрузить только изменённое; обратно
                // приходят изменённые файлы и новая опись, поверх локальной цели. Удалений
                // точка входа не возвращает: их раннер переносит сам по двум описям.
                Exchange::Sftp => {
                    make_output_dir(handle, &exchange, &target_relative)?;
                    let dump_info = resolved.platform_target_path.join(VERSION_FILE_NAME);
                    // Прежняя опись читается до того, как выгрузка её перепишет.
                    let elements_before = match how {
                        OverDirectory::ByVersionFile => {
                            crate::platform::dump_format::read_dump_elements(&dump_info)
                        }
                        OverDirectory::Whole(_) => None,
                    };
                    if dump_info.is_file() {
                        stage_file(
                            handle,
                            &exchange,
                            &format!("{target_relative}/ConfigDumpInfo.xml"),
                            &dump_info,
                        )?;
                    }
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
                    if let Some(before) = &elements_before {
                        remove_dropped_elements(config, set, &dump_info, before)?;
                    }
                    (transcript, DumpNotes::default())
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
                    let target = expose_output_dir(
                        user_dir,
                        &target_relative,
                        &resolved.platform_target_path,
                    )?;
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
    let mut fact = None;
    let changed_note = match (set, plan) {
        (Some(set), DumpPlan::Full | DumpPlan::OverDirectory(_))
            if context.external_full_pull().is_none()
                && crate::use_cases::interruption::pending_interruption_error(
                    context,
                    "the configuration generation",
                )
                .is_none() =>
        {
            // Перечень каталога идёт, пока агент отвечает про поколение (#571).
            let (after, observed) = observe_dump_writes_while(
                Some(set),
                &config.work_path,
                by_fact,
                watch.as_ref(),
                &|| crate::use_cases::interruption::interruption_pending(context),
                || generation_id(handle.session(), extension, wait).ok(),
            );
            fact = observed.fact;
            merge_optional_messages(
                crate::use_cases::exchange_guard::record_after_dump(
                    config,
                    set,
                    Provider::Agent,
                    Some(&generation),
                    after.as_deref(),
                ),
                remember_dump_writes(
                    set,
                    &config.work_path,
                    observed.writes,
                    Some(&generation),
                    after.as_deref(),
                ),
            )
        }
        _ => None,
    };
    let mut notes = cleanup.after(foreign_note);
    notes.message = merge_optional_messages(notes.message, changed_note);
    notes.fact = fact;
    Ok((transcript, notes, false))
}

/// Удаления выгрузки по SFTP (`INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND`):
/// файл, чей владелец был в прежней описи `before` и пропал из новой в `dump_info`, удаляется
/// из каталога набора — в его области обхода, без ссылок, до того как раннер запомнит
/// записанное. Новая опись не разобрана — не удаляется ничего. Неудачное удаление — отказ с
/// перечнем оставшегося: каталог тогда не равен базе, и молчать об этом нельзя.
fn remove_dropped_elements(
    config: &AppConfig,
    set: Option<&SourceSetContext>,
    dump_info: &Path,
    before: &std::collections::HashSet<String>,
) -> Result<(), AppError> {
    let (Some(set), Some(after)) = (
        set,
        crate::platform::dump_format::read_dump_elements(dump_info),
    ) else {
        return Ok(());
    };
    let dropped = crate::change_detection::analyzer::files_of_dropped_elements(
        set,
        &config.work_path,
        before,
        &after,
    )
    .map_err(|error| {
        AppError::Runtime(format!(
            "the dump of source-set '{}' is in place, but the files of removed elements were not found: {error}",
            set.name()
        ))
    })?;
    let mut left = Vec::new();
    for file in &dropped {
        if let Err(error) = std::fs::remove_file(file) {
            if error.kind() != std::io::ErrorKind::NotFound {
                left.push(format!("'{}': {error}", file.display()));
            }
            continue;
        }
        // Каталоги, опустевшие от удалений, уходят вслед; непустой каталог не удаляется.
        let mut dir = file.parent();
        while let Some(current) = dir.filter(|current| *current != set.path()) {
            if std::fs::remove_dir(current).is_err() {
                break;
            }
            dir = current.parent();
        }
    }
    if left.is_empty() {
        return Ok(());
    }
    Err(AppError::Runtime(format!(
        "the dump of source-set '{}' is in place, but {} file(s) of removed elements were not deleted: {}",
        set.name(),
        left.len(),
        left.join("; ")
    )))
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
    if context.external_full_pull().is_some() {
        let publication = super::prepare_full_dump(context, resolved)?;
        return publish_full_dump(context, config, resolved, &publication);
    }
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
