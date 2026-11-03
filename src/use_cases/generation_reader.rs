//! Поколение конфигурации, прочитанное процессом платформы: Конфигуратором
//! (`/GetConfigGenerationID`) или `ibcmd config generation-id`.
//!
//! Единственный такой читатель: его зовут `push` перед загрузкой и после неё, `pull` до и
//! после выгрузки, `status --deep`, а `apply` и `reset` — инструментом записи набора
//! ([`read_by_record_tool`]). У агента поколение читает его сессия
//! (`agent_session::generation_id`). Что ответ значит для обмена, решает
//! `exchange_guard::predict`.
//!
//! Чтение объявляет собственный предел (`GENERATION_PROBE_CAP`,
//! `INV.USE-CASES.A-GENERATION-READ-DECLARES-ITS-OWN-CAP`): это вопрос, а не работа, а у
//! платформы, которой команда поколения не знакома, запуск Конфигуратора конца не имеет —
//! без своего предела чтение держало бы команду вечно. Снятый по пределу запуск —
//! отсутствие ответа, а не отказ; платформу без команды и не спрашивают
//! (`generation::knows_generation_id`).
//!
//! Здесь же — признак непринятого для `status --deep` ([`read_unapplied`]): тот же процесс
//! сохраняет основную конфигурацию и конфигурацию базы данных в файлы.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::model::AppConfig;
use crate::domain::capability::Provider;
use crate::platform::designer::DesignerDsl;
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl};
use crate::platform::locator::UtilityType;
use crate::platform::process::ProcessRunner;
use crate::platform::utilities::PlatformUtilities;
use crate::support::error::AppError;
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};

/// Собственный предел чтения поколения
/// (DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE: предел объявляет шаг). Ответ приходит за
/// секунды — отдельным процессом за четыре и больше
/// ([замер](../../references/1c/confirmed-runtime-measurements.md)); предел много больше, а
/// у платформы без команды поколения снятый им запуск конца не имел бы вовсе.
const GENERATION_PROBE_CAP: Duration = Duration::from_secs(30);

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
/// отмены — его файлы и каталоги после отмены не создаются. Запуск, снятый собственным
/// пределом шага (`GENERATION_PROBE_CAP`), — тоже отсутствие ответа, а не отказ.
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
    let policy = context.process_policy(
        InterruptionSafetyClass::GracefulThenKill,
        Some(GENERATION_PROBE_CAP),
    );
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

/// Исполнитель команды: инструмент и найденная им утилита.
#[derive(Clone, Copy)]
pub(crate) struct LocatedTool<'a> {
    pub(crate) provider: Provider,
    pub(crate) binary: Option<&'a Path>,
}

impl<'a> LocatedTool<'a> {
    /// Утилита исполнителя; не найдена до работы — ошибка сценария.
    pub(crate) fn path(self, context: &ExecutionContext) -> Result<&'a Path, AppError> {
        self.binary.ok_or_else(|| {
            AppError::Runtime(format!(
                "{} was not located before the {}",
                self.provider,
                context.command().as_str()
            ))
        })
    }
}

/// Поколение инструментом `tool` — тем, что сделал запись набора: токены разных инструментов
/// несравнимы. Свой инструмент исполнителя спрашивает сам; Конфигуратор и `ibcmd` ищутся,
/// если исполнитель другой, и ненайденный — нет ответа. Агент здесь не спрашивается: его
/// поколение читает сессия, которую держит исполнитель-агент, а у другого исполнителя
/// ответа агента нет. `None` — ответа нет; `name` — имя файла `/Out` Конфигуратора.
pub(crate) fn read_by_record_tool(
    context: &ExecutionContext,
    config: &AppConfig,
    tool: Provider,
    executor: LocatedTool<'_>,
    utilities: &mut PlatformUtilities,
    name: &str,
    extension: Option<&str>,
) -> Result<Option<String>, AppError> {
    /// Инструмент записи, который отвечает процессом платформы.
    #[derive(Clone, Copy)]
    enum Process {
        Designer,
        Ibcmd,
    }
    let (process, utility) = match tool {
        Provider::Designer => (Process::Designer, UtilityType::V8),
        Provider::Ibcmd => (Process::Ibcmd, UtilityType::Ibcmd),
        Provider::Agent | Provider::IbcmdRs | Provider::Webinst => return Ok(None),
    };
    let binary = if tool == executor.provider {
        executor.path(context)?.to_path_buf()
    } else {
        match utilities.locate(utility) {
            Ok(location) => location.path,
            Err(error) => {
                tracing::debug!(%error, %tool, "the tool of the generation record is not found");
                return Ok(None);
            }
        }
    };
    let runner = utilities.runner_for(utility);
    read_generation(
        context,
        config,
        || {
            Ok(match process {
                Process::Ibcmd => GenerationProcess::Ibcmd {
                    binary: &binary,
                    runner,
                    data_path: None,
                },
                Process::Designer => GenerationProcess::Designer {
                    binary: &binary,
                    runner,
                    log_file: designer_log_file(config, name)?,
                },
            })
        },
        extension,
    )
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

#[cfg(test)]
mod tests {
    use super::{designer_log_file, read_generation, GenerationProcess, GENERATION_PROBE_CAP};
    use crate::config::loader::load_config;
    use crate::config::model::{AppConfig, InfobaseSelector};
    use crate::platform::process::{
        ProcessError, ProcessExecutionPolicy, ProcessInterruptionSafety, ProcessRequest,
        ProcessResult, ProcessRunner, SpawnResult, WorkGiven,
    };
    use crate::support::error::AppError;
    use crate::use_cases::context::{CommandName, ExecutionContext};
    use std::cell::RefCell;
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// Запоминает политику, с которой шаг запустил чтение.
    struct CapturingRunner(RefCell<Option<ProcessExecutionPolicy>>);

    impl ProcessRunner for CapturingRunner {
        fn run_with_policy(
            &self,
            _request: &ProcessRequest,
            policy: &ProcessExecutionPolicy,
        ) -> Result<ProcessResult, ProcessError> {
            *self.0.borrow_mut() = Some(policy.clone());
            Ok(ProcessResult {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                interruption: None,
            })
        }

        fn spawn(
            &self,
            _request: &ProcessRequest,
            _work: &WorkGiven,
        ) -> Result<SpawnResult, ProcessError> {
            panic!("the generation read runs synchronously")
        }
    }

    /// Запуск, которого отменили.
    struct CancellingRunner;

    impl ProcessRunner for CancellingRunner {
        fn run_with_policy(
            &self,
            _request: &ProcessRequest,
            _policy: &ProcessExecutionPolicy,
        ) -> Result<ProcessResult, ProcessError> {
            Err(ProcessError::Cancelled {
                cmd: "1cv8".to_owned(),
                delivered: false,
            })
        }

        fn spawn(
            &self,
            _request: &ProcessRequest,
            _work: &WorkGiven,
        ) -> Result<SpawnResult, ProcessError> {
            panic!("the generation read runs synchronously")
        }
    }

    fn config() -> AppConfig {
        let dir = tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("src")).expect("src dir");
        std::fs::write(
            dir.path().join("v8project.yaml"),
            "workPath: work\nformat: DESIGNER\ninfobase:\n  connection: \"File=build/ib\"\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .expect("write config");
        load_config(
            dir.path().join("v8project.yaml").to_str(),
            None,
            &InfobaseSelector::Default,
        )
        .map(|loaded| loaded.config)
        .expect("load config")
    }

    /// Предел чтения поколения объявляет сам шаг: политика, с которой запускается
    /// Конфигуратор, несёт `GENERATION_PROBE_CAP`, а не наследует срок команды
    /// (`DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE`). Если предел убрать, тест падает.
    #[test]
    fn the_generation_read_declares_its_own_cap() {
        let config = config();
        let context = ExecutionContext::cli(CommandName::Build);
        let runner = CapturingRunner(RefCell::new(None));
        let binary = PathBuf::from("C:")
            .join("1cv8")
            .join("8.3.27.2074")
            .join("bin")
            .join("1cv8.exe");

        let answer = read_generation(
            &context,
            &config,
            || {
                Ok(GenerationProcess::Designer {
                    binary: &binary,
                    runner: &runner,
                    log_file: designer_log_file(&config, "test-cap")?,
                })
            },
            None,
        )
        .expect("a successful run of the probe");

        let policy = runner.0.borrow().as_ref().expect("the probe ran").clone();
        assert_eq!(policy.timeout, Some(GENERATION_PROBE_CAP));
        assert_eq!(policy.safety, ProcessInterruptionSafety::GracefulThenKill);
        assert_eq!(answer, None, "the platform log is missing: no answer");
    }

    /// Отмена остаётся отменой, как обещает правило: снятый отменой запуск — не
    /// отсутствие ответа.
    #[test]
    fn a_cancelled_generation_read_stays_a_failure() {
        let config = config();
        let context = ExecutionContext::cli(CommandName::Build);
        let binary = PathBuf::from("C:")
            .join("1cv8")
            .join("8.3.27.2074")
            .join("bin")
            .join("1cv8.exe");

        let error = read_generation(
            &context,
            &config,
            || {
                Ok(GenerationProcess::Designer {
                    binary: &binary,
                    runner: &CancellingRunner,
                    log_file: designer_log_file(&config, "test-cancel")?,
                })
            },
            None,
        )
        .expect_err("cancellation stays a failure");

        assert!(matches!(
            error,
            AppError::PlatformProcess(ProcessError::Cancelled { .. })
        ));
    }

    /// Предел доходит и до `ibcmd`-маршрута: политика одна на оба чтения, но у
    /// `IbcmdDsl` она проходит через `for_reading` — если предел обронен там, этот
    /// тест падает.
    #[test]
    fn the_ibcmd_generation_read_declares_its_own_cap() {
        let config = config();
        let context = ExecutionContext::cli(CommandName::Build);
        let runner = CapturingRunner(RefCell::new(None));
        let binary = PathBuf::from("/opt/1cv8/x86_64")
            .join("8.3.27.2074")
            .join("ibcmd");

        let answer = read_generation(
            &context,
            &config,
            || {
                Ok(GenerationProcess::Ibcmd {
                    binary: &binary,
                    runner: &runner,
                    data_path: None,
                })
            },
            None,
        )
        .expect("a successful run of the probe");

        let policy = runner.0.borrow().as_ref().expect("the probe ran").clone();
        assert_eq!(policy.timeout, Some(GENERATION_PROBE_CAP));
        assert_eq!(policy.safety, ProcessInterruptionSafety::GracefulThenKill);
        assert_eq!(answer, None, "stdout without a token: no answer");
    }
}
