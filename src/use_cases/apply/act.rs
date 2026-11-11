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
//! Политику чужих сеансов (`push`/`apply --sessions`, #211) владелец берёт из контекста
//! команды ([`super::sessions`]), и все пути получают её разом: отказ, при котором базу держат,
//! становится `infobase_sessions` или `infobase_designer_busy`, а `--sessions force` у сервера
//! повторяет применение с ключом завершения сеансов исполнителя.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::sessions::{self, SessionsPolicy};
use crate::config::model::AppConfig;
use crate::domain::capability::Provider;
use crate::domain::sessions::SessionsOutcome;
use crate::platform::agent::{AgentError, AgentErrorType, WaitPolicy};
use crate::platform::designer::DesignerDsl;
use crate::platform::process::ProcessRunner;
use crate::platform::session_list::{parse_session_list, SessionTerminate};
use crate::support::error::AppError;
use crate::use_cases::agent_session::{
    argument, critical_failure, run_critical_typed, AgentHandle,
};
use crate::use_cases::build_progress::{log_timeline_stage, TimelineStageStatus};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::dump_config::helpers::build_ibcmd_dsl;
use crate::use_cases::ibcmd_diagnostics::ensure_succeeded;
use crate::use_cases::interruption::{self, Deferrals};

/// Чем применяется: процесс платформы или команда в открытой сессии агента. Процессы
/// владелец строит сам — с классом прерывания критической фазы.
pub(crate) enum Applier<'a> {
    /// `1cv8 DESIGNER /UpdateDBCfg [-Extension X] -Dynamic-`; журнал платформы — `log_file`.
    Designer {
        binary: &'a Path,
        runner: &'a dyn ProcessRunner,
        log_file: PathBuf,
    },
    /// `ibcmd config apply [--extension X] --force --dynamic=disable`.
    Ibcmd {
        binary: &'a Path,
        runner: &'a dyn ProcessRunner,
    },
    /// `config update-db-cfg [--extension=X] --dynamic-disable` в сессии агента.
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
///
/// Отказ, при котором базу держат, решается по состоянию ([`super::sessions`]): без
/// `--sessions force` — отказ со списком сеансов; с ним у сервера — второе применение с ключом
/// завершения сеансов, и исход называет завершённые из списка первого отказа. Файловой базе
/// ключ не помогает (замер #211), и туда он не идёт: отказ тот же, что без `force`.
pub(crate) fn apply(
    context: &ExecutionContext,
    config: &AppConfig,
    mut applier: Applier<'_>,
    subject: &Subject<'_>,
    deferrals: &mut Deferrals,
) -> Result<(), AppError> {
    let (verb, executor) = match &applier {
        Applier::Designer { .. } => ("update_db_cfg", "Конфигуратор"),
        Applier::Ibcmd { .. } => ("ibcmd apply", "ibcmd"),
        Applier::Agent { .. } => ("update_db_cfg", "агент"),
    };
    let safe_point = format!("{verb} for {} '{}'", subject.kind.as_str(), subject.name);
    if let Some(error) = interruption::interruption_before_safe_point(context, safe_point.clone()) {
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
    let gate = context.sessions();
    let force = sessions::force_message(gate.policy(), config);
    let refused = match run(
        context,
        config,
        &mut applier,
        subject,
        deferrals,
        SessionTerminate::Keep,
    ) {
        Ok(()) => {
            if matches!(gate.policy(), SessionsPolicy::Force { .. }) {
                gate.note(SessionsOutcome {
                    terminated: Some(Vec::new()),
                    lock: sessions::session_lock(config),
                    message_delivered: false,
                });
            }
            return Ok(());
        }
        Err(refused) => refused,
    };
    if refused.error.cancellation().is_some() {
        return Err(refused.error);
    }
    let executor = refused.executor;
    let holder = sessions::holder(config, executor, &refused.evidence());
    let Some(message) = force else {
        return Err(refuse(gate, config, holder, refused.error));
    };
    // Сервер: сеансы завершает ключ исполнителя. У агента признак «держат сеансы» —
    // `error-type`; у Конфигуратора и `ibcmd` сервера признака нет, и второе применение идёт
    // после любого их отказа: с `--sessions force` платформа завершает сеансы, если монопольный
    // доступ нужен, а отказ по другой причине повторится и дойдёт до команды.
    let terminated = match holder {
        Some(sessions::Holder::Sessions(list)) => list,
        Some(sessions::Holder::Designer) => {
            return Err(refuse(gate, config, holder, refused.error));
        }
        None if executor == sessions::Executor::Agent => return Err(refused.error),
        None => refused.text.as_deref().and_then(parse_session_list),
    };
    if let Some(error) = interruption::interruption_before_safe_point(context, safe_point) {
        return Err(error);
    }
    // Ключ отдан платформе: исход называет сеансы, которые она завершала, и при удаче, и при
    // отказе второго запуска.
    gate.note(SessionsOutcome {
        terminated,
        lock: sessions::session_lock(config),
        message_delivered: matches!(applier, Applier::Ibcmd { .. }),
    });
    run(
        context,
        config,
        &mut applier,
        subject,
        deferrals,
        SessionTerminate::Force { message: &message },
    )
    .map_err(|refused| {
        if refused.error.cancellation().is_some() {
            return refused.error;
        }
        let holder = sessions::holder(config, refused.executor, &refused.evidence());
        refuse(gate, config, holder, refused.error)
    })
}

/// Отказ, при котором базу держат, — отказ «базу держат»; иначе — отказ исполнителя как есть.
fn refuse(
    gate: &sessions::SessionsGate,
    config: &AppConfig,
    holder: Option<sessions::Holder>,
    error: AppError,
) -> AppError {
    match holder {
        Some(holder) => sessions::refusal(gate, config, holder, &error),
        None => error,
    }
}

/// Отказ одного запуска исполнителя и то, что из него берёт политика сеансов.
struct Refused {
    error: AppError,
    executor: sessions::Executor,
    /// Текст отказа: `/Out` Конфигуратора, `message` агента. Источник списка, а не признак.
    text: Option<String>,
    /// `error-type` агента.
    agent_error: Option<AgentErrorType>,
    /// Агент ответил вопросом, и раннер ответил «Отмена».
    agent_asked: bool,
}

impl Refused {
    fn process(error: AppError, text: Option<String>) -> Self {
        Self {
            error,
            executor: sessions::Executor::Process,
            text,
            agent_error: None,
            agent_asked: false,
        }
    }

    fn evidence(&self) -> sessions::Evidence<'_> {
        sessions::Evidence {
            text: self.text.as_deref(),
            agent_error: self.agent_error.as_ref(),
            agent_asked: self.agent_asked,
        }
    }
}

impl From<AppError> for Refused {
    fn from(error: AppError) -> Self {
        Self::process(error, None)
    }
}

/// Один запуск исполнителя с данным ключом сеансов.
fn run(
    context: &ExecutionContext,
    config: &AppConfig,
    applier: &mut Applier<'_>,
    subject: &Subject<'_>,
    deferrals: &mut Deferrals,
    sessions: SessionTerminate<'_>,
) -> Result<(), Refused> {
    match applier {
        Applier::Designer {
            binary,
            runner,
            log_file,
        } => {
            let result = DesignerDsl::new(
                binary.to_path_buf(),
                config.v8_connection(),
                *runner,
                Some(log_file.clone()),
                context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None),
            )
            .update_db_cfg(subject.extension, sessions)
            .map_err(AppError::from)?;
            deferrals.note_result("update_db_cfg", &result);
            ensure_succeeded(
                "update_db_cfg",
                subject.kind.as_str(),
                subject.name,
                &result,
            )
            .map_err(|error| Refused::process(error, result.platform_log.clone()))
        }
        Applier::Ibcmd { binary, runner } => {
            let result = build_ibcmd_dsl(
                context,
                config,
                binary,
                *runner,
                InterruptionSafetyClass::CriticalNonAbortable,
            )?
            .config_apply(subject.extension, sessions)
            .map_err(AppError::from)?;
            deferrals.note_result("apply", &result);
            ensure_succeeded("apply", subject.kind.as_str(), subject.name, &result)
                .map_err(Refused::from)
        }
        Applier::Agent { handle, wait } => {
            let command = agent_update_command(subject.extension, sessions);
            let error = match run_critical_typed(handle, "update_db_cfg", &command, wait, deferrals)
            {
                Ok(_) => return Ok(()),
                Err(error) => error,
            };
            let (text, agent_error, agent_asked) = match &error {
                AgentError::Command {
                    error_type,
                    message,
                } => (Some(message.clone()), Some(error_type.clone()), false),
                AgentError::Question { message } => (Some(message.clone()), None, true),
                _ => (None, None, false),
            };
            if agent_asked {
                // Агент ждёт ответа на «Отмена»/«Повторить»: без него следующая строка сессии
                // ушла бы ответом. «Отмена» (`1`) возвращает сессию к командам (замер #211).
                let _ = handle.run("1", wait);
            }
            Err(Refused {
                error: critical_failure("update_db_cfg", error),
                executor: sessions::Executor::Agent,
                text,
                agent_error,
                agent_asked,
            })
        }
    }
}

/// `config update-db-cfg [--extension=X] --dynamic-disable [--session-terminate=force]`.
/// Динамическое обновление запрещено всегда, как у Конфигуратора (`-Dynamic-`) и `ibcmd`
/// (`--dynamic=disable`): без ключа платформа его разрешает
/// (`INV.USE-CASES.A-DB-UPDATE-NEVER-GOES-DYNAMIC`). Ключ завершения сеансов идёт только к
/// серверу: у файловой базы агент на него не отказывает, а спрашивает «Отмена»/«Повторить» и
/// ждёт ответа (замер #211). Сообщение (`--session-terminate-message`) агент знает только с
/// 8.3.27, и раннер его агенту не передаёт.
fn agent_update_command(extension: Option<&str>, sessions: SessionTerminate<'_>) -> String {
    let mut command = String::from("config update-db-cfg");
    if let Some(extension) = extension {
        // Запись в `String` не отказывает.
        let _ = write!(command, " --extension={}", argument(extension));
    }
    command.push_str(" --dynamic-disable");
    if let SessionTerminate::Force { .. } = sessions {
        command.push_str(" --session-terminate=force");
    }
    command
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use tempfile::tempdir;

    use super::{agent_update_command, apply, Applier, Subject, SubjectKind};
    use crate::config::model::{AppConfig, InfobaseConfig, SourceFormat, TestsConfig, ToolsConfig};
    use crate::domain::next_step::NextStep;
    use crate::domain::sessions::SessionLockState;
    use crate::platform::process::{
        ProcessError, ProcessExecutionPolicy, ProcessInterruptionSafety, ProcessRequest,
        ProcessResult, ProcessRunner, SpawnResult, WorkGiven,
    };
    use crate::platform::session_list::tests::MEASURED_LIST_RU;
    use crate::platform::session_list::SessionTerminate;
    use crate::use_cases::apply::sessions::{SessionsGate, SessionsPolicy};
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
            local_path: dir.join("base").join(".local"),
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

    /// Агент обновляет конфигурацию базы данных только с запретом динамического обновления —
    /// у основной конфигурации и у расширения (`INV.USE-CASES.A-DB-UPDATE-NEVER-GOES-DYNAMIC`).
    #[test]
    fn an_agent_update_always_forbids_dynamic_update() {
        assert_eq!(
            agent_update_command(None, SessionTerminate::Keep),
            "config update-db-cfg --dynamic-disable"
        );
        assert_eq!(
            agent_update_command(Some("ext"), SessionTerminate::Keep),
            "config update-db-cfg --extension=ext --dynamic-disable"
        );
        assert_eq!(
            agent_update_command(None, SessionTerminate::Force { message: "m" }),
            "config update-db-cfg --dynamic-disable --session-terminate=force"
        );
    }

    /// Поддельный Конфигуратор: ответы по очереди — код выхода и текст `/Out`.
    struct Scripted {
        answers: Mutex<Vec<(i32, &'static str)>>,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl Scripted {
        fn new(answers: Vec<(i32, &'static str)>) -> Self {
            Self {
                answers: Mutex::new(answers),
                calls: Mutex::default(),
            }
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().expect("calls").clone()
        }
    }

    impl ProcessRunner for Scripted {
        fn run_with_policy(
            &self,
            request: &ProcessRequest,
            policy: &ProcessExecutionPolicy,
        ) -> Result<ProcessResult, ProcessError> {
            policy.mark_started_for_test();
            self.calls.lock().expect("calls").push(request.args.clone());
            let (exit_code, out) = self.answers.lock().expect("answers").remove(0);
            if let Some(at) = request.args.iter().position(|arg| arg == "/Out") {
                std::fs::write(&request.args[at + 1], out).expect("/Out");
            }
            Ok(ProcessResult {
                exit_code,
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

    fn server_config(dir: &Path) -> AppConfig {
        AppConfig {
            infobase: InfobaseConfig::file("Srvr=host;Ref=ib"),
            ..config(dir)
        }
    }

    fn designer_apply(
        context: &ExecutionContext,
        config: &AppConfig,
        runner: &Scripted,
        dir: &Path,
    ) -> Result<(), crate::support::error::AppError> {
        std::fs::create_dir_all(dir.join("logs")).expect("logs");
        collecting_deferrals(|deferrals| {
            apply(
                context,
                config,
                Applier::Designer {
                    binary: Path::new("1cv8"),
                    runner,
                    log_file: dir.join("logs").join("update.log"),
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
        .map(|_| ())
    }

    fn forced(command: &str) -> ExecutionContext {
        ExecutionContext::cli(CommandName::Apply).with_sessions(SessionsGate::new(
            SessionsPolicy::Force {
                message: "bye".to_owned(),
            },
            NextStep::command(command),
        ))
    }

    /// Сервер, `--sessions force`: первый отказ даёт список, второе применение идёт с
    /// `-SessionTerminate force`, а исход называет завершённые сеансы из списка до завершения
    /// (`INV.CLI.APPLY-SESSIONS-GOVERNS-FOREIGN-SESSIONS`).
    #[test]
    fn a_server_apply_with_force_ends_the_sessions_and_names_them() {
        let dir = tempdir().expect("tempdir");
        let config = server_config(dir.path());
        let runner = Scripted::new(vec![(1, MEASURED_LIST_RU), (0, "")]);
        let context = forced("apply");

        designer_apply(&context, &config, &runner, dir.path()).expect("applied");

        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert!(!calls[0].iter().any(|arg| arg == "-SessionTerminate"));
        assert!(calls[1].ends_with(&["-SessionTerminate".to_owned(), "force".to_owned()]));
        let outcome = context.sessions().outcome().expect("outcome");
        let terminated = outcome.terminated.expect("terminated");
        assert_eq!(terminated.len(), 2);
        assert_eq!(terminated[0].application_title, "Тонкий клиент");
        assert_eq!(outcome.lock.state, SessionLockState::Unknown);
        assert!(
            !outcome.message_delivered,
            "the Designer key carries no message"
        );
    }

    /// Без `force` отказ сервера у Конфигуратора признака «держат сеансы» не несёт: решать по
    /// тексту нельзя, и отказ доходит до команды как отказ платформы, без второго запуска.
    #[test]
    fn a_server_apply_without_force_keeps_the_platform_refusal() {
        let dir = tempdir().expect("tempdir");
        let config = server_config(dir.path());
        let runner = Scripted::new(vec![(1, MEASURED_LIST_RU)]);

        let error = designer_apply(
            &ExecutionContext::cli(CommandName::Apply),
            &config,
            &runner,
            dir.path(),
        )
        .expect_err("refused");

        assert_eq!(runner.calls().len(), 1);
        assert!(
            matches!(error, crate::support::error::AppError::Platform(_)),
            "{error:?}"
        );
    }

    /// Применение, которому никто не мешал, с `force` называет пустой список завершённых.
    #[test]
    fn a_forced_apply_nobody_held_terminates_nothing() {
        let dir = tempdir().expect("tempdir");
        let config = server_config(dir.path());
        let runner = Scripted::new(vec![(0, "")]);
        let context = forced("push");

        designer_apply(&context, &config, &runner, dir.path()).expect("applied");

        assert_eq!(runner.calls().len(), 1);
        let outcome = context.sessions().outcome().expect("outcome");
        assert_eq!(outcome.terminated, Some(Vec::new()));
    }

    /// Файловую базу держит сеанс (файл открыт, как его держит клиент на Windows): отказ
    /// `infobase_sessions` со списком из `/Out`, `terminable: false`, и `force` ключа
    /// платформе не передаёт — у файловой базы он сеанс не завершает (замер #211).
    #[cfg(windows)]
    #[test]
    fn a_file_base_held_by_a_session_is_refused_with_its_list_even_with_force() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("ib");
        std::fs::create_dir_all(&base).expect("base");
        std::fs::write(base.join("1Cv8.1CD"), b"db").expect("db");
        let config = AppConfig {
            infobase: InfobaseConfig::file(format!("File={}", base.display())),
            ..config(dir.path())
        };
        let _client = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2)
            .open(base.join("1Cv8.1CD"))
            .expect("client handle");

        let plain = ExecutionContext::cli(CommandName::Apply).with_sessions(SessionsGate::new(
            SessionsPolicy::Disable,
            NextStep::command("apply"),
        ));
        for context in [plain, forced("apply")] {
            let runner = Scripted::new(vec![(1, MEASURED_LIST_RU)]);
            let error =
                designer_apply(&context, &config, &runner, dir.path()).expect_err("refused");

            assert_eq!(runner.calls().len(), 1, "no second run with the key");
            let refusal = crate::use_cases::result::UseCaseError::from(error);
            assert_eq!(
                refusal.kind(),
                crate::use_cases::result::UseCaseErrorKind::InfobaseSessions
            );
            let sessions = refusal.sessions().expect("sessions");
            assert!(!sessions.terminable);
            assert_eq!(sessions.list.as_ref().map(Vec::len), Some(2));
            assert_eq!(sessions.lock.state, SessionLockState::None);
            let next = refusal.next().expect("next");
            assert_eq!(next.command, "apply");
            assert!(
                next.keys.is_empty(),
                "a file base offers no force: {next:?}"
            );
        }
    }

    /// Файловую базу держит Конфигуратор: отдельный отказ без списка и без `force`.
    #[cfg(windows)]
    #[test]
    fn a_file_base_open_in_the_designer_is_refused_as_designer_busy() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempdir().expect("tempdir");
        let base = dir.path().join("ib");
        std::fs::create_dir_all(&base).expect("base");
        std::fs::write(base.join("1Cv8.1CD"), b"db").expect("db");
        std::fs::write(base.join("1Cv8.cgr"), b"cgr").expect("cgr");
        let config = AppConfig {
            infobase: InfobaseConfig::file(format!("File={}", base.display())),
            ..config(dir.path())
        };
        let _designer = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(base.join("1Cv8.cgr"))
            .expect("designer handle");
        let runner = Scripted::new(vec![(
            1,
            "Ошибка блокировки информационной базы для конфигурирования.",
        )]);

        let error = designer_apply(
            &ExecutionContext::cli(CommandName::Apply).with_sessions(SessionsGate::new(
                SessionsPolicy::Disable,
                NextStep::command("apply"),
            )),
            &config,
            &runner,
            dir.path(),
        )
        .expect_err("refused");

        let refusal = crate::use_cases::result::UseCaseError::from(error);
        assert_eq!(
            refusal.kind(),
            crate::use_cases::result::UseCaseErrorKind::InfobaseDesignerBusy
        );
        let sessions = refusal.sessions().expect("sessions");
        assert_eq!(sessions.list, None);
        assert!(!sessions.terminable);
        assert!(refusal.next().expect("next").keys.is_empty());
    }
}
