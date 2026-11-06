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

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::config::model::AppConfig;
use crate::domain::capability::Provider;
use crate::platform::agent::WaitPolicy;
use crate::platform::designer::DesignerDsl;
use crate::platform::ibcmd::DynamicUpdateMode;
use crate::platform::process::ProcessRunner;
use crate::support::error::AppError;
use crate::use_cases::agent_session::{argument, run_critical, AgentHandle};
use crate::use_cases::build_progress::{log_timeline_stage, TimelineStageStatus};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::dump_config::helpers::build_ibcmd_dsl;
use crate::use_cases::ibcmd_diagnostics::ensure_succeeded;
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
                context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None),
            )
            .update_db_cfg(subject.extension)
            .map_err(AppError::from)?;
            deferrals.note_result("update_db_cfg", &result);
            ensure_succeeded(
                "update_db_cfg",
                subject.kind.as_str(),
                subject.name,
                &result,
            )
        }
        Applier::Ibcmd { binary, runner } => {
            let result = build_ibcmd_dsl(
                context,
                config,
                binary,
                runner,
                InterruptionSafetyClass::CriticalNonAbortable,
            )?
            .config_apply(subject.extension, DynamicUpdateMode::Auto)
            .map_err(AppError::from)?;
            deferrals.note_result("apply", &result);
            ensure_succeeded("apply", subject.kind.as_str(), subject.name, &result)
        }
        Applier::Agent { handle, wait } => {
            let mut command = String::from("config update-db-cfg");
            if let Some(extension) = subject.extension {
                // Запись в `String` не отказывает.
                let _ = write!(command, " --extension={}", argument(extension));
            }
            run_critical(handle, "update_db_cfg", &command, wait, deferrals).map(|_| ())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use tempfile::tempdir;

    use super::{apply, Applier, Subject, SubjectKind};
    use crate::config::model::{AppConfig, InfobaseConfig, SourceFormat, TestsConfig, ToolsConfig};
    use crate::platform::process::{
        ProcessError, ProcessExecutionPolicy, ProcessInterruptionSafety, ProcessRequest,
        ProcessResult, ProcessRunner, SpawnResult, WorkGiven,
    };
    use crate::use_cases::context::{CommandName, ExecutionContext};
    use crate::use_cases::interruption::collecting_deferrals;

    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<(Vec<String>, ProcessInterruptionSafety)>>,
    }

    impl ProcessRunner for Recorder {
        fn run_with_policy(
            &self,
            request: &ProcessRequest,
            policy: &ProcessExecutionPolicy,
        ) -> Result<ProcessResult, ProcessError> {
            policy.mark_started_for_test();
            self.calls
                .lock()
                .expect("calls")
                .push((request.args.clone(), policy.safety));
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
            unreachable!("an apply runs to its end")
        }
    }

    fn config(dir: &Path) -> AppConfig {
        AppConfig {
            base_path: dir.join("base"),
            work_path: dir.join("work"),
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: InfobaseConfig::file("File=/tmp/ib"),
            infobases: Default::default(),
            infobase_name: None,
            working_copy: None,
            source_sets: Vec::new(),
            tools: ToolsConfig::default(),
            mcp: Default::default(),
            tests: TestsConfig::default(),
        }
    }

    /// Применение через `ibcmd` держит каталог данных проекта — параллельный запуск в другом
    /// проекте не упрётся в его блокировку (`rc=254`) — и остаётся критической фазой: общий
    /// каталог не снимает класс прерывания записи в базу.
    #[test]
    fn an_ibcmd_apply_holds_the_data_directory_of_the_project_as_a_critical_phase() {
        let dir = tempdir().expect("tempdir");
        let config = config(dir.path());
        let runner = Recorder::default();

        collecting_deferrals(|deferrals| {
            apply(
                &ExecutionContext::cli(CommandName::Apply),
                &config,
                Applier::Ibcmd {
                    binary: &PathBuf::from("/tmp/fake-ibcmd"),
                    runner: &runner,
                },
                &Subject {
                    kind: SubjectKind::SourceSet,
                    name: "main",
                    extension: None,
                    timeline: "main",
                },
                deferrals,
            )
        })
        .expect("applied");

        let calls = runner.calls.lock().expect("calls");
        let [(args, safety)] = calls.as_slice() else {
            panic!("one call expected: {calls:?}");
        };
        let data = config.work_path.join("ibcmd-data");
        assert!(data.is_dir(), "{}", data.display());
        assert_eq!(
            args[..3],
            [
                "infobase".to_owned(),
                "--data".to_owned(),
                data.display().to_string()
            ],
            "{args:?}"
        );
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "config" && pair[1] == "apply"),
            "{args:?}"
        );
        assert_eq!(*safety, ProcessInterruptionSafety::CriticalNonAbortable);
    }
}
