use super::*;
use crate::domain::capability::{Operation, Provider};

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

    let resolved = match resolve_target(config, args) {
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
            &mode,
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
            (SourceFormat::Designer, DumpMode::Incremental, Provider::Designer, _, _) => {
                run_incremental_dump_designer(
                    context,
                    config,
                    &resolved,
                    binary.as_path(),
                    utilities.runner_for(UtilityType::V8),
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
    drop(lock_guard);

    match result {
        Ok((platform_result, cleanup_message)) => Ok(DumpResult {
            provider: None,
            provider_dispatched: true,
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
                    provider_dispatched: true,
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
