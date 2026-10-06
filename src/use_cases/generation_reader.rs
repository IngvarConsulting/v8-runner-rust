//! Поколение конфигурации, прочитанное процессом платформы: Конфигуратором
//! (`/GetConfigGenerationID`) или `ibcmd config generation-id`.
//!
//! Единственный такой читатель: его зовут `push` перед загрузкой и после неё, `pull` до и
//! после выгрузки и `status --deep`. У агента поколение читает его сессия
//! (`agent_session::generation_id`). Что ответ значит для обмена, решает
//! `exchange_guard::predict`.

use std::path::{Path, PathBuf};

use crate::config::model::AppConfig;
use crate::platform::designer::DesignerDsl;
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl};
use crate::platform::process::ProcessRunner;
use crate::support::error::AppError;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};

/// Процесс, который спрашивает поколение.
pub(crate) enum GenerationProcess<'a> {
    /// Конфигуратор пишет ответ в `/Out`; `log_file` — этот файл.
    Designer {
        binary: &'a Path,
        runner: &'a dyn ProcessRunner,
        log_file: PathBuf,
    },
    /// `ibcmd` отвечает в stdout; `data_path` — каталог данных автономного сервера, если
    /// команда держит свой.
    Ibcmd {
        binary: &'a Path,
        runner: &'a dyn ProcessRunner,
        data_path: Option<PathBuf>,
    },
}

/// Поколение основной конфигурации или расширения `extension`; `None` — ответа нет. После
/// отмены процесс не запускается: ответа нет.
pub(crate) fn read_generation(
    context: &ExecutionContext,
    config: &AppConfig,
    process: GenerationProcess<'_>,
    extension: Option<&str>,
) -> Result<Option<String>, AppError> {
    if crate::use_cases::interruption::pending_interruption_error(
        context,
        "the configuration generation",
    )
    .is_some()
    {
        return Ok(None);
    }
    let policy = context.process_policy(InterruptionSafetyClass::GracefulThenKill, None);
    match process {
        GenerationProcess::Designer {
            binary,
            runner,
            log_file,
        } => DesignerDsl::new(
            binary.to_path_buf(),
            config.v8_connection(),
            runner,
            Some(log_file),
            policy,
        )
        .config_generation_id(extension)
        .map_err(AppError::from),
        GenerationProcess::Ibcmd {
            binary,
            runner,
            data_path,
        } => {
            let connection = IbcmdConnection::from_infobase(&config.infobase)?;
            let mut dsl = IbcmdDsl::new(binary.to_path_buf(), connection, runner, policy);
            if let Some(data_path) = data_path {
                dsl = dsl.with_data_path(data_path);
            }
            dsl.config_generation_id(extension).map_err(AppError::from)
        }
    }
}

/// Файл `/Out` Конфигуратора для чтения поколения под журналами платформы.
pub(crate) fn designer_log_file(config: &AppConfig, name: &str) -> Result<PathBuf, AppError> {
    crate::support::temp::platform_logs_dir(&config.work_path)
        .map(|dir| dir.join(format!("{name}.log")))
        .map_err(|error| AppError::Runtime(format!("failed to create platform logs dir: {error}")))
}
