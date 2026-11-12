use super::helpers::read_dump_generation;
use super::*;
use crate::domain::capability::{Operation, OperationSize, Provider};
use crate::domain::progress::ProgressPhase;
use crate::platform::locator::{UtilityLocation, UtilityVersion};
use crate::use_cases::agent_session::EarlyAgent;
use crate::use_cases::destruction_guard::{
    discard_note, guard_replacement, losses_in, preview_note, Destruction, DestructionConsent,
    Losses, WaysOut,
};
use crate::use_cases::version_file::RunnerVersionFile;
use std::fmt::Write as _;

pub(super) fn run_dump_with_context(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &DumpArgs,
) -> UseCaseResult<DumpResult> {
    let started = Instant::now();
    let mode = match args.mode {
        DumpModeRequest::Full => DumpMode::Full,
        DumpModeRequest::Incremental => DumpMode::Incremental,
        DumpModeRequest::Partial => DumpMode::Partial,
    };
    debug!(
        mode = ?mode,
        source_set = args.source_set.as_deref().unwrap_or("<auto>"),
        extension = args.extension.as_deref().unwrap_or("<none>"),
        "starting dump"
    );
    // Живой агент поднимается сразу, пока выгрузка готовится (#574): все отказы и сторож
    // идут до первой рабочей команды платформе, а не до её запуска.
    let mut early = super::agent::start_early(context, config, args, &mode);
    let outcome = prepare_and_dump(context, config, args, mode, started, &mut early);
    // Агент, которого выгрузка не взяла, снимается здесь, до возврата из сценария.
    drop(early);
    outcome
}

fn prepare_and_dump(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &DumpArgs,
    mode: DumpMode,
    started: Instant,
    early: &mut Option<EarlyAgent>,
) -> UseCaseResult<DumpResult> {
    // Отказ несёт форму ответа: без неё конверт `--json-message` не печатается.
    if let Err(error) = super::refuse_hidden_sets(config, args) {
        let message = error.to_string();
        return Err(DumpExecutionFailure::with_payload(
            error,
            empty_result(
                mode,
                started,
                args.source_set.clone(),
                args.extension.clone(),
                None,
                None,
                Some(message),
            ),
        ));
    }

    if let Some(error) = validate_supported_matrix(config) {
        return Err(DumpExecutionFailure::with_payload(
            error,
            empty_result(
                mode,
                started,
                None,
                None,
                None,
                None,
                Some(SUPPORTED_DUMP_ERROR.to_owned()),
            ),
        ));
    }

    let partial_objects = match validate_dump_objects(&mode, &args.objects) {
        Ok(objects) => objects,
        Err(error) => {
            let message = error.to_string();
            return Err(DumpExecutionFailure::with_payload(
                error,
                empty_result(
                    mode,
                    started,
                    args.source_set.clone(),
                    args.extension.clone(),
                    None,
                    None,
                    Some(message),
                ),
            ));
        }
    };
    let selectors = partial_objects.as_ref().map(|objects| {
        objects
            .iter()
            .map(|selector| DumpSelectorResult {
                requested: selector.requested().to_owned(),
                normalized: selector.normalized(),
            })
            .collect()
    });

    let resolved = match resolve_target(config, args).and_then(|resolved| {
        // Внешняя стадия Unica лежит у неё, а не в `workPath`: её том сверяет она.
        if context.external_full_pull().is_none() {
            super::helpers::refuse_a_work_path_on_another_volume(&resolved)?;
        }
        Ok(resolved)
    }) {
        Ok(resolved) => resolved,
        Err(error) => {
            let message = error.to_string();
            return Err(DumpExecutionFailure::with_payload(
                error,
                empty_result(
                    mode,
                    started,
                    args.source_set.clone(),
                    args.extension.clone(),
                    selectors.clone(),
                    None,
                    Some(message),
                ),
            ));
        }
    };

    let mut utilities = PlatformUtilities::from_config(config);
    let size = dump_size(config, &resolved, &mode, &mut utilities);
    let selected = match crate::use_cases::provider_selection::select_sized(
        config,
        &mut utilities,
        Operation::Dump,
        size,
    ) {
        Ok(selected) => selected,
        Err((error, receipt)) => {
            let message = error.to_string();
            let mut result = empty_result(
                mode,
                started,
                args.source_set.clone(),
                args.extension.clone(),
                selectors.clone(),
                None,
                Some(message),
            );
            result.provider = Some(receipt);
            return Err(DumpExecutionFailure::with_payload(error, result));
        }
    };
    if let Some(external) = context.external_full_pull() {
        use crate::use_cases::full_pull_publication::{ExecutorKind, FullPullExecutor};
        let remote = config
            .infobase
            .standalone
            .as_ref()
            .and_then(|base| base.gate.as_deref());
        let attached = config.tools.designer_agent.attach.as_deref();
        let (kind, endpoint) = if selected.provider != Provider::Agent {
            (ExecutorKind::Local, None)
        } else if remote.is_some() {
            (ExecutorKind::Remote, remote)
        } else if attached.is_some() {
            (ExecutorKind::Attached, attached)
        } else {
            (ExecutorKind::Local, None)
        };
        if let Err(error) = external.authorize(&FullPullExecutor {
            provider: &selected.receipt,
            kind,
            executable: selected
                .location
                .as_ref()
                .map(|location| location.path.as_path()),
            endpoint,
        }) {
            let mut result = empty_result(
                mode,
                started,
                args.source_set.clone(),
                args.extension.clone(),
                selectors.clone(),
                Some(resolved.target_path.clone()),
                Some(error.to_string()),
            );
            result.provider = Some(selected.receipt);
            return Err(DumpExecutionFailure::with_payload(error, result));
        }
    }
    let receipt = selected.receipt.clone();
    let outcome = run_dump_selected(
        context,
        config,
        args,
        mode,
        started,
        selectors,
        partial_objects,
        resolved,
        utilities,
        selected,
        early,
    );
    crate::use_cases::provider_selection::attach(outcome, &receipt)
}

#[allow(clippy::too_many_arguments)]
fn run_dump_selected(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &DumpArgs,
    mode: DumpMode,
    started: Instant,
    selectors: Option<Vec<DumpSelectorResult>>,
    partial_objects: Option<Vec<PartialDumpSelector>>,
    resolved: ResolvedDumpTarget,
    mut utilities: PlatformUtilities,
    selected: crate::use_cases::provider_selection::SelectedProvider,
    early: &mut Option<EarlyAgent>,
) -> Result<DumpResult, DumpExecutionFailure> {
    let provider = selected.provider;
    let location = selected.location;
    // Выбран не агент: поднятый заранее агент отпускается до работы другого исполнителя —
    // его сессия не держит базу, пока работает `ibcmd` или Конфигуратор.
    if provider != Provider::Agent || config.format != SourceFormat::Designer {
        if let Some(early) = early.take() {
            early.release_before_another_executor();
        }
    }
    // Исполнителю без утилиты (чужой агент) путь не нужен; остальным его даёт выбор,
    // и пустой путь ниже недостижим — арка-страж перед матчем отказывает раньше.
    let binary = location
        .as_ref()
        .map(|found| found.path.clone())
        .unwrap_or_default();
    let edt_binary = if config.format == SourceFormat::Edt {
        Some(match utilities.locate(UtilityType::EdtCli) {
            Ok(location) => location.path,
            Err(error) => {
                let message = error.to_string();
                let app_error = AppError::from(error);
                return Err(DumpExecutionFailure::with_payload(
                    app_error,
                    empty_result(
                        mode,
                        started,
                        Some(resolved.source_set_name.clone()),
                        resolved.extension.clone(),
                        selectors.clone(),
                        Some(resolved.target_path.clone()),
                        Some(message),
                    ),
                ));
            }
        })
    } else {
        None
    };

    if args.dry_run {
        // Both utilities are located above, so a missing platform refuses in the preview; the
        // dump lock below is this command's first filesystem write. Следа превью не
        // оставляет вовсе — ни рабочего каталога, ни журнала действий; запись о вызове
        // несёт конверт на stdout (`DEC.2026-09-23.A-PREVIEW-LEAVES-NO-TRACE`).
        crate::use_cases::progress::log_live_stage(
            ProgressPhase::Preview,
            None,
            "dump: preview",
            "[Dump] preview only, nothing written",
        );
        // Превью называет режим, который выполнит выгрузка: тот же план по тому файлу
        // версий, который оставит в каталоге сверка с копией раннера.
        let plan = match preview_plan(config, &resolved, &mode, location.as_ref()) {
            Ok(plan) => plan,
            Err(error) => {
                let message = error.to_string();
                return Err(DumpExecutionFailure::with_payload(
                    error,
                    empty_result(
                        mode,
                        started,
                        Some(resolved.source_set_name.clone()),
                        resolved.extension.clone(),
                        selectors.clone(),
                        Some(resolved.target_path.clone()),
                        Some(message),
                    ),
                ));
            }
        };
        let planned = plan.mode();
        let mut message = format!(
            "would dump {planned:?} into '{}' via {}; nothing written",
            resolved.target_path.display(),
            match location.as_ref() {
                Some(found) => found.path.display().to_string(),
                None => "the attached Designer agent".to_owned(),
            }
        );
        if let Some(reason) = plan.whole_reason() {
            let _ = write!(
                message,
                "; {}: the dump would run full instead of incremental{}",
                reason.describe(&resolved.platform_target_path),
                whole_consequences(context, config, &resolved)
            );
        }
        // Превью спрашивает сторожа о том же каталоге и так же, как выгрузка, но ничего не
        // трогает: называет, на чём она остановится без согласия или что уничтожит с ним.
        let (how, regenerated) = destruction_of(config, &plan);
        let losses = match (how, &resolved.consent) {
            (_, DestructionConsent::AskFirst(_))
            | (Destruction::Replace, DestructionConsent::Granted) => {
                losses_in(&resolved.target_path, regenerated)
            }
            // Перезапись с согласием уничтоженным ничего не называет — как и сама
            // выгрузка ниже; каталог раннера спрашивать не о чем.
            (Destruction::Overwrite, DestructionConsent::Granted)
            | (_, DestructionConsent::RunnerOwned) => Losses::default(),
        };
        if let Some(note) = preview_note(
            context,
            &resolved.target_path,
            &resolved.consent,
            &losses,
            how,
        ) {
            let _ = write!(message, "; {note}");
        }
        let (reported, mode_reason) = plan.reported(None);
        let mut preview = empty_result(
            mode,
            started,
            Some(resolved.source_set_name.clone()),
            resolved.extension.clone(),
            selectors.clone(),
            Some(resolved.target_path.clone()),
            Some(message),
        );
        preview.ok = true;
        preview.mode = reported;
        preview.mode_reason = mode_reason;
        preview.losses = losses.report();
        return Ok(preview);
    }

    // The opt-in caller supplies public ancestors before Continue. Taking the
    // ordinary target lock must not create them for it.
    let target_lock = if context.external_full_pull().is_some()
        && !resolved.lock_path.parent().is_some_and(Path::is_dir)
    {
        Err(AppError::Validation(
            "external full-pull target parent must be an existing directory when the caller continues executor authorization".into(),
        ))
    } else {
        acquire_advisory_lock(&resolved.lock_path).map_err(|error| {
            AppError::Runtime(format!(
                "failed to acquire dump lock '{}': {error}",
                resolved.lock_path.display()
            ))
        })
    };
    let lock_guard = match target_lock {
        Ok(lock_guard) => lock_guard,
        Err(app_error) => {
            let message = app_error.to_string();
            return Err(DumpExecutionFailure::with_payload(
                app_error,
                empty_result(
                    mode,
                    started,
                    Some(resolved.source_set_name.clone()),
                    resolved.extension.clone(),
                    selectors.clone(),
                    Some(resolved.target_path.clone()),
                    Some(message),
                ),
            ));
        }
    };

    if let Err(error) = if context.external_full_pull().is_none() {
        cleanup_orphan_dirs(&resolved)
    } else {
        Ok(())
    } {
        let message = format!("failed to cleanup stale dump temp dirs: {error}");
        let app_error = AppError::Runtime(message.clone());
        return Err(DumpExecutionFailure::with_payload(
            app_error,
            empty_result(
                mode,
                started,
                Some(resolved.source_set_name.clone()),
                resolved.extension.clone(),
                selectors.clone(),
                Some(resolved.target_path.clone()),
                Some(message),
            ),
        ));
    }
    if context.external_full_pull().is_none()
        && resolved.platform_target_path != resolved.target_path
    {
        if let Err(error) = cleanup_platform_orphan_dirs(&resolved) {
            let message = format!("failed to cleanup stale dump platform temp dirs: {error}");
            let app_error = AppError::Runtime(message.clone());
            return Err(DumpExecutionFailure::with_payload(
                app_error,
                empty_result(
                    mode,
                    started,
                    Some(resolved.source_set_name.clone()),
                    resolved.extension.clone(),
                    selectors.clone(),
                    Some(resolved.target_path.clone()),
                    Some(message),
                ),
            ));
        }
    }

    if let Err(error) = validate_publish_target(&resolved) {
        let message = error.to_string();
        return Err(DumpExecutionFailure::with_payload(
            error,
            empty_result(
                mode,
                started,
                Some(resolved.source_set_name.clone()),
                resolved.extension.clone(),
                selectors.clone(),
                Some(resolved.target_path.clone()),
                Some(message),
            ),
        ));
    }
    if resolved.platform_target_path != resolved.target_path {
        if let Err(error) = validate_platform_target(&resolved) {
            let message = error.to_string();
            return Err(DumpExecutionFailure::with_payload(
                error,
                empty_result(
                    mode,
                    started,
                    Some(resolved.source_set_name.clone()),
                    resolved.extension.clone(),
                    selectors.clone(),
                    Some(resolved.target_path.clone()),
                    Some(message),
                ),
            ));
        }
    }

    // Копия описи раннера (#646): по ней идёт выгрузка по изменившемуся и выборка `ibcmd`,
    // её пишет удачная выгрузка. Выборка Конфигуратора и агента её не трогает.
    let version_file = match (mode, provider) {
        (DumpMode::Partial, Provider::Ibcmd) | (DumpMode::Incremental | DumpMode::Full, _) => {
            runner_version_file(config, &resolved)
        }
        (DumpMode::Partial, _) => None,
    };
    let copy = match (&mode, &version_file) {
        (DumpMode::Incremental, Some(version_file)) => version_file.usable(),
        _ => Ok(None),
    };

    // Выгрузка по изменившемуся держится на копии описи раннера. Нет годной копии или формат в
    // ней чужой — она становится полной до запуска платформы.
    let plan = copy
        .and_then(|copy| plan_dump(&mode, copy, platform_of(location.as_ref())))
        .and_then(|plan| {
            // Каталог человека сторож спрашивает до платформы при любом плане: выгрузка поверх
            // каталога переписывает его файлы на месте, замена — стирает лишнее. Без согласия
            // безвозвратное останавливает работу здесь, пока ничего не тронуто.
            let (how, regenerated) = destruction_of(config, &plan);
            match (how, &resolved.consent) {
                (_, DestructionConsent::AskFirst(_)) => {
                    guard_replacement(
                        context,
                        &resolved.target_path,
                        &resolved.consent,
                        regenerated,
                        how,
                    )?;
                }
                // Замену с согласием сторож спрашивает при публикации: там он и называет
                // уничтоженное.
                (Destruction::Replace, DestructionConsent::Granted) => {}
                // Перезапись с согласием уничтоженным ничего не называет: лишнего она не
                // удаляет, неотслеживаемое, которого платформа не касается, остаётся на месте,
                // а какие файлы она перепишет, заранее неизвестно. Из командной строки сюда не
                // попасть — `--force` делает выгрузку полной, то есть заменой.
                (Destruction::Overwrite, DestructionConsent::Granted) => {}
                // Каталог раннера спрашивать не о чем.
                (_, DestructionConsent::RunnerOwned) => {}
            }
            Ok(plan)
        });
    let plan = match plan {
        Ok(plan) => plan,
        Err(error) => {
            let message = error.to_string();
            return Err(DumpExecutionFailure::with_payload(
                error,
                empty_result(
                    mode,
                    started,
                    Some(resolved.source_set_name.clone()),
                    resolved.extension.clone(),
                    selectors.clone(),
                    Some(resolved.target_path.clone()),
                    Some(message),
                ),
            ));
        }
    };
    let whole_note = plan.whole_reason().map(|reason| {
        format!(
            "{}: the dump ran full instead of incremental{}",
            reason.describe(&resolved.platform_target_path),
            whole_consequences(context, config, &resolved)
        )
    });
    let requested = mode;
    // Выборка Конфигуратора и агента пишет прямо в каталог набора, и пишет ли она опись версий,
    // не замерено. Опись, которой до выборки в каталоге не было, раннер после неё убирает:
    // в каталог исходников он описи не кладёт (#646). Оставшийся от прежних версий файл не
    // трогается.
    let version_file_before_partial = matches!(plan, DumpPlan::Partial)
        && provider != Provider::Ibcmd
        && config.format == SourceFormat::Designer
        && !resolved
            .platform_target_path
            .join(VERSION_FILE_NAME)
            .exists();

    let partial_objects = partial_objects.as_deref();
    let edt_binary = edt_binary.as_deref();
    // Агент отвечает ещё и «выгружать нечего» — это состояние ответа, а не проза, и
    // у остальных исполнителей его нет.
    let (result, up_to_date) = if provider == Provider::Agent
        && config.format == SourceFormat::Designer
    {
        match super::agent::run_dump_agent(
            context,
            config,
            &resolved,
            &plan,
            partial_objects,
            location.as_ref(),
            &mut utilities,
            version_file
                .as_ref()
                .filter(|_| context.external_full_pull().is_none()),
            early.take(),
        ) {
            Ok((platform_result, message, up_to_date)) => {
                (Ok((platform_result.platform_log_path, message)), up_to_date)
            }
            Err(error) => (Err(error), false),
        }
    } else {
        // Поколение спрашивается до выгрузки и после: одно и то же — память записывается,
        // разное — базу правили во время выгрузки, и ответ это называет. Выборка объектов
        // каталог с базой не сводит, поэтому поколения не пишет.
        let tool_runner = match provider {
            Provider::Ibcmd => utilities.runner_for(UtilityType::Ibcmd),
            // Поколение здесь читают только Конфигуратор и `ibcmd` (`read_dump_generation`):
            // остальным бегун не понадобится.
            Provider::Designer | Provider::Agent | Provider::IbcmdRs | Provider::Webinst => {
                utilities.runner_for(UtilityType::V8)
            }
        };
        let reads_generation = location.is_some() && !matches!(plan, DumpPlan::Partial);
        let generation_before = reads_generation
            .then(|| {
                read_dump_generation(context, config, provider, &binary, tool_runner, &resolved)
            })
            .flatten();
        if let Some(external) = context.external_full_pull() {
            external.generation_before(provider, generation_before.clone());
        }
        // Как у агента: поколение до выгрузки по изменившемуся совпало с записанным тем же
        // инструментом — выгружать нечего, и платформа больше не запускается.
        let unchanged = generation_before
            .as_deref()
            .filter(|_| {
                matches!(plan, DumpPlan::OverDirectory(OverDirectory::ByVersionFile))
                    && partial_objects.is_none()
            })
            .and_then(|token| {
                SourceSetInventory::new(config)
                    .designer_context(&resolved.source_set_name)
                    .and_then(|set| {
                        crate::use_cases::exchange_guard::unchanged_since_the_record(
                            config, set, provider, token,
                        )
                    })
            });
        if let Some(unchanged) = unchanged {
            (
                // Платформа не запускалась: журнала у пропуска нет.
                Ok((None, DumpNotes::message(Some(unchanged)))),
                true,
            )
        } else {
            // Начало выгрузки по изменившемуся отмечается в каталоге, куда она ляжет: по нему
            // память узнаёт записанное, а Конфигуратор, который прогноза не спрашивает, — свой
            // режим по факту; `ibcmd` режим называет прогнозом. Без поколения до выгрузки
            // записанное не запоминается: список дерева до неё не нужен.
            let inventory = SourceSetInventory::new(config);
            let mark = mark_start(context, &resolved, &plan, partial_objects);
            let fact_mark = mark.as_ref().filter(|_| provider == Provider::Designer);
            let watch = generation_before.as_ref().and_then(|_| {
                dump_watch(
                    config,
                    inventory.designer_context(&resolved.source_set_name),
                    marked_start(mark.as_ref()),
                )
            });
            let result = match (config.format, &plan, provider, partial_objects, edt_binary) {
                (_, _, other, _, _) if location.is_none() && other != Provider::Agent => Err(
                    crate::use_cases::unimplemented_provider(Operation::Dump, other),
                ),
                // Выгрузка по изменившемуся без годного файла версий — полная поверх каталога:
                // каталог человека она не заменяет, лишнего в нём не удаляет.
                (
                    SourceFormat::Designer,
                    DumpPlan::OverDirectory(how),
                    Provider::Designer,
                    _,
                    _,
                ) => run_dump_over_directory_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::V8),
                    how,
                    version_file.as_ref(),
                ),
                (SourceFormat::Designer, DumpPlan::OverDirectory(how), Provider::Ibcmd, _, _) => {
                    run_dump_over_directory_ibcmd(
                        context,
                        config,
                        &resolved,
                        binary.as_path(),
                        utilities.runner_for(UtilityType::Ibcmd),
                        how,
                        version_file.as_ref(),
                    )
                }
                (SourceFormat::Designer, DumpPlan::Full, Provider::Designer, _, _) => {
                    run_full_dump_designer(
                        context,
                        config,
                        &resolved,
                        binary.as_path(),
                        utilities.runner_for(UtilityType::V8),
                    )
                }
                (SourceFormat::Designer, DumpPlan::Full, Provider::Ibcmd, _, _) => {
                    run_full_dump_ibcmd(
                        context,
                        config,
                        &resolved,
                        binary.as_path(),
                        utilities.runner_for(UtilityType::Ibcmd),
                    )
                }
                (
                    SourceFormat::Designer,
                    DumpPlan::Partial,
                    Provider::Designer,
                    Some(objects),
                    _,
                ) => run_partial_dump_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::V8),
                    objects,
                ),
                (SourceFormat::Designer, DumpPlan::Partial, Provider::Ibcmd, Some(_), _) => {
                    run_partial_dump_ibcmd(
                        context,
                        config,
                        &resolved,
                        binary.as_path(),
                        utilities.runner_for(UtilityType::Ibcmd),
                        version_file.as_ref(),
                    )
                }
                (
                    SourceFormat::Edt,
                    DumpPlan::OverDirectory(OverDirectory::ByVersionFile),
                    Provider::Designer,
                    _,
                    Some(edt_binary),
                ) => run_incremental_dump_edt_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    edt_binary,
                    utilities.runner_for(UtilityType::V8),
                    utilities.runner_for(UtilityType::EdtCli),
                    version_file.as_ref(),
                ),
                (
                    SourceFormat::Edt,
                    DumpPlan::OverDirectory(OverDirectory::ByVersionFile),
                    Provider::Ibcmd,
                    _,
                    Some(edt_binary),
                ) => run_incremental_dump_edt_ibcmd(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    edt_binary,
                    utilities.runner_for(UtilityType::Ibcmd),
                    utilities.runner_for(UtilityType::EdtCli),
                    version_file.as_ref(),
                ),
                (
                    SourceFormat::Edt,
                    DumpPlan::Full | DumpPlan::OverDirectory(OverDirectory::Whole(_)),
                    Provider::Designer,
                    _,
                    Some(edt_binary),
                ) => run_full_dump_edt_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    edt_binary,
                    utilities.runner_for(UtilityType::V8),
                    utilities.runner_for(UtilityType::EdtCli),
                ),
                (
                    SourceFormat::Edt,
                    DumpPlan::Full | DumpPlan::OverDirectory(OverDirectory::Whole(_)),
                    Provider::Ibcmd,
                    _,
                    Some(edt_binary),
                ) => run_full_dump_edt_ibcmd(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    edt_binary,
                    utilities.runner_for(UtilityType::Ibcmd),
                    utilities.runner_for(UtilityType::EdtCli),
                ),
                (
                    SourceFormat::Edt,
                    DumpPlan::Partial,
                    Provider::Designer,
                    Some(objects),
                    Some(edt_binary),
                ) => run_partial_dump_edt_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    edt_binary,
                    utilities.runner_for(UtilityType::V8),
                    utilities.runner_for(UtilityType::EdtCli),
                    objects,
                ),
                (
                    SourceFormat::Edt,
                    DumpPlan::Partial,
                    Provider::Ibcmd,
                    Some(_),
                    Some(edt_binary),
                ) => run_partial_dump_edt_ibcmd(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    edt_binary,
                    utilities.runner_for(UtilityType::Ibcmd),
                    utilities.runner_for(UtilityType::EdtCli),
                    version_file.as_ref(),
                ),
                (_, DumpPlan::Partial, _, None, _) => Err(AppError::Runtime(
                    "partial dump objects were not validated before execution".to_owned(),
                )),
                (SourceFormat::Edt, _, _, _, None) => Err(AppError::Runtime(
                    "EDT binary must be resolved before executing format=EDT dump".to_owned(),
                )),
                // Исполнитель без адаптера выгрузки: до сюда его не пускает поиск утилиты выше,
                // но матрица может опередить код, и тогда это отказ, а не паника.
                (_, _, other, _, _) => Err(crate::use_cases::unimplemented_provider(
                    Operation::Dump,
                    other,
                )),
            };
            let result = result.map(|(platform_result, mut notes)| {
                let set = context
                    .external_full_pull()
                    .is_none()
                    .then(|| inventory.designer_context(&resolved.source_set_name))
                    .flatten();
                // Перечень каталога не ждёт платформу: он идёт, пока она отвечает про
                // поколение. `watch` есть, только когда поколение до выгрузки прочитано.
                // Ответ, который ничего не решает, не спрашивается (#637).
                let asks_after = set.is_some_and(|set| {
                    crate::use_cases::exchange_guard::generation_after_dump_decides(
                        set,
                        watch.is_some(),
                    )
                });
                let (generation_after, observed) = observe_dump_writes_while(
                    set,
                    &config.work_path,
                    fact_mark,
                    watch.as_ref(),
                    None,
                    &|| crate::use_cases::interruption::interruption_pending(context),
                    || {
                        // У внешнего полного `pull` набора здесь нет, и `asks_after` ложно.
                        if !asks_after {
                            None
                        } else {
                            generation_before.as_ref().and_then(|_| {
                                read_dump_generation(
                                    context,
                                    config,
                                    provider,
                                    &binary,
                                    tool_runner,
                                    &resolved,
                                )
                            })
                        }
                    },
                );
                let note = set.and_then(|set| {
                    merge_optional_messages(
                        crate::use_cases::exchange_guard::record_after_dump(
                            config,
                            set,
                            provider,
                            generation_before.as_deref(),
                            generation_after.as_deref(),
                        ),
                        remember_dump_writes(
                            set,
                            &config.work_path,
                            observed.writes,
                            generation_before.as_deref(),
                            generation_after.as_deref(),
                        ),
                    )
                });
                notes.message = merge_optional_messages(notes.message, note);
                notes.evidence = notes.evidence.or(observed.fact.map(ModeEvidence::Fact));
                (platform_result.platform_log_path, notes)
            });
            (result, false)
        }
    };
    // Копию пишет сама выгрузка под тем же замком и только после удачи: сбой оставляет прежнюю.
    let result = result.map(|(platform_log_path, notes)| {
        // Уничтоженное называет вопрос к сторожу при публикации замены.
        let discarded = notes.discarded;
        let evidence = notes.evidence;
        let message = merge_optional_messages(
            whole_note,
            merge_optional_messages(
                discard_note(&resolved.target_path, &discarded),
                notes.message,
            ),
        );
        let losses = discarded.report();
        (platform_log_path, message, losses, evidence)
    });
    if version_file_before_partial {
        let written = resolved.platform_target_path.join(VERSION_FILE_NAME);
        if let Err(error) = std::fs::remove_file(&written) {
            if error.kind() != std::io::ErrorKind::NotFound {
                debug!(path = %written.display(), %error, "the version file of a partial dump was not removed");
            }
        }
    }
    drop(lock_guard);

    match result {
        Ok((platform_log_path, cleanup_message, losses, evidence)) => {
            let (reported, mode_reason) = plan.reported(evidence);
            Ok(DumpResult {
                provider: None,
                provider_dispatched: false,
                up_to_date,
                ok: true,
                source_set: Some(resolved.source_set_name),
                extension: resolved.extension,
                selectors,
                requested_mode: requested,
                mode: reported,
                mode_reason,
                target_path: resolved.target_path,
                platform_log_path,
                duration_ms: started.elapsed().as_millis() as u64,
                message: cleanup_message
                    .or_else(|| Some(crate::domain::dump::DUMP_SUCCESS_MESSAGE.to_owned())),
                losses,
            })
        }
        Err(error) => {
            let message = error.to_string();
            // Отказ называет режим плана и его причину: ни прогноз, ни факт до ответа не дошли.
            let (reported, mode_reason) = plan.reported(None);
            Err(DumpExecutionFailure::with_payload(
                error,
                DumpResult {
                    provider: None,
                    provider_dispatched: false,
                    up_to_date: false,
                    ok: false,
                    source_set: Some(resolved.source_set_name),
                    extension: resolved.extension,
                    selectors,
                    requested_mode: requested,
                    mode: reported,
                    mode_reason,
                    target_path: resolved.target_path,
                    platform_log_path: None,
                    duration_ms: started.elapsed().as_millis() as u64,
                    message: Some(message),
                    losses: None,
                },
            ))
        }
    }
}

/// Как выгрузка по плану обходится с каталогом человека и какие файлы в его корне не теряет.
/// Проект EDT при любом плане заменяется целиком; в формате Конфигуратора полная выгрузка
/// заменяет каталог, остальные ложатся поверх. `ConfigDumpInfo.xml` в корне, оставшийся от
/// прежних версий раннера, не потеря (#646): выгрузка поверх каталога его не трогает, а замена
/// переносит в новое дерево.
fn destruction_of(config: &AppConfig, plan: &DumpPlan) -> (Destruction, &'static [&'static str]) {
    match (config.format, plan) {
        (SourceFormat::Edt, _) => (Destruction::Replace, &[]),
        (SourceFormat::Designer, DumpPlan::Full) => (Destruction::Replace, &[VERSION_FILE_NAME]),
        (SourceFormat::Designer, DumpPlan::Partial | DumpPlan::OverDirectory(_)) => {
            (Destruction::Overwrite, &[VERSION_FILE_NAME])
        }
    }
}

/// Файл версий набора и копия раннера: есть у набора с памятью базы.
fn runner_version_file(
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
) -> Option<RunnerVersionFile> {
    SourceSetInventory::new(config)
        .designer_context(&resolved.source_set_name)
        .and_then(RunnerVersionFile::of)
}

/// Чего не делает полная выгрузка поверх каталога Конфигуратора: лишнего не удаляет и
/// хеш-память не пишет — каталог с файлами, которых нет в базе, базу не описывает. Совет —
/// полная выгрузка со ступенчатой публикацией, если вызывающего можно к ней отправить.
/// Снимок EDT заменяется целиком, и оговорки у него нет.
fn whole_consequences(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
) -> String {
    match config.format {
        SourceFormat::Edt => String::new(),
        SourceFormat::Designer => {
            let advice = match &resolved.consent {
                // Набора ещё нет в проекте: `pull <SET> --force` в него не попадёт.
                DestructionConsent::AskFirst(WaysOut::SaveWork | WaysOut::Undeclared { .. }) => {
                    String::new()
                }
                DestructionConsent::AskFirst(
                    WaysOut::PullForce { .. } | WaysOut::SameCallWithForce,
                )
                | DestructionConsent::Granted
                | DestructionConsent::RunnerOwned => format!(
                    "; run {} to replace the directory with the base and record its hashes",
                    context.advised_pull_force(&resolved.source_set_name)
                ),
            };
            format!(
                "; files the base does not have stay in the directory and hash memory is not updated{advice}"
            )
        }
    }
}

/// Версия платформы выбранной утилиты, если раннер её знает.
pub(super) fn platform_of(location: Option<&UtilityLocation>) -> Option<&PlatformVersion> {
    location.and_then(|found| match &found.version {
        Some(UtilityVersion::Platform(version)) => Some(version),
        Some(UtilityVersion::Edt(_)) | None => None,
    })
}

/// Объём выгрузки до выбора исполнителя (#559): целиком — `--force` и выгрузка по
/// изменившемуся, которую план ([`preview_plan`], ничего не пишет) делает полной поверх
/// каталога: файла версий нет или он чужой. Выборка объектов и выгрузка по файлу версий — по
/// изменившемуся. Чужой формат файла сверяется с версией платформы найденного `ibcmd` —
/// того, кого целиком выбрали бы первым; без неё, как и прежде, о чужом формате не судят.
/// Где порядок цепочки от объёма не зависит, план не строится вовсе.
fn dump_size(
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    mode: &DumpMode,
    utilities: &mut PlatformUtilities,
) -> OperationSize {
    match mode {
        DumpMode::Full => return OperationSize::Whole,
        DumpMode::Partial => return OperationSize::ByChanges,
        DumpMode::Incremental => {}
    }
    if !config.orders_by_size(Operation::Dump) {
        return OperationSize::ByChanges;
    }
    let ibcmd = utilities.locate(UtilityType::Ibcmd).ok();
    match preview_plan(config, resolved, mode, ibcmd.as_ref()) {
        Ok(DumpPlan::OverDirectory(OverDirectory::Whole(_)) | DumpPlan::Full) => {
            OperationSize::Whole
        }
        Ok(DumpPlan::OverDirectory(OverDirectory::ByVersionFile) | DumpPlan::Partial) | Err(_) => {
            OperationSize::ByChanges
        }
    }
}

/// План превью: тот же [`plan_dump`], что у выгрузки, по копии описи раннера, которую
/// выгрузка сочтёт годной. Ничего не пишет.
fn preview_plan(
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    mode: &DumpMode,
    location: Option<&UtilityLocation>,
) -> Result<DumpPlan, AppError> {
    let version_file = runner_version_file(config, resolved);
    let copy = match (mode, &version_file) {
        (DumpMode::Incremental, Some(version_file)) => version_file.would_be_usable()?,
        (DumpMode::Incremental | DumpMode::Full | DumpMode::Partial, _) => None,
    };
    plan_dump(mode, copy, platform_of(location))
}
