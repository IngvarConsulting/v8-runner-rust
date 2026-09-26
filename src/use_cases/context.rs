use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::platform::process::{ProcessExecutionPolicy, ProcessInterruptionSafety, WorkGiven};

/// Identifies the logical command being executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandName {
    Bootstrap,
    ToolsDownload,
    Init,
    Extensions,
    Build,
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

/// Command-boundary interruption signal observed at safe points.
///
/// A command carries no deadline (DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE), so the only
/// thing that interrupts one at a safe point is the operator. A step that overruns its own
/// cap is a different signal and arrives as `ProcessInterruptionReason::TimedOut`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionInterruption {
    Cancelled,
}

impl ExecutionInterruption {
    pub const fn message(self, command: CommandName) -> &'static str {
        match (self, command) {
            (Self::Cancelled, _) => "execution cancelled before reaching a safe completion point",
        }
    }
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

    use super::{
        CommandName, ExecutionContext, ExecutionInterruption, ExecutionTransport,
        InterruptionSafetyClass,
    };

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
