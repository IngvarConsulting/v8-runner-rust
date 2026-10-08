//! Акт применения: основная конфигурация (или расширение) становится конфигурацией базы
//! данных. Владелец один на все пути, которые его делают, — `push` (Конфигуратор, `ibcmd`,
//! агент), расширение-инструмент и команда `apply`: точка безопасности перед ним, класс
//! прерывания `CriticalNonAbortable`, учёт отложенной отмены и проверка итога
//! (`INV.CLI.APPLY-IS-A-SEPARATE-STEP`, `INV.USE-CASES.A-DATABASE-WRITE-IS-A-CRITICAL-PHASE`).
//!
//! Вне владельца остаются два применения, и это решение, а не недосмотр:
//!
//! - `upload` (`load_artifact.rs`) применяет пакет в своей грамматике шагов
//!   `ExecutionOutcome`: его точка безопасности (`SafePointCancel`), признак
//!   `update_db_cfg_ran` и текст шага — часть формы `upload`, и перевод их на этого
//!   владельца поменял бы форму ответа, а не только код;
//! - `infobase create` (`init_project.rs`) собирает новую базу: у `ibcmd` загрузка и
//!   применение — одна команда `infobase create --import --apply`, у Конфигуратора база ещё
//!   ничья, и отложенной отмене там нечего защищать — сеансов нет.
//!
//! Политику чужих сеансов при применении (`apply --sessions`, #211) владелец пока не
//! принимает: параметр без исполнителя был бы мёртвым кодом. Когда #211 придёт, он ляжет
//! сюда — в [`Applier`] или рядом с ним, — и все пути получат его разом.

use std::path::{Path, PathBuf};

use crate::config::model::AppConfig;
use crate::domain::capability::Provider;
use crate::platform::agent::WaitPolicy;
use crate::platform::designer::DesignerDsl;
use crate::platform::ibcmd::{DynamicUpdateMode, IbcmdConnection, IbcmdDsl};
use crate::platform::process::ProcessRunner;
use crate::platform::result::PlatformCommandResult;
use crate::support::error::AppError;
use crate::use_cases::agent_session::{argument, run_critical, AgentHandle};
use crate::use_cases::build_progress::{log_timeline_stage, TimelineStageStatus};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::ibcmd_diagnostics::format_ibcmd_failure_details;
use crate::use_cases::interruption::{self, Deferrals};

/// Чем применяется: процесс платформы или команда в открытой сессии агента. Процессы
/// владелец строит сам — с классом прерывания критической фазы.
pub(crate) enum Applier<'a> {
    /// `1cv8 DESIGNER /UpdateDBCfg [-Extension X]`; журнал платформы — `log_file`.
    Designer {
        binary: &'a Path,
        runner: &'a dyn ProcessRunner,
        log_file: PathBuf,
    },
    /// `ibcmd config apply [--extension X] --force --dynamic auto`.
    Ibcmd {
        binary: &'a Path,
        runner: &'a dyn ProcessRunner,
    },
    /// `config update-db-cfg [--extension=X]` в сессии агента.
    Agent {
        handle: &'a mut AgentHandle,
        wait: &'a WaitPolicy,
    },
}

impl Applier<'_> {
    /// Инструмент применения.
    pub(crate) const fn tool(&self) -> Provider {
        match self {
            Self::Designer { .. } => Provider::Designer,
            Self::Ibcmd { .. } => Provider::Ibcmd,
            Self::Agent { .. } => Provider::Agent,
        }
    }
}

/// Род того, что применяется.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubjectKind {
    SourceSet,
    /// Расширение-инструмент клиентского MCP.
    ToolExtension,
}

impl SubjectKind {
    /// Имя рода в текстах отказа.
    const fn as_str(self) -> &'static str {
        match self {
            Self::SourceSet => "source-set",
            Self::ToolExtension => "tool extension",
        }
    }
}

/// Что применяется: набор исходников или расширение-инструмент.
pub(crate) struct Subject<'a> {
    pub(crate) kind: SubjectKind,
    pub(crate) name: &'a str,
    /// Имя расширения; `None` — основная конфигурация.
    pub(crate) extension: Option<&'a str>,
    /// Метка строки живой ленты.
    pub(crate) timeline: &'a str,
}

/// Применяет основную конфигурацию предмета к конфигурации базы данных. Отмена, замеченная
/// до запуска, останавливает здесь — до записи в базу; начатое применение доходит до конца,
/// а отмена, которую оно отложило, попадает в `deferrals` раньше проверки итога.
pub(crate) fn apply(
    context: &ExecutionContext,
    config: &AppConfig,
    applier: Applier<'_>,
    subject: &Subject<'_>,
    deferrals: &mut Deferrals,
) -> Result<(), AppError> {
    let (verb, executor) = match &applier {
        Applier::Designer { .. } => ("update_db_cfg", "Конфигуратор"),
        Applier::Ibcmd { .. } => ("ibcmd apply", "ibcmd"),
        Applier::Agent { .. } => ("update_db_cfg", "агент"),
    };
    if let Some(error) = interruption::interruption_before_safe_point(
        context,
        format!("{verb} for {} '{}'", subject.kind.as_str(), subject.name),
    ) {
        return Err(error);
    }
    tracing::debug!(
        subject = subject.name,
        tool = applier.tool().as_str(),
        "applying the main configuration to the database configuration"
    );
    let stage = match &applier {
        Applier::Ibcmd { .. } => "ibcmd_apply",
        Applier::Designer { .. } | Applier::Agent { .. } => "update_db_cfg",
    };
    let detail = match subject.kind {
        SubjectKind::ToolExtension => {
            format!("[{executor}] Применение расширения {}", subject.name)
        }
        SubjectKind::SourceSet => format!("[{executor}] Применение изменений"),
    };
    log_timeline_stage(
        subject.timeline,
        stage,
        &detail,
        TimelineStageStatus::Running,
    );
    let policy = context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None);
    match applier {
        Applier::Designer {
            binary,
            runner,
            log_file,
        } => {
            let result = DesignerDsl::new(
                binary.to_path_buf(),
                config.v8_connection(),
                runner,
                Some(log_file),
                policy,
            )
            .update_db_cfg(subject.extension)
            .map_err(AppError::from)?;
            deferrals.note_result("update_db_cfg", &result);
            ensure_applied("update_db_cfg", subject, &result)
        }
        Applier::Ibcmd { binary, runner } => {
            let connection =
                IbcmdConnection::from_infobase(&config.infobase).map_err(AppError::from)?;
            let result = IbcmdDsl::new(binary.to_path_buf(), connection, runner, policy)
                .config_apply(subject.extension, DynamicUpdateMode::Auto)
                .map_err(AppError::from)?;
            deferrals.note_result("apply", &result);
            ensure_applied("apply", subject, &result)
        }
        Applier::Agent { handle, wait } => {
            let mut command = String::from("config update-db-cfg");
            if let Some(extension) = subject.extension {
                command.push_str(&format!(" --extension={}", argument(extension)));
            }
            run_critical(handle, "update_db_cfg", &command, wait, deferrals).map(|_| ())
        }
    }
}

fn ensure_applied(
    action: &str,
    subject: &Subject<'_>,
    result: &PlatformCommandResult,
) -> Result<(), AppError> {
    let Err(code) = result.process.outcome() else {
        return Ok(());
    };
    Err(AppError::Platform(format_ibcmd_failure_details(
        action,
        subject.kind.as_str(),
        subject.name,
        code.get(),
        &result.process.stdout,
        &result.process.stderr,
        result.platform_log.as_deref(),
        result.platform_log_path.as_deref(),
    )))
}
