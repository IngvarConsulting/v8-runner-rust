use super::helpers::{fail_with_remaining_steps, AnalysisByName};
use super::*;
use crate::domain::capability::{Operation, Provider};
use crate::domain::progress::ProgressPhase;
use crate::use_cases::context::shell_word;
use crate::use_cases::exchange_guard::{BeforeLoad, GenerationGate};
use crate::use_cases::request::ApplyPolicy;
use crate::use_cases::version_file::{RunnerVersionFile, VERSION_FILE_NAME};

/// Кто грузит набор исходников в базу: пакетный Конфигуратор или его агент.
///
/// Утилита ищется до превью (превью отказывает без платформы), а процесс или сессия
/// поднимаются только перед первой настоящей загрузкой: сборка без изменений
/// платформу не запускает.
pub(super) trait SourceSetLoader {
    fn locate(&mut self) -> Result<(), AppError>;

    /// Инструмент, которым читается и записывается поколение.
    fn tool(&self) -> Provider;

    /// Поколение базы для набора; `None` — ответа нет.
    fn read_generation(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        source_set: &SourceSetConfig,
        step_index: usize,
    ) -> Result<Option<String>, AppError>;

    #[allow(clippy::too_many_arguments)]
    fn load(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        source_set: &SourceSetConfig,
        source_context: &SourceSetContext,
        step_index: usize,
        partial_paths: Option<&[PathBuf]>,
        commit: &StepCommit,
        apply: ApplyPolicy,
    ) -> Result<Loaded, AppError>;

    /// Один файл версий набора в каталог `into` — свежая опись после крупной загрузки (#646).
    fn dump_version_file(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        source_set: &SourceSetConfig,
        step_index: usize,
        into: &Path,
    ) -> Result<(), AppError>;

    /// Только применение: отправка, которой нечего грузить, применяет непринятое своей
    /// прежней загрузки (`INV.USE-CASES.A-PUSH-WITH-NOTHING-TO-LOAD-APPLIES-ITS-OWN-UNAPPLIED`).
    fn apply_only(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        source_set: &SourceSetConfig,
        step_index: usize,
        deferrals: &mut crate::use_cases::interruption::Deferrals,
    ) -> Result<(), AppError>;

    /// Вызывается после последнего набора, и при отказе тоже.
    fn finish(&mut self, _context: &ExecutionContext) {}
}

/// Пакетный Конфигуратор: процесс на каждую команду, утилита — `1cv8`.
pub(super) struct DesignerLoader {
    utilities: PlatformUtilities,
    binary: Option<PathBuf>,
}

impl DesignerLoader {
    pub(super) fn new(config: &AppConfig) -> Self {
        Self {
            utilities: PlatformUtilities::from_config(config),
            binary: None,
        }
    }
}

impl DesignerLoader {
    fn binary(&self) -> Result<PathBuf, AppError> {
        self.binary
            .clone()
            .ok_or_else(|| AppError::Runtime("Designer was not located before the load".to_owned()))
    }
}

impl SourceSetLoader for DesignerLoader {
    fn locate(&mut self) -> Result<(), AppError> {
        if self.binary.is_none() {
            let location = self.utilities.locate(UtilityType::V8)?;
            self.binary = Some(location.path);
        }
        Ok(())
    }

    fn tool(&self) -> Provider {
        Provider::Designer
    }

    fn read_generation(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        source_set: &SourceSetConfig,
        step_index: usize,
    ) -> Result<Option<String>, AppError> {
        read_designer_generation(
            context,
            config,
            &self.binary()?,
            self.utilities.runner_for(UtilityType::V8),
            source_set,
            step_index,
        )
    }

    fn load(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        source_set: &SourceSetConfig,
        source_context: &SourceSetContext,
        step_index: usize,
        partial_paths: Option<&[PathBuf]>,
        commit: &StepCommit,
        apply: ApplyPolicy,
    ) -> Result<Loaded, AppError> {
        let binary = self.binary()?;
        execute_source_set_step(
            context,
            config,
            &binary,
            self.utilities.runner_for(UtilityType::V8),
            source_set,
            source_context,
            source_context,
            step_index,
            partial_paths,
            commit,
            apply,
        )
    }

    fn dump_version_file(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        source_set: &SourceSetConfig,
        step_index: usize,
        into: &Path,
    ) -> Result<(), AppError> {
        let binary = self.binary()?;
        let result = build_designer_dsl(
            context,
            config,
            &binary,
            self.utilities.runner_for(UtilityType::V8),
            &source_set.name,
            step_index,
            "dump-info",
            InterruptionSafetyClass::GracefulThenKill,
        )?
        .dump_config_dump_info_only(into, extension_name(source_set))
        .map_err(AppError::from)?;
        ensure_platform_success("dump_info", source_set, &result)
    }

    fn apply_only(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        source_set: &SourceSetConfig,
        step_index: usize,
        deferrals: &mut crate::use_cases::interruption::Deferrals,
    ) -> Result<(), AppError> {
        let binary = self.binary()?;
        crate::use_cases::apply::act::apply(
            context,
            config,
            crate::use_cases::apply::act::Applier::Designer {
                binary: &binary,
                runner: self.utilities.runner_for(UtilityType::V8),
                log_file: designer_log_file(config, &source_set.name, step_index, "update")?,
            },
            &apply_subject(source_set),
            deferrals,
        )
    }
}

pub(super) fn run_build_designer(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &BuildArgs,
    analysis: Option<AnalysisByName>,
) -> Result<BuildResult, BuildExecutionFailure> {
    run_build_with(
        context,
        config,
        args,
        analysis,
        &mut DesignerLoader::new(config),
    )
}

pub(super) fn run_build_agent(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &BuildArgs,
    analysis: Option<AnalysisByName>,
    early: Option<crate::use_cases::agent_session::EarlyAgent>,
) -> Result<BuildResult, BuildExecutionFailure> {
    let mut loader = super::agent::AgentLoader::new(config, early);
    let outcome = run_build_with(context, config, args, analysis, &mut loader);
    loader.finish(context);
    outcome
}

/// Разбор изменений исполнителя: сделанный до выбора исполнителя (`build_project::push_size`)
/// берётся как есть, без него исполнитель разбирает сам. Полной отправке разбор не нужен.
fn analysis_or_analyze(
    context: &ExecutionContext,
    inventory: &SourceSetInventory<'_>,
    contexts: &[SourceSetContext],
    args: &BuildArgs,
    analysis: Option<AnalysisByName>,
) -> Result<Option<AnalysisByName>, BuildExecutionFailure> {
    if args.load.is_whole() {
        return Ok(None);
    }
    match analysis {
        Some(analysis) => Ok(Some(analysis)),
        None => analyze_contexts_by_name(context, inventory, contexts, args.dry_run)
            .map(Some)
            .map_err(BuildExecutionFailure::without_payload),
    }
}

fn run_build_with(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &BuildArgs,
    analysis: Option<AnalysisByName>,
    loader: &mut dyn SourceSetLoader,
) -> Result<BuildResult, BuildExecutionFailure> {
    debug!(
        load = ?args.load,
        source_set = args.source_set.as_deref(),
        "preparing build plan"
    );

    let started = Instant::now();
    let inventory = SourceSetInventory::new(config);
    let ordered_source_sets =
        match selected_ordered_source_sets(&inventory, args.source_set.as_deref()) {
            Ok(source_sets) => source_sets,
            Err(error) => {
                return Err(BuildExecutionFailure::with_payload(
                    error,
                    BuildResult {
                        detection: Default::default(),
                        provider: None,
                        provider_dispatched: false,
                        ok: false,
                        steps: vec![],
                        duration_ms: started.elapsed().as_millis() as u64,
                        sessions: None,
                    },
                ));
            }
        };
    let selected_designer_contexts =
        designer_contexts_for_source_sets(&inventory, &ordered_source_sets);

    let analysis_by_name = analysis_or_analyze(
        context,
        &inventory,
        &selected_designer_contexts,
        args,
        analysis,
    )?;

    let mut steps = Vec::new();
    let gate = GenerationGate::new(context, config, args.load);
    let tool = loader.tool();
    let loading = sets_to_load(&inventory, &ordered_source_sets, args, |set| {
        loads_into_the_base(set, args.load, analysis_by_name.as_ref())
    });
    gate.check_foreign(&loading, tool).map_err(|(set, error)| {
        refused_before_the_loads(started, &ordered_source_sets, &set, error)
    })?;
    gate.check_early(&loading, tool, |set| {
        let Some((index, source_set)) = source_set_named(&ordered_source_sets, set.name()) else {
            return Ok(None);
        };
        // Исполнителя, которого не найти, назовёт загрузка первого набора.
        if loader.locate().is_err() {
            return Ok(None);
        }
        loader.read_generation(context, config, source_set, index)
    })
    .map_err(|(set, error)| refused_before_the_loads(started, &ordered_source_sets, &set, error))?;

    for (index, source_set) in ordered_source_sets.iter().enumerate() {
        let Some(source_context) = inventory.designer_context(&source_set.name).cloned() else {
            continue;
        };

        if source_set.purpose.is_external() {
            let step_started = Instant::now();
            let result = discover_designer_external_artifacts(
                &source_set.name,
                &inventory.source_path(source_set),
                source_set_external_kind(source_set).expect("external kind"),
            );
            match result {
                Ok(descriptors) => push_build_step(
                    &mut steps,
                    &source_set.name,
                    BuildMode::Skipped,
                    true,
                    format!(
                        "prepared {} external artifact(s) for packaging",
                        descriptors.len()
                    ),
                    step_started.elapsed().as_millis() as u64,
                ),
                Err(error) => {
                    let result = fail_from_source_set_index(
                        started,
                        steps,
                        &ordered_source_sets,
                        index,
                        source_set,
                        BuildMode::Skipped,
                        error.to_string(),
                    );
                    return Err(BuildExecutionFailure::with_payload(error, result));
                }
            }
            continue;
        }

        let plan = match plan_configurator_load_step(
            source_set,
            &source_context,
            args.load.is_whole(),
            analysis_by_name.as_ref(),
        ) {
            Ok(plan) => plan,
            Err(error) => {
                let error = change_detection_failure(&error, context)
                    .map_err(BuildExecutionFailure::without_payload)?;
                let result = fail_from_source_set_index(
                    started,
                    steps,
                    &ordered_source_sets,
                    index,
                    source_set,
                    BuildMode::Skipped,
                    error.clone(),
                );
                return Err(BuildExecutionFailure::with_payload(
                    AppError::Runtime(error),
                    result,
                ));
            }
        };

        match plan {
            StepPlan::Skip { message, ok } => {
                debug!(
                    source_set = source_set.name.as_str(),
                    message = message.as_str(),
                    "skipping build step"
                );
                let settled = skip_or_apply_unapplied(
                    &mut steps,
                    context,
                    config,
                    args,
                    &source_context,
                    &source_set.name,
                    loader.tool(),
                    message,
                    ok,
                    |record| {
                        loader.locate().and_then(|()| {
                            apply_unapplied(context, &source_context, record, &mut |op| match op {
                                UnappliedOp::Read => {
                                    loader.read_generation(context, config, source_set, index)
                                }
                                UnappliedOp::Apply(deferrals) => loader
                                    .apply_only(context, config, source_set, index, deferrals)
                                    .map(|()| None),
                            })
                        })
                    },
                );
                if let Err(error) = settled {
                    let result = fail_from_source_set_index(
                        started,
                        steps,
                        &ordered_source_sets,
                        index,
                        source_set,
                        BuildMode::Skipped,
                        error.to_string(),
                    );
                    return Err(BuildExecutionFailure::with_payload(error, result));
                }
            }
            StepPlan::Execute {
                mode,
                message,
                partial_paths,
                commit,
            } => {
                debug!(
                    source_set = source_set.name.as_str(),
                    mode = ?mode,
                    message = message.as_str(),
                    "executing build step"
                );
                if let Err(error) = loader.locate() {
                    let result = fail_from_source_set_index(
                        started,
                        steps,
                        &ordered_source_sets,
                        index,
                        source_set,
                        mode.clone(),
                        error.to_string(),
                    );
                    return Err(BuildExecutionFailure::with_payload(error, result));
                }

                if args.dry_run {
                    // Тем же путём идут Конфигуратор и агент: превью называет того, кто выбран,
                    // его именем из словаря исполнителей.
                    push_build_step(
                        &mut steps,
                        &source_set.name,
                        mode,
                        true,
                        format!("{message}; planned, {} not dispatched", loader.tool()),
                        0,
                    );
                    continue;
                }

                let step_started = Instant::now();
                let tool = loader.tool();
                let loaded = gate
                    .before_load(&source_context, tool, || {
                        loader.read_generation(context, config, source_set, index)
                    })
                    .and_then(|before| {
                        loader
                            .load(
                                context,
                                config,
                                source_set,
                                &source_context,
                                index,
                                partial_paths.as_deref(),
                                &commit,
                                args.apply,
                            )
                            .map(|loaded| (before, loaded))
                            .inspect_err(|error| gate.after_failed_load(&source_context, error))
                    })
                    .and_then(|(before, Loaded { warnings, apply })| {
                        // Ответ, который ничего не решает, не спрашивается (#637).
                        let read = if gate.generation_after_load_decides(&source_context) {
                            loader.read_generation(context, config, source_set, index)
                        } else {
                            Ok(None)
                        };
                        generation_after_load(&gate, &source_context, tool, warnings, read)
                            .map(|(warnings, token)| (before, warnings, token, apply))
                    });
                match loaded {
                    Ok((before, mut warnings, token, apply)) => {
                        let recorded = gate.after_load(
                            &source_context,
                            loader.tool(),
                            token.as_deref(),
                            apply.mark(),
                        );
                        warnings.extend(version_file_after_load(
                            context,
                            config,
                            &gate,
                            &source_context,
                            VersionFileLoad {
                                main_configuration: source_set.purpose
                                    == crate::config::model::SourceSetPurpose::Configuration,
                                partial_paths: partial_paths.as_deref(),
                                before: &before,
                                token: token.as_deref(),
                            },
                            |into| {
                                loader
                                    .dump_version_file(context, config, source_set, index, into)?;
                                loader.read_generation(context, config, source_set, index)
                            },
                        ));
                        let remembered = remember_unapplied(&apply, recorded, || {
                            commit_step_state(
                                source_set,
                                &source_context,
                                &config.work_path,
                                &commit,
                            )
                        });
                        match remembered {
                            Ok(notes) => warnings.extend(notes),
                            Err(error) => {
                                let result = fail_from_source_set_index(
                                    started,
                                    steps,
                                    &ordered_source_sets,
                                    index,
                                    source_set,
                                    mode,
                                    error.to_string(),
                                );
                                return Err(BuildExecutionFailure::with_payload(error, result));
                            }
                        }
                        match settle_loaded(context, &source_set.name, warnings, apply) {
                            Ok((warnings, applied)) => push_loaded_step(
                                &mut steps,
                                &source_set.name,
                                mode,
                                applied,
                                append_warnings(message, &warnings),
                                step_started.elapsed().as_millis() as u64,
                            ),
                            Err(error) => {
                                let result = fail_from_source_set_index(
                                    started,
                                    steps,
                                    &ordered_source_sets,
                                    index,
                                    source_set,
                                    mode,
                                    error.to_string(),
                                );
                                return Err(BuildExecutionFailure::with_payload(error, result));
                            }
                        }
                    }
                    Err(error) => {
                        let result = fail_from_source_set_index(
                            started,
                            steps,
                            &ordered_source_sets,
                            index,
                            source_set,
                            mode,
                            append_warnings(error.to_string(), &gate.failed_load_note()),
                        );
                        return Err(BuildExecutionFailure::with_payload(error, result));
                    }
                }
            }
        }
    }

    Ok(BuildResult {
        detection: Default::default(),
        provider: None,
        provider_dispatched: false,
        ok: true,
        steps,
        duration_ms: started.elapsed().as_millis() as u64,
        sessions: None,
    })
}

pub(super) fn run_build_ibcmd(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &BuildArgs,
    analysis: Option<AnalysisByName>,
) -> Result<BuildResult, BuildExecutionFailure> {
    debug!(
        load = ?args.load,
        source_set = args.source_set.as_deref(),
        "preparing ibcmd build plan"
    );

    let started = Instant::now();
    let inventory = SourceSetInventory::new(config);
    let ordered_source_sets =
        match selected_ordered_source_sets(&inventory, args.source_set.as_deref()) {
            Ok(source_sets) => source_sets,
            Err(error) => {
                return Err(BuildExecutionFailure::with_payload(
                    error,
                    BuildResult {
                        detection: Default::default(),
                        provider: None,
                        provider_dispatched: false,
                        ok: false,
                        steps: vec![],
                        duration_ms: started.elapsed().as_millis() as u64,
                        sessions: None,
                    },
                ));
            }
        };
    let selected_designer_contexts =
        designer_contexts_for_source_sets(&inventory, &ordered_source_sets);

    let analysis_by_name = analysis_or_analyze(
        context,
        &inventory,
        &selected_designer_contexts,
        args,
        analysis,
    )?;

    let mut utilities = PlatformUtilities::from_config(config);
    let mut ibcmd_binary: Option<PathBuf> = None;
    let mut steps = Vec::new();
    let gate = GenerationGate::new(context, config, args.load);
    let loading = sets_to_load(&inventory, &ordered_source_sets, args, |set| {
        loads_into_the_base(set, args.load, analysis_by_name.as_ref())
    });
    gate.check_foreign(&loading, Provider::Ibcmd)
        .map_err(|(set, error)| {
            refused_before_the_loads(started, &ordered_source_sets, &set, error)
        })?;
    gate.check_early(&loading, Provider::Ibcmd, |set| {
        let Some((_, source_set)) = source_set_named(&ordered_source_sets, set.name()) else {
            return Ok(None);
        };
        // `ibcmd`, которого не найти, назовёт загрузка первого набора.
        let Ok(binary) = locate_designer_loader(
            Provider::Ibcmd,
            &mut utilities,
            &mut None,
            &mut ibcmd_binary,
        ) else {
            return Ok(None);
        };
        read_ibcmd_generation(
            context,
            config,
            &binary,
            utilities.runner_for(UtilityType::Ibcmd),
            source_set,
        )
    })
    .map_err(|(set, error)| refused_before_the_loads(started, &ordered_source_sets, &set, error))?;

    for (index, source_set) in ordered_source_sets.iter().enumerate() {
        let Some(source_context) = inventory.designer_context(&source_set.name).cloned() else {
            continue;
        };

        let plan = match plan_configurator_load_step(
            source_set,
            &source_context,
            args.load.is_whole(),
            analysis_by_name.as_ref(),
        ) {
            Ok(plan) => plan,
            Err(error) => {
                let error = change_detection_failure(&error, context)
                    .map_err(BuildExecutionFailure::without_payload)?;
                let result = fail_from_source_set_index(
                    started,
                    steps,
                    &ordered_source_sets,
                    index,
                    source_set,
                    BuildMode::Skipped,
                    error.clone(),
                );
                return Err(BuildExecutionFailure::with_payload(
                    AppError::Runtime(error),
                    result,
                ));
            }
        };

        match plan {
            StepPlan::Skip { message, ok } => {
                debug!(
                    source_set = source_set.name.as_str(),
                    message = message.as_str(),
                    "skipping build step"
                );
                let settled = skip_or_apply_unapplied(
                    &mut steps,
                    context,
                    config,
                    args,
                    &source_context,
                    &source_set.name,
                    Provider::Ibcmd,
                    message,
                    ok,
                    |record| {
                        locate_designer_loader(
                            Provider::Ibcmd,
                            &mut utilities,
                            &mut None,
                            &mut ibcmd_binary,
                        )
                        .and_then(|binary| {
                            let runner = utilities.runner_for(UtilityType::Ibcmd);
                            apply_unapplied(context, &source_context, record, &mut |op| {
                                ibcmd_unapplied_op(context, config, &binary, runner, source_set, op)
                            })
                        })
                    },
                );
                if let Err(error) = settled {
                    let result = fail_from_source_set_index(
                        started,
                        steps,
                        &ordered_source_sets,
                        index,
                        source_set,
                        BuildMode::Skipped,
                        error.to_string(),
                    );
                    return Err(BuildExecutionFailure::with_payload(error, result));
                }
            }
            StepPlan::Execute {
                mode,
                message,
                partial_paths,
                commit,
            } => {
                debug!(
                    source_set = source_set.name.as_str(),
                    mode = ?mode,
                    message = message.as_str(),
                    "executing ibcmd build step"
                );
                let binary = match ibcmd_binary.clone() {
                    Some(path) => path,
                    None => {
                        let location = match utilities.locate(UtilityType::Ibcmd) {
                            Ok(location) => location,
                            Err(error) => {
                                let result = fail_from_source_set_index(
                                    started,
                                    steps,
                                    &ordered_source_sets,
                                    index,
                                    source_set,
                                    mode.clone(),
                                    error.to_string(),
                                );
                                return Err(BuildExecutionFailure::with_payload(
                                    AppError::from(error),
                                    result,
                                ));
                            }
                        };
                        ibcmd_binary = Some(location.path.clone());
                        location.path
                    }
                };

                if args.dry_run {
                    push_build_step(
                        &mut steps,
                        &source_set.name,
                        mode,
                        true,
                        format!("{message}; planned, {} not dispatched", Provider::Ibcmd),
                        0,
                    );
                    continue;
                }

                let step_started = Instant::now();
                let runner = utilities.runner_for(UtilityType::Ibcmd);
                let read = || read_ibcmd_generation(context, config, &binary, runner, source_set);
                // Опись расширения отдельной выгрузкой не замерена ни у одного исполнителя (#403, #646).
                let main_configuration =
                    source_set.purpose == crate::config::model::SourceSetPurpose::Configuration;
                match guarded_load_then(
                    &gate,
                    &source_context,
                    Provider::Ibcmd,
                    read,
                    |before, token| {
                        let load = VersionFileLoad {
                            main_configuration,
                            partial_paths: partial_paths.as_deref(),
                            before,
                            token,
                        };
                        version_file_after_load(
                            context,
                            config,
                            &gate,
                            &source_context,
                            load,
                            |into| {
                                let exported = build_ibcmd_dsl(
                                    context,
                                    config,
                                    &binary,
                                    runner,
                                    InterruptionSafetyClass::GracefulThenKill,
                                )?
                                .config_export_info(into)
                                .map_err(map_ibcmd_error)?;
                                exported.process.outcome().map_err(|error| {
                                    AppError::Runtime(format!(
                                        "`ibcmd config export info` failed: {error}"
                                    ))
                                })?;
                                read()
                            },
                        )
                    },
                    || {
                        execute_source_set_step_ibcmd(
                            context,
                            config,
                            &binary,
                            runner,
                            source_set,
                            &source_context,
                            &source_context,
                            partial_paths.as_deref(),
                            &commit,
                            args.apply,
                        )
                    },
                    || commit_step_state(source_set, &source_context, &config.work_path, &commit),
                )
                .and_then(|Loaded { warnings, apply }| {
                    settle_loaded(context, &source_set.name, warnings, apply)
                }) {
                    Ok((warnings, applied)) => push_loaded_step(
                        &mut steps,
                        &source_set.name,
                        mode,
                        applied,
                        append_warnings(message, &warnings),
                        step_started.elapsed().as_millis() as u64,
                    ),
                    Err(error) => {
                        let result = fail_from_source_set_index(
                            started,
                            steps,
                            &ordered_source_sets,
                            index,
                            source_set,
                            mode,
                            append_warnings(error.to_string(), &gate.failed_load_note()),
                        );
                        return Err(BuildExecutionFailure::with_payload(error, result));
                    }
                }
            }
        }
    }

    Ok(BuildResult {
        detection: Default::default(),
        provider: None,
        provider_dispatched: false,
        ok: true,
        steps,
        duration_ms: started.elapsed().as_millis() as u64,
        sessions: None,
    })
}

/// Утилита, которая загрузит файлы конфигуратора в базу. Кэш тот же, что у боевого
/// прогона, поэтому превью и запуск ищут одно и то же и в одном порядке.
fn locate_designer_loader(
    provider: Provider,
    utilities: &mut PlatformUtilities,
    designer_binary: &mut Option<PathBuf>,
    ibcmd_binary: &mut Option<PathBuf>,
) -> Result<PathBuf, AppError> {
    match provider {
        other @ (Provider::Agent | Provider::IbcmdRs | Provider::Webinst) => Err(
            crate::use_cases::unimplemented_provider(Operation::Build, other),
        ),
        Provider::Designer => {
            if let Some(path) = designer_binary.clone() {
                return Ok(path);
            }
            let path = utilities
                .locate(UtilityType::V8)
                .map_err(AppError::from)?
                .path;
            *designer_binary = Some(path.clone());
            Ok(path)
        }
        Provider::Ibcmd => {
            if let Some(path) = ibcmd_binary.clone() {
                return Ok(path);
            }
            let path = utilities
                .locate(UtilityType::Ibcmd)
                .map_err(AppError::from)?
                .path;
            *ibcmd_binary = Some(path.clone());
            Ok(path)
        }
    }
}

pub(super) fn run_build_edt(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &BuildArgs,
    provider: Provider,
) -> Result<BuildResult, BuildExecutionFailure> {
    debug!(
        load = ?args.load,
        source_set = args.source_set.as_deref(),
        "preparing edt build plan"
    );
    if let Some(error) = validate_edt_supported_matrix(config, provider) {
        return Err(BuildExecutionFailure::with_payload(
            error,
            BuildResult {
                detection: Default::default(),
                provider: None,
                provider_dispatched: false,
                ok: false,
                steps: vec![],
                duration_ms: 0,
                sessions: None,
            },
        ));
    }

    let started = Instant::now();
    let inventory = SourceSetInventory::new(config);
    let ordered_source_sets =
        match selected_ordered_source_sets(&inventory, args.source_set.as_deref()) {
            Ok(source_sets) => source_sets,
            Err(error) => {
                return Err(BuildExecutionFailure::with_payload(
                    error,
                    BuildResult {
                        detection: Default::default(),
                        provider: None,
                        provider_dispatched: false,
                        ok: false,
                        steps: vec![],
                        duration_ms: started.elapsed().as_millis() as u64,
                        sessions: None,
                    },
                ));
            }
        };
    let selected_edt_contexts = edt_contexts_for_source_sets(&inventory, &ordered_source_sets);

    let edt_analysis_by_name = if args.load.is_whole() {
        None
    } else {
        Some(
            analyze_contexts_by_name(context, &inventory, &selected_edt_contexts, args.dry_run)
                .map_err(BuildExecutionFailure::without_payload)?,
        )
    };

    let mut utilities = PlatformUtilities::from_config(config);
    let mut designer_binary: Option<PathBuf> = None;
    let mut ibcmd_binary: Option<PathBuf> = None;
    let mut edt_binary: Option<PathBuf> = None;
    let mut interactive_edt = None;
    let mut steps = Vec::new();
    let gate = GenerationGate::new(context, config, args.load);
    // Пойдёт ли набор в базу, видно по анализу исходников EDT, а у набора с пропущенным
    // этапом EDT — по анализу его копии Конфигуратора: она грузится, если изменилась сама.
    if matches!(provider, Provider::Designer | Provider::Ibcmd) {
        let loading = sets_to_load(&inventory, &ordered_source_sets, args, |set| {
            loads_into_the_base(set, args.load, edt_analysis_by_name.as_ref())
                || generated_copy_loads(context, set, &inventory, &config.work_path)
        });
        if let Some(error) = crate::use_cases::interruption::pending_interruption_error(
            context,
            "during generated-source analysis",
        ) {
            return Err(BuildExecutionFailure::without_payload(error));
        }
        gate.check_foreign(&loading, provider)
            .map_err(|(set, error)| {
                refused_before_the_loads(started, &ordered_source_sets, &set, error)
            })?;
        gate.check_early(&loading, provider, |set| {
            let Some((index, source_set)) = source_set_named(&ordered_source_sets, set.name())
            else {
                return Ok(None);
            };
            // Исполнителя, которого не найти, назовёт загрузка первого набора.
            let Ok(binary) = locate_designer_loader(
                provider,
                &mut utilities,
                &mut designer_binary,
                &mut ibcmd_binary,
            ) else {
                return Ok(None);
            };
            match provider {
                Provider::Ibcmd => read_ibcmd_generation(
                    context,
                    config,
                    &binary,
                    utilities.runner_for(UtilityType::Ibcmd),
                    source_set,
                ),
                Provider::Designer => read_designer_generation(
                    context,
                    config,
                    &binary,
                    utilities.runner_for(UtilityType::V8),
                    source_set,
                    index,
                ),
                Provider::Agent | Provider::IbcmdRs | Provider::Webinst => Ok(None),
            }
        })
        .map_err(|(set, error)| {
            refused_before_the_loads(started, &ordered_source_sets, &set, error)
        })?;
    }

    for (index, source_set) in ordered_source_sets.iter().enumerate() {
        let Some(edt_context) = inventory.edt_context(&source_set.name).cloned() else {
            continue;
        };
        let Some(designer_context) = inventory.designer_context(&source_set.name).cloned() else {
            continue;
        };

        let edt_stage = match plan_edt_export_step(
            source_set,
            args.load.is_whole(),
            edt_analysis_by_name.as_ref(),
        ) {
            Ok(plan) => plan,
            Err(error) => {
                let error = change_detection_failure(&error, context)
                    .map_err(BuildExecutionFailure::without_payload)?;
                let result = fail_from_source_set_index(
                    started,
                    steps,
                    &ordered_source_sets,
                    index,
                    source_set,
                    BuildMode::Skipped,
                    error.clone(),
                );
                return Err(BuildExecutionFailure::with_payload(
                    AppError::Runtime(error),
                    result,
                ));
            }
        };

        if source_set.purpose.is_external() {
            let edt = match edt_binary.clone() {
                Some(path) => path,
                None => {
                    let location = match utilities.locate(UtilityType::EdtCli) {
                        Ok(location) => location,
                        Err(error) => {
                            let result = fail_from_source_set_index(
                                started,
                                steps,
                                &ordered_source_sets,
                                index,
                                source_set,
                                BuildMode::EdtExport,
                                error.to_string(),
                            );
                            return Err(BuildExecutionFailure::with_payload(
                                AppError::from(error),
                                result,
                            ));
                        }
                    };
                    edt_binary = Some(location.path.clone());
                    location.path
                }
            };

            if args.dry_run {
                // Экспорт внешних артефактов пересоздаёт каталог в `workPath`, запускает
                // EDT CLI и фиксирует состояние обнаружения изменений; превью
                // останавливается до всех трёх. Остановка стоит там же, где у соседней
                // ветки набора исходников: после поиска утилиты.
                push_build_step(
                    &mut steps,
                    &source_set.name,
                    BuildMode::EdtExport,
                    true,
                    format!(
                        "would export the external artifacts of '{}' to Designer files via {}; planned, nothing dispatched",
                        source_set.name,
                        edt.display()
                    ),
                    0,
                );
                continue;
            }

            let export_started = Instant::now();
            if let Some(error) = interruption_before_safe_point(
                context,
                format!(
                    "EDT external artifact export for source-set '{}'",
                    source_set.name
                ),
            ) {
                let result = fail_from_source_set_index(
                    started,
                    steps,
                    &ordered_source_sets,
                    index,
                    source_set,
                    BuildMode::EdtExport,
                    error.to_string(),
                );
                return Err(BuildExecutionFailure::with_payload(error, result));
            }
            log_timeline_stage(
                &source_set.name,
                ProgressPhase::Convert,
                "[EDT] Конвертация внешних объектов в файлы конфигуратора",
                TimelineStageStatus::Running,
            );
            let export_result = if config.tools.edt_cli.interactive_mode {
                if interactive_edt.is_none() {
                    interactive_edt = Some(
                        match EdtSessionManager::for_config(
                            config,
                            EdtSessionHostOptions::for_cli_command(config),
                        ) {
                            Ok(manager) => match EdtDsl::new_shared_session(
                                edt.clone(),
                                config.work_path.join("edt-workspace"),
                                Arc::new(manager),
                                Duration::from_millis(config.tools.edt_cli.startup_timeout_ms),
                                Duration::from_millis(config.tools.edt_cli.command_timeout_ms),
                                context.process_policy(
                                    InterruptionSafetyClass::GracefulThenKill,
                                    None,
                                ),
                            ) {
                                Ok(dsl) => dsl,
                                Err(error) => {
                                    let app_error = AppError::from(error);
                                    let result = fail_from_source_set_index(
                                        started,
                                        steps,
                                        &ordered_source_sets,
                                        index,
                                        source_set,
                                        BuildMode::EdtExport,
                                        app_error.to_string(),
                                    );
                                    return Err(BuildExecutionFailure::with_payload(
                                        app_error, result,
                                    ));
                                }
                            },
                            Err(error) => {
                                let app_error = AppError::from(error);
                                let result = fail_from_source_set_index(
                                    started,
                                    steps,
                                    &ordered_source_sets,
                                    index,
                                    source_set,
                                    BuildMode::EdtExport,
                                    app_error.to_string(),
                                );
                                return Err(BuildExecutionFailure::with_payload(app_error, result));
                            }
                        },
                    );
                }
                prepare_edt_external_artifacts(
                    config,
                    source_set,
                    interactive_edt.as_ref().expect("interactive edt dsl"),
                )
            } else {
                let one_shot_edt = EdtDsl::new(
                    edt.clone(),
                    config.work_path.join("edt-workspace"),
                    utilities.runner_for(UtilityType::EdtCli),
                    context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
                );
                prepare_edt_external_artifacts(config, source_set, &one_shot_edt)
            };
            match export_result {
                Ok(descriptors) => {
                    match &edt_stage {
                        StepPlan::Execute { commit, .. } => {
                            if let Err(app_error) = commit_step_state(
                                source_set,
                                &edt_context,
                                &config.work_path,
                                commit,
                            ) {
                                let result = fail_from_source_set_index(
                                    started,
                                    steps,
                                    &ordered_source_sets,
                                    index,
                                    source_set,
                                    BuildMode::EdtExport,
                                    app_error.to_string(),
                                );
                                return Err(BuildExecutionFailure::with_payload(app_error, result));
                            }
                        }
                        StepPlan::Skip { .. } => {}
                    }
                    push_build_step(
                        &mut steps,
                        &source_set.name,
                        BuildMode::EdtExport,
                        true,
                        append_warnings(
                            format!(
                                "exported {} external artifact(s) to designer runtime",
                                descriptors.len()
                            ),
                            &[],
                        ),
                        export_started.elapsed().as_millis() as u64,
                    )
                }
                Err(error) => {
                    let result = fail_from_source_set_index(
                        started,
                        steps,
                        &ordered_source_sets,
                        index,
                        source_set,
                        BuildMode::EdtExport,
                        error.to_string(),
                    );
                    return Err(BuildExecutionFailure::with_payload(error, result));
                }
            }
            continue;
        }

        let edt_stage_skipped = matches!(&edt_stage, StepPlan::Skip { .. });

        match edt_stage {
            StepPlan::Skip { message, ok } => {
                push_build_step(
                    &mut steps,
                    &source_set.name,
                    BuildMode::Skipped,
                    ok,
                    message,
                    0,
                );
            }
            StepPlan::Execute {
                message: _,
                partial_paths: _,
                commit,
                mode: _,
            } => {
                let edt = match edt_binary.clone() {
                    Some(path) => path,
                    None => {
                        let location = match utilities.locate(UtilityType::EdtCli) {
                            Ok(location) => location,
                            Err(error) => {
                                let result = fail_from_source_set_index(
                                    started,
                                    steps,
                                    &ordered_source_sets,
                                    index,
                                    source_set,
                                    BuildMode::EdtExport,
                                    error.to_string(),
                                );
                                return Err(BuildExecutionFailure::with_payload(
                                    AppError::from(error),
                                    result,
                                ));
                            }
                        };
                        edt_binary = Some(location.path.clone());
                        location.path
                    }
                };

                if args.dry_run {
                    // Экспорт пишет снимок файлов конфигуратора, а следующий за ним шаг
                    // трогает базу; превью останавливается до обоих. Но сначала ищет и
                    // вторую утилиту: шаг обещает загрузку, и одобрить его, не зная, чем
                    // грузить, значит одобрить невыполнимое
                    // (`INV.CLI.PREVIEW-RETURNS-AFTER-TOOL-LOOKUP`).
                    let loader = match locate_designer_loader(
                        provider,
                        &mut utilities,
                        &mut designer_binary,
                        &mut ibcmd_binary,
                    ) {
                        Ok(path) => path,
                        Err(error) => {
                            let result = fail_from_source_set_index(
                                started,
                                steps,
                                &ordered_source_sets,
                                index,
                                source_set,
                                BuildMode::EdtExport,
                                error.to_string(),
                            );
                            return Err(BuildExecutionFailure::with_payload(error, result));
                        }
                    };
                    push_build_step(
                        &mut steps,
                        &source_set.name,
                        BuildMode::EdtExport,
                        true,
                        format!(
                            "would export '{}' to Designer files via {} and then load it via {}; planned, nothing dispatched",
                            source_set.name,
                            edt.display(),
                            loader.display()
                        ),
                        0,
                    );
                    continue;
                }

                let export_started = Instant::now();
                log_timeline_stage(
                    &source_set.name,
                    ProgressPhase::Convert,
                    "[EDT] Конвертация в файлы конфигуратора",
                    TimelineStageStatus::Running,
                );
                let export_result = if config.tools.edt_cli.interactive_mode {
                    if interactive_edt.is_none() {
                        interactive_edt = Some(
                            match EdtSessionManager::for_config(
                                config,
                                EdtSessionHostOptions::for_cli_command(config),
                            ) {
                                Ok(manager) => match EdtDsl::new_shared_session(
                                    edt.clone(),
                                    config.work_path.join("edt-workspace"),
                                    Arc::new(manager),
                                    Duration::from_millis(config.tools.edt_cli.startup_timeout_ms),
                                    Duration::from_millis(config.tools.edt_cli.command_timeout_ms),
                                    context.process_policy(
                                        InterruptionSafetyClass::GracefulThenKill,
                                        None,
                                    ),
                                ) {
                                    Ok(dsl) => dsl,
                                    Err(error) => {
                                        let app_error = AppError::from(error);
                                        let result = fail_from_source_set_index(
                                            started,
                                            steps,
                                            &ordered_source_sets,
                                            index,
                                            source_set,
                                            BuildMode::EdtExport,
                                            app_error.to_string(),
                                        );
                                        return Err(BuildExecutionFailure::with_payload(
                                            app_error, result,
                                        ));
                                    }
                                },
                                Err(error) => {
                                    let app_error = AppError::from(error);
                                    let result = fail_from_source_set_index(
                                        started,
                                        steps,
                                        &ordered_source_sets,
                                        index,
                                        source_set,
                                        BuildMode::EdtExport,
                                        app_error.to_string(),
                                    );
                                    return Err(BuildExecutionFailure::with_payload(
                                        app_error, result,
                                    ));
                                }
                            },
                        );
                    }
                    execute_edt_export_step(
                        context,
                        config,
                        interactive_edt.as_ref().expect("interactive edt dsl"),
                        source_set,
                        &edt_context,
                        // Снимок лежит под памятью выбранной базы: его путь называет
                        // контекст `designer-`.
                        designer_context.path(),
                        &format!("build-{index:02}"),
                    )
                } else {
                    let one_shot_edt = EdtDsl::new(
                        edt.clone(),
                        config.work_path.join("edt-workspace"),
                        utilities.runner_for(UtilityType::EdtCli),
                        context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
                    );
                    execute_edt_export_step(
                        context,
                        config,
                        &one_shot_edt,
                        source_set,
                        &edt_context,
                        // Снимок лежит под памятью выбранной базы: его путь называет
                        // контекст `designer-`.
                        designer_context.path(),
                        &format!("build-{index:02}"),
                    )
                };
                let export_warnings = match export_result {
                    Ok(warnings) => warnings,
                    Err(error) => {
                        let result = fail_from_source_set_index(
                            started,
                            steps,
                            &ordered_source_sets,
                            index,
                            source_set,
                            BuildMode::EdtExport,
                            error.to_string(),
                        );
                        return Err(BuildExecutionFailure::with_payload(error, result));
                    }
                };
                if let Err(app_error) =
                    commit_step_state(source_set, &edt_context, &config.work_path, &commit)
                {
                    let result = fail_from_source_set_index(
                        started,
                        steps,
                        &ordered_source_sets,
                        index,
                        source_set,
                        BuildMode::EdtExport,
                        app_error.to_string(),
                    );
                    return Err(BuildExecutionFailure::with_payload(app_error, result));
                }

                push_build_step(
                    &mut steps,
                    &source_set.name,
                    BuildMode::EdtExport,
                    true,
                    append_warnings("EDT export completed".to_owned(), &export_warnings),
                    export_started.elapsed().as_millis() as u64,
                );
            }
        }

        let designer_stage = match plan_generated_designer_load_step(
            source_set,
            &designer_context,
            args.load.is_whole(),
            edt_stage_skipped,
            &config.work_path,
            &mut || {
                crate::use_cases::interruption::pending_interruption_error(
                    context,
                    "during source analysis",
                )
                .is_some()
            },
        ) {
            Ok(plan) => plan,
            Err(error) => {
                let (error, message) = match change_detection_failure(&error, context) {
                    Ok(message) => (AppError::Runtime(message.clone()), message),
                    Err(error) => {
                        let message = error.to_string();
                        (error, message)
                    }
                };
                let result = fail_from_source_set_index(
                    started,
                    steps,
                    &ordered_source_sets,
                    index,
                    source_set,
                    BuildMode::Skipped,
                    message,
                );
                return Err(BuildExecutionFailure::with_payload(error, result));
            }
        };

        match designer_stage {
            StepPlan::Skip { message, ok } => {
                let settled = skip_or_apply_unapplied(
                    &mut steps,
                    context,
                    config,
                    args,
                    &designer_context,
                    &source_set.name,
                    provider,
                    message,
                    ok,
                    |record| {
                        locate_designer_loader(
                            provider,
                            &mut utilities,
                            &mut designer_binary,
                            &mut ibcmd_binary,
                        )
                        .and_then(|binary| {
                            apply_unapplied(context, &designer_context, record, &mut |op| {
                                match provider {
                                    Provider::Ibcmd => ibcmd_unapplied_op(
                                        context,
                                        config,
                                        &binary,
                                        utilities.runner_for(UtilityType::Ibcmd),
                                        source_set,
                                        op,
                                    ),
                                    Provider::Designer => designer_unapplied_op(
                                        context,
                                        config,
                                        &binary,
                                        utilities.runner_for(UtilityType::V8),
                                        source_set,
                                        index,
                                        op,
                                    ),
                                    other @ (Provider::Agent
                                    | Provider::IbcmdRs
                                    | Provider::Webinst) => {
                                        Err(crate::use_cases::unimplemented_provider(
                                            Operation::Build,
                                            other,
                                        ))
                                    }
                                }
                            })
                        })
                    },
                );
                if let Err(error) = settled {
                    let result = fail_from_source_set_index(
                        started,
                        steps,
                        &ordered_source_sets,
                        index,
                        source_set,
                        BuildMode::Skipped,
                        error.to_string(),
                    );
                    return Err(BuildExecutionFailure::with_payload(error, result));
                }
            }
            StepPlan::Execute {
                mode,
                message,
                partial_paths,
                commit,
            } => {
                let load_started = Instant::now();
                // Исполнитель загрузки ищется тем же помощником, что и в превью: иначе
                // два поиска разошлись бы, и превью одобряло бы то, чего запуск не может.
                let loader = match locate_designer_loader(
                    provider,
                    &mut utilities,
                    &mut designer_binary,
                    &mut ibcmd_binary,
                ) {
                    Ok(path) => path,
                    Err(error) => {
                        let result = fail_from_source_set_index(
                            started,
                            steps,
                            &ordered_source_sets,
                            index,
                            source_set,
                            mode.clone(),
                            error.to_string(),
                        );
                        return Err(BuildExecutionFailure::with_payload(error, result));
                    }
                };
                let load_result = match provider {
                    other @ (Provider::Agent | Provider::IbcmdRs | Provider::Webinst) => Err(
                        crate::use_cases::unimplemented_provider(Operation::Build, other),
                    ),
                    Provider::Designer => {
                        let designer = &loader;
                        // Загрузка сюда доходит и тогда, когда этап EDT пропущен: каталог
                        // файлов конфигуратора уже есть, а состояние Конфигуратора
                        // устарело. Превью останавливается здесь — дальше идёт запуск
                        // против базы и запись состояния.
                        if args.dry_run {
                            push_build_step(
                                &mut steps,
                                &source_set.name,
                                mode,
                                true,
                                format!("{message}; planned, {provider} not dispatched"),
                                0,
                            );
                            continue;
                        }
                        let runner = utilities.runner_for(UtilityType::V8);
                        let read = || {
                            read_designer_generation(
                                context, config, designer, runner, source_set, index,
                            )
                        };
                        guarded_load(
                            &gate,
                            &designer_context,
                            Provider::Designer,
                            read,
                            || {
                                execute_source_set_step(
                                    context,
                                    config,
                                    designer,
                                    runner,
                                    source_set,
                                    &designer_context,
                                    &designer_context,
                                    index,
                                    partial_paths.as_deref(),
                                    &commit,
                                    args.apply,
                                )
                            },
                            || {
                                commit_step_state(
                                    source_set,
                                    &designer_context,
                                    &config.work_path,
                                    &commit,
                                )
                            },
                        )
                    }
                    Provider::Ibcmd => {
                        let ibcmd = &loader;
                        if args.dry_run {
                            push_build_step(
                                &mut steps,
                                &source_set.name,
                                mode,
                                true,
                                format!("{message}; planned, {provider} not dispatched"),
                                0,
                            );
                            continue;
                        }
                        let runner = utilities.runner_for(UtilityType::Ibcmd);
                        let read =
                            || read_ibcmd_generation(context, config, ibcmd, runner, source_set);
                        guarded_load(
                            &gate,
                            &designer_context,
                            Provider::Ibcmd,
                            read,
                            || {
                                execute_source_set_step_ibcmd(
                                    context,
                                    config,
                                    ibcmd,
                                    runner,
                                    source_set,
                                    &designer_context,
                                    &designer_context,
                                    partial_paths.as_deref(),
                                    &commit,
                                    args.apply,
                                )
                            },
                            || {
                                commit_step_state(
                                    source_set,
                                    &designer_context,
                                    &config.work_path,
                                    &commit,
                                )
                            },
                        )
                    }
                };
                match load_result.and_then(|Loaded { warnings, apply }| {
                    settle_loaded(context, &source_set.name, warnings, apply)
                }) {
                    Ok((warnings, applied)) => push_loaded_step(
                        &mut steps,
                        &source_set.name,
                        mode,
                        applied,
                        append_warnings(message, &warnings),
                        load_started.elapsed().as_millis() as u64,
                    ),
                    Err(error) => {
                        let result = fail_from_source_set_index(
                            started,
                            steps,
                            &ordered_source_sets,
                            index,
                            source_set,
                            mode,
                            append_warnings(error.to_string(), &gate.failed_load_note()),
                        );
                        return Err(BuildExecutionFailure::with_payload(error, result));
                    }
                }
            }
        }
    }

    Ok(BuildResult {
        detection: Default::default(),
        provider: None,
        provider_dispatched: false,
        ok: true,
        steps,
        duration_ms: started.elapsed().as_millis() as u64,
        sessions: None,
    })
}

/// Загрузка набора под сверкой поколения: до неё — отказ, если база ушла вперёд записанного
/// сравнимым инструментом; после — запись поколения. Файл версий загрузка не пишет: копия
/// раннера устаревает на загруженное, и следующий `pull` его выгрузит (#646).
pub(super) fn guarded_load(
    gate: &GenerationGate<'_>,
    set: &SourceSetContext,
    tool: Provider,
    read: impl Fn() -> Result<Option<String>, AppError>,
    load: impl FnOnce() -> Result<Loaded, AppError>,
    commit: impl FnOnce() -> Result<(), AppError>,
) -> Result<Loaded, AppError> {
    guarded_load_then(gate, set, tool, read, |_, _| None, load, commit)
}

/// [`guarded_load`] с шагом `after` после удачной загрузки: он видит сверку перед ней и токен
/// поколения после неё и возвращает строку для ответа (свежий файл версий, #646).
fn guarded_load_then(
    gate: &GenerationGate<'_>,
    set: &SourceSetContext,
    tool: Provider,
    read: impl Fn() -> Result<Option<String>, AppError>,
    after: impl FnOnce(&BeforeLoad, Option<&str>) -> Option<String>,
    load: impl FnOnce() -> Result<Loaded, AppError>,
    commit: impl FnOnce() -> Result<(), AppError>,
) -> Result<Loaded, AppError> {
    let before = gate.before_load(set, tool, &read)?;
    let Loaded { warnings, apply } =
        load().inspect_err(|error| gate.after_failed_load(set, error))?;
    // Ответ, который ничего не решает, не спрашивается (#637).
    let read_after = if gate.generation_after_load_decides(set) {
        read()
    } else {
        Ok(None)
    };
    let (mut warnings, token) = generation_after_load(gate, set, tool, warnings, read_after)?;
    let recorded = gate.after_load(set, tool, token.as_deref(), apply.mark());
    warnings.extend(after(&before, token.as_deref()));
    warnings.extend(remember_unapplied(&apply, recorded, commit)?);
    Ok(Loaded { warnings, apply })
}

/// Со скольких загруженных файлов отправка по изменившемуся сразу получает свежий файл версий
/// (#646): один порог владельца для модулей, описаний объектов и форм. Замер на КА 8.3.27 с
/// живым агентом: запрос одной описи стоит отправке 1,6–1,7 с после модулей и 6–7 с после
/// описаний объектов, а `pull` после отправки до 300 модулей или описаний без него дороже лишь
/// на 0,8–1,7 с. Обратная выгрузка у дерева КА — около 5 мс на файл: с тысячи модулей она дороже
/// запроса, у тысячи описаний сравнима с ним. Раздел «Свежий файл
/// версий после крупной загрузки (#646)» в `references/1c/confirmed-runtime-measurements.md`.
const VERSION_FILE_REFRESH_FILES: usize = 1000;

/// Получает ли загрузка свежий файл версий: полная — всегда; по изменившемуся — только если
/// поколение перед ней совпало с записанным (иначе база могла нести правки, которых нет в
/// каталоге, и свежая опись заявила бы их выгруженными) и если без неё следующая выгрузка
/// пошла бы полной ([`partial_load::next_dump_goes_full`]) или загружено не меньше
/// [`VERSION_FILE_REFRESH_FILES`] файлов. Список — тот же, что грузит частичная загрузка.
fn refreshes_the_version_file(
    partial_paths: Option<&[PathBuf]>,
    source_root: &Path,
    before: &BeforeLoad,
) -> bool {
    let Some(paths) = partial_paths else {
        return true;
    };
    if *before != BeforeLoad::Matched {
        return false;
    }
    crate::change_detection::partial_load::next_dump_goes_full(paths, source_root)
        || paths.len() >= VERSION_FILE_REFRESH_FILES
}

/// Что удачная загрузка набора говорит шагу свежего файла версий.
struct VersionFileLoad<'a> {
    /// Набор — основная конфигурация: опись расширения отдельной выгрузкой не замерена.
    main_configuration: bool,
    /// Список частичной загрузки; `None` — полная.
    partial_paths: Option<&'a [PathBuf]>,
    /// Сверка поколения перед загрузкой.
    before: &'a BeforeLoad,
    /// Поколение после загрузки, если его спросили.
    token: Option<&'a str>,
}

/// Единственный шаг свежего файла версий после удачной загрузки и записи её поколения (#646),
/// общий для всех исполнителей: решает [`refreshes_the_version_file`], пишет
/// [`refresh_version_file`]. Строка — для ответа.
fn version_file_after_load(
    context: &ExecutionContext,
    config: &AppConfig,
    gate: &GenerationGate<'_>,
    set: &SourceSetContext,
    load: VersionFileLoad<'_>,
    dump_and_reread: impl FnOnce(&Path) -> Result<Option<String>, AppError>,
) -> Option<String> {
    if !load.main_configuration
        || !refreshes_the_version_file(load.partial_paths, set.path(), load.before)
    {
        return None;
    }
    let version = RunnerVersionFile::of(set)?;
    // Без журнала поколение после загрузки не спрашивается, и сказать нечего.
    let token = match load.token {
        Some(token) => token,
        None if gate.generation_after_load_decides(set) => {
            return Some(not_refreshed(
                set,
                "the configuration generation after the load is unknown",
            ))
        }
        None => return None,
    };
    if crate::use_cases::interruption::pending_interruption_error(
        context,
        "the version file after the load",
    )
    .is_some()
    {
        return Some(not_refreshed(set, "the command was cancelled"));
    }
    if version.removals_unknown() {
        return Some(not_refreshed(
            set,
            &format!(
                "the last pull did not learn which elements the infobase dropped, and a fresh description would forget them while their files stay in the directory; run {} first",
                context.advised_pull_force(set.name())
            ),
        ));
    }
    refresh_version_file(config, &version, token, dump_and_reread)
        .unwrap_or_else(|why| Some(not_refreshed(set, &why)))
}

/// Свежий файл версий (#646): `dump_and_reread` пишет одну опись во временный каталог под
/// `workPath/stage` и читает поколение снова. Поколение то же, что после загрузки (`token`), —
/// опись становится копией раннера (`Ok` с предупреждением, если её не записать); иначе, без
/// ответа или при сбое копия остаётся прежней, и `Err` называет причину. В каталог набора опись
/// не попадает. Загрузка уже прошла: сбой здесь её не отменяет. Временный каталог, оставленный
/// снятым процессом, убирает следующая выгрузка ([`STALE_VERSION_FILE_DIR_PREFIX`]).
fn refresh_version_file(
    config: &AppConfig,
    version: &RunnerVersionFile,
    token: &str,
    dump_and_reread: impl FnOnce(&Path) -> Result<Option<String>, AppError>,
) -> Result<Option<String>, String> {
    let stage = config.work_path.join("stage");
    let dir = std::fs::create_dir_all(&stage)
        .and_then(|()| {
            tempfile::Builder::new()
                .prefix(crate::use_cases::version_file::STALE_VERSION_FILE_DIR_PREFIX)
                .tempdir_in(&stage)
        })
        .map_err(|error| format!("failed to prepare '{}': {error}", stage.display()))?;
    let reread = dump_and_reread(dir.path()).map_err(|error| error.to_string())?;
    if reread.as_deref() != Some(token) {
        return Err(
            "the configuration generation did not confirm that the infobase stayed unchanged"
                .to_owned(),
        );
    }
    let staged = dir.path().join(VERSION_FILE_NAME);
    if !staged.is_file() {
        return Err(format!("the platform wrote no {VERSION_FILE_NAME}"));
    }
    Ok(version.record_from(&staged))
}

fn not_refreshed(set: &SourceSetContext, why: &str) -> String {
    format!(
        "the runner's copy of {VERSION_FILE_NAME} for source-set '{}' was not refreshed after the load: {why}; the next pull dumps the loaded objects again",
        set.name()
    )
}

/// Загрузка без применения помнится памятью исходников только вместе с записью поколения
/// `applied: false`, которая действительно легла в журнал: без неё отправка, которой нечего
/// грузить, не узнала бы о непринятом и не применила бы его
/// (`INV.USE-CASES.A-LOAD-WITHOUT-APPLY-IS-REMEMBERED-AS-UNAPPLIED`). Записи нет — ответа не
/// было, запись не удалась или журнала нет — память не фиксируется, и следующая отправка
/// загрузит и применит набор снова. Применённую загрузку фиксирует сам шаг. Строки — для
/// ответа.
fn remember_unapplied(
    apply: &AfterLoad,
    recorded: crate::use_cases::exchange_guard::LoadRecord,
    commit: impl FnOnce() -> Result<(), AppError>,
) -> Result<Vec<String>, AppError> {
    use crate::use_cases::exchange_guard::LoadRecord;
    if apply.applied() {
        return Ok(recorded.into_note().into_iter().collect());
    }
    match recorded {
        LoadRecord::Recorded => {
            commit()?;
            Ok(Vec::new())
        }
        LoadRecord::Erased(_) | LoadRecord::NotErased(_) | LoadRecord::NoLedger => {
            let mut notes: Vec<String> = recorded.into_note().into_iter().collect();
            notes.push(
                "the load without apply is not remembered: no generation record could be kept for it, so the next push loads the set again and applies it".to_owned(),
            );
            Ok(notes)
        }
    }
}

/// Шаг удачной загрузки по исходу применения: применено — удача; `--no-apply` — удача,
/// которая называет выход `apply`; применение отказало — отказ шага, который говорит, что
/// загрузка сохранена, и называет тот же выход
/// (`INV.USE-CASES.A-PUSH-WHOSE-APPLY-FAILED-KEEPS-THE-LOAD`). Второе — признак применения.
pub(super) fn settle_loaded(
    context: &ExecutionContext,
    set: &str,
    mut warnings: Vec<String>,
    apply: AfterLoad,
) -> Result<(Vec<String>, bool), AppError> {
    match apply {
        AfterLoad::Applied => Ok((warnings, true)),
        AfterLoad::Deferred => {
            warnings.push(format!(
                "loaded without apply: the database configuration is unchanged until {}",
                context.advised_command(&format!("apply {}", shell_word(set)))
            ));
            Ok((warnings, false))
        }
        AfterLoad::Failed(error) => {
            let error = if warnings.is_empty() {
                error
            } else {
                error.with_context(warnings.join("; "))
            };
            Err(crate::use_cases::apply::load_kept_unapplied(
                context, set, error,
            ))
        }
    }
}

/// Непринятая запись набора, которую видит отправка без изменений.
enum Unapplied {
    /// Запись инструмента, чей токен сравним с этим (у основной конфигурации файловой базы —
    /// любого из агента, Конфигуратора и `ibcmd`, #559): её применяет отправка, если поколение
    /// базы ей равно.
    Ours(crate::use_cases::agent_session::GenerationRecord),
    /// Запись другого инструмента: его токен с этим несравним, и отправка только называет
    /// непринятое и выход `apply`.
    OtherTool(String),
}

/// Непринятая запись набора по плану отправки; у `--no-apply` применять нечего. Превью запись
/// читает — это файл, — а исполнителя не зовёт.
fn unapplied_record(
    context: &ExecutionContext,
    config: &AppConfig,
    args: &BuildArgs,
    set: &SourceSetContext,
    tool: Provider,
) -> Option<Unapplied> {
    if args.apply == ApplyPolicy::Defer {
        return None;
    }
    let record = crate::use_cases::exchange_guard::recorded_generation(set)
        .filter(|record| !record.applied)?;
    let scope = crate::use_cases::exchange_guard::token_scope(config, set);
    Some(if scope.compares(record.tool, tool) {
        Unapplied::Ours(record)
    } else {
        Unapplied::OtherTool(format!(
            "source-set '{}' was loaded without apply earlier by {}, and this push reads the generation with {tool}, so it does not apply it; apply it with {}",
            set.name(),
            record.tool,
            advised_apply(context, set.name())
        ))
    })
}

fn advised_apply(context: &ExecutionContext, set: &str) -> String {
    context.advised_command(&format!("apply {}", shell_word(set)))
}

/// Что спрашивают у исполнителя при применении непринятого: поколение или само применение.
pub(super) enum UnappliedOp<'d> {
    Read,
    Apply(&'d mut crate::use_cases::interruption::Deferrals),
}

/// Исход попытки отправки без изменений применить своё непринятое.
enum UnappliedStep {
    /// Применено; строки — предупреждения шага.
    Applied(Vec<String>),
    /// База ушла от записи: признаку не верят, шаг остаётся пропуском и называет выход.
    NotTrusted(String),
}

/// Применение непринятого набора отправкой, которой нечего грузить
/// (`INV.USE-CASES.A-PUSH-WITH-NOTHING-TO-LOAD-APPLIES-ITS-OWN-UNAPPLIED`): только если
/// поколение базы равно записи. После применения запись переносится на ответ того же
/// инструмента, без ответа стирается; отмена, пришедшая после применения, называется.
/// Отказ применения называет выход `apply`.
fn apply_unapplied(
    context: &ExecutionContext,
    set: &SourceSetContext,
    record: &crate::use_cases::agent_session::GenerationRecord,
    run: &mut dyn FnMut(UnappliedOp<'_>) -> Result<Option<String>, AppError>,
) -> Result<UnappliedStep, AppError> {
    if run(UnappliedOp::Read)?.as_deref() != Some(record.token.as_str()) {
        return Ok(UnappliedStep::NotTrusted(format!(
            "source-set '{}' was loaded without apply earlier, and the infobase moved away from that record, so this push does not apply it; apply it with {}",
            set.name(),
            advised_apply(context, set.name())
        )));
    }
    let ((), deferrals) =
        collecting_deferrals(|deferrals| run(UnappliedOp::Apply(deferrals)).map(|_| ())).map_err(
            |error| crate::use_cases::apply::load_kept_unapplied(context, set.name(), error),
        )?;
    let (_, note) = crate::use_cases::generation_record::carry_record(
        crate::use_cases::generation_record::RecordStep::Apply,
        set,
        record,
        run(UnappliedOp::Read),
    )?;
    let cancelled = crate::use_cases::apply::cancellation_after(context, &deferrals, "apply");
    Ok(UnappliedStep::Applied(
        deferrals.into_iter().chain(note).chain(cancelled).collect(),
    ))
}

/// Шаг набора, которому нечего грузить: пропуск, а если запись этого же инструмента
/// помечена «не применено» — применение её непринятого
/// (`INV.USE-CASES.A-PUSH-WITH-NOTHING-TO-LOAD-APPLIES-ITS-OWN-UNAPPLIED`). Одно место у всех
/// исполнителей: каждый передаёт только `apply` — найти себя и применить через
/// [`apply_unapplied`]. Превью исполнителя не зовёт и только называет предстоящее
/// применение.
#[allow(clippy::too_many_arguments)]
fn skip_or_apply_unapplied(
    steps: &mut Vec<crate::domain::build::BuildStep>,
    context: &ExecutionContext,
    config: &AppConfig,
    args: &BuildArgs,
    set: &SourceSetContext,
    name: &str,
    tool: Provider,
    message: String,
    ok: bool,
    apply: impl FnOnce(
        &crate::use_cases::agent_session::GenerationRecord,
    ) -> Result<UnappliedStep, AppError>,
) -> Result<(), AppError> {
    let unapplied = if ok {
        unapplied_record(context, config, args, set, tool)
    } else {
        None
    };
    let record = match unapplied {
        Some(Unapplied::Ours(record)) if args.dry_run => {
            push_build_step(
                steps,
                name,
                BuildMode::Skipped,
                ok,
                append_warnings(
                    message,
                    &[format!(
                        "would apply the configuration loaded earlier without apply via {}, if the infobase generation still matches the record; planned, nothing dispatched",
                        record.tool
                    )],
                ),
                0,
            );
            return Ok(());
        }
        Some(Unapplied::Ours(record)) => record,
        Some(Unapplied::OtherTool(note)) => {
            push_build_step(
                steps,
                name,
                BuildMode::Skipped,
                ok,
                append_warnings(message, &[note]),
                0,
            );
            return Ok(());
        }
        None => {
            push_build_step(steps, name, BuildMode::Skipped, ok, message, 0);
            return Ok(());
        }
    };
    let started = Instant::now();
    settle_unapplied(steps, name, message, apply(&record), started)
}

/// Шаг отправки без изменений после попытки применить непринятое.
fn settle_unapplied(
    steps: &mut Vec<crate::domain::build::BuildStep>,
    set: &str,
    message: String,
    applied: Result<UnappliedStep, AppError>,
    started: Instant,
) -> Result<(), AppError> {
    match applied? {
        UnappliedStep::NotTrusted(warning) => push_build_step(
            steps,
            set,
            BuildMode::Skipped,
            true,
            append_warnings(message, &[warning]),
            0,
        ),
        UnappliedStep::Applied(warnings) => push_loaded_step(
            steps,
            set,
            BuildMode::Skipped,
            true,
            append_warnings(
                format!("{message}; applied the configuration loaded earlier without apply"),
                &warnings,
            ),
            started.elapsed().as_millis() as u64,
        ),
    }
    Ok(())
}

/// Чтение поколения и применение у `ibcmd` для отправки без изменений.
fn ibcmd_unapplied_op(
    context: &ExecutionContext,
    config: &AppConfig,
    binary: &Path,
    runner: &dyn crate::platform::process::ProcessRunner,
    source_set: &SourceSetConfig,
    op: UnappliedOp<'_>,
) -> Result<Option<String>, AppError> {
    match op {
        UnappliedOp::Read => read_ibcmd_generation(context, config, binary, runner, source_set),
        UnappliedOp::Apply(deferrals) => crate::use_cases::apply::act::apply(
            context,
            config,
            crate::use_cases::apply::act::Applier::Ibcmd { binary, runner },
            &apply_subject(source_set),
            deferrals,
        )
        .map(|()| None),
    }
}

/// Чтение поколения и применение у Конфигуратора для отправки без изменений.
fn designer_unapplied_op(
    context: &ExecutionContext,
    config: &AppConfig,
    binary: &Path,
    runner: &dyn crate::platform::process::ProcessRunner,
    source_set: &SourceSetConfig,
    step_index: usize,
    op: UnappliedOp<'_>,
) -> Result<Option<String>, AppError> {
    match op {
        UnappliedOp::Read => {
            read_designer_generation(context, config, binary, runner, source_set, step_index)
        }
        UnappliedOp::Apply(deferrals) => crate::use_cases::apply::act::apply(
            context,
            config,
            crate::use_cases::apply::act::Applier::Designer {
                binary,
                runner,
                log_file: designer_log_file(config, &source_set.name, step_index, "update")?,
            },
            &apply_subject(source_set),
            deferrals,
        )
        .map(|()| None),
    }
}

/// Поколение после удачной загрузки — одинаково у всех исполнителей. Отмена, отложенная
/// загрузкой, останавливает шаг здесь и называется вместе с тем, что загрузка отложила;
/// запись о поколении тогда стирается — прежний токен описывает не ту базу, — и это тоже
/// называется. Прочий сбой чтения — отсутствие ответа.
pub(super) fn generation_after_load(
    gate: &GenerationGate<'_>,
    set: &SourceSetContext,
    tool: Provider,
    mut warnings: Vec<String>,
    read: Result<Option<String>, AppError>,
) -> Result<(Vec<String>, Option<String>), AppError> {
    match read {
        Ok(token) => Ok((warnings, token)),
        Err(error) if error.cancellation().is_some() => {
            warnings.extend(
                gate.after_load(
                    set,
                    tool,
                    None,
                    crate::use_cases::agent_session::ApplyMark::Applied,
                )
                .into_note(),
            );
            Err(if warnings.is_empty() {
                error
            } else {
                error.with_context(warnings.join("; "))
            })
        }
        Err(error) => {
            debug!(%error, "the generation after the load is not known");
            Ok((warnings, None))
        }
    }
}

/// Пойдёт ли набор в базу по плану команды: внешние обработки не грузятся, полная загрузка
/// грузит всё, а по изменившемуся — всё, кроме набора без изменений.
fn loads_into_the_base(
    source_set: &SourceSetConfig,
    load: PushMode,
    analysis_by_name: Option<&AnalysisByName>,
) -> bool {
    !source_set.purpose.is_external()
        && (load.is_whole()
            || !matches!(
                analysis_by_name.and_then(|analysis| analysis.get(&source_set.name)),
                Some(Ok(analyzer::AnalysisOutcome::NoChanges))
            ))
}

/// Наборы, которые по плану команды пойдут в базу: их поколение сверяется до первой загрузки
/// (`GenerationGate::check_early`). У превью их нет: оно не грузит.
fn sets_to_load<'i>(
    inventory: &'i SourceSetInventory,
    ordered_source_sets: &[&SourceSetConfig],
    args: &BuildArgs,
    loads: impl Fn(&SourceSetConfig) -> bool,
) -> Vec<&'i SourceSetContext> {
    if args.dry_run {
        return Vec::new();
    }
    ordered_source_sets
        .iter()
        .filter(|set| loads(set))
        .filter_map(|set| inventory.designer_context(&set.name))
        .collect()
}

/// Набор по имени и его место в порядке команды.
fn source_set_named<'a>(
    ordered_source_sets: &[&'a SourceSetConfig],
    name: &str,
) -> Option<(usize, &'a SourceSetConfig)> {
    ordered_source_sets
        .iter()
        .enumerate()
        .find(|(_, set)| set.name == name)
        .map(|(index, set)| (index, *set))
}

/// Ответ команды, которой сверка заранее отказала: набор назван отказавшим, остальные не
/// тронуты.
fn refused_before_the_loads(
    started: Instant,
    ordered_source_sets: &[&SourceSetConfig],
    refused: &str,
    error: AppError,
) -> BuildExecutionFailure {
    let Some((_, source_set)) = source_set_named(ordered_source_sets, refused) else {
        return BuildExecutionFailure::without_payload(error);
    };
    let remaining = std::iter::once(source_set)
        .chain(
            ordered_source_sets
                .iter()
                .copied()
                .filter(|other| other.name != source_set.name),
        )
        .collect();
    let result = fail_with_remaining_steps(
        started,
        Vec::new(),
        remaining,
        source_set,
        BuildMode::Skipped,
        error.to_string(),
    );
    BuildExecutionFailure::with_payload(error, result)
}

/// Копия Конфигуратора набора EDT, чей этап EDT пропущен, всё равно пойдёт в базу, если
/// изменилась сама копия — так бывает после оборванного прогона. Сбой анализа — «пойдёт»:
/// лишняя сверка дешевле пропущенной.
fn generated_copy_loads(
    execution: &ExecutionContext,
    source_set: &SourceSetConfig,
    inventory: &SourceSetInventory,
    work_path: &Path,
) -> bool {
    !source_set.purpose.is_external()
        && inventory
            .designer_context(&source_set.name)
            .is_some_and(|copy| {
                copy.path().exists()
                    && !matches!(
                        analyzer::analyze_context(copy, work_path, &mut || {
                            crate::use_cases::interruption::pending_interruption_error(
                                execution,
                                "during generated-source analysis",
                            )
                            .is_some()
                        })
                        .outcome,
                        Ok(analyzer::AnalysisOutcome::NoChanges)
                    )
            })
}

#[cfg(test)]
mod version_refresh_tests {
    use super::{refreshes_the_version_file, VERSION_FILE_REFRESH_FILES};
    use crate::use_cases::exchange_guard::BeforeLoad;
    use std::path::{Path, PathBuf};

    fn files(root: &Path, name: &str, count: usize) -> Vec<PathBuf> {
        (0..count)
            .map(|index| root.join(format!("CommonModules/M{index}/Ext/{name}")))
            .collect()
    }

    /// Полная загрузка получает свежий файл версий всегда; по изменившемуся — только при
    /// совпавшем перед загрузкой поколении и с порога числа файлов (#646).
    #[test]
    fn a_full_or_a_large_matched_load_refreshes_the_version_file() {
        let root = Path::new("/sources");
        let large = files(root, "Module.bsl", VERSION_FILE_REFRESH_FILES);
        let small = files(root, "Module.bsl", VERSION_FILE_REFRESH_FILES - 1);
        assert!(refreshes_the_version_file(
            None,
            root,
            &BeforeLoad::Unchecked
        ));
        assert!(refreshes_the_version_file(
            Some(&large),
            root,
            &BeforeLoad::Matched
        ));
        assert!(!refreshes_the_version_file(
            Some(&small),
            root,
            &BeforeLoad::Matched
        ));
        assert!(!refreshes_the_version_file(
            Some(&large),
            root,
            &BeforeLoad::Unchecked
        ));
    }

    /// Описания объектов и модули считаются одной мерой: отдельного веса нет.
    #[test]
    fn modules_and_descriptions_count_alike() {
        let root = Path::new("/sources");
        let half = VERSION_FILE_REFRESH_FILES / 2;
        let mut mixed = files(root, "Module.bsl", half);
        mixed.extend(
            (0..VERSION_FILE_REFRESH_FILES - half)
                .map(|index| root.join(format!("Catalogs/C{index}.xml"))),
        );
        assert!(refreshes_the_version_file(
            Some(&mixed),
            root,
            &BeforeLoad::Matched
        ));
        mixed.pop();
        assert!(!refreshes_the_version_file(
            Some(&mixed),
            root,
            &BeforeLoad::Matched
        ));
    }

    /// Изменение, после которого выгрузка по прежней описи шла бы полной, освежает опись при
    /// любом числе файлов — но только при совпавшем поколении.
    #[test]
    fn a_change_the_platform_dumps_in_full_refreshes_the_version_file() {
        let root = Path::new("/sources");
        for path in [
            "Configuration.xml",
            "Catalogs/X/Ext/Predefined.xml",
            "BusinessProcesses/P/Ext/Flowchart.xml",
        ] {
            let list = [root.join(path)];
            assert!(
                refreshes_the_version_file(Some(&list), root, &BeforeLoad::Matched),
                "{path}"
            );
            assert!(
                !refreshes_the_version_file(Some(&list), root, &BeforeLoad::Unchecked),
                "{path}"
            );
        }
    }
}
