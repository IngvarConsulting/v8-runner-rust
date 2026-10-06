use super::*;
use crate::domain::capability::{Operation, Provider};
use crate::use_cases::version_file::{remove_left_candidates, RunnerVersionFile};

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
        let mut preview = empty_result(
            mode.clone(),
            started,
            Some(resolved.source_set_name.clone()),
            resolved.extension.clone(),
            selectors.clone(),
            Some(resolved.target_path.clone()),
            Some(format!(
                "would dump {:?} into '{}' via {}; nothing written",
                mode.clone(),
                resolved.target_path.display(),
                match location.as_ref() {
                    Some(found) => found.path.display().to_string(),
                    None => "the attached Designer agent".to_owned(),
                }
            )),
        );
        preview.ok = true;
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
            SourceSetInventory::new(config)
                .designer_context(&resolved.source_set_name)
                .and_then(|source| RunnerVersionFile::of(config, source))
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
    let full_instead = match mode {
        DumpMode::Incremental => {
            match version_file_verdict(&resolved.platform_target_path, location.as_ref()) {
                Ok(verdict) => verdict,
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
            }
        }
        DumpMode::Full | DumpMode::Partial => None,
    };
    let over_directory = match full_instead {
        None => OverDirectory::ByVersionFile,
        Some(_) => OverDirectory::Whole,
    };
    let mode = match over_directory {
        OverDirectory::ByVersionFile => mode,
        OverDirectory::Whole => DumpMode::Full,
    };

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
            match over_directory {
                OverDirectory::ByVersionFile => &mode,
                OverDirectory::Whole => &DumpMode::Incremental,
            },
            over_directory,
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
        let result = match (config.format, &mode, provider, partial_objects, edt_binary) {
            (_, _, other, _, _) if location.is_none() && other != Provider::Agent => Err(
                crate::use_cases::unimplemented_provider(Operation::Dump, other),
            ),
            // Выгрузка по изменившемуся без файла версий — полная поверх каталога: каталог
            // человека она не заменяет, лишнего в нём не удаляет.
            (SourceFormat::Designer, DumpMode::Incremental, Provider::Designer, _, _) => {
                run_incremental_dump_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::V8),
                )
            }
            (SourceFormat::Designer, DumpMode::Full, Provider::Designer, _, _)
                if over_directory == OverDirectory::Whole =>
            {
                run_dump_over_directory_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::V8),
                    OverDirectory::Whole,
                )
            }
            (SourceFormat::Designer, DumpMode::Incremental, Provider::Ibcmd, _, _) => {
                run_incremental_dump_ibcmd(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::Ibcmd),
                )
            }
            (SourceFormat::Designer, DumpMode::Full, Provider::Ibcmd, _, _)
                if over_directory == OverDirectory::Whole =>
            {
                run_dump_over_directory_ibcmd(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::Ibcmd),
                    OverDirectory::Whole,
                )
            }
            (SourceFormat::Designer, DumpMode::Full, Provider::Designer, _, _) => {
                run_full_dump_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::V8),
                )
            }
            (SourceFormat::Designer, DumpMode::Full, Provider::Ibcmd, _, _) => run_full_dump_ibcmd(
                context,
                config,
                &resolved,
                binary.as_path(),
                utilities.runner_for(UtilityType::Ibcmd),
            ),
            (SourceFormat::Designer, DumpMode::Partial, Provider::Designer, Some(objects), _) => {
                run_partial_dump_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::V8),
                    objects,
                )
            }
            (SourceFormat::Designer, DumpMode::Partial, Provider::Ibcmd, Some(objects), _) => {
                run_partial_dump_ibcmd(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::Ibcmd),
                    objects,
                )
            }
            (SourceFormat::Edt, DumpMode::Incremental, Provider::Designer, _, Some(edt_binary)) => {
                run_incremental_dump_edt_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    edt_binary,
                    utilities.runner_for(UtilityType::V8),
                    utilities.runner_for(UtilityType::EdtCli),
                )
            }
            (SourceFormat::Edt, DumpMode::Incremental, Provider::Ibcmd, _, Some(edt_binary)) => {
                run_incremental_dump_edt_ibcmd(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    edt_binary,
                    utilities.runner_for(UtilityType::Ibcmd),
                    utilities.runner_for(UtilityType::EdtCli),
                )
            }
            (SourceFormat::Edt, DumpMode::Full, Provider::Designer, _, Some(edt_binary)) => {
                run_full_dump_edt_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    edt_binary,
                    utilities.runner_for(UtilityType::V8),
                    utilities.runner_for(UtilityType::EdtCli),
                )
            }
            (SourceFormat::Edt, DumpMode::Full, Provider::Ibcmd, _, Some(edt_binary)) => {
                run_full_dump_edt_ibcmd(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    edt_binary,
                    utilities.runner_for(UtilityType::Ibcmd),
                    utilities.runner_for(UtilityType::EdtCli),
                )
            }
            (
                SourceFormat::Edt,
                DumpMode::Partial,
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
                DumpMode::Partial,
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
            (_, DumpMode::Partial, _, None, _) => Err(AppError::Runtime(
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
    let result = result.map(|(platform_result, message)| {
        let copy_warning = version_file.as_ref().and_then(RunnerVersionFile::record);
        (
            platform_result,
            merge_optional_messages(
                full_instead.clone(),
                merge_optional_messages(message, copy_warning),
            ),
        )
    });
    drop(lock_guard);

    match result {
        Ok((platform_result, cleanup_message)) => Ok(DumpResult {
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
                },
            ))
        }
    }
}

/// Почему выгрузка по изменившемуся не может опереться на файл версий в каталоге `dir`:
/// файла нет, версия его формата не распознана или не та, что пишет выбранная платформа.
/// `None` — файл годится. Версию, которую пишет платформа, раннер знает не для всех
/// платформ; где не знает, о чужой версии не судит.
pub(super) fn version_file_verdict(
    dir: &Path,
    location: Option<&crate::platform::locator::UtilityLocation>,
) -> Result<Option<String>, AppError> {
    use crate::platform::dump_format::{read_recorded, written_by, RecordedFormat};
    use crate::platform::locator::UtilityVersion;
    let path = dir.join(crate::use_cases::ignored_files::VERSION_FILE_NAME);
    let recorded = read_recorded(&path).map_err(|error| {
        AppError::Runtime(format!("failed to read '{}': {error}", path.display()))
    })?;
    let platform = location.and_then(|found| match &found.version {
        Some(UtilityVersion::Platform(version)) => Some(version),
        Some(UtilityVersion::Edt(_)) | None => None,
    });
    Ok(match recorded {
        RecordedFormat::Missing => Some(format!(
            "no version file ConfigDumpInfo.xml in '{}': the dump ran full instead of incremental and wrote it",
            dir.display()
        )),
        RecordedFormat::Unrecognized => Some(format!(
            "the format version of '{}' is not recognized: the dump ran full instead of incremental",
            path.display()
        )),
        RecordedFormat::Version(found) => platform
            .and_then(|platform| written_by(platform).map(|written| (platform, written)))
            .filter(|(_, written)| *written != found)
            .map(|(platform, written)| {
                format!(
                    "'{}' is in format {found}, and platform {platform} writes {written}: the dump ran full instead of incremental",
                    path.display()
                )
            }),
    })
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
