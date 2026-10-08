use std::path::PathBuf;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::domain::capability::SessionEndpoint;
use crate::platform::process::{ProcessExecutionPolicy, ProcessInterruptionSafety, WorkGiven};

/// Identifies the logical command being executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandName {
    Bootstrap,
    ToolsDownload,
    Init,
    Extensions,
    Build,
    Apply,
    Reset,
    Load,
    Test,
    Dump,
    InfobaseConfigurationExport,
    InfobaseDump,
    InfobaseRestore,
    Convert,
    Artifacts,
    Syntax,
    Launch,
    Publish,
    Status,
}

impl CommandName {
    /// Returns the stable command label used in logs and CLI envelopes.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bootstrap => "clone",
            Self::ToolsDownload => "tools download",
            Self::Init => "infobase create",
            Self::Extensions => "extensions",
            Self::Build => "push",
            Self::Apply => "apply",
            Self::Reset => "reset",
            Self::Load => "upload",
            Self::Test => "test",
            Self::Dump => "pull",
            Self::InfobaseConfigurationExport => "download",
            Self::InfobaseDump => "infobase.dump",
            Self::InfobaseRestore => "infobase.restore",
            Self::Convert => "convert",
            Self::Artifacts => "make",
            Self::Syntax => "check",
            Self::Launch => "launch",
            Self::Publish => "publish",
            Self::Status => "status",
        }
    }
}

/// Describes the transport that invoked the use case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionTransport {
    /// Invocation from the existing CLI surface.
    Cli,
    /// Invocation from MCP over stdio.
    McpStdio,
    /// Invocation from MCP over HTTP.
    McpHttp,
}

/// Global keys of a command line that reach the same project, infobase and work directory
/// as this run.
///
/// A refusal that names a command to run must name one that hits the same target when it
/// is executed literally — from another directory, or for an MCP server started with
/// `--infobase`. The transport knows how it was started; the use case only appends them.
///
/// A connection string never gets here: the loader refuses credentials in it, yet the
/// address of a foreign base and parameters the loader does not know stay in it, and an
/// advice is shown, logged and pasted. With
/// [`AdvisedInfobase::SameConnection`] the advice asks for "the same `--infobase` value"
/// instead of repeating it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandLineTarget {
    /// Absolute path of the primary `v8project.yaml`. `None` where it did not resolve (and
    /// in unit tests, where nothing was loaded from a file): the advice then names the
    /// project directory as the place to run it from instead of `--config`.
    pub config: Option<PathBuf>,
    /// How the advice names the base when the run did not select the default one.
    pub infobase: Option<AdvisedInfobase>,
    /// The effective work directory when `--workdir` overrode it.
    pub workdir: Option<PathBuf>,
}

/// How a refusal advice names the base that `--infobase` selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdvisedInfobase {
    /// A base declared in the project, repeated as `--infobase <name>`.
    Name(String),
    /// A connection string: the advice asks for the same `--infobase` value in words and
    /// never repeats the string.
    SameConnection,
}

impl CommandLineTarget {
    /// `v8-runner <global keys> <tail>`, with every value quoted for the shell of this
    /// platform where it needs quoting (see [`shell_word`]). A connection string is not
    /// among the keys: [`ExecutionContext::advised_command`] asks for it in words.
    pub fn command(&self, tail: &str) -> String {
        let infobase = match &self.infobase {
            Some(AdvisedInfobase::Name(name)) => Some(name.clone()),
            Some(AdvisedInfobase::SameConnection) | None => None,
        };
        let keys = [
            ("--config", self.config.as_deref().map(path_text)),
            ("--infobase", infobase),
            ("--workdir", self.workdir.as_deref().map(path_text)),
        ];
        let global_keys = keys
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| format!("{key} {}", shell_word(&value))));
        std::iter::once("v8-runner".to_owned())
            .chain(global_keys)
            .chain(std::iter::once(tail.to_owned()))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn path_text(path: &std::path::Path) -> String {
    path.display().to_string()
}

/// One shell word: as is when it holds nothing the shell would split, expand or unescape,
/// otherwise quoted for the shell of this platform.
///
/// On Unix the advice is for a POSIX shell: single quotes, and a backslash is not plain,
/// because unquoted `sh` drops it. On Windows the same text has to run in PowerShell and in
/// `cmd`: double quotes are the only quoting both understand, and a backslash is an
/// ordinary path character in both. Inside double quotes PowerShell still expands `$` and
/// `` ` `` and `cmd` expands `%`; no quoting serves both shells for those. The words quoted
/// here are paths and source-set names, and a Windows path cannot hold `"`.
pub(crate) fn shell_word(value: &str) -> String {
    let plain = !value.is_empty() && value.chars().all(is_plain_shell_char);
    if plain {
        value.to_owned()
    } else if cfg!(windows) {
        format!("\"{value}\"")
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

fn is_plain_shell_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '_' | '-' | '.' | '/' | ':' | '=' | '+' | ',' | '@' | '%'
        )
        || (cfg!(windows) && ch == '\\')
}

/// Command-boundary interruption signal observed at safe points.
///
/// A command carries no deadline (DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE), so the only
/// thing that interrupts one at a safe point is the operator. A step that overruns its own
/// cap is a different signal and arrives as `ProcessInterruptionReason::TimedOut`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionInterruption {
    Cancelled,
}

/// Command-level interruption safety contract from DEC.2026-04-20.A-MUTATING-CRITICAL-PHASE-IS-NOT-HARD-KILLED.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptionSafetyClass {
    Interruptible,
    GracefulThenKill,
    CriticalNonAbortable,
    NoExternalProcess,
}

impl InterruptionSafetyClass {
    /// Returns the subset of process-runner safety semantics for commands that spawn a child.
    pub const fn process_safety(self) -> ProcessInterruptionSafety {
        match self {
            Self::Interruptible => ProcessInterruptionSafety::Interruptible,
            Self::GracefulThenKill => ProcessInterruptionSafety::GracefulThenKill,
            Self::CriticalNonAbortable | Self::NoExternalProcess => {
                ProcessInterruptionSafety::CriticalNonAbortable
            }
        }
    }
}

/// Result of a critical in-process phase that must complete without mid-phase aborts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CriticalPhaseResult<T> {
    pub value: T,
    pub deferred_interruption: Option<ExecutionInterruption>,
}

/// Per-invocation metadata passed into transport-neutral use cases.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    command: CommandName,
    transport: ExecutionTransport,
    edt_timeout: Option<Duration>,
    cancellation: CancellationToken,
    /// Получил ли исполнитель работу этой команды; отмечает платформа, читает ответ.
    work: WorkGiven,
    /// Точка входа сессии агента, которую открыла эта команда; отмечает подключение,
    /// читает квитанция исполнителя. Клоны контекста делят этот слот, как и `work`:
    /// клон для другой команды унаследовал бы отметку.
    session: std::sync::Arc<std::sync::Mutex<Option<SessionEndpoint>>>,
    /// Глобальные ключи командной строки, которые ведут к той же цели.
    command_line: CommandLineTarget,
}

impl ExecutionContext {
    /// Creates an execution context for the specified command and transport.
    pub fn new(command: CommandName, transport: ExecutionTransport) -> Self {
        Self {
            command,
            transport,
            edt_timeout: None,
            cancellation: CancellationToken::new(),
            work: WorkGiven::for_command(),
            session: std::sync::Arc::default(),
            command_line: CommandLineTarget::default(),
        }
    }

    /// Creates a CLI execution context for the specified command.
    pub fn cli(command: CommandName) -> Self {
        Self::new(command, ExecutionTransport::Cli)
    }

    /// Creates an MCP stdio execution context for the specified command.
    #[cfg(test)]
    pub fn mcp_stdio(command: CommandName) -> Self {
        Self::new(command, ExecutionTransport::McpStdio)
    }

    /// Creates an MCP HTTP execution context for the specified command.
    #[cfg(test)]
    pub fn mcp_http(command: CommandName) -> Self {
        Self::new(command, ExecutionTransport::McpHttp)
    }

    /// Returns the command being executed.
    pub const fn command(&self) -> CommandName {
        self.command
    }

    /// Returns the transport that initiated this execution.
    pub const fn transport(&self) -> ExecutionTransport {
        self.transport
    }

    /// Attaches an EDT subprocess timeout budget to the execution context.
    pub fn with_edt_timeout(mut self, edt_timeout: Option<Duration>) -> Self {
        self.edt_timeout = edt_timeout;
        self
    }

    /// Attaches a cancellation token shared with the caller transport.
    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    /// Attaches the global keys a command line needs to reach the same target.
    pub fn with_command_line(mut self, command_line: CommandLineTarget) -> Self {
        self.command_line = command_line;
        self
    }

    /// `` `v8-runner <global keys> <tail>` `` and where to run it, for a refusal that
    /// advises a command: the command reaches the same target as this run, and an MCP
    /// client learns that it runs from the command line, over HTTP on the server's machine.
    ///
    /// Without a resolved config path the advice says to run it from the project directory
    /// instead of naming `--config`; a base selected by a connection string is asked for as
    /// "the same `--infobase` value", and the string itself is never repeated.
    pub fn advised_command(&self, tail: &str) -> String {
        let command = self.command_line.command(tail);
        let config_known = self.command_line.config.is_some();
        let place = match (self.transport, config_known) {
            (ExecutionTransport::Cli, true) => "",
            (ExecutionTransport::Cli, false) => " from the project directory",
            (ExecutionTransport::McpStdio, true) => " from the command line",
            (ExecutionTransport::McpStdio, false) => " from the command line in the project directory",
            (ExecutionTransport::McpHttp, true) => {
                " from the command line on the machine where the MCP server runs"
            }
            (ExecutionTransport::McpHttp, false) => {
                " from the command line in the project directory on the machine where the MCP server runs"
            }
        };
        let same_connection = match (&self.command_line.infobase, self.transport) {
            (Some(AdvisedInfobase::SameConnection), ExecutionTransport::Cli) => {
                ", with the same `--infobase` value as this command"
            }
            (
                Some(AdvisedInfobase::SameConnection),
                ExecutionTransport::McpStdio | ExecutionTransport::McpHttp,
            ) => ", with the same `--infobase` value the MCP server was started with",
            (Some(AdvisedInfobase::Name(_)) | None, _) => "",
        };
        format!("`{command}`{place}{same_connection}")
    }

    /// [`Self::advised_command`] for `pull <SET> --force`: the one spelling of the full
    /// replacement of a source set that refusals send the caller to.
    pub fn advised_pull_force(&self, source_set: &str) -> String {
        self.advised_command(&format!("pull {} --force", shell_word(source_set)))
    }

    /// Returns the EDT subprocess timeout budget for this execution.
    pub const fn edt_timeout(&self) -> Option<Duration> {
        self.edt_timeout
    }

    /// Returns the shared cancellation token for this execution.
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Builds a process policy bounded by the step's own cap, if the step declares one.
    ///
    /// There is no command budget to cap it against: see
    /// DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE. A step that passes `None` runs until it
    /// reaches a terminal outcome.
    pub fn process_policy(
        &self,
        safety: InterruptionSafetyClass,
        timeout_cap: Option<Duration>,
    ) -> ProcessExecutionPolicy {
        ProcessExecutionPolicy::new(
            timeout_cap,
            self.cancellation(),
            safety.process_safety(),
            self.work.clone(),
        )
    }

    /// Отметка работы исполнителя для этой команды: `provider_dispatched` ответа.
    pub fn work(&self) -> &WorkGiven {
        &self.work
    }

    /// Отмечает точку входа открытой сессии агента. Сессий у команды бывает несколько,
    /// но конфиг у неё один, и точка входа у них общая: первая отметка остаётся.
    pub(crate) fn note_session(&self, endpoint: SessionEndpoint) {
        self.session_slot().get_or_insert(endpoint);
    }

    /// Точка входа сессии агента, если команда её открывала.
    pub(crate) fn opened_session(&self) -> Option<SessionEndpoint> {
        self.session_slot().clone()
    }

    /// Отметка — простое значение: паника другого потока его не портит.
    fn session_slot(&self) -> std::sync::MutexGuard<'_, Option<SessionEndpoint>> {
        self.session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Returns the pending command-boundary interruption, if any.
    ///
    /// The operator's interrupt is the only thing that ends a command early.
    pub fn interruption(&self) -> Option<ExecutionInterruption> {
        self.cancellation
            .is_cancelled()
            .then_some(ExecutionInterruption::Cancelled)
    }

    /// Runs a non-process critical phase and reports whether interruption was deferred until
    /// after the operation completed.
    pub fn run_no_process_critical_phase<T, E>(
        &self,
        operation: impl FnOnce() -> Result<T, E>,
    ) -> Result<CriticalPhaseResult<T>, E> {
        let value = operation()?;
        Ok(CriticalPhaseResult {
            value,
            deferred_interruption: self.interruption(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio_util::sync::CancellationToken;

    use crate::platform::process::ProcessInterruptionSafety;

    use std::path::PathBuf;

    use super::{
        AdvisedInfobase, CommandLineTarget, CommandName, ExecutionContext, ExecutionInterruption,
        ExecutionTransport, InterruptionSafetyClass,
    };

    /// Команда называет только те глобальные ключи, что меняют цель, и каждое значение
    /// переживает оболочку POSIX: пробел, кавычка и обратная косая черта не дробят слово.
    #[cfg(not(windows))]
    #[test]
    fn a_command_line_names_the_global_keys_of_the_target() {
        assert_eq!(
            CommandLineTarget::default().command("pull main --force"),
            "v8-runner pull main --force"
        );
        let elsewhere = CommandLineTarget {
            config: Some(PathBuf::from("/srv/it's mine/v8project.yaml")),
            infobase: Some(AdvisedInfobase::Name("staging".to_owned())),
            workdir: Some(PathBuf::from(r"/var/C:\work")),
        };
        assert_eq!(
            elsewhere.command("pull main --force"),
            r"v8-runner --config '/srv/it'\''s mine/v8project.yaml' --infobase staging --workdir '/var/C:\work' pull main --force"
        );
    }

    /// На Windows совет выполняют PowerShell и `cmd`: значение с пробелом — в двойных
    /// кавычках, которые понимают обе оболочки, обратная косая черта — обычный знак пути.
    #[cfg(windows)]
    #[test]
    fn a_command_line_names_the_global_keys_of_the_target() {
        let elsewhere = CommandLineTarget {
            config: Some(PathBuf::from(r"C:\it's mine\v8project.yaml")),
            infobase: Some(AdvisedInfobase::Name("staging".to_owned())),
            workdir: Some(PathBuf::from(r"C:\work")),
        };
        assert_eq!(
            elsewhere.command("pull main --force"),
            r#"v8-runner --config "C:\it's mine\v8project.yaml" --infobase staging --workdir C:\work pull main --force"#
        );
    }

    /// Строка соединения несёт адрес чужой базы и параметры, которых загрузчик не знает, и
    /// совет её не повторяет ни в одном транспорте: просит то же значение `--infobase`
    /// словами.
    #[test]
    fn an_advice_never_repeats_a_connection_string() {
        let target = CommandLineTarget {
            config: Some(PathBuf::from("/srv/project/v8project.yaml")),
            infobase: Some(AdvisedInfobase::SameConnection),
            workdir: None,
        };
        for context in [
            ExecutionContext::cli(CommandName::Dump),
            ExecutionContext::mcp_stdio(CommandName::Dump),
            ExecutionContext::mcp_http(CommandName::Dump),
        ] {
            let advice = context
                .with_command_line(target.clone())
                .advised_pull_force("main");
            assert!(!advice.contains("--infobase "), "{advice}");
            assert!(
                advice.contains("with the same `--infobase` value"),
                "{advice}"
            );
        }
    }

    /// Путь конфига не разрешился — готовой команды с `--config` нет, и совет называет
    /// каталог проекта местом запуска.
    #[test]
    fn an_advice_without_a_config_path_names_the_project_directory() {
        let cli = ExecutionContext::cli(CommandName::Dump).advised_pull_force("main");
        assert_eq!(
            cli,
            "`v8-runner pull main --force` from the project directory"
        );
        let http = ExecutionContext::mcp_http(CommandName::Dump).advised_pull_force("main");
        assert_eq!(
            http,
            "`v8-runner pull main --force` from the command line in the project directory on the machine where the MCP server runs"
        );
    }

    #[test]
    fn constructs_mcp_contexts() {
        let cancellation = CancellationToken::new();
        let stdio = ExecutionContext::mcp_stdio(CommandName::Build)
            .with_edt_timeout(Some(Duration::from_secs(5)))
            .with_cancellation(cancellation.clone());
        let http = ExecutionContext::mcp_http(CommandName::Test);

        assert_eq!(stdio.command(), CommandName::Build);
        assert_eq!(stdio.transport(), ExecutionTransport::McpStdio);
        assert_eq!(stdio.edt_timeout(), Some(Duration::from_secs(5)));
        assert_eq!(http.command(), CommandName::Test);
        assert_eq!(http.transport(), ExecutionTransport::McpHttp);
        assert_eq!(http.edt_timeout(), None);
        assert_eq!(stdio.interruption(), None);
        cancellation.cancel();
        assert_eq!(stdio.interruption(), Some(ExecutionInterruption::Cancelled));
    }

    /// DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE, and the guard against bringing one back.
    ///
    /// `ExecutionContext` has no deadline to set, so a step's policy can only ever carry the
    /// cap that step itself declared. A step that declares none runs until it reaches a
    /// terminal outcome. Should anyone reintroduce a command budget, they have to add a way
    /// to put it here first, and this assertion is what it collides with.
    #[test]
    fn a_step_policy_carries_the_steps_own_cap_and_nothing_above_it() {
        let context = ExecutionContext::cli(CommandName::Build);

        let declared = context.process_policy(
            InterruptionSafetyClass::GracefulThenKill,
            Some(Duration::from_millis(100)),
        );
        assert_eq!(declared.timeout, Some(Duration::from_millis(100)));
        assert_eq!(declared.safety, ProcessInterruptionSafety::GracefulThenKill);

        let undeclared =
            context.process_policy(InterruptionSafetyClass::CriticalNonAbortable, None);
        assert_eq!(
            undeclared.timeout, None,
            "a step that declares no cap must not inherit one from the command"
        );
    }

    #[test]
    fn the_operators_interrupt_is_the_only_command_boundary_interruption() {
        let cancellation = CancellationToken::new();
        let context =
            ExecutionContext::cli(CommandName::Test).with_cancellation(cancellation.clone());

        assert_eq!(context.interruption(), None);
        cancellation.cancel();
        assert_eq!(
            context.interruption(),
            Some(ExecutionInterruption::Cancelled)
        );
    }

    #[test]
    fn no_process_critical_phase_reports_deferred_cancellation() {
        let cancellation = CancellationToken::new();
        let context =
            ExecutionContext::cli(CommandName::Artifacts).with_cancellation(cancellation.clone());

        let result = context
            .run_no_process_critical_phase(|| {
                cancellation.cancel();
                Ok::<_, ()>("published")
            })
            .expect("critical phase");

        assert_eq!(result.value, "published");
        assert_eq!(
            result.deferred_interruption,
            Some(ExecutionInterruption::Cancelled)
        );
    }
}
