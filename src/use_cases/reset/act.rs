//! Акт отката: основная конфигурация (или расширение) возвращается к конфигурации базы
//! данных — `1cv8 DESIGNER /RollbackCfg [-Extension X]` или `ibcmd config reset
//! [--extension X]`. Владелец один: точка безопасности перед откатом, класс прерывания
//! `CriticalNonAbortable`, учёт отложенной отмены и проверка итога
//! (`INV.CLI.RESET-DISCARDS-THE-UNAPPLIED`, `INV.USE-CASES.A-DATABASE-WRITE-IS-A-CRITICAL-PHASE`).
//!
//! Откат пишет в базу — в её основную конфигурацию, — и снятый посреди процесс оставил бы
//! её в состоянии, которое не назовёт никто, поэтому он критическая фаза, как применение.
//! У агента команды отката нет (справка 8.5.1.1150).

use std::path::{Path, PathBuf};

use crate::config::model::AppConfig;
use crate::domain::capability::Provider;
use crate::platform::designer::DesignerDsl;
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl};
use crate::platform::process::ProcessRunner;
use crate::support::error::AppError;
use crate::use_cases::build_progress::{log_timeline_stage, TimelineStageStatus};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::ibcmd_diagnostics::ensure_succeeded;
use crate::use_cases::interruption::{self, Deferrals};

/// Почему откат чаще всего отказывает. Конфигуратор, открытый на этой базе, держит её
/// блокировку для конфигурирования, и оба инструмента тогда отказывают (замер 08.10.2026,
/// 8.3.27.2074); различить это можно только текстом платформы, и его раннер не разбирает —
/// подсказка стоит у каждого отказа отката.
const OPEN_DESIGNER_HINT: &str = "a Designer that has this infobase open holds its configuration lock and makes the rollback fail: close it and run reset again";

/// Чем откатывается: процесс платформы. Процессы владелец строит сам — с классом прерывания
/// критической фазы.
pub(crate) enum RollingBack<'a> {
    /// `1cv8 DESIGNER /RollbackCfg [-Extension X]`; журнал платформы — `log_file`.
    Designer {
        binary: &'a Path,
        runner: &'a dyn ProcessRunner,
        log_file: PathBuf,
    },
    /// `ibcmd config reset [--extension X]`.
    Ibcmd {
        binary: &'a Path,
        runner: &'a dyn ProcessRunner,
    },
}

impl RollingBack<'_> {
    /// Инструмент отката.
    pub(crate) const fn tool(&self) -> Provider {
        match self {
            Self::Designer { .. } => Provider::Designer,
            Self::Ibcmd { .. } => Provider::Ibcmd,
        }
    }
}

/// Откатывает основную конфигурацию набора `set` (расширения `extension`) к конфигурации
/// базы данных. Отмена, замеченная до запуска, останавливает здесь — до записи в базу; начатый
/// откат доходит до конца, а отмена, которую он отложил, попадает в `deferrals` раньше
/// проверки итога.
pub(crate) fn roll_back(
    context: &ExecutionContext,
    config: &AppConfig,
    tool: RollingBack<'_>,
    set: &str,
    extension: Option<&str>,
    deferrals: &mut Deferrals,
) -> Result<(), AppError> {
    if let Some(error) = interruption::interruption_before_safe_point(
        context,
        format!("reset for source-set '{set}'"),
    ) {
        return Err(error);
    }
    tracing::debug!(
        set,
        tool = tool.tool().as_str(),
        "rolling the main configuration back to the database configuration"
    );
    let executor = match &tool {
        RollingBack::Designer { .. } => "Конфигуратор",
        RollingBack::Ibcmd { .. } => "ibcmd",
    };
    log_timeline_stage(
        set,
        "reset",
        &format!("[{executor}] Возврат к конфигурации базы данных"),
        TimelineStageStatus::Running,
    );
    let policy = context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None);
    let (action, result) = match tool {
        RollingBack::Designer {
            binary,
            runner,
            log_file,
        } => (
            "rollback_cfg",
            DesignerDsl::new(
                binary.to_path_buf(),
                config.v8_connection(),
                runner,
                Some(log_file),
                policy,
            )
            .rollback_cfg(extension)
            .map_err(AppError::from)?,
        ),
        RollingBack::Ibcmd { binary, runner } => {
            let connection =
                IbcmdConnection::from_infobase(&config.infobase).map_err(AppError::from)?;
            (
                "config reset",
                IbcmdDsl::new(binary.to_path_buf(), connection, runner, policy)
                    .config_reset(extension)
                    .map_err(AppError::from)?,
            )
        }
    };
    deferrals.note_result(action, &result);
    ensure_succeeded(action, "source-set", set, &result).map_err(|error| match error {
        AppError::Platform(text) => AppError::Platform(format!("{text}; {OPEN_DESIGNER_HINT}")),
        other => other,
    })
}
