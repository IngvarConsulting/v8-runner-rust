//! Выгрузка через агентский shell Конфигуратора.
//!
//! Агент читает и пишет только внутри своего `AgentBaseDir`, поэтому каталог выставляется
//! ему символической ссылкой: выгрузка по изменившемуся пишет в стадию раннера по копии
//! описи (#646) и ложится поверх каталога так же, как у Конфигуратора; выборка обновляет
//! цель на месте, а полная идёт в каталог пользователя агента и оттуда переносится той же
//! ступенчатой публикацией. Перед
//! инкрементальной выгрузкой агента спрашивают о поколении конфигурации: если оно
//! не изменилось с последней удачной загрузки или выгрузки, выгружать нечего.

use super::*;
use crate::domain::capability::Provider;
use crate::domain::capability::{Operation, OperationSize};
use crate::domain::progress::ProgressPhase;
use crate::platform::agent::WaitPolicy;
use crate::platform::locator::UtilityLocation;
use crate::platform::locator::UtilityType;
use crate::platform::process::ProcessResult;
use crate::support::fs::move_dir;
use crate::use_cases::agent_session::{
    argument, collect_dir, collect_into_dir, connect, expose_file, expose_output_dir,
    generation_id, make_output_dir, run_id, stage_file, tidy, tidy_run_path, transcript_log,
    wait_policy, withdraw_dir, write_text, AgentHandle, EarlyAgent, EarlySession, Exchange,
    GenerationLedger, Recorded,
};
use crate::use_cases::version_file::RunnerVersionFile;

/// Живой агент, которого `pull` поднимает в начале команды, пока сам готовится (#574): только
/// если агент, судя по дешёвому прогнозу, и станет исполнителем. Прогноз — цепочка по объёму,
/// который план выгрузки скажет по копии описи раннера; ошибка прогноза стоит
/// поднятого живого агента, который живёт по обычным правилам. Превью, внешняя публикация,
/// расширение и неназванный набор из нескольких не поднимают ничего.
pub(super) fn start_early(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &DumpArgs,
    mode: &DumpMode,
) -> Option<EarlyAgent> {
    if args.dry_run
        || context.external_full_pull().is_some()
        || config.format != SourceFormat::Designer
        || args.extension.is_some()
    {
        return None;
    }
    let location = PlatformUtilities::from_config(config)
        .locate(UtilityType::V8)
        .ok()?;
    let size = match mode {
        DumpMode::Full => OperationSize::Whole,
        DumpMode::Partial => OperationSize::ByChanges,
        DumpMode::Incremental => {
            let inventory = SourceSetInventory::new(config);
            let set = match args.source_set.as_deref() {
                Some(name) => inventory.designer_context(name)?,
                None => match inventory.designer_contexts() {
                    [only] => only,
                    _ => return None,
                },
            };
            let version = RunnerVersionFile::of(set);
            let copy = version
                .as_ref()
                .map(RunnerVersionFile::would_be_usable)
                .transpose()
                .ok()?
                .flatten();
            match plan_dump(mode, copy, super::coordinator::platform_of(Some(&location))) {
                Ok(DumpPlan::OverDirectory(OverDirectory::ByVersionFile) | DumpPlan::Partial) => {
                    OperationSize::ByChanges
                }
                Ok(DumpPlan::OverDirectory(OverDirectory::Whole(_)) | DumpPlan::Full) | Err(_) => {
                    OperationSize::Whole
                }
            }
        }
    };
    let (plan, _) = config.provider_plan_sized(Operation::Dump, size);
    if plan.candidates().first() != Some(&Provider::Agent) {
        return None;
    }
    EarlyAgent::start(context, config, &location.path, EarlySession::Now)
}

/// Выгрузка одного плана через одну сессию. Выгрузка по изменившемуся без годного файла
/// версий (`OverDirectory::Whole`) идёт полной поверх каталога, без `--update`.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_dump_agent(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    plan: &DumpPlan,
    objects: Option<&[PartialDumpSelector]>,
    location: Option<&UtilityLocation>,
    utilities: &mut PlatformUtilities,
    version_file: Option<&RunnerVersionFile>,
    early: Option<EarlyAgent>,
) -> Result<(PlatformCommandResult, DumpNotes, bool), AppError> {
    let wait = wait_policy(context);
    let transcript = transcript_log(config, &format!("dump-{}", resolved.source_set_name))?;

    log_live_stage(
        ProgressPhase::Session,
        Some(resolved.source_set_name.as_str()),
        "dump: agent",
        "[агент] opening the Designer agent session",
    );
    let mut session = Some(connect(
        context,
        config,
        utilities,
        location.map(|found| found.path.as_path()),
        transcript.clone(),
        &wait,
        early,
    )?);
    let outcome = dump_through(
        context,
        config,
        resolved,
        plan,
        objects,
        version_file,
        &mut session,
        &wait,
    );
    // Удачная выгрузка закрывает сессию сама, пока перечисляет каталог (#574); иначе — здесь.
    if let Some(handle) = session {
        handle.finish(context, &wait);
    }
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
    version_file: Option<&RunnerVersionFile>,
    session: &mut Option<AgentHandle>,
    wait: &WaitPolicy,
) -> Result<(String, DumpNotes, bool), AppError> {
    let handle = session
        .as_mut()
        .expect("the session is open until the dump is observed");
    let exchange = handle.exchange(config)?;
    let extension = resolved.extension.as_deref();
    let inventory = SourceSetInventory::new(config);
    let set = inventory.designer_context(&resolved.source_set_name);
    let ledger = set.and_then(GenerationLedger::of);

    // Поколение спрашивается до выгрузки: сравнение на равенство с записью после
    // последней удачной операции и говорит, есть ли что выгружать; выгрузку, которая будет
    // писать, сторож спрашивает следом (#683). Пока агент отвечает, перечисляется дерево, в
    // которое ляжет выгрузка (#574): отказы до платформы уже позади.
    let (asked, before) = list_before_dump_while(
        set,
        &config.work_path,
        watches_the_dump(context, config, plan, objects),
        &|| crate::use_cases::interruption::interruption_pending(context),
        || {
            let generation = generation_id(handle, extension, wait)?;
            let decision = super::coordinator::generation_then_guard(
                context,
                config,
                resolved,
                plan,
                objects,
                Provider::Agent,
                Some(&generation),
            )?;
            Ok::<_, AppError>((generation, decision))
        },
        |asked| matches!(asked, Ok((_, super::coordinator::GenerationDecision::Dump))),
    );
    let (generation, decision) = asked?;
    let recorded = ledger
        .as_ref()
        .map_or(Recorded::Nothing, GenerationLedger::read);
    let foreign_note = match &recorded {
        Recorded::Foreign { identity } => Some(format!(
            "the recorded configuration generation belongs to {identity}, not to this base and directory; it was not used"
        )),
        Recorded::Nothing | Recorded::Ours(_) => None,
    };
    if let super::coordinator::GenerationDecision::UpToDate(unchanged) = decision {
        return Ok((String::new(), DumpNotes::message(Some(unchanged)), true));
    }

    // Агент прогноза не спрашивает: начало выгрузки по изменившемуся отмечается в каталоге,
    // куда она ляжет, и режим узнаётся по факту. Дерево перечислено до отметки: правка
    // человека между ними стоит раньше начала и остаётся изменением.
    let mark = mark_start(context, resolved, plan, objects);
    let watch = before
        .zip(marked_start(mark.as_ref()))
        .map(|(before, start)| before.watch(start));
    let run = run_id();
    let stage_name = match plan.mode() {
        DumpMode::Full => "dump: full",
        DumpMode::Incremental => "dump: incremental",
        DumpMode::Partial => "dump: partial",
    };
    // Индекс git для хешера записанного читается, пока идёт выгрузка в каталог (#574).
    let mut hashing = None;
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
            log_live_stage(
                ProgressPhase::Dump,
                Some(resolved.source_set_name.as_str()),
                stage_name,
                "[агент] exporting configuration files",
            );
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
                external.generation_after(generation_id(handle, extension, wait).ok());
            }
            let cleanup = publish_full(context, config, resolved, &produced);
            if context.external_full_pull().is_none() {
                let _ = std::fs::remove_dir(produced.parent().unwrap_or(&produced));
            }
            (transcript, cleanup?)
        }
        DumpPlan::OverDirectory(how) => {
            ensure_dir(&resolved.platform_target_path).map_err(|error| {
                AppError::Runtime(format!("failed to create target dir: {error}"))
            })?;
            let target_relative = format!("target/{run}");
            let info_relative = format!("dump-info/{run}.xml");
            let (dumped, landed) = super::landing::dump_over_directory(
                context,
                config,
                resolved,
                how,
                version_file,
                |into| match &exchange {
                    // Каталог точки входа виден раннеру: стадия выставляется ссылкой, опись
                    // раннера — файлом рядом (`--config-dump-info-for-changes`, замер #646), и
                    // агент пишет в ещё не созданный подкаталог стадии.
                    Exchange::Dir(user_dir) => {
                        let started = std::time::Instant::now();
                        let stage = into.output.parent().unwrap_or(into.output);
                        let exposed = expose_output_dir(user_dir, &target_relative, stage)?;
                        let output = format!(
                            "{exposed}/{}",
                            into.output
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        );
                        let mut command =
                            format!("config dump-config-to-files --dir={}", argument(&output));
                        if let Some(description) = into.description {
                            let info = expose_file(user_dir, &info_relative, description)?;
                            command.push_str(&format!(
                                " --update --config-dump-info-for-changes={}",
                                argument(&info)
                            ));
                        }
                        debug!(
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "the local agent dump paths were prepared"
                        );
                        let command = with_extension(command, extension);
                        log_live_stage(
                            ProgressPhase::Dump,
                            Some(resolved.source_set_name.as_str()),
                            stage_name,
                            "[агент] exporting configuration files",
                        );
                        let (outcome, prepared) = beside(
                            "dump-hashing",
                            || watch.as_ref().and(set).map(DumpHashing::of),
                            || run_command(handle, &command, wait),
                        );
                        let started = std::time::Instant::now();
                        withdraw_dir(user_dir, &target_relative);
                        tidy_run_path(user_dir, &info_relative);
                        debug!(
                            elapsed_ms = started.elapsed().as_millis() as u64,
                            "the local agent dump paths were withdrawn"
                        );
                        outcome.map(|transcript| (transcript, prepared))
                    }
                    // По сети стадия целиком не возится: точке входа хватает описи, чтобы
                    // выгрузить только изменённое; обратно приходят изменённые файлы и новая
                    // опись — в стадию раннера, а не в каталог.
                    Exchange::Sftp => {
                        make_output_dir(handle, &exchange, &target_relative)?;
                        let update = match into.description {
                            Some(description) => {
                                stage_file(
                                    handle,
                                    &exchange,
                                    &format!("{target_relative}/{VERSION_FILE_NAME}"),
                                    description,
                                )?;
                                " --update"
                            }
                            None => "",
                        };
                        let command = with_extension(
                            format!(
                                "config dump-config-to-files --dir={}{update}",
                                argument(&target_relative)
                            ),
                            extension,
                        );
                        log_live_stage(
                            ProgressPhase::Dump,
                            Some(resolved.source_set_name.as_str()),
                            stage_name,
                            "[агент] exporting changed configuration files",
                        );
                        let outcome = run_command(handle, &command, wait);
                        let collected = outcome.as_ref().ok().map(|_| {
                            ensure_dir(into.output)
                                .map_err(|error| {
                                    AppError::Runtime(format!(
                                        "failed to prepare the stage: {error}"
                                    ))
                                })
                                .and_then(|()| {
                                    collect_into_dir(
                                        handle,
                                        &exchange,
                                        &target_relative,
                                        into.output,
                                    )
                                })
                        });
                        tidy(handle, &exchange, &target_relative);
                        let transcript = outcome?;
                        collected.transpose()?;
                        Ok((transcript, None))
                    }
                },
            )?;
            let (transcript, prepared) = dumped;
            hashing = prepared;
            (transcript, DumpNotes::message(landed))
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
                    log_live_stage(
                        ProgressPhase::Dump,
                        Some(resolved.source_set_name.as_str()),
                        stage_name,
                        "[агент] exporting selected configuration objects",
                    );
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
                    log_live_stage(
                        ProgressPhase::Dump,
                        Some(resolved.source_set_name.as_str()),
                        stage_name,
                        "[агент] exporting selected configuration objects",
                    );
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
    let mut evidence = None;
    let changed_note = match (set, plan) {
        (Some(set), DumpPlan::Full | DumpPlan::OverDirectory(_))
            if context.external_full_pull().is_none()
                && crate::use_cases::interruption::pending_interruption_error(
                    context,
                    "the configuration generation",
                )
                .is_none() =>
        {
            // Перечень каталога идёт, пока агент отвечает про поколение (#571), а сторож
            // закрывает сессию команды (#574): ни поколению, ни памяти сессия больше не нужна.
            // Ответ, который ничего не решает, не спрашивается (#637).
            let asks_after = crate::use_cases::exchange_guard::generation_after_dump_decides(
                set,
                watch.is_some(),
            );
            let (after, observed) = observe_dump_writes_while(
                Some(set),
                &config.work_path,
                mark.as_ref(),
                watch.as_ref(),
                hashing,
                &|| crate::use_cases::interruption::interruption_pending(context),
                || {
                    let mut handle = session
                        .take()
                        .expect("the session is open until the dump is observed");
                    let after = if asks_after {
                        generation_id(&mut handle, extension, wait).ok()
                    } else {
                        None
                    };
                    handle.finish(context, wait);
                    debug!("the agent session of the dump was closed");
                    after
                },
            );
            evidence = observed.fact.map(ModeEvidence::Fact);
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
    notes.evidence = evidence;
    Ok((transcript, notes, false))
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
    let reply = handle.run(command, wait).map_err(AppError::from)?;
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
    let publication = StagedPublication::prepare_dir_in(
        &resolved.stage_root,
        &resolved.platform_target_path,
        &resolved.platform_target_identity,
        super::DUMP_STAGE_PREFIX,
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
