//! Выгрузка через агентский shell Конфигуратора.
//!
//! Агент пишет только внутрь своего `AgentBaseDir`, поэтому выгрузка идёт в две
//! стадии: агент кладёт файлы в свой пользовательский каталог, раннер переносит их в
//! цель той же публикацией, что и у пакетного Конфигуратора. Инкрементальный режим у
//! агента сводится к полному: обновлять чужой каталог агент не может, а полная
//! выгрузка даёт то же дерево.

use super::*;
use crate::config::model::DesignerAgentMode;
use crate::platform::agent::{
    AgentEndpoint, AgentError, AgentLaunch, AgentSession, AgentSessionRequest, ManagedAgent,
    WaitPolicy,
};
use crate::platform::process::ProcessResult;
use crate::support::fs::{copy_dir_recursively, move_dir};
use crate::support::temp::platform_logs_dir;

/// Открытая точка входа: свой процесс с сессией или только сессия к чужому.
enum AgentHandle {
    Managed(ManagedAgent),
    Attached {
        session: AgentSession,
        base_dir: PathBuf,
    },
}

impl AgentHandle {
    fn session(&mut self) -> &mut AgentSession {
        match self {
            Self::Managed(agent) => agent.session(),
            Self::Attached { session, .. } => session,
        }
    }

    fn base_dir(&self) -> &Path {
        match self {
            Self::Managed(agent) => agent.base_dir(),
            Self::Attached { base_dir, .. } => base_dir,
        }
    }

    fn finish(self, wait: &WaitPolicy) {
        match self {
            Self::Managed(agent) => agent.shutdown(wait),
            Self::Attached { session, .. } => session.close(),
        }
    }
}

/// Открывает точку входа по конфигу: управляемую поднимает, к объявленной подключается.
/// Локация из выбора исполнителя — `1cv8` у управляемого агента, `ssh` у чужого.
fn connect(
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    location: &Path,
    transcript_log: PathBuf,
    wait: &WaitPolicy,
) -> Result<AgentHandle, AppError> {
    let agent = &config.tools.designer_agent;
    let mode = agent.mode().map_err(AppError::Validation)?;
    let connection = config.v8_connection();
    let user = connection.user.clone().unwrap_or_default();
    let password = connection.password.clone().unwrap_or_default();
    let askpass_dir = config.work_path.join("agent").join("ssh");

    match mode {
        DesignerAgentMode::Attached { host, port } => {
            let base_dir = agent.base_dir.clone().ok_or_else(|| {
                AppError::CapabilityUnavailable(
                    "dump through an attached agent reads its files from disk and needs tools.designer_agent.base-dir".to_owned(),
                )
            })?;
            let request = AgentSessionRequest {
                ssh: location.to_path_buf(),
                endpoint: AgentEndpoint { host, port },
                user,
                password,
                askpass_dir,
                transcript_log: Some(transcript_log),
            };
            // Чужая точка входа не поднимается заново: сначала структурная проба, что
            // там кто-то слушает, и только потом сессия.
            crate::platform::agent::probe_reachable(&request.endpoint, Duration::from_secs(5))
                .map_err(map_agent_error)?;
            let session = AgentSession::open(&request, wait).map_err(map_agent_error)?;
            Ok(AgentHandle::Attached { session, base_dir })
        }
        DesignerAgentMode::Managed { port } => {
            let ssh = utilities
                .locate(UtilityType::Ssh)
                .map_err(|error| AppError::EnvironmentUnavailable(error.to_string()))?;
            let launch = AgentLaunch {
                v8: location.to_path_buf(),
                infobase_args: connection.infobase_args(),
                port,
                host_key: agent.host_key.clone(),
                base_dir: config.work_path.join("agent").join("base"),
                process_log: transcript_log.with_extension("process"),
            };
            let request = AgentSessionRequest {
                ssh: ssh.path,
                endpoint: launch.endpoint(),
                user,
                password,
                askpass_dir,
                transcript_log: Some(transcript_log),
            };
            let managed = ManagedAgent::launch(
                utilities.runner_for(UtilityType::V8),
                &launch,
                request,
                Duration::from_millis(agent.startup_timeout_ms.max(1)),
                wait,
            )
            .map_err(map_agent_error)?;
            Ok(AgentHandle::Managed(managed))
        }
    }
}

/// Отказы агента раскладываются по типам раннера: среда, срок, отмена, платформа.
fn map_agent_error(error: AgentError) -> AppError {
    match error {
        AgentError::TimedOut { .. } => AppError::TimedOut(error.to_string()),
        AgentError::Cancelled { .. } => AppError::Cancelled(error.to_string()),
        AgentError::Command { .. }
        | AgentError::Canceled { .. }
        | AgentError::Question { .. }
        | AgentError::NoTerminalMessage { .. }
        | AgentError::InvalidReply { .. }
        | AgentError::UserDirUnknown { .. } => AppError::Platform(error.to_string()),
        AgentError::Stdin(_) | AgentError::Workspace { .. } => AppError::Runtime(error.to_string()),
        AgentError::SshSpawn { .. }
        | AgentError::SessionClosed { .. }
        | AgentError::Unreachable { .. }
        | AgentError::Launch(_)
        | AgentError::StartupTimedOut { .. } => AppError::EnvironmentUnavailable(error.to_string()),
    }
}

/// Значение параметра агентской команды. Пробелы в значении экранируются кавычками —
/// грамматика shell документацией не описана, у путей раннера пробелов нет.
fn argument(value: &str) -> String {
    if value.chars().any(char::is_whitespace) {
        format!("\"{value}\"")
    } else {
        value.to_owned()
    }
}

/// Выгрузка одного режима через одну сессию.
pub(super) fn run_dump_agent(
    context: &ExecutionContext,
    config: &AppConfig,
    resolved: &ResolvedDumpTarget,
    mode: &DumpMode,
    objects: Option<&[PartialDumpSelector]>,
    location: &Path,
    utilities: &mut PlatformUtilities,
) -> Result<(PlatformCommandResult, Option<String>), AppError> {
    let policy = context.process_policy(InterruptionSafetyClass::GracefulThenKill, None);
    let wait = WaitPolicy {
        timeout: policy.timeout,
        cancellation: policy.cancellation.clone(),
    };
    let log_dir = platform_logs_dir(&config.work_path).map_err(|error| {
        AppError::Runtime(format!("failed to create platform logs dir: {error}"))
    })?;
    let transcript_log = log_dir.join(format!("dump-{}-agent.log", resolved.source_set_name));
    remove_path_if_exists(&transcript_log)
        .map_err(|error| AppError::Runtime(format!("failed to reset agent log: {error}")))?;

    log_live_stage("dump: agent", "[агент] opening the Designer agent session");
    let mut handle = connect(config, utilities, location, transcript_log.clone(), &wait)?;
    let outcome = dump_through(context, config, resolved, mode, objects, &mut handle, &wait);
    handle.finish(&wait);
    let (reply_transcript, cleanup_message) = outcome?;

    Ok((
        PlatformCommandResult {
            process: ProcessResult {
                exit_code: 0,
                stdout: reply_transcript,
                stderr: String::new(),
                interruption: None,
            },
            platform_log_path: Some(transcript_log),
            platform_log: None,
            platform_log_read_error: None,
        },
        cleanup_message,
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
) -> Result<(String, Option<String>), AppError> {
    let run_id = format!("{}-{:x}", std::process::id(), chrono_now_nanos());
    let out_relative = format!("dump/{run_id}");
    let mut command = format!(
        "config dump-config-to-files --dir={}",
        argument(&out_relative)
    );
    if let Some(extension) = resolved.extension.as_deref() {
        command.push_str(&format!(" --extension={}", argument(extension)));
    }
    let list_relative = objects.map(|_| format!("dump-lists/{run_id}.txt"));
    if let Some(list) = list_relative.as_deref() {
        command.push_str(&format!(" --list-file={}", argument(list)));
    }

    // Список объектов кладётся в каталог пользователя агента до команды: агент читает
    // его относительно того же каталога, куда пишет выгрузку. Карту каталогов пишет
    // платформа, поэтому в остальных режимах она читается после команды.
    let user = agent_user(config);
    let mut user_dir = None;
    if let (Some(list), Some(objects)) = (list_relative.as_deref(), objects) {
        let dir =
            crate::platform::agent::user_dir(handle.base_dir(), &user).map_err(map_agent_error)?;
        let list_path = dir.join(list);
        user_dir = Some(dir);
        if let Some(parent) = list_path.parent() {
            ensure_dir(parent).map_err(|error| {
                AppError::Runtime(format!("failed to create agent list dir: {error}"))
            })?;
        }
        let body = objects
            .iter()
            .map(|object| format!("{}\n", object.normalized()))
            .collect::<String>();
        std::fs::write(&list_path, body).map_err(|error| {
            AppError::Runtime(format!("failed to write partial dump list: {error}"))
        })?;
    }

    let stage = match mode {
        DumpMode::Full => "dump: full",
        DumpMode::Incremental => "dump: incremental",
        DumpMode::Partial => "dump: partial",
    };
    log_live_stage(stage, "[агент] exporting configuration files");
    let reply = handle
        .session()
        .run(&command, wait)
        .map_err(map_agent_error)?;
    reply.outcome().map_err(map_agent_error)?;
    let transcript = reply.transcript();

    let user_dir = match user_dir {
        Some(dir) => dir,
        None => {
            crate::platform::agent::user_dir(handle.base_dir(), &user).map_err(map_agent_error)?
        }
    };
    let produced = user_dir.join(&out_relative);
    if !produced.is_dir() {
        return Err(AppError::Platform(format!(
            "agent reported success but wrote nothing into '{}'",
            produced.display()
        )));
    }

    let (cleanup, degraded) = match mode {
        DumpMode::Partial => {
            ensure_dir(&resolved.platform_target_path).map_err(|error| {
                AppError::Runtime(format!("failed to create target dir: {error}"))
            })?;
            copy_dir_recursively(&produced, &resolved.platform_target_path).map_err(|error| {
                AppError::Runtime(format!("failed to merge partial dump into target: {error}"))
            })?;
            let _ = std::fs::remove_dir_all(&produced);
            (None, None)
        }
        DumpMode::Full | DumpMode::Incremental => {
            let cleanup = publish_full(context, resolved, &produced)?;
            let degraded = (*mode == DumpMode::Incremental).then(|| {
                format!(
                    "the agent cannot update a dump outside its base dir; ran a full export for source-set '{}' instead",
                    resolved.source_set_name
                )
            });
            (cleanup, degraded)
        }
    };
    Ok((transcript, merge_optional_messages(degraded, cleanup)))
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
        .publish_dir(context, DUMP_BACKUP_PREFIX, "failed to publish staged dump")
        .map_err(|error| publication.cleanup_failure(error))?;
    Ok(merge_optional_messages(
        publish_phase.cleanup_warning,
        dump_publication_warning(context.command(), publish_phase.deferred_interruption),
    ))
}

fn agent_user(config: &AppConfig) -> String {
    config.infobase.user.clone().unwrap_or_default()
}

fn chrono_now_nanos() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
}
