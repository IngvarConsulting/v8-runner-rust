//! Точка входа агента для сценариев: одна на команду, общая для `dump` и `build`.
//!
//! Сценарий открывает точку входа по конфигу — управляемую поднимает, к чужой
//! подключается, — получает каталог пользователя агента и работает с файлами через
//! него: агент читает и пишет только внутри своего `AgentBaseDir`, поэтому чужие
//! каталоги выставляются туда символической ссылкой. Здесь же живёт учёт
//! `generation-id`: токен поколения конфигурации, записанный после успешной загрузки
//! или выгрузки и сравниваемый перед следующей выгрузкой.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::model::{AppConfig, DesignerAgentMode};
use crate::platform::agent::{
    self, AgentEndpoint, AgentError, AgentLaunch, AgentSession, AgentSessionRequest, ManagedAgent,
    WaitPolicy,
};
use crate::platform::locator::{UtilityLocation, UtilityType};
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::support::fs::copy_dir_recursively;
use crate::support::temp::platform_logs_dir;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};

/// Открытая точка входа: свой процесс с сессией или только сессия к чужому.
pub(crate) enum AgentHandle {
    Managed(ManagedAgent),
    Attached {
        session: AgentSession,
        base_dir: PathBuf,
    },
}

impl AgentHandle {
    pub(crate) fn session(&mut self) -> &mut AgentSession {
        match self {
            Self::Managed(agent) => agent.session(),
            Self::Attached { session, .. } => session,
        }
    }

    pub(crate) fn base_dir(&self) -> &Path {
        match self {
            Self::Managed(agent) => agent.base_dir(),
            Self::Attached { base_dir, .. } => base_dir,
        }
    }

    /// Каталог пользователя агента: относительно него агент трактует пути команд.
    pub(crate) fn user_dir(&self, config: &AppConfig) -> Result<PathBuf, AppError> {
        agent::user_dir(self.base_dir(), &agent_user(config)).map_err(map_agent_error)
    }

    /// Управляемый агент гасится, чужой — только отпускается.
    pub(crate) fn finish(self, wait: &WaitPolicy) {
        match self {
            Self::Managed(agent) => agent.shutdown(wait),
            Self::Attached { session, .. } => session.close(),
        }
    }
}

/// Срок и отмена сессии — с границы команды.
pub(crate) fn wait_policy(context: &ExecutionContext) -> WaitPolicy {
    let policy = context.process_policy(InterruptionSafetyClass::GracefulThenKill, None);
    WaitPolicy {
        timeout: policy.timeout,
        cancellation: policy.cancellation.clone(),
    }
}

/// Журнал сессии рядом с журналами платформы, свежий на каждую команду.
pub(crate) fn transcript_log(config: &AppConfig, name: &str) -> Result<PathBuf, AppError> {
    let log_dir = platform_logs_dir(&config.work_path).map_err(|error| {
        AppError::Runtime(format!("failed to create platform logs dir: {error}"))
    })?;
    let path = log_dir.join(format!("{name}-agent.log"));
    crate::support::fs::remove_path_if_exists(&path)
        .map_err(|error| AppError::Runtime(format!("failed to reset agent log: {error}")))?;
    Ok(path)
}

/// Открывает точку входа по конфигу: управляемую поднимает, к объявленной подключается.
/// Локация из выбора исполнителя — `1cv8` у управляемого агента; у чужого её нет.
pub(crate) fn connect(
    config: &AppConfig,
    utilities: &mut PlatformUtilities,
    location: Option<&UtilityLocation>,
    transcript_log: PathBuf,
    wait: &WaitPolicy,
) -> Result<AgentHandle, AppError> {
    let agent = &config.tools.designer_agent;
    let mode = agent.mode().map_err(AppError::Validation)?;
    let connection = config.v8_connection();
    let user = connection.user.clone().unwrap_or_default();
    let password = connection.password.clone().unwrap_or_default();

    match mode {
        DesignerAgentMode::Attached { host, port } => {
            let base_dir = agent.base_dir.clone().ok_or_else(|| {
                AppError::CapabilityUnavailable(
                    "working through an attached agent exchanges files through its base dir and needs tools.designer_agent.base-dir".to_owned(),
                )
            })?;
            let request = AgentSessionRequest {
                endpoint: AgentEndpoint { host, port },
                user,
                password,
                transcript_log: Some(transcript_log),
            };
            // Чужая точка входа не поднимается заново: недоступная — типизированный отказ.
            let session = AgentSession::open(&request, wait).map_err(map_agent_error)?;
            Ok(AgentHandle::Attached { session, base_dir })
        }
        DesignerAgentMode::Managed { port } => {
            let v8 = location.ok_or_else(|| {
                AppError::EnvironmentUnavailable(
                    "the managed Designer agent needs the local platform; no 1cv8 was selected"
                        .to_owned(),
                )
            })?;
            let launch = AgentLaunch {
                v8: v8.path.clone(),
                infobase_args: connection.infobase_args(),
                port,
                host_key: agent.host_key.clone(),
                base_dir: config.work_path.join("agent").join("base"),
                process_log: transcript_log.with_extension("process"),
            };
            let request = AgentSessionRequest {
                endpoint: launch.endpoint(),
                user,
                password,
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
pub(crate) fn map_agent_error(error: AgentError) -> AppError {
    match error {
        AgentError::TimedOut { .. } => AppError::TimedOut(error.to_string()),
        AgentError::Cancelled { .. } => AppError::Cancelled(error.to_string()),
        AgentError::Command { .. }
        | AgentError::Canceled { .. }
        | AgentError::Question { .. }
        | AgentError::NoTerminalMessage { .. }
        | AgentError::InvalidReply { .. }
        | AgentError::SessionClosed { .. }
        | AgentError::Transport { .. }
        | AgentError::UserDirUnknown { .. } => AppError::Platform(error.to_string()),
        AgentError::Workspace { .. } => AppError::Runtime(error.to_string()),
        AgentError::Unreachable { .. }
        | AgentError::Handshake { .. }
        | AgentError::AuthenticationRejected { .. }
        | AgentError::Channel { .. }
        | AgentError::Launch(_)
        | AgentError::StartupTimedOut { .. } => AppError::EnvironmentUnavailable(error.to_string()),
    }
}

/// Значение параметра агентской команды. Пробелы в значении экранируются кавычками —
/// грамматика shell документацией не описана, у путей раннера пробелов нет.
pub(crate) fn argument(value: &str) -> String {
    if value.chars().any(char::is_whitespace) {
        format!("\"{value}\"")
    } else {
        value.to_owned()
    }
}

/// Имя пользователя, под которым открыта сессия: пустое у базы без пользователей.
pub(crate) fn agent_user(config: &AppConfig) -> String {
    config.infobase.user.clone().unwrap_or_default()
}

/// Уникальный ключ одного прогона для имён внутри каталога пользователя агента.
pub(crate) fn run_id() -> String {
    format!(
        "{}-{:x}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
}

/// Выставляет чужой каталог внутрь каталога пользователя агента под относительным
/// именем и возвращает это имя для команды. Агент ходит по символическим ссылкам
/// (замер 15.09.2026: выгрузка и загрузка через ссылку); там, где ссылку создать
/// нельзя, каталог копируется — медленно, но той же формы.
pub(crate) fn expose_dir(
    user_dir: &Path,
    relative: &str,
    target: &Path,
) -> Result<String, AppError> {
    let link = user_dir.join(relative);
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::Runtime(format!(
                "failed to prepare the agent exchange dir '{}': {error}",
                parent.display()
            ))
        })?;
    }
    let target = target.canonicalize().map_err(|error| {
        AppError::Runtime(format!(
            "source dir '{}' is not reachable: {error}",
            target.display()
        ))
    })?;
    let _ = crate::support::fs::remove_path_if_exists(&link);
    if symlink_dir(&target, &link).is_err() {
        copy_dir_recursively(&target, &link).map_err(|error| {
            AppError::Runtime(format!(
                "failed to expose '{}' to the agent at '{}': {error}",
                target.display(),
                link.display()
            ))
        })?;
    }
    Ok(relative.to_owned())
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

/// Снимает выставленный каталог: ссылку — как ссылку, копию — целиком.
pub(crate) fn withdraw_dir(user_dir: &Path, relative: &str) {
    let link = user_dir.join(relative);
    match std::fs::symlink_metadata(&link) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let _ = std::fs::remove_file(&link);
        }
        Ok(metadata) if metadata.is_dir() => {
            let _ = std::fs::remove_dir_all(&link);
        }
        _ => {}
    }
}

/// Токен поколения конфигурации у агента: `config generation-id [--extension=<имя>]`.
///
/// Тело ответа — строка из 40 hex; сравнивать её можно только на равенство
/// (Приложение 7, `/GetConfigGenerationID`).
pub(crate) fn generation_id(
    session: &mut AgentSession,
    extension: Option<&str>,
    wait: &WaitPolicy,
) -> Result<String, AppError> {
    let mut command = String::from("config generation-id");
    if let Some(extension) = extension {
        command.push_str(&format!(" --extension={}", argument(extension)));
    }
    let reply = session.run(&command, wait).map_err(map_agent_error)?;
    let body = reply.outcome().map_err(map_agent_error)?;
    body.and_then(|value| value.as_str())
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError::InvalidOutput(
                "agent answered generation-id without a token in the body".to_owned(),
            )
        })
}

/// Запись о поколении конфигурации, увиденном после последней удачной операции.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct GenerationRecord {
    pub token: String,
    /// Что было сделано, когда токен записан: `build` или `dump`.
    pub after: String,
    pub recorded_at: String,
}

/// Учёт поколений по предмету: файл на каждый набор исходников под `workPath`.
pub(crate) struct GenerationLedger {
    dir: PathBuf,
}

impl GenerationLedger {
    pub(crate) fn new(config: &AppConfig) -> Self {
        Self::at(config.work_path.join("agent").join("generation"))
    }

    fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, source_set: &str) -> PathBuf {
        self.dir.join(format!("{source_set}.json"))
    }

    pub(crate) fn read(&self, source_set: &str) -> Option<GenerationRecord> {
        let text = std::fs::read_to_string(self.path(source_set)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub(crate) fn record(
        &self,
        source_set: &str,
        token: &str,
        after: &str,
    ) -> Result<(), AppError> {
        std::fs::create_dir_all(&self.dir).map_err(|error| {
            AppError::Runtime(format!(
                "failed to create the generation ledger '{}': {error}",
                self.dir.display()
            ))
        })?;
        let record = GenerationRecord {
            token: token.to_owned(),
            after: after.to_owned(),
            recorded_at: chrono::Utc::now().to_rfc3339(),
        };
        let text = serde_json::to_string_pretty(&record)
            .map_err(|error| AppError::Runtime(format!("failed to encode generation: {error}")))?;
        std::fs::write(self.path(source_set), text).map_err(|error| {
            AppError::Runtime(format!(
                "failed to write the generation ledger '{}': {error}",
                self.path(source_set).display()
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_symlinked_dir_is_exposed_under_the_user_dir_and_withdrawn_as_a_link() {
        let root = tempfile::tempdir().expect("tempdir");
        let user_dir = root.path().join("0");
        let sources = root.path().join("src");
        std::fs::create_dir_all(&sources).expect("sources");
        std::fs::write(sources.join("Configuration.xml"), "<x/>").expect("marker");

        let relative = expose_dir(&user_dir, "build/run/main", &sources).expect("expose");
        assert_eq!(relative, "build/run/main");
        assert!(user_dir.join("build/run/main/Configuration.xml").is_file());

        withdraw_dir(&user_dir, "build/run/main");
        assert!(!user_dir.join("build/run/main").exists());
        assert!(
            sources.join("Configuration.xml").is_file(),
            "withdrawing the link must not touch the sources"
        );
    }

    #[test]
    fn the_ledger_keeps_one_record_per_source_set() {
        let root = tempfile::tempdir().expect("tempdir");
        let ledger = GenerationLedger::at(root.path().join("generation"));
        assert!(ledger.read("main").is_none());
        ledger.record("main", "abc", "build").expect("record");
        let record = ledger.read("main").expect("record");
        assert_eq!(record.token, "abc");
        assert_eq!(record.after, "build");
        assert!(ledger.read("ext").is_none());
    }
}
