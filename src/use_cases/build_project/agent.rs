//! Сборка через агентский shell Конфигуратора: одна сессия на команду.
//!
//! Загрузка исходников и обновление конфигурации базы — два шага одного разговора,
//! а не два процесса; наборы исходников идут друг за другом в той же сессии. Агент
//! читает только внутри своего `AgentBaseDir`, поэтому каталог набора выставляется
//! ему символической ссылкой. Поколение конфигурации у агента спрашивает сверка
//! координатора — до загрузки и после неё — тем же разговором.

use super::coordinator::SourceSetLoader;
use super::helpers::apply_subject;
use super::*;
use crate::platform::agent::WaitPolicy;
use crate::platform::locator::UtilityLocation;
use crate::use_cases::agent_session::{
    argument, connect, generation_id, run_critical, run_id, stage_dir, stage_dir_partially, tidy,
    transcript_log, unstage, wait_policy, write_bytes, AgentHandle, Exchange,
};
use crate::use_cases::interruption::Deferrals;

pub(super) struct AgentLoader {
    utilities: PlatformUtilities,
    /// Управляемому агенту нужна платформа на этой машине; чужому — ничего.
    managed: bool,
    location: Option<UtilityLocation>,
    /// Сессия и её политика ожидания живут вместе: политика несёт абсолютный срок
    /// команды, и сессия без неё означала бы ожидание без срока.
    session: Option<(AgentHandle, WaitPolicy)>,
    run: String,
}

impl AgentLoader {
    pub(super) fn new(config: &AppConfig) -> Self {
        Self {
            utilities: PlatformUtilities::from_config(config),
            // Платформа нужна только управляемому агенту; чужой агент и шлюз
            // автономного сервера поднимает не раннер.
            managed: config.infobase.standalone.is_none()
                && !matches!(
                    config.tools.designer_agent.mode(),
                    Ok(crate::config::model::DesignerAgentMode::Attached { .. })
                ),
            location: None,
            session: None,
            run: run_id(),
        }
    }

    /// Сессия открывается перед первой настоящей загрузкой и живёт до конца команды.
    fn handle(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
    ) -> Result<(&mut AgentHandle, WaitPolicy), AppError> {
        if self.session.is_none() {
            let wait = wait_policy(context);
            let transcript = transcript_log(config, "build")?;
            log_timeline_stage(
                "agent",
                "session",
                "[агент] opening the Designer agent session",
                TimelineStageStatus::Running,
            );
            let handle = connect(
                context,
                config,
                &mut self.utilities,
                self.location.as_ref().map(|found| found.path.as_path()),
                transcript,
                &wait,
            )?;
            self.session = Some((handle, wait));
        }
        let (handle, wait) = self
            .session
            .as_mut()
            .expect("agent session was just opened");
        Ok((handle, wait.clone()))
    }
}

impl SourceSetLoader for AgentLoader {
    fn locate(&mut self) -> Result<(), AppError> {
        if self.managed && self.location.is_none() {
            self.location = Some(self.utilities.locate(UtilityType::V8)?);
        }
        Ok(())
    }

    fn tool(&self) -> Provider {
        Provider::Agent
    }

    /// Поколение спрашивается в той же сессии, что и загрузка: она открывается здесь, если
    /// её ещё нет. После отмены сессия команд запроса не отдаёт и отвечает отменой.
    fn read_generation(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        source_set: &SourceSetConfig,
        _step_index: usize,
    ) -> Result<Option<String>, AppError> {
        let (handle, wait) = self.handle(context, config)?;
        generation_id(handle.session(), extension_name(source_set), &wait).map(Some)
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
        // Всё, что идёт после отложенной отмены, — провал следующей команды, безопасная
        // точка, фиксация состояния, — выходит через учёт, который её называет.
        // Версия формата сверяется до сессии: формат новее платформы — отказ. Версию
        // платформы раннер знает только у своего агента.
        let platform = self.location.as_ref().and_then(|location| {
            crate::platform::locator::platform_version_of(location.utility, &location.path)
        });
        let format_notice = crate::use_cases::version_file::check_load_format(
            &config.work_path,
            source_context,
            platform.as_ref(),
        )?;
        collecting_deferrals(|deferrals| {
            if let Some(error) = interruption_before_safe_point(
                context,
                format!("build load for source-set '{}'", source_set.name),
            ) {
                return Err(error);
            }
            let run = self.run.clone();
            let (handle, wait) = self.handle(context, config)?;
            let exchange = handle.exchange(config)?;
            let relative = format!("build/{run}/{step_index:02}-{}", source_set.name);
            // Недоставленные исходники (канал отверг запись) не оставляют на стороне точки
            // входа и половины каталога.
            let staged = match partial_paths {
                Some(paths) => {
                    stage_dir_partially(handle, &exchange, &relative, source_context.path(), paths)
                }
                None => stage_dir(handle, &exchange, &relative, source_context.path()),
            };
            let exposed = match staged {
                Ok(exposed) => exposed,
                Err(error) => {
                    unstage(handle, &exchange, &relative);
                    return Err(error);
                }
            };
            let extension = extension_name(source_set);

            let outcome = load_and_update(
                context,
                config,
                handle,
                &wait,
                &exchange,
                &exposed,
                source_set,
                source_context,
                extension,
                partial_paths,
                apply,
                deferrals,
            );
            unstage(handle, &exchange, &exposed);
            let apply = outcome?;

            if apply.applied() {
                commit_step_state(source_set, source_context, &config.work_path, commit)?;
            }
            Ok(apply)
        })
        .map(|(apply, mut warnings)| {
            warnings.extend(format_notice);
            Loaded { warnings, apply }
        })
    }

    fn apply_only(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
        source_set: &SourceSetConfig,
        _step_index: usize,
        deferrals: &mut Deferrals,
    ) -> Result<(), AppError> {
        let (handle, wait) = self.handle(context, config)?;
        crate::use_cases::apply::act::apply(
            context,
            config,
            crate::use_cases::apply::act::Applier::Agent {
                handle,
                wait: &wait,
            },
            &apply_subject(source_set),
            deferrals,
        )
    }

    fn finish(&mut self) {
        if let Some((handle, wait)) = self.session.take() {
            handle.finish(&wait);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn load_and_update(
    context: &ExecutionContext,
    config: &AppConfig,
    handle: &mut AgentHandle,
    wait: &WaitPolicy,
    exchange: &Exchange,
    exposed: &str,
    source_set: &SourceSetConfig,
    source_context: &SourceSetContext,
    extension: Option<&str>,
    partial_paths: Option<&[PathBuf]>,
    apply: ApplyPolicy,
    deferrals: &mut Deferrals,
) -> Result<AfterLoad, AppError> {
    // Обе команды меняют базу: загрузка переписывает конфигурацию, а `update-db-cfg`
    // перестраивает таблицы; загрузка идёт через `run_critical`, применение — через
    // владельца акта (`apply::act`), который зовёт тот же `run_critical`.
    let mut load = format!(
        "config load-config-from-files --dir={} --update-config-dump-info",
        argument(exposed)
    );
    if let Some(paths) = partial_paths {
        // Список относительных путей — тот же файл, что для Конфигуратора, только
        // лежит в каталоге пользователя агента; пути в нём — относительно каталога
        // загрузки.
        let list_relative = format!("{exposed}.list.txt");
        let list =
            partial_load::list_file_bytes(paths, source_context.path()).map_err(|error| {
                AppError::Runtime(format!("failed to write partial load list: {error}"))
            })?;
        write_bytes(handle, exchange, &list_relative, &list)?;
        load.push_str(&format!(
            " --partial --list-file={}",
            argument(&list_relative)
        ));
        log_timeline_stage(
            &source_set.name,
            &build_mode_label(&BuildMode::Partial {
                file_count: paths.len(),
            }),
            "[агент] Загрузка изменений в базу",
            TimelineStageStatus::Running,
        );
    } else {
        log_timeline_stage(
            &source_set.name,
            "full",
            "[агент] Загрузка в базу",
            TimelineStageStatus::Running,
        );
    }
    if let Some(extension) = extension {
        load.push_str(&format!(" --extension={}", argument(extension)));
    }
    let loaded = run_critical(handle, "load", &load, wait, deferrals);
    if partial_paths.is_some() {
        tidy(handle, exchange, &format!("{exposed}.list.txt"));
    }
    loaded?;

    if apply == ApplyPolicy::Defer {
        return Ok(AfterLoad::Deferred);
    }
    Ok(
        match crate::use_cases::apply::act::apply(
            context,
            config,
            crate::use_cases::apply::act::Applier::Agent { handle, wait },
            &apply_subject(source_set),
            deferrals,
        ) {
            Ok(()) => AfterLoad::Applied,
            Err(error) => AfterLoad::Failed(error),
        },
    )
}
