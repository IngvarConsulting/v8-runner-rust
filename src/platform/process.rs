use std::io::Read;
use std::num::NonZeroI32;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::platform::secrets::render_masked_command;

const EXECUTABLE_BUSY_MAX_RETRIES: usize = 5;
const EXECUTABLE_BUSY_RETRY_DELAY: Duration = Duration::from_millis(10);
#[cfg(any(windows, test))]
const WINDOWS_ERROR_INVALID_HANDLE: i32 = 6;
/// Строка журнала, с которой раннер откладывает прерывание критического процесса или
/// критической команды агента.
pub(crate) const CRITICAL_INTERRUPTION_DEFERRED: &str =
    "interruption requested during critical process phase; waiting for terminal outcome";

/// Request for launching an external utility.
#[derive(Debug, Clone)]
pub struct ProcessRequest {
    /// Absolute path to the executable to run.
    pub program: PathBuf,
    /// Command-line arguments passed to the executable.
    pub args: Vec<String>,
    /// Optional working directory for the child process.
    pub workdir: Option<PathBuf>,
    /// Optional path where runner-captured stdout is mirrored.
    pub stdout_log_path: Option<PathBuf>,
    /// Optional path where runner-captured stderr is mirrored.
    pub stderr_log_path: Option<PathBuf>,
    /// Optional grace period used by `spawn()` to detect immediate startup failures.
    pub startup_probe: Option<Duration>,
}

/// Result of a completed `run()` invocation.
#[derive(Debug, Clone)]
pub struct ProcessResult {
    /// Child exit code.
    pub exit_code: i32,
    /// Captured stdout as UTF-8 (lossy-decoded).
    pub stdout: String,
    /// Captured stderr as UTF-8 (lossy-decoded).
    pub stderr: String,
    /// Command-boundary interruption observed while the child was running.
    pub interruption: Option<ProcessInterruption>,
}

impl ProcessResult {
    /// Исход утилиты по её коду выхода: ноль — утилита сообщила удачу, любой другой код —
    /// отказ, и код идёт с ним уликой для текста ответа. Код читает этот слой, а сценарий
    /// получает исход (INV.PLATFORM.EXIT-CODES-ARE-READ-IN-THE-PLATFORM-LAYER).
    pub fn outcome(&self) -> Result<(), NonZeroI32> {
        match NonZeroI32::new(self.exit_code) {
            None => Ok(()),
            Some(code) => Err(code),
        }
    }
}

/// Result of a detached `spawn()` invocation.
#[derive(Debug, Clone)]
pub struct SpawnResult {
    /// Operating system process identifier.
    pub pid: u32,
    /// Binary that was used to start the process.
    pub binary: PathBuf,
}

/// Managed process handle used while the caller still needs a cleanup boundary.
pub struct ManagedSpawnResult {
    result: SpawnResult,
    child: Option<SpawnedChild>,
    rendered_command: String,
    /// Стал ли этот процесс работой команды: его запуск отметил работу. Процесс самой
    /// платформы — агент — работой не становится.
    delivered: bool,
}

/// Managed spawn lifecycle behaviour used by current callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedSpawnMode {
    Detached,
    Wait,
}

impl ManagedSpawnResult {
    /// Operating system process identifier.
    pub fn pid(&self) -> u32 {
        self.result.pid
    }

    /// Binary that was used to start the process.
    pub fn binary(&self) -> &PathBuf {
        &self.result.binary
    }

    /// Convert the managed handle into a detached result after external checks succeed.
    pub fn detach(mut self) -> SpawnResult {
        let result = self.result.clone();
        self.child.take();
        result
    }

    /// Terminate the managed process and wait for it to exit.
    pub fn terminate(mut self) {
        if let Some(mut spawned) = self.child.take() {
            terminate_child_group_gracefully(&mut spawned, Duration::from_millis(250));
            let _ = spawned.child.wait();
        }
    }

    /// Снимает процесс по отмене команды и называет это отменой: ошибка несёт, был ли
    /// процесс работой команды.
    #[must_use]
    pub fn cancel(self) -> ProcessError {
        let error = ProcessError::Cancelled {
            cmd: self.rendered_command.clone(),
            delivered: self.delivered,
        };
        self.terminate();
        error
    }

    /// Waits for a managed client and guarantees process-group cleanup at timeout.
    pub fn wait_for_exit(
        mut self,
        policy: &ProcessExecutionPolicy,
    ) -> Result<ManagedProcessOutcome, ProcessError> {
        let mut spawned = self
            .child
            .take()
            .ok_or_else(|| ProcessError::StartupCheckFailed {
                cmd: self.rendered_command.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "managed child missing",
                ),
            })?;
        let started = std::time::Instant::now();
        loop {
            if let Some(status) =
                spawned
                    .child
                    .try_wait()
                    .map_err(|source| ProcessError::StartupCheckFailed {
                        cmd: self.rendered_command.clone(),
                        source,
                    })?
            {
                return Ok(ManagedProcessOutcome {
                    exit_code: Some(status.code().unwrap_or(-1)),
                    timed_out: false,
                });
            }
            if policy.cancellation.is_cancelled() {
                terminate_child_group_gracefully(&mut spawned, policy.graceful_shutdown_timeout);
                let _ = spawned.child.wait();
                return Err(ProcessError::Cancelled {
                    cmd: self.rendered_command.clone(),
                    delivered: self.delivered,
                });
            }
            if policy
                .timeout
                .is_some_and(|timeout| started.elapsed() >= timeout)
            {
                terminate_child_group_gracefully(&mut spawned, policy.graceful_shutdown_timeout);
                let _ = spawned.child.wait();
                return Ok(ManagedProcessOutcome {
                    exit_code: None,
                    timed_out: true,
                });
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Terminal state returned by an explicitly managed wait boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagedProcessOutcome {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

impl Drop for ManagedSpawnResult {
    fn drop(&mut self) {
        if let Some(mut spawned) = self.child.take() {
            terminate_child_group_gracefully(&mut spawned, Duration::from_millis(250));
            let _ = spawned.child.wait();
        }
    }
}

/// Safety class applied by the process runner when interruption arrives mid-flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessInterruptionSafety {
    Interruptible,
    GracefulThenKill,
    CriticalNonAbortable,
}

/// Normalized interruption reason shared across timeout and cancellation paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessInterruptionReason {
    Cancelled,
    TimedOut,
}

/// How the runner handled the interruption after it arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessInterruptionAction {
    Deferred,
}

/// Metadata preserved when the runner observes interruption during process execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessInterruption {
    pub reason: ProcessInterruptionReason,
    pub action: ProcessInterruptionAction,
}

impl ProcessInterruption {
    /// Прерывание, которое критическая фаза отложила до своего исхода.
    pub fn deferred(reason: ProcessInterruptionReason) -> Self {
        Self {
            reason,
            action: ProcessInterruptionAction::Deferred,
        }
    }
}

/// Получил ли исполнитель работу этой команды. Отмечает её платформа в тот миг, когда работа
/// передаётся: запущен процесс, который выполняет запрос, или работающей сессии отдана
/// команда запроса. Подъём сессии и её служебные команды отметки не ставят. Читает её
/// сценарий, когда собирает ответ; клоны делят одну отметку.
///
/// Шаг самой платформы — служебная команда сессии, ожидание выхода агента — отметки не
/// несёт вовсе: носитель держит `Option<WorkGiven>`, и `None` у него значит «не работа
/// команды». `Default` нет нарочно: отметка, созданная мимо команды, молча теряла бы работу.
#[derive(Debug, Clone)]
pub struct WorkGiven(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl WorkGiven {
    /// Отметка одной команды: её заводит контекст команды.
    pub(crate) fn for_command() -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )))
    }

    /// Работа передана исполнителю.
    pub(crate) fn mark_work_given(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn given(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Сколько снимаемый процесс ждёт мягкого завершения, прежде чем его убьют.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(250);

/// Shared execution policy passed from transport-neutral command context into the runner.
#[derive(Debug, Clone)]
pub struct ProcessExecutionPolicy {
    pub timeout: Option<Duration>,
    pub cancellation: CancellationToken,
    pub safety: ProcessInterruptionSafety,
    pub graceful_shutdown_timeout: Duration,
    /// Куда отметить, что процесс запущен: запуск разового процесса — работа команды.
    /// `None` — у шага самой платформы, который работой команды не является; объявить так
    /// шаг может только платформа: поле за её пределами не видно.
    pub(in crate::platform) work: Option<WorkGiven>,
}

/// Только для тестов: в работе политику строит контекст команды, и отметка работы у неё
/// своя, а не пустая.
#[cfg(test)]
impl Default for ProcessExecutionPolicy {
    fn default() -> Self {
        Self::new(
            None,
            CancellationToken::new(),
            ProcessInterruptionSafety::Interruptible,
            WorkGiven::for_command(),
        )
    }
}

impl ProcessExecutionPolicy {
    /// Политика шага команды: запуск процесса под ней — работа команды.
    pub fn new(
        timeout: Option<Duration>,
        cancellation: CancellationToken,
        safety: ProcessInterruptionSafety,
        work: WorkGiven,
    ) -> Self {
        Self {
            timeout,
            cancellation,
            safety,
            graceful_shutdown_timeout: GRACEFUL_SHUTDOWN_TIMEOUT,
            work: Some(work),
        }
    }

    /// Политика шага самой платформы — ожидания выхода агента: работы команды он не
    /// отмечает.
    pub(in crate::platform) fn platform_step(
        timeout: Option<Duration>,
        cancellation: CancellationToken,
        safety: ProcessInterruptionSafety,
    ) -> Self {
        Self {
            timeout,
            cancellation,
            safety,
            graceful_shutdown_timeout: GRACEFUL_SHUTDOWN_TIMEOUT,
            work: None,
        }
    }

    /// Та же политика для служебной команды платформы — перехода интерактивной сессии в
    /// рабочее пространство: работы команды она не отмечает.
    pub(in crate::platform) fn without_work(&self) -> Self {
        Self {
            work: None,
            ..self.clone()
        }
    }

    /// Та же политика для команды, которая базу только читает: критическая фаза — запись, а
    /// чтение отмена снимает (INV.USE-CASES.A-DATABASE-WRITE-IS-A-CRITICAL-PHASE). Под
    /// критической политикой чтение снимается мягко — SIGTERM, затем kill; защищённее своего
    /// класса оно не становится. Отмена, предел и отметка работы у него те же.
    pub(in crate::platform) fn for_reading(&self) -> Self {
        let safety = match self.safety {
            ProcessInterruptionSafety::Interruptible => ProcessInterruptionSafety::Interruptible,
            ProcessInterruptionSafety::GracefulThenKill
            | ProcessInterruptionSafety::CriticalNonAbortable => {
                ProcessInterruptionSafety::GracefulThenKill
            }
        };
        Self {
            safety,
            ..self.clone()
        }
    }

    /// Двойник исполнителя в тестах сценариев отмечает работу, как настоящий, едва
    /// «запустил» процесс.
    #[cfg(test)]
    pub(crate) fn mark_started_for_test(&self) {
        if let Some(work) = &self.work {
            work.mark_work_given();
        }
    }
}

/// Runner-level process execution failures.
#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("failed to spawn process '{cmd}': {source}")]
    SpawnFailed { cmd: String, source: std::io::Error },

    #[error("failed to observe process startup '{cmd}': {source}")]
    StartupCheckFailed { cmd: String, source: std::io::Error },

    #[error("process exited before startup completed '{cmd}' (exit {exit_code})")]
    ExitedEarly { cmd: String, exit_code: i32 },

    #[error("failed to write stdout log '{path}': {source}")]
    StdoutLogIo {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to write stderr log '{path}': {source}")]
    StderrLogIo {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("process cancelled '{cmd}' before reaching a safe completion point")]
    Cancelled {
        cmd: String,
        /// Успел ли запуск стать работой команды: процесс запущен под её отметкой работы.
        /// Отказ до запуска и шаг самой платформы работы не несут — они на границе.
        delivered: bool,
    },

    #[error("process timed out '{cmd}' after {timeout_ms}ms")]
    TimedOut { cmd: String, timeout_ms: u64 },

    #[error("managed process spawn is not supported for '{cmd}'")]
    ManagedSpawnUnsupported { cmd: String },
}

/// Boundary for synchronous and detached process execution.
pub trait ProcessRunner {
    /// Execute a process under the caller's execution policy.
    ///
    /// Right after the process has started, the implementation marks `policy.work` when
    /// there is one: a started request process is the command's work, whatever happens to
    /// it later. A run refused before the start marks nothing.
    ///
    /// No default: an implementation must answer for the whole policy, not just its
    /// timeout. Since a command carries no deadline
    /// (DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE), `policy.timeout` is `None` at most call
    /// sites, and a default that fell through to `run` would silently drop the operator's
    /// interrupt and the interruption safety class — the one thing that still ends a run.
    fn run_with_policy(
        &self,
        request: &ProcessRequest,
        policy: &ProcessExecutionPolicy,
    ) -> Result<ProcessResult, ProcessError>;

    /// Start a process in fire-and-forget mode without waiting for completion. A process
    /// that passed its startup probe is the command's work: the implementation marks `work`.
    fn spawn(
        &self,
        request: &ProcessRequest,
        work: &WorkGiven,
    ) -> Result<SpawnResult, ProcessError>;

    /// Start a process and keep a handle until the caller detaches or terminates it. The
    /// implementation marks `work`, when there is one, once the process has passed its
    /// startup probe: a client that exits inside the probe is a start that failed. A
    /// session's own process comes without it.
    fn spawn_managed(
        &self,
        request: &ProcessRequest,
        mode: ManagedSpawnMode,
        work: Option<&WorkGiven>,
    ) -> Result<ManagedSpawnResult, ProcessError> {
        let _ = (mode, work);
        Err(ProcessError::ManagedSpawnUnsupported {
            cmd: render_command(request),
        })
    }
}

/// Standard subprocess runner backed by `std::process::Command`.
pub struct ProcessExecutor;

impl ProcessRunner for ProcessExecutor {
    fn run_with_policy(
        &self,
        request: &ProcessRequest,
        policy: &ProcessExecutionPolicy,
    ) -> Result<ProcessResult, ProcessError> {
        self.run_internal(request, policy)
    }

    fn spawn(
        &self,
        request: &ProcessRequest,
        work: &WorkGiven,
    ) -> Result<SpawnResult, ProcessError> {
        let rendered_command = render_command(request);
        debug!(command = rendered_command.as_str(), "spawning process");
        let spawned = spawn_checked_child(request, ProcessIoMode::Detached, &rendered_command)?;
        work.mark_work_given();
        let pid = spawned.child.id();

        debug!(command = rendered_command.as_str(), pid, "process started");
        Ok(SpawnResult {
            pid,
            binary: request.program.clone(),
        })
    }

    fn spawn_managed(
        &self,
        request: &ProcessRequest,
        mode: ManagedSpawnMode,
        work: Option<&WorkGiven>,
    ) -> Result<ManagedSpawnResult, ProcessError> {
        let rendered_command = render_command(request);
        debug!(
            command = rendered_command.as_str(),
            "spawning managed process"
        );
        let io_mode = match mode {
            ManagedSpawnMode::Detached => ProcessIoMode::ManagedDetached,
            ManagedSpawnMode::Wait => ProcessIoMode::ManagedWait,
        };
        let spawned = spawn_checked_child(request, io_mode, &rendered_command)?;
        if let Some(work) = work {
            work.mark_work_given();
        }
        let pid = spawned.child.id();

        debug!(
            command = rendered_command.as_str(),
            pid, "managed process started"
        );
        Ok(ManagedSpawnResult {
            result: SpawnResult {
                pid,
                binary: request.program.clone(),
            },
            child: Some(spawned),
            rendered_command,
            delivered: work.is_some(),
        })
    }
}

impl ProcessExecutor {
    fn run_internal(
        &self,
        request: &ProcessRequest,
        policy: &ProcessExecutionPolicy,
    ) -> Result<ProcessResult, ProcessError> {
        let rendered_command = render_command(request);
        debug!(
            command = rendered_command.as_str(),
            timeout_ms = policy.timeout.map(|value| value.as_millis() as u64),
            safety = ?policy.safety,
            "running process"
        );
        if policy.cancellation.is_cancelled() {
            return Err(ProcessError::Cancelled {
                cmd: rendered_command,
                delivered: false,
            });
        }
        if policy.timeout.is_some_and(|timeout| timeout.is_zero()) {
            return Err(ProcessError::TimedOut {
                cmd: rendered_command,
                timeout_ms: 0,
            });
        }
        let spawned = spawn_command(request, ProcessIoMode::Captured, &rendered_command)?;
        if let Some(work) = &policy.work {
            work.mark_work_given();
        }
        let output = wait_for_output(spawned, &rendered_command, policy)?;
        debug!(
            command = rendered_command.as_str(),
            exit_code = output.status.code().unwrap_or(-1),
            stdout_bytes = output.stdout.len(),
            stderr_bytes = output.stderr.len(),
            "process finished"
        );

        if let Some(path) = &request.stdout_log_path {
            std::fs::write(path, &output.stdout).map_err(|source| ProcessError::StdoutLogIo {
                path: path.clone(),
                source,
            })?;
        }

        if let Some(path) = &request.stderr_log_path {
            std::fs::write(path, &output.stderr).map_err(|source| ProcessError::StderrLogIo {
                path: path.clone(),
                source,
            })?;
        }

        Ok(ProcessResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            interruption: output.interruption,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum ProcessIoMode {
    Detached,
    #[cfg(any(unix, windows))]
    UnicaOwnedDetached,
    ManagedDetached,
    ManagedWait,
    Captured,
}

impl ProcessIoMode {
    const fn requires_standard_handle_isolation(self) -> bool {
        match self {
            Self::Detached | Self::ManagedDetached => true,
            #[cfg(any(unix, windows))]
            Self::UnicaOwnedDetached => true,
            Self::ManagedWait | Self::Captured => false,
        }
    }

    fn with_client_owner(self) -> std::io::Result<Self> {
        if !matches!(self, Self::Detached) {
            return Ok(self);
        }
        let Some(owner) = std::env::var_os("V8_RUNNER_CLIENT_OWNER") else {
            return Ok(self);
        };
        // This private opt-in trusts the caller to retain and release the tree.
        // Group/Job membership is a prerequisite, not caller identity attestation.
        if owner != "unica" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "unsupported V8_RUNNER_CLIENT_OWNER value",
            ));
        }
        #[cfg(unix)]
        {
            // The host owns this isolated group until it accepts the launch
            // result. Keeping the client in it also covers abrupt runner death.
            // SAFETY: these process identity queries have no pointer arguments.
            if unsafe { libc::getpgrp() != libc::getpid() } {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Unica client ownership requires an isolated runner process group",
                ));
            }
            Ok(Self::UnicaOwnedDetached)
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::JobObjects::IsProcessInJob;
            use windows_sys::Win32::System::Threading::GetCurrentProcess;
            let mut in_job = 0;
            // SAFETY: the pseudo-handle refers to this process, null asks about
            // any containing job, and in_job is a writable BOOL output.
            if unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job) }
                == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if in_job == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Unica client ownership requires the runner to belong to a Job Object",
                ));
            }
            Ok(Self::UnicaOwnedDetached)
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Unica client ownership is unsupported on this platform",
            ))
        }
    }
}

struct SpawnedChild {
    child: ChildHandle,
    io_mode: ProcessIoMode,
}

enum ChildHandle {
    Standard(std::process::Child),
    #[cfg(windows)]
    Wrapped(Box<dyn process_wrap::std::ChildWrapper>),
}

impl ChildHandle {
    fn id(&self) -> u32 {
        match self {
            Self::Standard(child) => child.id(),
            #[cfg(windows)]
            Self::Wrapped(child) => child.id(),
        }
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        match self {
            Self::Standard(child) => child.try_wait(),
            #[cfg(windows)]
            Self::Wrapped(child) => child.try_wait(),
        }
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        match self {
            Self::Standard(child) => child.wait(),
            #[cfg(windows)]
            Self::Wrapped(child) => child.wait(),
        }
    }

    #[cfg(not(unix))]
    fn start_kill(&mut self) -> std::io::Result<()> {
        match self {
            Self::Standard(child) => child.kill(),
            #[cfg(windows)]
            Self::Wrapped(child) => child.start_kill(),
        }
    }

    fn stdout(&mut self) -> &mut Option<std::process::ChildStdout> {
        match self {
            Self::Standard(child) => &mut child.stdout,
            #[cfg(windows)]
            Self::Wrapped(child) => child.stdout(),
        }
    }

    fn stderr(&mut self) -> &mut Option<std::process::ChildStderr> {
        match self {
            Self::Standard(child) => &mut child.stderr,
            #[cfg(windows)]
            Self::Wrapped(child) => child.stderr(),
        }
    }
}

fn spawn_checked_child(
    request: &ProcessRequest,
    io_mode: ProcessIoMode,
    rendered_command: &str,
) -> Result<SpawnedChild, ProcessError> {
    let mut spawned = spawn_command(request, io_mode, rendered_command)?;

    if let Some(startup_probe) = request.startup_probe {
        std::thread::sleep(startup_probe);
        if let Some(status) = startup_probe_status(&mut spawned).map_err(|source| {
            ProcessError::StartupCheckFailed {
                cmd: rendered_command.to_owned(),
                source,
            }
        })? {
            warn!(
                command = rendered_command,
                exit_code = status.code().unwrap_or(-1),
                "process exited during startup probe"
            );
            #[cfg(not(unix))]
            if matches!(
                spawned.io_mode,
                ProcessIoMode::ManagedDetached | ProcessIoMode::ManagedWait
            ) {
                terminate_child_group_gracefully(&mut spawned, Duration::from_millis(250));
                let _ = spawned.child.wait();
            }
            return Err(ProcessError::ExitedEarly {
                cmd: rendered_command.to_owned(),
                exit_code: status.code().unwrap_or(-1),
            });
        }
    }

    Ok(spawned)
}

#[cfg(unix)]
fn startup_probe_status(
    spawned: &mut SpawnedChild,
) -> std::io::Result<Option<std::process::ExitStatus>> {
    if matches!(spawned.io_mode, ProcessIoMode::UnicaOwnedDetached) {
        // This child shares the host-owned runner group. Unica retains and
        // cleans that group on failure; the child PID is not a process-group ID.
        return spawned.child.try_wait();
    }
    let pid = spawned.child.id();
    loop {
        // This module is the sole wait owner until startup returns. WNOWAIT
        // keeps its leader unreaped until the final group signal; it is not a
        // lock against an unrelated process-wide child reaper. An observation
        // error gives no authority to signal a numeric PGID.
        // SAFETY: zero is a valid initial value for the waitid output buffer.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: info is writable and pid identifies the child owned by spawned.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == -1 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        // SAFETY: waitid initialized the siginfo output on success.
        if unsafe { info.si_pid() } == 0 {
            return Ok(None);
        }
        // SAFETY: the unreaped leader still reserves this exact process group.
        if unsafe { libc::kill(-(pid as i32), libc::SIGKILL) } == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                // Darwin can report EPERM for a zombie-only group. Preserve the
                // observed early exit, but do not claim descendant cleanup.
                warn!(pid, %error, "startup failed; process-group cleanup could not be confirmed");
            }
        }
        return spawned.child.wait().map(Some);
    }
}

#[cfg(not(unix))]
fn startup_probe_status(
    spawned: &mut SpawnedChild,
) -> std::io::Result<Option<std::process::ExitStatus>> {
    spawned.child.try_wait()
}

fn spawn_command(
    request: &ProcessRequest,
    io_mode: ProcessIoMode,
    rendered_command: &str,
) -> Result<SpawnedChild, ProcessError> {
    let io_mode = io_mode
        .with_client_owner()
        .map_err(|source| ProcessError::SpawnFailed {
            cmd: rendered_command.to_owned(),
            source,
        })?;
    for attempt in 0..=EXECUTABLE_BUSY_MAX_RETRIES {
        if io_mode.requires_standard_handle_isolation() {
            isolate_inherited_standard_handles().map_err(|source| ProcessError::SpawnFailed {
                cmd: rendered_command.to_owned(),
                source,
            })?;
        }
        let cmd = build_command(request, io_mode, rendered_command)?;
        match spawn_child(cmd, io_mode) {
            Ok(child) => return Ok(SpawnedChild { child, io_mode }),
            Err(source) if is_executable_busy(&source) && attempt < EXECUTABLE_BUSY_MAX_RETRIES => {
                warn!(
                    command = rendered_command,
                    attempt = attempt + 1,
                    max_retries = EXECUTABLE_BUSY_MAX_RETRIES,
                    delay_ms = EXECUTABLE_BUSY_RETRY_DELAY.as_millis() as u64,
                    "spawn hit executable-busy race, retrying"
                );
                std::thread::sleep(EXECUTABLE_BUSY_RETRY_DELAY);
            }
            Err(source) => {
                return Err(ProcessError::SpawnFailed {
                    cmd: rendered_command.to_owned(),
                    source,
                });
            }
        }
    }

    unreachable!("spawn loop must return on success or final error");
}

/// Снимает наследование с собственных стандартных дескрипторов перед запуском
/// отсоединённого ребёнка.
///
/// `Stdio::null()` подменяет потоки ребёнка, но не мешает наследоваться второй,
/// наследуемой копии дескриптора самого раннера: отсоединённый клиент 1С держал бы
/// конвейер stdout обёртки открытым после выхода раннера.
///
/// Отвергнутые замены. `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` — правильный список
/// разрешённого в Win32, но `spawn_with_attributes` и `inherit_handles` в Rust
/// нестабильны, а свой `CreateProcessW` повторил бы кавычки командной строки,
/// окружение, рабочий каталог, поиск исполняемого файла, владение дескрипторами ребёнка
/// и работу с Job Object. Снять наследование на время и вернуть обратно — гонка с
/// одновременным созданием процессов, а сбой возврата после старта ребёнка уже нечем
/// обработать. Поэтому снятие постоянное: повторный вызов ничего не меняет.
#[cfg(windows)]
fn isolate_inherited_standard_handles() -> std::io::Result<()> {
    use windows_sys::Win32::Foundation::{
        SetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    for standard_handle in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: `standard_handle` is one of the three constants accepted by GetStdHandle.
        let handle = unsafe { GetStdHandle(standard_handle) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            continue;
        }

        // SAFETY: the value is passed back to Win32 without dereferencing; stale handles are
        // reported as errors, and changing the inherit flag does not transfer ownership.
        if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
            let error = std::io::Error::last_os_error();
            if is_invalid_standard_handle_error(&error) {
                continue;
            }
            return Err(error);
        }
    }

    Ok(())
}

#[cfg(any(windows, test))]
fn is_invalid_standard_handle_error(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(WINDOWS_ERROR_INVALID_HANDLE)
}

#[cfg(not(windows))]
fn isolate_inherited_standard_handles() -> std::io::Result<()> {
    Ok(())
}

fn spawn_child(mut cmd: Command, io_mode: ProcessIoMode) -> std::io::Result<ChildHandle> {
    #[cfg(windows)]
    {
        if matches!(
            io_mode,
            ProcessIoMode::ManagedDetached | ProcessIoMode::ManagedWait
        ) {
            use process_wrap::std::{CommandWrap, JobObject};

            let mut wrapped = CommandWrap::from(cmd);
            wrapped.wrap(JobObject);
            return wrapped.spawn().map(ChildHandle::Wrapped);
        }
    }

    let _ = io_mode;
    cmd.spawn().map(ChildHandle::Standard)
}

fn build_command(
    request: &ProcessRequest,
    io_mode: ProcessIoMode,
    rendered_command: &str,
) -> Result<Command, ProcessError> {
    let mut cmd = Command::new(&request.program);
    // This private host/runner protocol must never become a child's launch policy.
    cmd.env_remove("V8_RUNNER_CLIENT_OWNER");
    cmd.args(&request.args);
    if let Some(workdir) = &request.workdir {
        cmd.current_dir(workdir);
    }
    cmd.stdin(Stdio::null());
    match io_mode {
        ProcessIoMode::Detached => {
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::null());
            set_child_process_group(&mut cmd);
        }
        #[cfg(any(unix, windows))]
        ProcessIoMode::UnicaOwnedDetached => {
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::null());
        }
        ProcessIoMode::ManagedDetached => {
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::null());
            set_child_process_group(&mut cmd);
        }
        ProcessIoMode::ManagedWait => {
            cmd.stdout(Stdio::null());
            let path =
                request
                    .stderr_log_path
                    .as_ref()
                    .ok_or_else(|| ProcessError::StderrLogIo {
                        path: PathBuf::new(),
                        source: std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "stderr log path is required",
                        ),
                    })?;
            let stderr =
                std::fs::File::create(path).map_err(|source| ProcessError::StderrLogIo {
                    path: path.clone(),
                    source,
                })?;
            cmd.stderr(Stdio::from(stderr));
            set_child_process_group(&mut cmd);
        }
        ProcessIoMode::Captured => {
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());
            set_child_process_group(&mut cmd);
        }
    }
    let _ = rendered_command;
    Ok(cmd)
}

fn set_child_process_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    #[cfg(not(unix))]
    {
        let _ = cmd;
    }
}

fn is_executable_busy(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        matches!(error.raw_os_error(), Some(libc::ETXTBSY))
            || error.kind() == std::io::ErrorKind::ExecutableFileBusy
    }

    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

fn wait_for_output(
    mut spawned: SpawnedChild,
    rendered_command: &str,
    policy: &ProcessExecutionPolicy,
) -> Result<ObservedOutput, ProcessError> {
    let mut stdout =
        spawned
            .child
            .stdout()
            .take()
            .ok_or_else(|| ProcessError::StartupCheckFailed {
                cmd: rendered_command.to_owned(),
                source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stdout pipe missing"),
            })?;
    let mut stderr =
        spawned
            .child
            .stderr()
            .take()
            .ok_or_else(|| ProcessError::StartupCheckFailed {
                cmd: rendered_command.to_owned(),
                source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stderr pipe missing"),
            })?;
    let stdout_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let start = std::time::Instant::now();
    let mut observed_interruption: Option<ProcessInterruptionReason> = None;
    loop {
        if let Some(status) =
            spawned
                .child
                .try_wait()
                .map_err(|source| ProcessError::StartupCheckFailed {
                    cmd: rendered_command.to_owned(),
                    source,
                })?
        {
            // Известный предел: потоки чтения присоединяются после выхода процесса. Внук,
            // переживший родителя и держащий трубу, задержит присоединение, и прерывание в
            // это время не наблюдается: цикл уже вышел. Группу снимают только пути
            // прерывания выше — `Interruptible` и `GracefulThenKill`, — а обычный выход её
            // не трогает.
            let stdout = stdout_reader.join().unwrap_or_default();
            let stderr = stderr_reader.join().unwrap_or_default();
            return match observed_interruption {
                Some(ProcessInterruptionReason::Cancelled)
                    if policy.safety != ProcessInterruptionSafety::CriticalNonAbortable =>
                {
                    Err(ProcessError::Cancelled {
                        cmd: rendered_command.to_owned(),
                        delivered: policy.work.is_some(),
                    })
                }
                Some(ProcessInterruptionReason::TimedOut)
                    if policy.safety != ProcessInterruptionSafety::CriticalNonAbortable =>
                {
                    Err(ProcessError::TimedOut {
                        cmd: rendered_command.to_owned(),
                        timeout_ms: policy.timeout.unwrap_or_default().as_millis() as u64,
                    })
                }
                Some(reason) => Ok(ObservedOutput {
                    status,
                    stdout,
                    stderr,
                    interruption: Some(ProcessInterruption::deferred(reason)),
                }),
                None => Ok(ObservedOutput {
                    status,
                    stdout,
                    stderr,
                    interruption: None,
                }),
            };
        }

        if observed_interruption.is_none() {
            if policy.cancellation.is_cancelled() {
                observed_interruption = Some(ProcessInterruptionReason::Cancelled);
                if let Some(error) = interrupt_child(
                    &mut spawned,
                    rendered_command,
                    policy,
                    ProcessInterruptionReason::Cancelled,
                )? {
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(error);
                }
            } else if let Some(limit) = policy.timeout {
                if start.elapsed() >= limit {
                    observed_interruption = Some(ProcessInterruptionReason::TimedOut);
                    if let Some(error) = interrupt_child(
                        &mut spawned,
                        rendered_command,
                        policy,
                        ProcessInterruptionReason::TimedOut,
                    )? {
                        let _ = stdout_reader.join();
                        let _ = stderr_reader.join();
                        return Err(error);
                    }
                }
            }
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}

struct ObservedOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    interruption: Option<ProcessInterruption>,
}

fn interrupt_child(
    spawned: &mut SpawnedChild,
    rendered_command: &str,
    policy: &ProcessExecutionPolicy,
    reason: ProcessInterruptionReason,
) -> Result<Option<ProcessError>, ProcessError> {
    match policy.safety {
        ProcessInterruptionSafety::CriticalNonAbortable => {
            warn!(
                command = rendered_command,
                reason = ?reason,
                "{}",
                CRITICAL_INTERRUPTION_DEFERRED
            );
            Ok(None)
        }
        ProcessInterruptionSafety::Interruptible => {
            terminate_child_group(spawned);
            let _ = spawned.child.wait();
            Ok(Some(process_error_from_reason(
                rendered_command,
                policy,
                reason,
            )))
        }
        ProcessInterruptionSafety::GracefulThenKill => {
            terminate_child_group_gracefully(spawned, policy.graceful_shutdown_timeout);
            let _ = spawned.child.wait();
            Ok(Some(process_error_from_reason(
                rendered_command,
                policy,
                reason,
            )))
        }
    }
}

/// Процесс уже запущен: отмена обрывает работу команды, если он был ею.
fn process_error_from_reason(
    rendered_command: &str,
    policy: &ProcessExecutionPolicy,
    reason: ProcessInterruptionReason,
) -> ProcessError {
    match reason {
        ProcessInterruptionReason::Cancelled => ProcessError::Cancelled {
            cmd: rendered_command.to_owned(),
            delivered: policy.work.is_some(),
        },
        ProcessInterruptionReason::TimedOut => ProcessError::TimedOut {
            cmd: rendered_command.to_owned(),
            timeout_ms: policy.timeout.unwrap_or_default().as_millis() as u64,
        },
    }
}

fn terminate_child_group(spawned: &mut SpawnedChild) {
    #[cfg(windows)]
    {
        terminate_windows_process_tree(spawned.child.id());
        let _ = spawned.child.start_kill();
    }

    #[cfg(unix)]
    {
        terminate_unix_process_group(spawned.child.id() as i32, libc::SIGKILL);
    }

    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = spawned.child.start_kill();
    }
}

fn terminate_child_group_gracefully(spawned: &mut SpawnedChild, timeout: Duration) {
    #[cfg(windows)]
    {
        let _ = timeout;
        terminate_windows_process_tree(spawned.child.id());
        let _ = spawned.child.start_kill();
    }

    #[cfg(unix)]
    {
        let pgid = spawned.child.id() as i32;
        terminate_unix_process_group(pgid, libc::SIGTERM);

        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if spawned.child.try_wait().is_err() || !unix_process_group_exists(pgid) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        terminate_child_group(spawned);
    }

    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = timeout;
        let _ = spawned.child.start_kill();
    }
}

#[cfg(unix)]
fn terminate_unix_process_group(pgid: i32, signal: i32) {
    unsafe {
        let _ = libc::kill(-pgid, signal);
    }
}

#[cfg(unix)]
fn unix_process_group_exists(pgid: i32) -> bool {
    unsafe {
        if libc::kill(-pgid, 0) == 0 {
            return true;
        }
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
fn terminate_windows_process_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(all(not(unix), not(windows)))]
fn terminate_windows_process_tree(pid: u32) {
    let _ = pid;
}

/// Показ команды для отказа и журнала. Секреты маскирует
/// [`crate::platform::secrets`] — единственный владелец правила.
fn render_command(request: &ProcessRequest) -> String {
    render_masked_command(&request.program, &request.args)
}

/// Тестовая отметка: раннер отложил прерывание критического процесса. Тест ждёт её, а не
/// отсчёта времени, прежде чем отпустить подставной процесс. Снаружи модуля её не видно:
/// рукопожатие с ней ведёт [`HeldCommand::interrupt_during`].
#[cfg(all(test, unix))]
#[derive(Clone, Default)]
struct DeferralWatch(std::sync::Arc<std::sync::atomic::AtomicBool>);

#[cfg(all(test, unix))]
impl DeferralWatch {
    fn observed(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Выполняет `operation` на этом потоке и отмечает строку журнала об отложенном
    /// прерывании. Процесс ждёт раннер на вызывающем потоке, поэтому подписчика хватает.
    fn during<T>(&self, operation: impl FnOnce() -> T) -> T {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(self.clone())
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, operation)
    }
}

#[cfg(all(test, unix))]
impl std::io::Write for DeferralWatch {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if String::from_utf8_lossy(buf).contains(CRITICAL_INTERRUPTION_DEFERRED) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(all(test, unix))]
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for DeferralWatch {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Команда подставной программы, которую тест держит: начавшись, она отмечается файлом и
/// ждёт, пока её не отпустят. Порядок «отмена пришла во время записи, запись кончилась,
/// когда раннер уже отложил отмену» задают рукопожатия, а не отсчёт времени.
#[cfg(all(test, unix))]
pub(crate) struct HeldCommand {
    started: PathBuf,
    release: PathBuf,
}

#[cfg(all(test, unix))]
impl HeldCommand {
    pub(crate) fn in_dir(dir: &std::path::Path) -> Self {
        Self::with_markers(dir.join("held-started"), dir.join("held-release"))
    }

    /// Команда скрипта, который тест пишет целиком: начавшись, она создаёт `started` и ждёт,
    /// пока не появится `release`.
    pub(crate) fn with_markers(started: PathBuf, release: PathBuf) -> Self {
        Self { started, release }
    }

    /// Ветка sh-скрипта с аргументами в `$args`: команда, в которой есть `pattern`,
    /// отмечается, ждёт отпуска и выходит с `exit_code`.
    pub(crate) fn script_branch(&self, pattern: &str, exit_code: i32) -> String {
        format!(
            "if printf '%s' \"$args\" | grep -F -q -- '{pattern}'; then\n\
               : > '{started}'\n\
               waited=0\n\
               while [ ! -e '{release}' ] && [ \"$waited\" -lt 300 ]; do\n\
                 sleep 0.1\n\
                 waited=$((waited + 1))\n\
               done\n\
               exit {exit_code}\n\
             fi\n",
            started = self.started.display(),
            release = self.release.display(),
        )
    }

    /// Выполняет `run` на этом потоке, пока оператор отменяет команду: дождаться её начала,
    /// отменить `cancellation`, дождаться, пока раннер отложит отмену, и отпустить команду.
    /// Отсрочку раннер должен записать на этом же потоке — отметку слушает подписчик потока.
    /// Тест падает, если команда не началась или раннер отсрочки так и не записал.
    #[track_caller]
    pub(crate) fn interrupt_during<T>(
        &self,
        cancellation: CancellationToken,
        run: impl FnOnce() -> T,
    ) -> T {
        let watch = DeferralWatch::default();
        let operator = {
            let started = self.started.clone();
            let release = self.release.clone();
            let watch = watch.clone();
            std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(30);
                let began = wait_until(deadline, || started.exists());
                cancellation.cancel();
                let deferred = wait_until(deadline, || watch.observed());
                std::fs::write(&release, "").expect("release the held command");
                (began, deferred)
            })
        };
        let outcome = watch.during(run);
        let (began, deferred) = operator
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        assert!(began, "the held command never started");
        assert!(
            deferred,
            "the runner never logged that it deferred the cancellation"
        );
        outcome
    }

    /// Оператор для некритической команды — [`cancel_when_started`] по отметке её начала.
    pub(crate) fn cancel_when_started(
        &self,
        cancellation: CancellationToken,
    ) -> std::thread::JoinHandle<bool> {
        cancel_when_started(&self.started, cancellation)
    }
}

/// Оператор теста: дождаться, пока команда отметится файлом `started`, и отменить. Команду он
/// не отпускает — отмена снимает её сама. Поток отвечает, дождался ли он отметки.
#[cfg(all(test, unix))]
pub(crate) fn cancel_when_started(
    started: &std::path::Path,
    cancellation: CancellationToken,
) -> std::thread::JoinHandle<bool> {
    let started = started.to_path_buf();
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let appeared = wait_until(deadline, || started.exists());
        cancellation.cancel();
        appeared
    })
}

/// Ждёт, пока `condition` не станет верным или не выйдет `deadline`; отвечает, дождался ли.
#[cfg(all(test, unix))]
fn wait_until(deadline: std::time::Instant, condition: impl Fn() -> bool) -> bool {
    while !condition() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    condition()
}

#[cfg(test)]
mod tests {
    use super::{
        is_invalid_standard_handle_error, render_command, ManagedSpawnMode, ProcessError,
        ProcessExecutionPolicy, ProcessExecutor, ProcessInterruptionAction,
        ProcessInterruptionReason, ProcessInterruptionSafety, ProcessIoMode, ProcessRequest,
        ProcessResult, ProcessRunner, WorkGiven, WINDOWS_ERROR_INVALID_HANDLE,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    /// Удача — только нулевой код; любой другой, отрицательный тоже, — отказ со своим кодом.
    #[test]
    fn the_exit_code_becomes_an_outcome_here() {
        let finished = |exit_code| ProcessResult {
            exit_code,
            stdout: String::new(),
            stderr: String::new(),
            interruption: None,
        };

        assert_eq!(finished(0).outcome(), Ok(()));
        for code in [1, 101, 255, -1] {
            assert_eq!(
                finished(code).outcome().map_err(std::num::NonZeroI32::get),
                Err(code)
            );
        }
    }

    #[test]
    fn ignores_only_windows_invalid_handle_errors() {
        let invalid_handle = std::io::Error::from_raw_os_error(WINDOWS_ERROR_INVALID_HANDLE);
        let access_denied = std::io::Error::from_raw_os_error(5);

        assert!(is_invalid_standard_handle_error(&invalid_handle));
        assert!(!is_invalid_standard_handle_error(&access_denied));
    }

    #[test]
    fn detached_modes_require_standard_handle_isolation() {
        assert!(ProcessIoMode::Detached.requires_standard_handle_isolation());
        assert!(ProcessIoMode::ManagedDetached.requires_standard_handle_isolation());
        assert!(!ProcessIoMode::Captured.requires_standard_handle_isolation());
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    fn gave_work(policy: &ProcessExecutionPolicy) -> bool {
        policy.work.as_ref().is_some_and(WorkGiven::given)
    }

    #[cfg(unix)]
    fn plain_request(program: PathBuf) -> ProcessRequest {
        ProcessRequest {
            program,
            args: vec![],
            workdir: None,
            stdout_log_path: None,
            stderr_log_path: None,
            startup_probe: None,
        }
    }

    /// Запущенный процесс — работа команды, чем бы он ни кончился. Отказ до запуска — отмена,
    /// нулевой срок, программа, которую не удалось запустить, — работы не даёт.
    #[cfg(unix)]
    #[test]
    fn only_a_started_process_marks_the_work() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("fails.sh");
        write_script(&script, "exit 3");
        let runner = ProcessExecutor;

        let started = ProcessExecutionPolicy::default();
        let result = runner
            .run_with_policy(&plain_request(script.clone()), &started)
            .expect("the process ran");
        assert_eq!(result.exit_code, 3);
        assert!(gave_work(&started), "a started process got the work");

        let cancelled = ProcessExecutionPolicy::default();
        cancelled.cancellation.cancel();
        let outcome = runner.run_with_policy(&plain_request(script.clone()), &cancelled);
        assert!(
            matches!(
                outcome,
                Err(ProcessError::Cancelled {
                    delivered: false,
                    ..
                })
            ),
            "a refusal before the start delivers nothing: {outcome:?}"
        );
        assert!(
            !gave_work(&cancelled),
            "a cancel before the start gives no work"
        );

        let zero = ProcessExecutionPolicy {
            timeout: Some(Duration::ZERO),
            ..ProcessExecutionPolicy::default()
        };
        let outcome = runner.run_with_policy(&plain_request(script), &zero);
        assert!(
            matches!(outcome, Err(ProcessError::TimedOut { .. })),
            "{outcome:?}"
        );
        assert!(!gave_work(&zero), "a zero timeout refuses before the start");

        let missing = ProcessExecutionPolicy::default();
        let outcome = runner.run_with_policy(&plain_request(dir.path().join("absent")), &missing);
        assert!(
            matches!(outcome, Err(ProcessError::SpawnFailed { .. })),
            "{outcome:?}"
        );
        assert!(
            !gave_work(&missing),
            "a process that could not start got no work"
        );
    }

    /// Процесс, снятый уже после запуска, работу получил. Порядок задан рукопожатием: отмена
    /// приходит, когда скрипт отметился, что запущен.
    #[cfg(unix)]
    #[test]
    fn a_process_cancelled_after_its_start_still_got_the_work() {
        let dir = tempdir().expect("tempdir");
        let started_marker = dir.path().join("started");
        let script = dir.path().join("waits.sh");
        write_script(
            &script,
            &format!(": > '{}'\nsleep 30", started_marker.display()),
        );
        let policy = ProcessExecutionPolicy::default();
        let operator = super::cancel_when_started(&started_marker, policy.cancellation.clone());

        let outcome = ProcessExecutor.run_with_policy(&plain_request(script), &policy);
        assert!(
            operator.join().expect("operator"),
            "the process never started"
        );

        assert!(
            matches!(
                outcome,
                Err(ProcessError::Cancelled {
                    delivered: true,
                    ..
                })
            ),
            "the cancel cut the command's work short: {outcome:?}"
        );
        assert!(
            gave_work(&policy),
            "the process had started before the cancel"
        );
    }

    /// Шаг самой платформы работой команды не является: отмена, снявшая его процесс, работы
    /// не обрывает — ни у разового процесса, ни у управляемого.
    #[cfg(unix)]
    #[test]
    fn a_cancelled_platform_step_delivered_no_work() {
        let dir = tempdir().expect("tempdir");
        let started_marker = dir.path().join("started");
        let script = dir.path().join("waits.sh");
        write_script(
            &script,
            &format!(": > '{}'\nsleep 30", started_marker.display()),
        );
        let policy = ProcessExecutionPolicy::platform_step(
            None,
            CancellationToken::new(),
            ProcessInterruptionSafety::Interruptible,
        );
        let operator = super::cancel_when_started(&started_marker, policy.cancellation.clone());

        let outcome = ProcessExecutor.run_with_policy(&plain_request(script), &policy);
        assert!(
            operator.join().expect("operator"),
            "the process never started"
        );

        assert!(
            matches!(
                outcome,
                Err(ProcessError::Cancelled {
                    delivered: false,
                    ..
                })
            ),
            "{outcome:?}"
        );
    }

    /// Управляемый процесс помнит, стал ли он работой команды: отмена ожидания его выхода
    /// называет это так же, как отмена разового процесса.
    #[cfg(unix)]
    #[test]
    fn a_cancelled_managed_wait_names_whether_the_process_was_the_work() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("waits.sh");
        write_script(&script, "sleep 30");
        for with_work in [true, false] {
            let work = WorkGiven::for_command();
            let request = ProcessRequest {
                stdout_log_path: Some(dir.path().join("stdout.log")),
                stderr_log_path: Some(dir.path().join("stderr.log")),
                ..plain_request(script.clone())
            };
            let managed = ProcessExecutor
                .spawn_managed(&request, ManagedSpawnMode::Wait, with_work.then_some(&work))
                .expect("spawn managed");
            let policy = ProcessExecutionPolicy::default();
            policy.cancellation.cancel();

            let outcome = managed.wait_for_exit(&policy);

            assert!(
                matches!(
                    outcome,
                    Err(ProcessError::Cancelled { delivered, .. }) if delivered == with_work
                ),
                "with work {with_work}: {outcome:?}"
            );
            assert_eq!(work.given(), with_work);
        }
    }

    /// Чтение после записи критической фазой не бывает и защищённее своего класса не
    /// становится; отмена, предел и отметка работы у него те же, что у записи.
    #[test]
    fn a_read_is_never_a_critical_phase() {
        for (safety, read) in [
            (
                ProcessInterruptionSafety::Interruptible,
                ProcessInterruptionSafety::Interruptible,
            ),
            (
                ProcessInterruptionSafety::GracefulThenKill,
                ProcessInterruptionSafety::GracefulThenKill,
            ),
            (
                ProcessInterruptionSafety::CriticalNonAbortable,
                ProcessInterruptionSafety::GracefulThenKill,
            ),
        ] {
            let write = ProcessExecutionPolicy::new(
                Some(Duration::from_secs(7)),
                CancellationToken::new(),
                safety,
                WorkGiven::for_command(),
            );

            let reading = write.for_reading();

            assert_eq!(reading.safety, read, "{safety:?}");
            assert_eq!(reading.timeout, write.timeout, "{safety:?}");
            assert_eq!(
                reading.graceful_shutdown_timeout, write.graceful_shutdown_timeout,
                "{safety:?}"
            );
            write.cancellation.cancel();
            assert!(
                reading.cancellation.is_cancelled(),
                "{safety:?}: the read answers to the command's cancel"
            );
            reading.mark_started_for_test();
            assert!(
                gave_work(&write),
                "{safety:?}: the read marks the command's work"
            );
        }
    }

    #[cfg(unix)]
    fn write_script(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        let staged = path.with_extension("tmp");
        fs::write(&staged, format!("#!/bin/sh\n{body}\n")).expect("write script");
        make_executable(&staged);
        fs::rename(&staged, path).expect("rename script");
    }

    #[cfg(unix)]
    #[test]
    fn run_captures_output_and_mirrors_logs() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("echo.sh");
        let stdout_log = dir.path().join("stdout.log");
        let stderr_log = dir.path().join("stderr.log");
        write_script(&script, "echo hello\nprintf 'oops\\n' >&2\nexit 3");

        let runner = ProcessExecutor;
        let result = runner
            .run_with_policy(
                &ProcessRequest {
                    program: script,
                    args: vec![],
                    workdir: None,
                    stdout_log_path: Some(stdout_log.clone()),
                    stderr_log_path: Some(stderr_log.clone()),
                    startup_probe: None,
                },
                &ProcessExecutionPolicy::default(),
            )
            .expect("run");

        assert_eq!(result.exit_code, 3);
        assert_eq!(result.stdout.trim(), "hello");
        assert_eq!(result.stderr.trim(), "oops");
        assert_eq!(
            fs::read_to_string(stdout_log).expect("stdout log").trim(),
            "hello"
        );
        assert_eq!(
            fs::read_to_string(stderr_log).expect("stderr log").trim(),
            "oops"
        );
    }

    #[test]
    fn render_command_masks_ibcmd_password_flags() {
        let rendered = render_command(&ProcessRequest {
            program: Path::new("/tmp/ibcmd").to_path_buf(),
            args: vec![
                "--user".to_owned(),
                "admin".to_owned(),
                "/N".to_owned(),
                "operator".to_owned(),
                "/p".to_owned(),
                "secret".to_owned(),
                "--database-user=postgres".to_owned(),
                "--DATABASE-password=pg-secret".to_owned(),
                "-p=legacy-secret".to_owned(),
                "--target-db-pwd".to_owned(),
                "target-secret".to_owned(),
            ],
            workdir: None,
            stdout_log_path: None,
            stderr_log_path: None,
            startup_probe: None,
        });

        assert!(rendered.contains("--user ***"));
        assert!(rendered.contains("/N ***"));
        assert!(rendered.contains("/p ***"));
        assert!(rendered.contains("--database-user=***"));
        assert!(rendered.contains("--DATABASE-password=***"));
        assert!(rendered.contains("-p=***"));
        assert!(rendered.contains("--target-db-pwd ***"));
        assert!(!rendered.contains("admin"));
        assert!(!rendered.contains("operator"));
        assert!(!rendered.contains("postgres"));
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("pg-secret"));
        assert!(!rendered.contains("legacy-secret"));
        assert!(!rendered.contains("target-secret"));
    }

    /// Пароль внутри строки соединения приезжает одним аргументом, и до 16.09.2026
    /// показ команды печатал его целиком — а этот показ уходит в текст отказа и в
    /// журнал. Читаемым остаётся всё, что не секрет: адрес сервера и имя базы.
    #[test]
    fn render_command_masks_the_password_inside_a_connection_string() {
        let rendered = render_command(&ProcessRequest {
            program: PathBuf::from("1cv8c"),
            args: vec![
                "/IBConnectionString".to_owned(),
                "Srvr=host;Ref=base;Usr=alice;Pwd=secret".to_owned(),
            ],
            workdir: None,
            stdout_log_path: None,
            stderr_log_path: None,
            startup_probe: None,
        });

        assert_eq!(
            rendered,
            "1cv8c /IBConnectionString Srvr=host;Ref=base;Usr=***;Pwd=***"
        );
    }

    /// Строка соединения приезжает и склеенной с ключом, и с закавыченным паролем,
    /// внутри которого есть `;`. Ни одна из этих форм не должна показать пароль.
    #[test]
    fn render_command_masks_the_password_in_every_connection_string_form() {
        for (arg, password) in [
            (
                "/IBConnectionString=File=/tmp/ib;usr=alice;PWD=secret",
                "secret",
            ),
            (
                "/IBConnectionStringSrvr=host;Ref=base;Usr=alice;Pwd=secret",
                "secret",
            ),
            ("File=/tmp/ib;Usr=alice;Pwd=\"sec;ret\";Ref=base", "sec;ret"),
            ("\"Srvr=host;Ref=base;Usr=alice;Pwd=secret\"", "secret"),
        ] {
            let rendered = render_command(&ProcessRequest {
                program: PathBuf::from("1cv8c"),
                args: vec![arg.to_owned()],
                workdir: None,
                stdout_log_path: None,
                stderr_log_path: None,
                startup_probe: None,
            });

            assert!(!rendered.contains(password), "{arg} -> {rendered}");
            assert!(!rendered.contains("alice"), "{arg} -> {rendered}");
        }
    }

    /// `/WSP` — пароль пользователя веб-сервера, и до 16.09.2026 его не знал ни один
    /// из двух маскировщиков. В argv он попадает через `--raw-key`.
    #[test]
    fn render_command_masks_the_web_server_password() {
        let rendered = render_command(&ProcessRequest {
            program: PathBuf::from("1cv8c"),
            args: vec![
                "/WSN".to_owned(),
                "alice".to_owned(),
                "/WSP".to_owned(),
                "secret".to_owned(),
            ],
            workdir: None,
            stdout_log_path: None,
            stderr_log_path: None,
            startup_probe: None,
        });

        assert_eq!(rendered, "1cv8c /WSN *** /WSP ***");
    }

    #[cfg(unix)]
    struct DetachedFixturePeer(std::os::unix::net::UnixStream);

    #[cfg(unix)]
    impl std::io::Read for DetachedFixturePeer {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            use std::os::fd::AsRawFd;
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    return Err(std::io::ErrorKind::TimedOut.into());
                }
                let mut descriptor = libc::pollfd {
                    fd: self.0.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: descriptor describes a live, owned socket and is writable.
                let result =
                    unsafe { libc::poll(&mut descriptor, 1, remaining.as_millis().max(1) as i32) };
                if result == -1 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                if result == 0 {
                    return Err(std::io::ErrorKind::TimedOut.into());
                }
                return self.0.read(buffer);
            }
        }
    }

    #[cfg(unix)]
    impl Drop for DetachedFixturePeer {
        fn drop(&mut self) {
            use std::io::Write;
            let _ = self.0.write_all(b"stop");
            let _ = self.0.shutdown(std::net::Shutdown::Both);
        }
    }

    #[cfg(unix)]
    struct RetainedTestWrapper(Option<std::process::Child>);

    #[cfg(unix)]
    impl Drop for RetainedTestWrapper {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                // This guard never reaps before signalling: Child still reserves the PID.
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.wait();
            }
        }
    }

    #[cfg(unix)]
    fn accept_detached_fixture(listener: &std::os::unix::net::UnixListener) -> DetachedFixturePeer {
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    // Darwin may refuse timeout options once the peer has
                    // closed. poll bounds reads without mutating socket options.
                    stream.set_nonblocking(true).unwrap();
                    return DetachedFixturePeer(stream);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "fixture did not connect"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept fixture: {error}"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "subprocess fixture invoked explicitly by detached lifecycle tests"]
    fn detached_lifecycle_client_fixture() {
        use std::io::{Read, Write};
        let root = PathBuf::from(std::env::var_os("V8_RUNNER_DETACHED_FIXTURE_ROOT").unwrap());
        let mut stream = std::os::unix::net::UnixStream::connect(root.join("s")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        assert!(std::env::var_os("V8_RUNNER_CLIENT_OWNER").is_none());
        fs::write(root.join("ready"), b"ready").unwrap();
        let mut message = [0; 4];
        while stream.read_exact(&mut message).is_ok() {
            if &message == b"ping" {
                if stream.write_all(b"pong").is_err() {
                    break;
                }
            } else {
                break;
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn detached_spawn_survives_wrapper_exit_and_group_cleanup() {
        use std::io::{Read, Write};
        use std::os::unix::process::CommandExt;
        const TEST: &str =
            "platform::process::tests::detached_spawn_survives_wrapper_exit_and_group_cleanup";
        if std::env::var_os("V8_RUNNER_DETACHED_WRAPPER").is_some() {
            let work = WorkGiven::for_command();
            let mut request = plain_request(std::env::current_exe().unwrap());
            request.args = vec![
                "--exact".into(),
                "platform::process::tests::detached_lifecycle_client_fixture".into(),
                "--ignored".into(),
            ];
            request.startup_probe = Some(Duration::from_millis(250));
            ProcessExecutor
                .spawn(&request, &work)
                .expect("successful detached startup");
            assert!(work.given());
            return;
        }
        // Keep the socket path below the macOS sockaddr_un length limit.
        let dir = tempfile::Builder::new()
            .prefix("v8d")
            .tempdir_in("/tmp")
            .unwrap();
        let listener = std::os::unix::net::UnixListener::bind(dir.path().join("s")).unwrap();
        let helper = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST])
            .env("V8_RUNNER_DETACHED_WRAPPER", "1")
            .env_remove("V8_RUNNER_CLIENT_OWNER")
            .env("V8_RUNNER_DETACHED_FIXTURE_ROOT", dir.path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut wrapper = RetainedTestWrapper(Some(helper));
        let mut peer = accept_detached_fixture(&listener);
        let pid = wrapper.0.as_ref().unwrap().id();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            // Observe exit without relinquishing PID/PGID authority.
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result == -1
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
            {
                continue;
            }
            assert_eq!(result, 0, "waitid: {}", std::io::Error::last_os_error());
            // SAFETY: successful waitid initialized info; WNOHANG leaves si_pid zero while running.
            if unsafe { info.si_pid() } == pid as i32 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "wrapper failed to exit"
            );
            thread::sleep(Duration::from_millis(10));
        }
        // Unica cleans the runner group after normal exit, before reaping its leader.
        // SAFETY: WNOWAIT retained our exact wrapper leader, reserving this PGID.
        let signalled = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        if signalled == -1 {
            let error = std::io::Error::last_os_error();
            // Darwin reports EPERM when only the retained zombie remains in the group.
            assert!(
                cfg!(target_vendor = "apple") && error.raw_os_error() == Some(libc::EPERM),
                "wrapper group cleanup failed: {error}"
            );
        }
        assert!(wrapper.0.take().unwrap().wait().unwrap().success());
        peer.0
            .write_all(b"ping")
            .expect("detached child must survive wrapper cleanup");
        let mut reply = [0; 4];
        peer.read_exact(&mut reply)
            .expect("detached child must answer AFTER wrapper cleanup");
        assert_eq!(&reply, b"pong");
    }

    #[cfg(unix)]
    #[test]
    fn detached_client_dies_with_wrapper_before_startup_handoff() {
        use std::io::{Read, Write};
        use std::os::unix::process::CommandExt;
        const TEST: &str =
            "platform::process::tests::detached_client_dies_with_wrapper_before_startup_handoff";
        if std::env::var_os("V8_RUNNER_PREHANDOFF_WRAPPER").is_some() {
            let root = PathBuf::from(std::env::var_os("V8_RUNNER_DETACHED_FIXTURE_ROOT").unwrap());
            let work = WorkGiven::for_command();
            let mut request = plain_request(std::env::current_exe().unwrap());
            request.args = vec![
                "--exact".into(),
                "platform::process::tests::detached_lifecycle_client_fixture".into(),
                "--ignored".into(),
            ];
            request.startup_probe = Some(Duration::from_secs(30));
            ProcessExecutor
                .spawn(&request, &work)
                .expect("startup probe must finish before handoff");
            fs::write(root.join("handed-off"), b"receipt returned").unwrap();
            return;
        }

        let dir = tempfile::Builder::new()
            .prefix("v8p")
            .tempdir_in("/tmp")
            .unwrap();
        let listener = std::os::unix::net::UnixListener::bind(dir.path().join("s")).unwrap();
        let helper = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST])
            .env("V8_RUNNER_PREHANDOFF_WRAPPER", "1")
            .env("V8_RUNNER_CLIENT_OWNER", "unica")
            .env("V8_RUNNER_DETACHED_FIXTURE_ROOT", dir.path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut wrapper = RetainedTestWrapper(Some(helper));
        // The socket handshake proves that spawn happened. The 30-second probe
        // has not returned a receipt, so ownership has not been handed off.
        let mut peer = accept_detached_fixture(&listener);
        assert!(!dir.path().join("handed-off").exists());
        let pid = wrapper.0.as_ref().unwrap().id();
        // SAFETY: this test is the sole wait owner and has not reaped the wrapper.
        // The signal targets only the wrapper's dedicated process group.
        assert_eq!(
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) },
            0,
            "wrapper group cleanup failed: {}",
            std::io::Error::last_os_error()
        );
        let status = wrapper.0.take().unwrap().wait().unwrap();
        assert!(
            !status.success(),
            "wrapper unexpectedly completed the probe"
        );
        assert!(!dir.path().join("handed-off").exists());

        // Linux can report ECONNRESET when the killed peer closes with this
        // ping unread. Both EOF and reset prove closure; data and timeout do not.
        let _ = peer.0.write_all(b"ping");
        let mut reply = [0; 4];
        let received = peer.read(&mut reply);
        assert!(
            matches!(&received, Ok(0))
                || matches!(&received, Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset),
            "client must close its socket before startup handoff: {received:?}; reply: {reply:?}"
        );
        // Peer Drop sends stop even when the assertion fails, so the detached
        // counterexample is cleaned up without signalling a numeric client PID.
    }

    #[cfg(unix)]
    #[test]
    fn unica_owned_client_survives_handoff_and_failed_start_waits_for_host_cleanup() {
        use std::io::{Read, Write};
        use std::os::unix::process::CommandExt;
        const TEST: &str = "platform::process::tests::unica_owned_client_survives_handoff_and_failed_start_waits_for_host_cleanup";
        if let Some(case) = std::env::var_os("V8_RUNNER_OWNER_FIXTURE") {
            let root = PathBuf::from(std::env::var_os("V8_RUNNER_DETACHED_FIXTURE_ROOT").unwrap());
            let work = WorkGiven::for_command();
            if case == "failure" {
                let error = ProcessExecutor
                    .spawn(&failed_detached_fixture_request(&root), &work)
                    .unwrap_err();
                assert!(matches!(
                    error,
                    ProcessError::ExitedEarly { exit_code: 7, .. }
                ));
                assert!(!work.given());
            } else {
                let mut request = plain_request(std::env::current_exe().unwrap());
                request.args = vec![
                    "--exact".into(),
                    "platform::process::tests::detached_lifecycle_client_fixture".into(),
                    "--ignored".into(),
                ];
                request.startup_probe = Some(Duration::from_millis(250));
                ProcessExecutor.spawn(&request, &work).unwrap();
                assert!(work.given());
            }
            fs::write(root.join("receipt"), case.to_string_lossy().as_bytes()).unwrap();
            return;
        }

        for case in ["success", "failure"] {
            let dir = tempfile::Builder::new()
                .prefix("v8o")
                .tempdir_in("/tmp")
                .unwrap();
            let listener = std::os::unix::net::UnixListener::bind(dir.path().join("s")).unwrap();
            let helper = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST])
                .env("V8_RUNNER_OWNER_FIXTURE", case)
                .env("V8_RUNNER_CLIENT_OWNER", "unica")
                .env("V8_RUNNER_DETACHED_FIXTURE_ROOT", dir.path())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit())
                .process_group(0)
                .spawn()
                .unwrap();
            let mut wrapper = RetainedTestWrapper(Some(helper));
            let mut peer = accept_detached_fixture(&listener);
            let pid = wrapper.0.as_ref().unwrap().id();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                // SAFETY: the output is writable; this test alone owns the wrapper wait.
                let result = unsafe {
                    libc::waitid(
                        libc::P_PID,
                        pid,
                        &mut info,
                        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                    )
                };
                assert_eq!(
                    result,
                    0,
                    "waitid failed: {}",
                    std::io::Error::last_os_error()
                );
                if unsafe { info.si_pid() } != 0 {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "wrapper did not finish"
                );
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                fs::read_to_string(dir.path().join("receipt")).unwrap(),
                case
            );
            assert_eq!(fs::read(dir.path().join("ready")).unwrap(), b"ready");
            peer.0.write_all(b"ping").unwrap();
            let mut reply = [0; 4];
            peer.read_exact(&mut reply).unwrap();
            assert_eq!(
                &reply, b"pong",
                "runner must not clean the host-owned group"
            );
            if case == "failure" {
                // SAFETY: the wrapper has not been reaped and the test is its sole wait owner.
                assert_eq!(unsafe { libc::kill(-(pid as i32), libc::SIGKILL) }, 0);
                assert!(wrapper.0.take().unwrap().wait().unwrap().success());
                assert_eq!(
                    peer.read(&mut reply).unwrap(),
                    0,
                    "host cleans failed launch"
                );
            } else {
                // The host accepts the receipt and disarms cleanup before reaping.
                assert!(wrapper.0.take().unwrap().wait().unwrap().success());
                drop(wrapper);
                peer.0.write_all(b"ping").unwrap();
                peer.read_exact(&mut reply).unwrap();
                assert_eq!(&reply, b"pong", "handed-off client must survive owner drop");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn client_owner_rejects_unknown_value_and_unisolated_runner_before_spawn() {
        use std::os::unix::process::CommandExt;
        const TEST: &str = "platform::process::tests::client_owner_rejects_unknown_value_and_unisolated_runner_before_spawn";
        if std::env::var_os("V8_RUNNER_OWNER_REFUSAL_FIXTURE").is_some() {
            let work = WorkGiven::for_command();
            let mut request = plain_request(std::env::current_exe().unwrap());
            request.args = vec!["--list".into()];
            let error = ProcessExecutor.spawn(&request, &work).unwrap_err();
            let ProcessError::SpawnFailed { source, .. } = error else {
                panic!("unexpected refusal: {error}");
            };
            assert_eq!(source.kind(), std::io::ErrorKind::InvalidInput);
            assert!(!work.given());
            return;
        }
        for owner in ["unica", "unsupported", ""] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", TEST])
                .env("V8_RUNNER_OWNER_REFUSAL_FIXTURE", "1")
                .env("V8_RUNNER_CLIENT_OWNER", owner);
            if owner != "unica" {
                command.process_group(0);
            }
            assert!(command.status().unwrap().success(), "owner={owner:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn detached_spawn_cleans_descendant_when_startup_probe_fails() {
        assert_failed_detached_start_cleans_descendant(false);
    }

    #[cfg(unix)]
    #[test]
    fn managed_spawn_cleans_live_descendant_when_startup_probe_fails() {
        assert_failed_detached_start_cleans_descendant(true);
    }

    #[cfg(unix)]
    fn assert_failed_detached_start_cleans_descendant(managed: bool) {
        use std::io::Read;
        let dir = tempfile::Builder::new()
            .prefix("v8f")
            .tempdir_in("/tmp")
            .unwrap();
        let listener = std::os::unix::net::UnixListener::bind(dir.path().join("s")).unwrap();
        let request = failed_detached_fixture_request(dir.path());
        let work = WorkGiven::for_command();
        let result = if managed {
            ProcessExecutor
                .spawn_managed(&request, ManagedSpawnMode::Detached, Some(&work))
                .map(|child| {
                    child.terminate();
                })
        } else {
            ProcessExecutor.spawn(&request, &work).map(|_| ())
        };
        // Always acquire cleanup before assertions, including unexpected successful startup.
        let mut peer = accept_detached_fixture(&listener);
        assert!(
            matches!(result, Err(ProcessError::ExitedEarly { exit_code: 7, .. })),
            "{result:?}"
        );
        assert!(!work.given(), "failed startup must not transfer ownership");
        let mut byte = [0];
        assert_eq!(
            peer.read(&mut byte)
                .expect("failed startup must close descendant connection"),
            0,
            "failed startup must terminate the descendant, not only its exited parent"
        );
    }

    #[cfg(unix)]
    fn failed_detached_fixture_request(root: &Path) -> ProcessRequest {
        // Positional arguments avoid interpolating executable or temporary paths into shell code.
        ProcessRequest {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(),
                "export V8_RUNNER_DETACHED_FIXTURE_ROOT=\"$2\"; \"$1\" --exact platform::process::tests::detached_lifecycle_client_fixture --ignored & attempt=0; while [ ! -f \"$2/ready\" ]; do attempt=$((attempt + 1)); [ \"$attempt\" -lt 500 ] || exit 8; sleep 0.01; done; exit 7".into(),
                "fixture".into(), std::env::current_exe().unwrap().display().to_string(), root.display().to_string()],
            workdir: None, stdout_log_path: None, stderr_log_path: None,
            startup_probe: Some(Duration::from_secs(2)),
        }
    }

    #[cfg(unix)]
    #[test]
    fn startup_observation_error_preserves_descendant_after_leader_was_reaped() {
        use std::io::{Read, Write};
        let dir = tempfile::Builder::new()
            .prefix("v8e")
            .tempdir_in("/tmp")
            .unwrap();
        let listener = std::os::unix::net::UnixListener::bind(dir.path().join("s")).unwrap();
        let request = failed_detached_fixture_request(dir.path());
        let mut spawned = super::spawn_command(&request, ProcessIoMode::Detached, "fixture")
            .expect("spawn process whose ownership will be lost");
        let mut peer = accept_detached_fixture(&listener);
        assert_eq!(spawned.child.wait().unwrap().code(), Some(7));
        // Deliberately reap first: this is real lost wait authority, without an injection seam.
        let error =
            super::startup_probe_status(&mut spawned).expect_err("leader was already reaped");
        assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
        peer.0.write_all(b"ping").unwrap();
        let mut reply = [0; 4];
        peer.read_exact(&mut reply)
            .expect("observer error must not signal a stale process group");
        assert_eq!(&reply, b"pong");
    }

    #[cfg(unix)]
    #[test]
    fn spawn_returns_pid_and_binary_without_waiting() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("sleep.sh");
        write_script(&script, "sleep 0.1");

        let runner = ProcessExecutor;
        let work = WorkGiven::for_command();
        let result = runner
            .spawn(&plain_request(script.clone()), &work)
            .expect("spawn");

        assert!(result.pid > 0);
        assert_eq!(result.binary, script);
        assert!(work.given(), "a started process is the command's work");
    }

    #[cfg(unix)]
    #[test]
    fn spawn_detects_immediate_exit_when_probe_is_requested() {
        let false_binary = PathBuf::from("/usr/bin/false");
        assert!(false_binary.exists(), "/usr/bin/false must exist on Unix");

        let runner = ProcessExecutor;
        let work = WorkGiven::for_command();
        let err = runner
            .spawn(
                &ProcessRequest {
                    program: false_binary,
                    args: vec![],
                    workdir: None,
                    stdout_log_path: None,
                    stderr_log_path: None,
                    startup_probe: Some(Duration::from_millis(250)),
                },
                &work,
            )
            .expect_err("expected early exit");

        assert!(
            matches!(err, ProcessError::ExitedEarly { exit_code: 1, .. }),
            "{err:?}"
        );
        assert!(
            !work.given(),
            "a process that failed its startup probe did not start"
        );
    }

    /// Клиент, запущенный с ручкой, — тоже работа команды: отметку ставит платформа, как
    /// только процесс прошёл пробу старта.
    #[cfg(unix)]
    #[test]
    fn a_managed_process_marks_the_work_once_started() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("client.sh");
        write_script(&script, "sleep 30");
        let work = WorkGiven::for_command();

        let managed = ProcessExecutor
            .spawn_managed(
                &plain_request(script),
                ManagedSpawnMode::Detached,
                Some(&work),
            )
            .expect("spawn managed");
        let started = work.given();
        managed.terminate();

        assert!(started, "a started client is the command's work");
    }

    #[cfg(unix)]
    #[test]
    fn spawn_managed_cleans_process_group_when_startup_probe_detects_early_exit() {
        let dir = tempdir().expect("tempdir");
        let child_pid_path = dir.path().join("child.pid");
        let script = format!(
            "sleep 30 &\nprintf '%s' \"$!\" > '{}'\nexit 0",
            child_pid_path.display()
        );

        let runner = ProcessExecutor;
        let work = WorkGiven::for_command();
        let err = match runner.spawn_managed(
            &ProcessRequest {
                program: PathBuf::from("/bin/sh"),
                args: vec!["-c".to_owned(), script],
                workdir: None,
                stdout_log_path: None,
                stderr_log_path: None,
                startup_probe: Some(Duration::from_secs(2)),
            },
            ManagedSpawnMode::Detached,
            Some(&work),
        ) {
            Ok(managed) => {
                managed.terminate();
                panic!("expected managed startup probe to detect early exit");
            }
            Err(error) => error,
        };

        assert!(matches!(err, ProcessError::ExitedEarly { .. }), "{err:?}");
        assert!(!work.given(), "an early exit is a start that failed");
        let child_pid = read_pid(&child_pid_path);
        if !wait_for_process_exit(child_pid, Duration::from_secs(5)) {
            unsafe {
                let _ = libc::kill(child_pid, libc::SIGKILL);
            }
            panic!("managed startup failure should terminate process group child {child_pid}");
        }
    }

    #[cfg(windows)]
    mod windows_client_owner {
        use super::*;
        use std::io::{self, Read, Write};
        use std::net::{Shutdown, TcpListener, TcpStream};
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, TerminateJobObject,
        };
        use windows_sys::Win32::System::Threading::{
            CreateEventW, GetCurrentProcess, OpenEventW, OpenProcess, SetEvent,
            WaitForSingleObject, EVENT_MODIFY_STATE, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE,
        };

        const CASE: &str = "V8_RUNNER_WINDOWS_OWNER_CASE";
        const ADDRESS: &str = "V8_RUNNER_WINDOWS_OWNER_ADDRESS";
        const READY_EVENT: &str = "V8_RUNNER_WINDOWS_OWNER_READY_EVENT";
        const RECEIPT: &str = "V8_RUNNER_WINDOWS_OWNER_RECEIPT";
        const CLIENT_TEST: &str = "platform::process::tests::windows_client_owner::client_fixture";

        struct HostJob(Option<OwnedHandle>);

        impl Drop for HostJob {
            fn drop(&mut self) {
                if let Some(job) = &self.0 {
                    // This exact retained Job owns the fixture tree; never kill by a sampled PID.
                    unsafe { TerminateJobObject(job.as_raw_handle(), 1) };
                }
            }
        }

        struct Wrapper(std::process::Child);

        impl Drop for Wrapper {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        struct Peer(TcpStream);

        impl Drop for Peer {
            fn drop(&mut self) {
                let _ = self.0.write_all(b"stop");
                let _ = self.0.shutdown(Shutdown::Both);
            }
        }

        fn owned_handle(handle: std::os::windows::io::RawHandle) -> OwnedHandle {
            assert!(
                !handle.is_null(),
                "Windows fixture API: {}",
                io::Error::last_os_error()
            );
            // The successful creation/open call transfers this handle to this guard exactly once.
            unsafe { OwnedHandle::from_raw_handle(handle) }
        }

        fn wide(value: &str) -> Vec<u16> {
            value.encode_utf16().chain(std::iter::once(0)).collect()
        }

        fn wait(handle: &impl AsRawHandle) {
            assert_eq!(
                unsafe { WaitForSingleObject(handle.as_raw_handle(), 10_000) },
                WAIT_OBJECT_0
            );
        }

        fn ping(peer: &mut Peer) {
            peer.0.write_all(b"ping").expect("ping client");
            let mut reply = [0; 4];
            peer.0
                .read_exact(&mut reply)
                .expect("client replies after host release");
            assert_eq!(&reply, b"pong");
        }

        #[test]
        #[ignore = "subprocess fixture"]
        fn client_fixture() {
            assert!(std::env::var_os("V8_RUNNER_CLIENT_OWNER").is_none());
            let mut in_job = 0;
            assert_ne!(
                unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job) },
                0
            );
            assert_ne!(in_job, 0, "client inherits the host Job");
            let mut socket = TcpStream::connect(std::env::var(ADDRESS).expect("fixture address"))
                .expect("connect host");
            socket
                .set_read_timeout(Some(Duration::from_secs(15)))
                .expect("bound fixture lifetime");
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .expect("bound fixture writes");
            socket
                .write_all(&std::process::id().to_le_bytes())
                .expect("send client identity");
            let event_name = wide(&std::env::var(READY_EVENT).expect("fixture ready event"));
            let event =
                owned_handle(unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, event_name.as_ptr()) });
            assert_ne!(unsafe { SetEvent(event.as_raw_handle()) }, 0);
            loop {
                let mut message = [0; 4];
                if socket.read_exact(&mut message).is_err() || &message == b"stop" {
                    return;
                }
                assert_eq!(&message, b"ping");
                if socket.write_all(b"pong").is_err() {
                    return;
                }
            }
        }

        #[test]
        fn host_job_terminates_client_before_startup_handoff() {
            exercise("startup", "platform::process::tests::windows_client_owner::host_job_terminates_client_before_startup_handoff");
        }

        #[test]
        fn released_host_job_preserves_client_after_runner_exit() {
            exercise("handoff", "platform::process::tests::windows_client_owner::released_host_job_preserves_client_after_runner_exit");
        }

        fn exercise(case: &str, test_name: &str) {
            if std::env::var(CASE).as_deref() == Ok(case) {
                // Host must assign this runner to its Job before real owner-mode admission.
                let mut start = [0; 2];
                io::stdin()
                    .read_exact(&mut start)
                    .expect("host admission barrier");
                assert_eq!(&start, b"go");
                let work = WorkGiven::for_command();
                let spawned = ProcessExecutor
                    .spawn(
                        &ProcessRequest {
                            program: std::env::current_exe().expect("test executable"),
                            args: vec![
                                "--exact".into(),
                                CLIENT_TEST.into(),
                                "--ignored".into(),
                                "--nocapture".into(),
                            ],
                            workdir: None,
                            stdout_log_path: None,
                            stderr_log_path: None,
                            startup_probe: Some(Duration::from_secs(if case == "startup" {
                                30
                            } else {
                                1
                            })),
                        },
                        &work,
                    )
                    .expect("real owner-mode spawn");
                assert!(work.given());
                fs::write(
                    std::env::var_os(RECEIPT).expect("receipt path"),
                    spawned.pid.to_string(),
                )
                .expect("publish successful handoff receipt");
                return;
            }

            let dir = tempdir().expect("fixture directory");
            let receipt = dir.path().join("receipt");
            let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
            listener
                .set_nonblocking(true)
                .expect("accept only after ready event");
            let event_name = format!("Local\\v8-runner-owner-{}", uuid::Uuid::new_v4());
            let encoded_event_name = wide(&event_name);
            let event = owned_handle(unsafe {
                CreateEventW(std::ptr::null(), 1, 0, encoded_event_name.as_ptr())
            });
            // Deliberately no KILL_ON_JOB_CLOSE: successful release must preserve the client.
            let mut job = HostJob(Some(owned_handle(unsafe {
                CreateJobObjectW(std::ptr::null(), std::ptr::null())
            })));
            let mut wrapper = Wrapper(
                std::process::Command::new(std::env::current_exe().expect("test executable"))
                    .args(["--exact", test_name, "--nocapture"])
                    .env(CASE, case)
                    .env("V8_RUNNER_CLIENT_OWNER", "unica")
                    .env(
                        ADDRESS,
                        listener.local_addr().expect("listener address").to_string(),
                    )
                    .env(READY_EVENT, event_name)
                    .env(RECEIPT, &receipt)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::inherit())
                    .spawn()
                    .expect("runner wrapper"),
            );
            let job_handle = job.0.as_ref().expect("owned host job").as_raw_handle();
            assert_ne!(
                unsafe { AssignProcessToJobObject(job_handle, wrapper.0.as_raw_handle()) },
                0,
                "assign runner to host Job: {}",
                io::Error::last_os_error()
            );
            wrapper
                .0
                .stdin
                .take()
                .expect("start barrier")
                .write_all(b"go")
                .expect("admit runner");
            wait(&event);
            let mut peer = Peer(
                listener
                    .accept()
                    .expect("client connected before ready event")
                    .0,
            );
            // Winsock inherits the listener's nonblocking mode. Timeout options
            // only bound blocking reads; reset the accepted socket explicitly.
            peer.0
                .set_nonblocking(false)
                .expect("blocking fixture protocol after ready event");
            peer.0
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("bound host reads");
            peer.0
                .set_write_timeout(Some(Duration::from_secs(2)))
                .expect("bound host writes");
            let mut pid = [0; 4];
            peer.0.read_exact(&mut pid).expect("client PID");
            let pid = u32::from_le_bytes(pid);
            let client = owned_handle(unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    pid,
                )
            });
            let mut in_host_job = 0;
            assert_ne!(
                unsafe { IsProcessInJob(client.as_raw_handle(), job_handle, &mut in_host_job) },
                0
            );
            assert_ne!(
                in_host_job, 0,
                "real runner client must remain in the exact host Job"
            );
            if case == "startup" {
                assert!(
                    !receipt.exists(),
                    "client ready while spawn has not returned a receipt"
                );
                assert_ne!(unsafe { TerminateJobObject(job_handle, 1) }, 0);
                wait(&client);
                wait(&wrapper.0);
                assert!(!receipt.exists(), "host teardown precedes handoff");
                // A signaled retained process handle proves termination even if TCP reports reset.
            } else {
                wait(&wrapper.0);
                assert!(wrapper.0.wait().expect("runner exit").success());
                assert_eq!(
                    fs::read_to_string(&receipt).expect("handoff receipt"),
                    pid.to_string()
                );
                drop(wrapper);
                drop(job.0.take()); // Explicit successful host release; no termination on drop.
                ping(&mut peer);
                drop(peer);
                wait(&client);
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn detached_child_does_not_hold_redirected_stdout_open() {
        assert_redirected_stdout_reaches_eof(
            RedirectedStdoutSpawnMode::Detached,
            "V8_RUNNER_WINDOWS_STDIO_ISOLATION_DETACHED_HELPER",
            "platform::process::tests::detached_child_does_not_hold_redirected_stdout_open",
        );
    }

    #[cfg(windows)]
    #[test]
    fn managed_detached_child_does_not_hold_redirected_stdout_open() {
        assert_redirected_stdout_reaches_eof(
            RedirectedStdoutSpawnMode::ManagedDetached,
            "V8_RUNNER_WINDOWS_STDIO_ISOLATION_MANAGED_HELPER",
            "platform::process::tests::managed_detached_child_does_not_hold_redirected_stdout_open",
        );
    }

    #[cfg(windows)]
    #[derive(Debug, Clone, Copy)]
    enum RedirectedStdoutSpawnMode {
        Detached,
        ManagedDetached,
    }

    #[cfg(windows)]
    fn assert_redirected_stdout_reaches_eof(
        spawn_mode: RedirectedStdoutSpawnMode,
        helper_env: &str,
        test_name: &str,
    ) {
        const PID_FILE_ENV: &str = "V8_RUNNER_WINDOWS_STDIO_ISOLATION_PID_FILE";

        if std::env::var_os(helper_env).is_some() {
            let pid_file = PathBuf::from(
                std::env::var_os(PID_FILE_ENV).expect("helper PID file environment variable"),
            );
            let request = ProcessRequest {
                program: PathBuf::from("powershell.exe"),
                args: vec![
                    "-NoProfile".to_owned(),
                    "-Command".to_owned(),
                    "Start-Sleep -Seconds 30".to_owned(),
                ],
                workdir: None,
                stdout_log_path: None,
                stderr_log_path: None,
                startup_probe: None,
            };
            let pid = match spawn_mode {
                RedirectedStdoutSpawnMode::Detached => {
                    ProcessExecutor
                        .spawn(&request, &WorkGiven::for_command())
                        .expect("spawn detached helper child")
                        .pid
                }
                RedirectedStdoutSpawnMode::ManagedDetached => {
                    ProcessExecutor
                        .spawn_managed(&request, ManagedSpawnMode::Detached, None)
                        .expect("spawn managed-detached helper child")
                        .detach()
                        .pid
                }
            };
            fs::write(pid_file, pid.to_string()).expect("write detached child PID");
            return;
        }

        let dir = tempdir().expect("tempdir");
        let pid_file = dir.path().join("detached-child.pid");
        let mut helper = std::process::Command::new(
            std::env::current_exe().expect("current unit-test executable"),
        )
        .args(["--exact", test_name, "--nocapture"])
        .env(helper_env, "1")
        .env(PID_FILE_ENV, &pid_file)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn test helper");
        let mut stdout = helper.stdout.take().expect("helper stdout pipe");
        let (eof_sender, eof_receiver) = std::sync::mpsc::channel();
        let reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = std::io::Read::read_to_end(&mut stdout, &mut bytes).map(|_| bytes);
            let _ = eof_sender.send(result);
        });

        let detached_pid = read_pid(&pid_file);
        let eof_before_cleanup = eof_receiver.recv_timeout(Duration::from_secs(2));
        let detached_child_was_alive = process_exists(detached_pid);

        let cleanup_status = terminate_windows_process_tree_for_test(detached_pid);
        let eof_after_cleanup = if eof_before_cleanup.is_err() {
            Some(eof_receiver.recv_timeout(Duration::from_secs(2)))
        } else {
            None
        };
        let helper_status = wait_for_test_child_exit(&mut helper, Duration::from_secs(2));
        if matches!(&helper_status, Ok(None)) {
            let _ = helper.kill();
        }
        drop(reader);

        assert!(
            matches!(&cleanup_status, Ok(status) if status.success()),
            "detached process tree cleanup must succeed: {cleanup_status:?}"
        );
        assert!(
            matches!(&helper_status, Ok(Some(status)) if status.success()),
            "test helper must exit successfully within the deadline: {helper_status:?}"
        );
        assert!(
            detached_child_was_alive,
            "detached child must still be alive when stdout reaches EOF"
        );
        assert!(
            matches!(eof_before_cleanup, Ok(Ok(_))),
            "redirected stdout did not reach EOF before detached child cleanup: {eof_before_cleanup:?}; post-cleanup result: {eof_after_cleanup:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn spawn_managed_terminates_windows_job_descendants() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("spawn-child.ps1");
        let child_pid_path = dir.path().join("child.pid");
        fs::write(
            &script,
            format!(
                "$child = Start-Process -FilePath powershell.exe -WindowStyle Hidden -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 30') -PassThru\nSet-Content -LiteralPath {} -Value $child.Id\nStart-Sleep -Seconds 30\n",
                powershell_literal(&child_pid_path)
            ),
        )
        .expect("write script");

        let runner = ProcessExecutor;
        let managed = runner
            .spawn_managed(
                &ProcessRequest {
                    program: PathBuf::from("powershell.exe"),
                    args: vec![
                        "-NoProfile".to_owned(),
                        "-ExecutionPolicy".to_owned(),
                        "Bypass".to_owned(),
                        "-File".to_owned(),
                        script.display().to_string(),
                    ],
                    workdir: None,
                    stdout_log_path: None,
                    stderr_log_path: None,
                    startup_probe: None,
                },
                ManagedSpawnMode::Detached,
                None,
            )
            .expect("spawn managed");

        let child_pid = read_pid(&child_pid_path);
        managed.terminate();
        if !wait_for_process_exit(child_pid, Duration::from_secs(2)) {
            let cleanup = terminate_windows_process_tree_for_test(child_pid);
            panic!(
                "managed termination should terminate Windows job child {child_pid}; fallback cleanup: {cleanup:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn spawn_managed_cleans_windows_job_when_startup_probe_detects_early_exit() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("spawn-child-and-exit.ps1");
        let child_pid_path = dir.path().join("child.pid");
        fs::write(
            &script,
            format!(
                "$child = Start-Process -FilePath powershell.exe -WindowStyle Hidden -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 30') -PassThru\nSet-Content -LiteralPath {} -Value $child.Id\nexit 0\n",
                powershell_literal(&child_pid_path)
            ),
        )
        .expect("write script");

        let runner = ProcessExecutor;
        let err = match runner.spawn_managed(
            &ProcessRequest {
                program: PathBuf::from("powershell.exe"),
                args: vec![
                    "-NoProfile".to_owned(),
                    "-ExecutionPolicy".to_owned(),
                    "Bypass".to_owned(),
                    "-File".to_owned(),
                    script.display().to_string(),
                ],
                workdir: None,
                stdout_log_path: None,
                stderr_log_path: None,
                startup_probe: Some(Duration::from_millis(200)),
            },
            ManagedSpawnMode::Detached,
            None,
        ) {
            Ok(managed) => {
                managed.terminate();
                panic!("expected managed startup probe to detect early exit");
            }
            Err(error) => error,
        };

        assert!(matches!(err, ProcessError::ExitedEarly { .. }));
        let child_pid = read_pid(&child_pid_path);
        if !wait_for_process_exit(child_pid, Duration::from_secs(2)) {
            let cleanup = terminate_windows_process_tree_for_test(child_pid);
            panic!(
                "managed startup failure should terminate Windows job child {child_pid}; fallback cleanup: {cleanup:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn run_surfaces_stdout_log_write_failures_separately() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("echo.sh");
        write_script(&script, "echo hello");

        let runner = ProcessExecutor;
        let err = runner
            .run_with_policy(
                &ProcessRequest {
                    program: script,
                    args: vec![],
                    workdir: None,
                    stdout_log_path: Some(dir.path().join("missing").join("stdout.log")),
                    stderr_log_path: None,
                    startup_probe: None,
                },
                &ProcessExecutionPolicy::default(),
            )
            .expect_err("expected log write failure");

        assert!(matches!(err, ProcessError::StdoutLogIo { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn run_with_timeout_returns_timeout_error() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("sleep.sh");
        write_script(&script, "sleep 2");

        let runner = ProcessExecutor;
        let err = runner
            .run_with_policy(
                &ProcessRequest {
                    program: script,
                    args: vec![],
                    workdir: None,
                    stdout_log_path: None,
                    stderr_log_path: None,
                    startup_probe: None,
                },
                &ProcessExecutionPolicy::new(
                    Some(Duration::from_millis(100)),
                    CancellationToken::new(),
                    ProcessInterruptionSafety::Interruptible,
                    crate::platform::process::WorkGiven::for_command(),
                ),
            )
            .expect_err("expected timeout");

        assert!(matches!(err, ProcessError::TimedOut { .. }));
    }

    /// Статус прерывания приходит, когда процесс уже подобран: ответ «отменено» или «предел
    /// истёк» при живом процессе врал бы, что работа прекращена. Подобранный процесс не
    /// отвечает на нулевой сигнал, а зомби ещё отвечал бы.
    #[cfg(unix)]
    #[test]
    fn an_interrupted_process_is_reaped_before_the_answer() {
        for safety in [
            ProcessInterruptionSafety::Interruptible,
            ProcessInterruptionSafety::GracefulThenKill,
        ] {
            for by_timeout in [true, false] {
                let dir = tempdir().expect("tempdir");
                let pid_file = dir.path().join("pid");
                let script = dir.path().join("sleep.sh");
                write_script(
                    &script,
                    &format!("echo $$ > '{}'\nexec sleep 10", pid_file.display()),
                );
                let cancellation = CancellationToken::new();
                let timeout = by_timeout.then_some(Duration::from_secs(3));
                let canceller = (!by_timeout).then(|| {
                    let cancellation = cancellation.clone();
                    let pid_file = pid_file.clone();
                    // Отмена приходит, когда процесс записал номер, и в любом случае: иначе
                    // тест ждал бы конца `sleep` и падал бы не там.
                    thread::spawn(move || {
                        let deadline = std::time::Instant::now() + Duration::from_secs(5);
                        while std::time::Instant::now() < deadline
                            && fs::read_to_string(&pid_file)
                                .ok()
                                .and_then(|text| text.trim().parse::<i32>().ok())
                                .is_none()
                        {
                            thread::sleep(Duration::from_millis(10));
                        }
                        cancellation.cancel();
                    })
                });

                let err = ProcessExecutor
                    .run_with_policy(
                        &ProcessRequest {
                            program: script,
                            args: vec![],
                            workdir: None,
                            stdout_log_path: None,
                            stderr_log_path: None,
                            startup_probe: None,
                        },
                        &ProcessExecutionPolicy::new(
                            timeout,
                            cancellation,
                            safety,
                            crate::platform::process::WorkGiven::for_command(),
                        ),
                    )
                    .expect_err("the process must be interrupted");
                if let Some(canceller) = canceller {
                    canceller.join().expect("canceller");
                }

                let expected = if by_timeout {
                    matches!(err, ProcessError::TimedOut { .. })
                } else {
                    matches!(err, ProcessError::Cancelled { .. })
                };
                assert!(expected, "{safety:?}, by timeout {by_timeout}: {err:?}");
                let pid = read_pid(&pid_file);
                assert!(
                    !process_exists(pid),
                    "{safety:?}, by timeout {by_timeout}: the interrupted process {pid} is still there"
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn run_with_policy_cancels_interruptible_process() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("sleep.sh");
        write_script(&script, "sleep 2");
        let cancellation = CancellationToken::new();
        let cancellation_clone = cancellation.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancellation_clone.cancel();
        });

        let runner = ProcessExecutor;
        let err = runner
            .run_with_policy(
                &ProcessRequest {
                    program: script,
                    args: vec![],
                    workdir: None,
                    stdout_log_path: None,
                    stderr_log_path: None,
                    startup_probe: None,
                },
                &ProcessExecutionPolicy::new(
                    None,
                    cancellation,
                    ProcessInterruptionSafety::Interruptible,
                    crate::platform::process::WorkGiven::for_command(),
                ),
            )
            .expect_err("expected cancellation");

        assert!(matches!(err, ProcessError::Cancelled { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn run_with_policy_defers_timeout_for_critical_process() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("sleep.sh");
        write_script(&script, "sleep 0.1\nprintf 'done\\n'");

        let runner = ProcessExecutor;
        let result = runner
            .run_with_policy(
                &ProcessRequest {
                    program: script,
                    args: vec![],
                    workdir: None,
                    stdout_log_path: None,
                    stderr_log_path: None,
                    startup_probe: None,
                },
                &ProcessExecutionPolicy::new(
                    Some(Duration::from_millis(10)),
                    CancellationToken::new(),
                    ProcessInterruptionSafety::CriticalNonAbortable,
                    crate::platform::process::WorkGiven::for_command(),
                ),
            )
            .expect("critical process must reach terminal success");

        assert_eq!(result.exit_code, 0);
        assert_eq!(
            result.interruption,
            Some(super::ProcessInterruption {
                reason: ProcessInterruptionReason::TimedOut,
                action: ProcessInterruptionAction::Deferred,
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_handles_large_stdout_without_deadlock() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("large.sh");
        write_script(
            &script,
            "i=0\nwhile [ \"$i\" -lt 20000 ]; do\n  printf 'line%05d\\n' \"$i\"\n  i=$((i+1))\ndone\nexit 0",
        );

        let runner = ProcessExecutor;
        let result = runner
            .run_with_policy(
                &ProcessRequest {
                    program: script,
                    args: vec![],
                    workdir: None,
                    stdout_log_path: None,
                    stderr_log_path: None,
                    startup_probe: None,
                },
                &ProcessExecutionPolicy::default(),
            )
            .expect("run");

        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("line19999"));
    }

    #[cfg(unix)]
    fn read_pid(path: &Path) -> i32 {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < deadline {
            // Оболочка создаёт файл раньше, чем пишет в него номер: пустой ещё не записан.
            if let Some(pid) = fs::read_to_string(path)
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                return pid;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("child pid file was not written: {}", path.display());
    }

    #[cfg(unix)]
    fn process_exists(pid: i32) -> bool {
        // An unreaped zombie still exists here: the tests also check that a child is reaped.
        u32::try_from(pid).is_ok_and(crate::support::machine::is_process_alive)
    }

    #[cfg(unix)]
    fn wait_for_process_exit(pid: i32, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if !process_exists(pid) {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        !process_exists(pid)
    }

    #[cfg(windows)]
    fn read_pid(path: &Path) -> u32 {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if let Ok(pid) = fs::read_to_string(path) {
                return pid.trim().parse().expect("child pid");
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("child pid file was not written: {}", path.display());
    }

    #[cfg(windows)]
    fn process_exists(pid: u32) -> bool {
        crate::support::machine::is_process_alive(pid)
    }

    #[cfg(windows)]
    fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if !process_exists(pid) {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        !process_exists(pid)
    }

    #[cfg(windows)]
    fn wait_for_test_child_exit(
        child: &mut std::process::Child,
        timeout: Duration,
    ) -> std::io::Result<Option<std::process::ExitStatus>> {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if let Some(status) = child.try_wait()? {
                return Ok(Some(status));
            }
            thread::sleep(Duration::from_millis(25));
        }
        child.try_wait()
    }

    #[cfg(windows)]
    fn terminate_windows_process_tree_for_test(
        pid: u32,
    ) -> std::io::Result<std::process::ExitStatus> {
        std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
    }

    #[cfg(windows)]
    fn powershell_literal(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "''"))
    }
}
