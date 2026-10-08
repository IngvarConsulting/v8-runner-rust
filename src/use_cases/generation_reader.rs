//! Поколение конфигурации, прочитанное процессом платформы: Конфигуратором
//! (`/GetConfigGenerationID`) или `ibcmd config generation-id`.
//!
//! Единственный такой читатель: его зовут `push` перед загрузкой и после неё, `pull` до и
//! после выгрузки и `status --deep`. У агента поколение читает его сессия
//! (`agent_session::generation_id`). Что ответ значит для обмена, решает
//! `exchange_guard::predict`.
//!
//! Здесь же — признак непринятого для `status --deep` ([`read_unapplied`]): тот же процесс
//! сохраняет основную конфигурацию и конфигурацию базы данных в файлы.

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
/// отмены процесс не запускается: ответа нет. `process` собирается только после проверки
/// отмены — его файлы и каталоги после отмены не создаются.
pub(crate) fn read_generation<'a>(
    context: &ExecutionContext,
    config: &AppConfig,
    process: impl FnOnce() -> Result<GenerationProcess<'a>, AppError>,
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
    match process()? {
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
        } => ibcmd_dsl(config, binary, runner, data_path, policy)?
            .config_generation_id(extension)
            .map_err(AppError::from),
    }
}

/// `ibcmd` к базе проекта — для обоих чтений этого модуля.
fn ibcmd_dsl<'a>(
    config: &AppConfig,
    binary: &Path,
    runner: &'a dyn ProcessRunner,
    data_path: Option<PathBuf>,
    policy: crate::platform::process::ProcessExecutionPolicy,
) -> Result<IbcmdDsl<'a>, AppError> {
    let connection = IbcmdConnection::from_infobase(&config.infobase)?;
    let dsl = IbcmdDsl::new(binary.to_path_buf(), connection, runner, policy);
    Ok(match data_path {
        Some(data_path) => dsl.with_data_path(data_path),
        None => dsl,
    })
}

/// Есть ли непринятое: основная конфигурация (или расширение `extension`) отличается от
/// конфигурации базы данных. Признак — побайтное неравенство двух сохранений: `/DumpCfg` и
/// `/DumpDBCfg` у Конфигуратора, `config save` и `config save --db` у `ibcmd` (замер #412:
/// у принятой базы сохранения равны и детерминированы, у непринятой расходятся). Ошибка
/// называет, почему ответа нет: отмена, отказ сохранения или нечитаемый файл.
pub(crate) fn read_unapplied<'a>(
    context: &ExecutionContext,
    config: &AppConfig,
    process: impl FnOnce() -> Result<GenerationProcess<'a>, AppError>,
    extension: Option<&str>,
) -> Result<bool, AppError> {
    let interrupted = || {
        crate::use_cases::interruption::pending_interruption_error(context, "the unapplied check")
            .map_or(Ok(()), Err)
    };
    let saved = |label: &str, outcome: Result<(), std::num::NonZeroI32>| {
        outcome
            .map_err(|code| AppError::Runtime(format!("the {label} save exited with code {code}")))
    };
    interrupted()?;
    // Сохранения — полная конфигурация: каталог закрыт для других пользователей.
    let dir = crate::support::temp::private_temp_dir(&config.work_path, "unapplied-")
        .map_err(|error| AppError::Runtime(format!("failed to create temp dir: {error}")))?;
    let main = dir.path().join("main.cf");
    let database = dir.path().join("database.cf");
    let policy = context.process_policy(InterruptionSafetyClass::GracefulThenKill, None);
    match process()? {
        GenerationProcess::Designer {
            binary,
            runner,
            log_file,
        } => {
            let dsl = DesignerDsl::new(
                binary.to_path_buf(),
                config.v8_connection(),
                runner,
                Some(log_file),
                policy,
            );
            let first = dsl.dump_cfg(&main, extension).map_err(AppError::from)?;
            saved("configuration", first.process.outcome())?;
            interrupted()?;
            let second = dsl
                .dump_db_cfg(&database, extension)
                .map_err(AppError::from)?;
            saved("database configuration", second.process.outcome())?;
        }
        GenerationProcess::Ibcmd {
            binary,
            runner,
            data_path,
        } => {
            let dsl = ibcmd_dsl(config, binary, runner, data_path, policy)?;
            let first = dsl
                .config_save(&main, false, extension)
                .map_err(AppError::from)?;
            saved("configuration", first.process.outcome())?;
            interrupted()?;
            let second = dsl
                .config_save(&database, true, extension)
                .map_err(AppError::from)?;
            saved("database configuration", second.process.outcome())?;
        }
    }
    let differ = files_differ(&main, &database)
        .map_err(|error| AppError::Runtime(format!("failed to compare the saves: {error}")));
    let closed = dir
        .close()
        .map_err(|error| AppError::Runtime(format!("failed to remove the saves: {error}")));
    let differ = differ?;
    closed?;
    Ok(differ)
}

/// Побайтное неравенство файлов: длины, затем блоки — сохранения бывают в сотни мегабайт.
fn files_differ(left: &Path, right: &Path) -> std::io::Result<bool> {
    use std::io::Read;

    if std::fs::metadata(left)?.len() != std::fs::metadata(right)?.len() {
        return Ok(true);
    }
    let mut left = std::io::BufReader::new(std::fs::File::open(left)?);
    let mut right = std::io::BufReader::new(std::fs::File::open(right)?);
    let mut left_block = vec![0u8; 64 * 1024];
    let mut right_block = vec![0u8; 64 * 1024];
    loop {
        let read = left.read(&mut left_block)?;
        if read == 0 {
            return Ok(false);
        }
        right.read_exact(&mut right_block[..read])?;
        if left_block[..read] != right_block[..read] {
            return Ok(true);
        }
    }
}

/// Файл `/Out` Конфигуратора для чтения поколения под журналами платформы.
pub(crate) fn designer_log_file(config: &AppConfig, name: &str) -> Result<PathBuf, AppError> {
    crate::support::temp::platform_logs_dir(&config.work_path)
        .map(|dir| dir.join(format!("{name}.log")))
        .map_err(|error| AppError::Runtime(format!("failed to create platform logs dir: {error}")))
}
