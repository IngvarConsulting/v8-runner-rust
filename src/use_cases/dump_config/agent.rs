//! Выгрузка через агентский shell Конфигуратора.
//!
//! Агент читает и пишет только внутри своего `AgentBaseDir`, поэтому цель выставляется
//! ему символической ссылкой: инкрементальная и частичная выгрузка обновляют цель на
//! месте, как у пакетного Конфигуратора, а полная идёт в каталог пользователя агента
//! и оттуда переносится той же ступенчатой публикацией. Перед полной и
//! инкрементальной выгрузкой агента спрашивают о поколении конфигурации: если оно
//! не изменилось с последней удачной загрузки или выгрузки, выгружать нечего.

use super::*;
use crate::platform::agent::WaitPolicy;
use crate::platform::locator::UtilityLocation;
use crate::platform::process::ProcessResult;
use crate::support::fs::move_dir;
use crate::use_cases::agent_session::{
    argument, collect_dir, collect_into_dir, connect, expose_dir, generation_id, make_output_dir,
    run_id, stage_file, tidy, transcript_log, wait_policy, withdraw_dir, write_text, AgentHandle,
    Exchange, GenerationLedger,
};

/// Выгрузка одного режима через одну сессию.
pub(super) fn run_dump_agent(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    mode: &DumpMode,
    objects: Option<&[PartialDumpSelector]>,
    location: Option<&UtilityLocation>,
    utilities: &mut PlatformUtilities,
) -> Result<(PlatformCommandResult, Option<String>, bool), AppError> {
    let wait = wait_policy(context);
    let transcript = transcript_log(config, &format!("dump-{}", resolved.source_set_name))?;

    log_live_stage("dump: agent", "[агент] opening the Designer agent session");
    let mut handle = connect(
        config,
        utilities,
        location.map(|found| found.path.as_path()),
        transcript.clone(),
        &wait,
    )?;
    let outcome = dump_through(context, config, resolved, mode, objects, &mut handle, &wait);
    handle.finish(&wait);
    let (reply_transcript, message, up_to_date) = outcome?;

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
        message,
        up_to_date,
    ))
}

fn dump_through(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    mode: &DumpMode,
    objects: Option<&[PartialDumpSelector]>,
    handle: &mut AgentHandle,
    wait: &WaitPolicy,
) -> Result<(String, Option<String>, bool), AppError> {
    let exchange = handle.exchange(config)?;
    let extension = resolved.extension.as_deref();
    let ledger = GenerationLedger::new(config);

    // Поколение спрашивается до выгрузки: сравнение на равенство с записью после
    // последней удачной операции и говорит, есть ли что выгружать.
    let generation = generation_id(handle.session(), extension, wait)?;
    if objects.is_none() {
        if let Some(record) = ledger.read(&resolved.source_set_name) {
            if record.token == generation {
                return Ok((
                    String::new(),
                    Some(format!(
                        "configuration generation {generation} is unchanged since the last {} ({}); nothing to dump",
                        record.after, record.recorded_at
                    )),
                    true,
                ));
            }
        }
    }

    let run = run_id();
    let stage = match mode {
        DumpMode::Full => "dump: full",
        DumpMode::Incremental => "dump: incremental",
        DumpMode::Partial => "dump: partial",
    };
    let (transcript, cleanup) = match mode {
        DumpMode::Full => {
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
            let cleanup = publish_full(context, resolved, &produced);
            let _ = std::fs::remove_dir(produced.parent().unwrap_or(&produced));
            (transcript, cleanup?)
        }
        DumpMode::Incremental => {
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
                    let command = with_extension(
                        format!(
                            "config dump-config-to-files --dir={} --update",
                            argument(&target)
                        ),
                        extension,
                    );
                    log_live_stage(stage, "[агент] exporting configuration files");
                    let outcome = run_command(handle, &command, wait);
                    withdraw_dir(user_dir, &target);
                    (outcome?, None)
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
                    let command = with_extension(
                        format!(
                            "config dump-config-to-files --dir={} --update",
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
                    (transcript, None)
                }
            }
        }
        DumpMode::Partial => {
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
            (outcome?, None)
        }
    };
    ledger.record(&resolved.source_set_name, &generation, "dump")?;
    Ok((transcript, cleanup, false))
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
    resolved: &ResolvedDumpTarget,
    produced: &Path,
) -> Result<Option<String>, AppError> {
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
    validate_platform_target(resolved).map_err(|error| publication.cleanup_failure(error))?;
    if let Some(error) = interruption_before_publish(context, "dump publication") {
        return Err(publication.cleanup_failure(error));
    }
    let publish_phase = publication
        .publish_dir(
            context,
            DUMP_BACKUP_PREFIX,
            "failed to publish staged dump",
            resolved.platform_consent(),
        )
        .map_err(|error| publication.cleanup_failure(error))?;
    Ok(merge_optional_messages(
        publish_phase.cleanup_warning,
        dump_publication_warning(context.command(), publish_phase.deferred_interruption),
    ))
}
