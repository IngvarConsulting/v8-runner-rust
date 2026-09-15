//! Сборка через агентский shell Конфигуратора: одна сессия на команду.
//!
//! Загрузка исходников и обновление конфигурации базы — два шага одного разговора,
//! а не два процесса; наборы исходников идут друг за другом в той же сессии. Агент
//! читает только внутри своего `AgentBaseDir`, поэтому каталог набора выставляется
//! ему символической ссылкой. После удачного шага у агента спрашивают поколение
//! конфигурации и записывают его в учёт: следующая выгрузка сравнит и, если
//! ничего не менялось, не станет ничего делать.

use super::coordinator::SourceSetLoader;
use super::*;
use crate::platform::agent::WaitPolicy;
use crate::platform::locator::UtilityLocation;
use crate::use_cases::agent_session::{
    argument, connect, expose_dir, generation_id, map_agent_error, run_id, transcript_log,
    wait_policy, withdraw_dir, AgentHandle, GenerationLedger,
};

pub(super) struct AgentLoader {
    utilities: PlatformUtilities,
    /// Управляемому агенту нужна платформа на этой машине; чужому — ничего.
    managed: bool,
    location: Option<UtilityLocation>,
    handle: Option<AgentHandle>,
    wait: Option<WaitPolicy>,
    run: String,
}

impl AgentLoader {
    pub(super) fn new(config: &AppConfig) -> Self {
        Self {
            utilities: PlatformUtilities::from_config(config),
            managed: !matches!(
                config.tools.designer_agent.mode(),
                Ok(crate::config::model::DesignerAgentMode::Attached { .. })
            ),
            location: None,
            handle: None,
            wait: None,
            run: run_id(),
        }
    }

    /// Сессия открывается перед первой настоящей загрузкой и живёт до конца команды.
    fn handle(
        &mut self,
        context: &ExecutionContext,
        config: &AppConfig,
    ) -> Result<(&mut AgentHandle, WaitPolicy), AppError> {
        if self.handle.is_none() {
            let wait = wait_policy(context);
            let transcript = transcript_log(config, "build")?;
            log_timeline_stage(
                "agent",
                "session",
                "[агент] opening the Designer agent session",
                TimelineStageStatus::Running,
            );
            let handle = connect(
                config,
                &mut self.utilities,
                self.location.as_ref(),
                transcript,
                &wait,
            )?;
            self.wait = Some(wait);
            self.handle = Some(handle);
        }
        let wait = handle_wait(&self.wait);
        Ok((self.handle.as_mut().expect("handle was just opened"), wait))
    }
}

impl SourceSetLoader for AgentLoader {
    fn locate(&mut self) -> Result<(), AppError> {
        if self.managed && self.location.is_none() {
            self.location = Some(self.utilities.locate(UtilityType::V8)?);
        }
        Ok(())
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
    ) -> Result<Vec<String>, AppError> {
        if let Some(error) = interruption_before_safe_point(
            context,
            format!("build load for source-set '{}'", source_set.name),
        ) {
            return Err(error);
        }
        let run = self.run.clone();
        let (handle, wait) = self.handle(context, config)?;
        let user_dir = handle.user_dir(config)?;
        let exposed = expose_dir(
            &user_dir,
            &format!("build/{run}/{step_index:02}-{}", source_set.name),
            source_context.path(),
        )?;
        let extension = extension_name(source_set);

        let outcome = load_and_update(
            context,
            handle,
            &wait,
            &user_dir,
            &exposed,
            source_set,
            source_context,
            extension,
            partial_paths,
        );
        withdraw_dir(&user_dir, &exposed);
        outcome?;

        commit_step_state(source_set, source_context, &config.work_path, commit)?;

        // Поколение записывается после удачной загрузки: следующая выгрузка сравнит его
        // и не станет выгружать то, что не менялось.
        let token = generation_id(handle.session(), extension, &wait)?;
        GenerationLedger::new(config).record(&source_set.name, &token, "build")?;
        Ok(Vec::new())
    }

    fn finish(&mut self) {
        if let Some(handle) = self.handle.take() {
            let wait = handle_wait(&self.wait);
            handle.finish(&wait);
        }
    }
}

fn handle_wait(wait: &Option<WaitPolicy>) -> WaitPolicy {
    wait.clone().unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn load_and_update(
    context: &ExecutionContext,
    handle: &mut AgentHandle,
    wait: &WaitPolicy,
    user_dir: &Path,
    exposed: &str,
    source_set: &SourceSetConfig,
    source_context: &SourceSetContext,
    extension: Option<&str>,
    partial_paths: Option<&[PathBuf]>,
) -> Result<(), AppError> {
    let mut load = format!(
        "config load-config-from-files --dir={} --update-config-dump-info",
        argument(exposed)
    );
    if let Some(paths) = partial_paths {
        // Список относительных путей — тот же файл, что для Конфигуратора, только
        // лежит в каталоге пользователя агента; пути в нём — относительно каталога
        // загрузки.
        let list_relative = format!("{exposed}.list.txt");
        let list_path = user_dir.join(&list_relative);
        partial_load::write_list_file(paths, source_context.path(), &list_path).map_err(
            |error| AppError::Runtime(format!("failed to write partial load list: {error}")),
        )?;
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
    run_command(handle, &load, wait)?;

    if let Some(error) = interruption_before_safe_point(
        context,
        format!("update_db_cfg for source-set '{}'", source_set.name),
    ) {
        return Err(error);
    }
    log_timeline_stage(
        &source_set.name,
        "update_db_cfg",
        "[агент] Применение изменений",
        TimelineStageStatus::Running,
    );
    let mut update = String::from("config update-db-cfg");
    if let Some(extension) = extension {
        update.push_str(&format!(" --extension={}", argument(extension)));
    }
    run_command(handle, &update, wait)?;
    Ok(())
}

fn run_command(handle: &mut AgentHandle, command: &str, wait: &WaitPolicy) -> Result<(), AppError> {
    let reply = handle
        .session()
        .run(command, wait)
        .map_err(map_agent_error)?;
    reply.outcome().map_err(map_agent_error)?;
    Ok(())
}
