use super::*;
use crate::domain::capability::{Operation, Provider};
use crate::platform::locator::{UtilityLocation, UtilityVersion};
use crate::use_cases::destruction_guard::{
    discard_note, guard_replacement, losses_in, preview_note, Destruction, DestructionConsent,
    Losses, WaysOut,
};
use crate::use_cases::version_file::{remove_left_candidates, RunnerVersionFile};
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
        // Платформа пишет опись версий в каталог выгрузки. В каталоге человека опись
        // из индекса гита останавливает выгрузку до платформы, превью — тоже:
        // проверка ничего не пишет. Служебный снимок EDT в `workPath` — забота
        // раннера, а не гита.
        if resolved.platform_target_path == resolved.target_path {
            crate::use_cases::ignored_files::refuse_tracked_version_file(&resolved.target_path)?;
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
    let selected =
        match crate::use_cases::provider_selection::select(config, &mut utilities, Operation::Dump)
        {
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
) -> Result<DumpResult, DumpExecutionFailure> {
    let provider = selected.provider;
    let location = selected.location;
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
        let losses = match resolved.consent {
            DestructionConsent::RunnerOwned => Losses::default(),
            DestructionConsent::AskFirst(_) | DestructionConsent::Granted => {
                losses_in(&resolved.target_path, regenerated)
            }
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
        let mut preview = empty_result(
            planned,
            started,
            Some(resolved.source_set_name.clone()),
            resolved.extension.clone(),
            selectors.clone(),
            Some(resolved.target_path.clone()),
            Some(message),
        );
        preview.ok = true;
        preview.losses = losses.into_paths();
        return Ok(preview);
    }

    let lock_guard = match acquire_advisory_lock(&resolved.lock_path) {
        Ok(lock_guard) => lock_guard,
        Err(error) => {
            let message = format!(
                "failed to acquire dump lock '{}': {error}",
                resolved.lock_path.display()
            );
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
    };

    if let Err(error) = cleanup_orphan_dirs(&resolved) {
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
    if resolved.platform_target_path != resolved.target_path {
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

    // Выгрузка по изменившемуся работает от файла версий в каталоге: подменённый файл
    // уступает место копии раннера до запуска платформы. Выборку `ibcmd` выгружает как
    // `--sync` по тому же файлу; что пишет в него выборочная выгрузка Конфигуратора,
    // раннер не знает и копию ею не меняет.
    // Временные файлы прошлых замен файла версий убираются в начале любой выгрузки: их
    // видит `git status` и сторож замены каталога.
    if let Err(error) = remove_left_candidates(&resolved.platform_target_path) {
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
    let version_file_use = match mode {
        DumpMode::Incremental => VersionFileUse::RestoreAndRecord,
        DumpMode::Partial if provider == Provider::Ibcmd => VersionFileUse::RestoreAndRecord,
        DumpMode::Partial => VersionFileUse::Untouched,
        DumpMode::Full => VersionFileUse::RecordOnly,
    };
    let version_file = match version_file_use {
        VersionFileUse::Untouched => None,
        VersionFileUse::RestoreAndRecord | VersionFileUse::RecordOnly => {
            runner_version_file(config, &resolved)
        }
    };
    if let (VersionFileUse::RestoreAndRecord, Some(version_file)) =
        (version_file_use, version_file.as_ref())
    {
        if let Err(error) = version_file.restore() {
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

    // Выгрузка по изменившемуся держится на файле версий в каталоге. Нет его или формат в
    // нём чужой — она становится полной до запуска платформы: `-update` у платформы без
    // файла отказывает, а при чужом формате считает разницу не от того.
    let plan = plan_dump(
        &mode,
        Some(&resolved.platform_target_path.join(VERSION_FILE_NAME)),
        platform_of(location.as_ref()),
    )
    .and_then(|plan| {
        // Каталог человека сторож спрашивает до платформы при любом плане: выгрузка поверх
        // каталога переписывает его файлы на месте, замена — стирает лишнее. Без согласия
        // безвозвратное останавливает работу здесь, пока ничего не тронуто. Замену с
        // согласием сторож спрашивает при публикации: там он и называет уничтоженное.
        let (how, regenerated) = destruction_of(config, &plan);
        let discarded = match (how, &resolved.consent) {
            (Destruction::Replace, DestructionConsent::Granted) => Losses::default(),
            _ => guard_replacement(
                context,
                &resolved.target_path,
                &resolved.consent,
                regenerated,
                how,
            )?,
        };
        Ok((plan, discarded))
    });
    let (plan, overwritten) = match plan {
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
    let mode = plan.mode();

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
        ) {
            Ok((platform_result, message, up_to_date)) => {
                (Ok((platform_result, message)), up_to_date)
            }
            Err(error) => (Err(error), false),
        }
    } else {
        let result = match (config.format, &plan, provider, partial_objects, edt_binary) {
            (_, _, other, _, _) if location.is_none() && other != Provider::Agent => Err(
                crate::use_cases::unimplemented_provider(Operation::Dump, other),
            ),
            // Выгрузка по изменившемуся без годного файла версий — полная поверх каталога:
            // каталог человека она не заменяет, лишнего в нём не удаляет.
            (SourceFormat::Designer, DumpPlan::OverDirectory(how), Provider::Designer, _, _) => {
                run_dump_over_directory_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::V8),
                    how,
                )
            }
            (SourceFormat::Designer, DumpPlan::OverDirectory(how), Provider::Ibcmd, _, _) => {
                run_dump_over_directory_ibcmd(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::Ibcmd),
                    how,
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
            (SourceFormat::Designer, DumpPlan::Full, Provider::Ibcmd, _, _) => run_full_dump_ibcmd(
                context,
                config,
                &resolved,
                binary.as_path(),
                utilities.runner_for(UtilityType::Ibcmd),
            ),
            (SourceFormat::Designer, DumpPlan::Partial, Provider::Designer, Some(objects), _) => {
                run_partial_dump_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::V8),
                    objects,
                )
            }
            (SourceFormat::Designer, DumpPlan::Partial, Provider::Ibcmd, Some(objects), _) => {
                run_partial_dump_ibcmd(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::Ibcmd),
                    objects,
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
                Some(objects),
                Some(edt_binary),
            ) => run_partial_dump_edt_ibcmd(
                context,
                config,
                &resolved,
                binary.as_path(),
                edt_binary,
                utilities.runner_for(UtilityType::Ibcmd),
                utilities.runner_for(UtilityType::EdtCli),
                objects,
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
        (result, false)
    };
    // Копия меняется под тем же замком и только после удачи: сбой оставляет прежнюю.
    let result = result.map(|(platform_result, notes)| {
        let copy_warning = version_file.as_ref().and_then(RunnerVersionFile::record);
        // Уничтоженное называет тот вопрос к сторожу, который пропустил работу: перезапись —
        // до платформы, замена — при публикации.
        let discarded = if notes.discarded.is_empty() {
            overwritten
        } else {
            notes.discarded
        };
        let message = merge_optional_messages(
            whole_note,
            merge_optional_messages(
                discard_note(&resolved.target_path, &discarded),
                merge_optional_messages(notes.message, copy_warning),
            ),
        );
        (platform_result, message, discarded.into_paths())
    });
    drop(lock_guard);

    match result {
        Ok((platform_result, cleanup_message, losses)) => Ok(DumpResult {
            provider: None,
            provider_dispatched: false,
            up_to_date,
            ok: true,
            source_set: Some(resolved.source_set_name),
            extension: resolved.extension,
            selectors,
            mode,
            target_path: resolved.target_path,
            platform_log_path: platform_result.platform_log_path,
            duration_ms: started.elapsed().as_millis() as u64,
            message: cleanup_message
                .or_else(|| Some(crate::domain::dump::DUMP_SUCCESS_MESSAGE.to_owned())),
            losses,
        }),
        Err(error) => {
            let message = error.to_string();
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
                    mode,
                    target_path: resolved.target_path,
                    platform_log_path: None,
                    duration_ms: started.elapsed().as_millis() as u64,
                    message: Some(message),
                    losses: Vec::new(),
                },
            ))
        }
    }
}

/// Как выгрузка по плану обходится с каталогом человека и какие файлы в его корне пишет
/// заново. Проект EDT при любом плане заменяется целиком, и импорт описи версий не пишет;
/// в формате Конфигуратора полная выгрузка заменяет каталог, остальные ложатся поверх, и
/// опись версий платформа пишет сама.
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
        .and_then(|source| RunnerVersionFile::of(config, source))
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
                DestructionConsent::AskFirst(WaysOut::SaveWork) => String::new(),
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
fn platform_of(location: Option<&UtilityLocation>) -> Option<&PlatformVersion> {
    location.and_then(|found| match &found.version {
        Some(UtilityVersion::Platform(version)) => Some(version),
        Some(UtilityVersion::Edt(_)) | None => None,
    })
}

/// План превью: тот же [`plan_dump`], что у выгрузки, по файлу версий, который оставит в
/// каталоге сверка с копией раннера. Ничего не пишет.
fn preview_plan(
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    mode: &DumpMode,
    location: Option<&UtilityLocation>,
) -> Result<DumpPlan, AppError> {
    let in_directory = resolved.platform_target_path.join(VERSION_FILE_NAME);
    let version_file = runner_version_file(config, resolved);
    let file = match (mode, &version_file) {
        (DumpMode::Incremental, Some(version_file)) => version_file.file_after_restore()?,
        (DumpMode::Incremental | DumpMode::Full | DumpMode::Partial, _) => {
            Some(in_directory.as_path())
        }
    };
    plan_dump(mode, file, platform_of(location))
}

/// Что выгрузка делает с файлом версий набора и копией раннера.
#[derive(Debug, Clone, Copy)]
enum VersionFileUse {
    /// Сверить файл в каталоге с копией до платформы, после удачи записать копию.
    RestoreAndRecord,
    /// Полная выгрузка пишет файл заново: только записать копию после удачи.
    RecordOnly,
    /// Не трогать ни файл, ни копию.
    Untouched,
}
