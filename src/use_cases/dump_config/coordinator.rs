use super::*;

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
                    Some(message),
                ),
            ));
        }
    };

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
                    None,
                    Some(message),
                ),
            ));
        }
    };

    let mut utilities = PlatformUtilities::from_config(config);
    let utility = match config.builder {
        BuilderBackend::Designer => UtilityType::V8,
        BuilderBackend::Ibcmd => UtilityType::Ibcmd,
    };
    let location = match utilities.locate(utility) {
        Ok(location) => location,
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
                    Some(resolved.target_path.clone()),
                    Some(message),
                ),
            ));
        }
    };
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
                        Some(resolved.target_path.clone()),
                        Some(message),
                    ),
                ));
            }
        })
    } else {
        None
    };

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
                    Some(resolved.target_path.clone()),
                    Some(message),
                ),
            ));
        }
    }

    let partial_objects = partial_objects.as_deref();
    let edt_binary = edt_binary.as_deref();
    let result = match (
        config.format,
        &mode,
        &config.builder,
        partial_objects,
        edt_binary,
    ) {
        (SourceFormat::Designer, DumpMode::Incremental, BuilderBackend::Designer, _, _) => {
            run_incremental_dump_designer(
                context,
                config,
                &resolved,
                location.path.as_path(),
                utilities.runner_for(UtilityType::V8),
            )
        }
        (SourceFormat::Designer, DumpMode::Incremental, BuilderBackend::Ibcmd, _, _) => {
            run_incremental_dump_ibcmd(
                context,
                config,
                &resolved,
                location.path.as_path(),
                utilities.runner_for(UtilityType::Ibcmd),
            )
        }
        (SourceFormat::Designer, DumpMode::Full, BuilderBackend::Designer, _, _) => {
            run_full_dump_designer(
                context,
                config,
                &resolved,
                location.path.as_path(),
                utilities.runner_for(UtilityType::V8),
            )
        }
        (SourceFormat::Designer, DumpMode::Full, BuilderBackend::Ibcmd, _, _) => {
            run_full_dump_ibcmd(
                context,
                config,
                &resolved,
                location.path.as_path(),
                utilities.runner_for(UtilityType::Ibcmd),
            )
        }
        (SourceFormat::Designer, DumpMode::Partial, BuilderBackend::Designer, Some(objects), _) => {
            run_partial_dump_designer(
                context,
                config,
                &resolved,
                location.path.as_path(),
                utilities.runner_for(UtilityType::V8),
                objects,
            )
        }
        (SourceFormat::Designer, DumpMode::Partial, BuilderBackend::Ibcmd, Some(objects), _) => {
            run_partial_dump_ibcmd(
                context,
                config,
                &resolved,
                location.path.as_path(),
                utilities.runner_for(UtilityType::Ibcmd),
                objects,
            )
        }
        (
            SourceFormat::Edt,
            DumpMode::Incremental,
            BuilderBackend::Designer,
            _,
            Some(edt_binary),
        ) => run_incremental_dump_edt_designer(
            context,
            config,
            &resolved,
            location.path.as_path(),
            edt_binary,
            utilities.runner_for(UtilityType::V8),
            utilities.runner_for(UtilityType::EdtCli),
        ),
        (SourceFormat::Edt, DumpMode::Incremental, BuilderBackend::Ibcmd, _, Some(edt_binary)) => {
            run_incremental_dump_edt_ibcmd(
                context,
                config,
                &resolved,
                location.path.as_path(),
                edt_binary,
                utilities.runner_for(UtilityType::Ibcmd),
                utilities.runner_for(UtilityType::EdtCli),
            )
        }
        (SourceFormat::Edt, DumpMode::Full, BuilderBackend::Designer, _, Some(edt_binary)) => {
            run_full_dump_edt_designer(
                context,
                config,
                &resolved,
                location.path.as_path(),
                edt_binary,
                utilities.runner_for(UtilityType::V8),
                utilities.runner_for(UtilityType::EdtCli),
            )
        }
        (SourceFormat::Edt, DumpMode::Full, BuilderBackend::Ibcmd, _, Some(edt_binary)) => {
            run_full_dump_edt_ibcmd(
                context,
                config,
                &resolved,
                location.path.as_path(),
                edt_binary,
                utilities.runner_for(UtilityType::Ibcmd),
                utilities.runner_for(UtilityType::EdtCli),
            )
        }
        (
            SourceFormat::Edt,
            DumpMode::Partial,
            BuilderBackend::Designer,
            Some(objects),
            Some(edt_binary),
        ) => run_partial_dump_edt_designer(
            context,
            config,
            &resolved,
            location.path.as_path(),
            edt_binary,
            utilities.runner_for(UtilityType::V8),
            utilities.runner_for(UtilityType::EdtCli),
            objects,
        ),
        (
            SourceFormat::Edt,
            DumpMode::Partial,
            BuilderBackend::Ibcmd,
            Some(objects),
            Some(edt_binary),
        ) => run_partial_dump_edt_ibcmd(
            context,
            config,
            &resolved,
            location.path.as_path(),
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
    };
    drop(lock_guard);

    match result {
        Ok((platform_result, cleanup_message)) => Ok(DumpResult {
            ok: true,
            source_set: Some(resolved.source_set_name),
            extension: resolved.extension,
            mode,
            target_path: resolved.target_path,
            platform_log_path: platform_result.platform_log_path,
            duration_ms: started.elapsed().as_millis() as u64,
            message: cleanup_message.or_else(|| Some("dump completed successfully".to_owned())),
            receipt: crate::domain::sync_receipt::SyncReceipt::empty_applied(),
        }),
        Err(error) => {
            let message = error.to_string();
            Err(DumpExecutionFailure::with_payload(
                error,
                DumpResult {
                    ok: false,
                    source_set: Some(resolved.source_set_name),
                    extension: resolved.extension,
                    mode,
                    target_path: resolved.target_path,
                    platform_log_path: None,
                    duration_ms: started.elapsed().as_millis() as u64,
                    message: Some(message),
                    receipt: crate::domain::sync_receipt::SyncReceipt::empty_failed(),
                },
            ))
        }
    }
}
