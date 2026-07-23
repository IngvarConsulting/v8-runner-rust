use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

const EXECUTABLE_BUSY_MAX_RETRIES: usize = 5;
const EXECUTABLE_BUSY_RETRY_DELAY: Duration = Duration::from_millis(10);

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

/// Shared execution policy passed from transport-neutral command context into the runner.
#[derive(Debug, Clone)]
pub struct ProcessExecutionPolicy {
    pub timeout: Option<Duration>,
    pub cancellation: CancellationToken,
    pub safety: ProcessInterruptionSafety,
    pub graceful_shutdown_timeout: Duration,
}

impl Default for ProcessExecutionPolicy {
    fn default() -> Self {
        Self {
            timeout: None,
            cancellation: CancellationToken::new(),
            safety: ProcessInterruptionSafety::Interruptible,
            graceful_shutdown_timeout: Duration::from_millis(250),
        }
    }
}

impl ProcessExecutionPolicy {
    pub fn new(
        timeout: Option<Duration>,
        cancellation: CancellationToken,
        safety: ProcessInterruptionSafety,
    ) -> Self {
        Self {
            timeout,
            cancellation,
            safety,
            ..Self::default()
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
    Cancelled { cmd: String },

    #[error("process timed out '{cmd}' after {timeout_ms}ms")]
    TimedOut { cmd: String, timeout_ms: u64 },

    #[error("managed process spawn is not supported for '{cmd}'")]
    ManagedSpawnUnsupported { cmd: String },
}

/// Boundary for synchronous and detached process execution.
pub trait ProcessRunner {
    /// Execute a process and wait for completion, capturing stdout/stderr.
    fn run(&self, request: &ProcessRequest) -> Result<ProcessResult, ProcessError>;

    /// Execute a process with a hard timeout, terminating the process group if needed.
    fn run_with_timeout(
        &self,
        request: &ProcessRequest,
        timeout: Duration,
    ) -> Result<ProcessResult, ProcessError>;

    /// Execute a process using the shared command-boundary execution policy.
    fn run_with_policy(
        &self,
        request: &ProcessRequest,
        policy: &ProcessExecutionPolicy,
    ) -> Result<ProcessResult, ProcessError> {
        match policy.timeout {
            Some(timeout) => self.run_with_timeout(request, timeout),
            None => self.run(request),
        }
    }

    /// Start a process in fire-and-forget mode without waiting for completion.
    fn spawn(&self, request: &ProcessRequest) -> Result<SpawnResult, ProcessError>;

    /// Start a process and keep a handle until the caller detaches or terminates it.
    fn spawn_managed(
        &self,
        request: &ProcessRequest,
        mode: ManagedSpawnMode,
    ) -> Result<ManagedSpawnResult, ProcessError> {
        let _ = mode;
        Err(ProcessError::ManagedSpawnUnsupported {
            cmd: render_command(request),
        })
    }
}

/// Standard subprocess runner backed by `std::process::Command`.
pub struct ProcessExecutor;

impl ProcessRunner for ProcessExecutor {
    fn run(&self, request: &ProcessRequest) -> Result<ProcessResult, ProcessError> {
        self.run_internal(request, &ProcessExecutionPolicy::default())
    }

    fn run_with_timeout(
        &self,
        request: &ProcessRequest,
        timeout: Duration,
    ) -> Result<ProcessResult, ProcessError> {
        self.run_internal(
            request,
            &ProcessExecutionPolicy::new(
                Some(timeout),
                CancellationToken::new(),
                ProcessInterruptionSafety::Interruptible,
            ),
        )
    }

    fn run_with_policy(
        &self,
        request: &ProcessRequest,
        policy: &ProcessExecutionPolicy,
    ) -> Result<ProcessResult, ProcessError> {
        self.run_internal(request, policy)
    }

    fn spawn(&self, request: &ProcessRequest) -> Result<SpawnResult, ProcessError> {
        let rendered_command = render_command(request);
        debug!(command = rendered_command.as_str(), "spawning process");
        let spawned = spawn_checked_child(request, ProcessIoMode::Detached, &rendered_command)?;
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
            });
        }
        if policy.timeout.is_some_and(|timeout| timeout.is_zero()) {
            return Err(ProcessError::TimedOut {
                cmd: rendered_command,
                timeout_ms: 0,
            });
        }
        let spawned = spawn_command(request, ProcessIoMode::Captured, &rendered_command)?;
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
    ManagedDetached,
    ManagedWait,
    Captured,
}

struct SpawnedChild {
    child: ChildHandle,
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
        if let Some(status) =
            spawned
                .child
                .try_wait()
                .map_err(|source| ProcessError::StartupCheckFailed {
                    cmd: rendered_command.to_owned(),
                    source,
                })?
        {
            warn!(
                command = rendered_command,
                exit_code = status.code().unwrap_or(-1),
                "process exited during startup probe"
            );
            if matches!(
                io_mode,
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

fn spawn_command(
    request: &ProcessRequest,
    io_mode: ProcessIoMode,
    rendered_command: &str,
) -> Result<SpawnedChild, ProcessError> {
    for attempt in 0..=EXECUTABLE_BUSY_MAX_RETRIES {
        let cmd = build_command(request, io_mode, rendered_command)?;
        match spawn_child(cmd, io_mode) {
            Ok(child) => return Ok(SpawnedChild { child }),
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
    cmd.args(&request.args);
    if let Some(workdir) = &request.workdir {
        cmd.current_dir(workdir);
    }
    cmd.stdin(Stdio::null());
    match io_mode {
        ProcessIoMode::Detached => {
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
            let stdout = stdout_reader.join().unwrap_or_default();
            let stderr = stderr_reader.join().unwrap_or_default();
            return match observed_interruption {
                Some(ProcessInterruptionReason::Cancelled)
                    if policy.safety != ProcessInterruptionSafety::CriticalNonAbortable =>
                {
                    Err(ProcessError::Cancelled {
                        cmd: rendered_command.to_owned(),
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
                    interruption: Some(ProcessInterruption {
                        reason,
                        action: ProcessInterruptionAction::Deferred,
                    }),
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
                "interruption requested during critical process phase; waiting for terminal outcome"
            );
            Ok(None)
        }
        ProcessInterruptionSafety::Interruptible => {
            terminate_child_group(spawned);
            let _ = spawned.child.wait();
            Ok(Some(process_error_from_reason(
                rendered_command,
                policy.timeout,
                reason,
            )))
        }
        ProcessInterruptionSafety::GracefulThenKill => {
            terminate_child_group_gracefully(spawned, policy.graceful_shutdown_timeout);
            let _ = spawned.child.wait();
            Ok(Some(process_error_from_reason(
                rendered_command,
                policy.timeout,
                reason,
            )))
        }
    }
}

fn process_error_from_reason(
    rendered_command: &str,
    timeout: Option<Duration>,
    reason: ProcessInterruptionReason,
) -> ProcessError {
    match reason {
        ProcessInterruptionReason::Cancelled => ProcessError::Cancelled {
            cmd: rendered_command.to_owned(),
        },
        ProcessInterruptionReason::TimedOut => ProcessError::TimedOut {
            cmd: rendered_command.to_owned(),
            timeout_ms: timeout.unwrap_or_default().as_millis() as u64,
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

fn render_command(request: &ProcessRequest) -> String {
    let mut parts = Vec::with_capacity(request.args.len() + 1);
    parts.push(request.program.display().to_string());
    let mut skip_next = false;
    for arg in &request.args {
        if skip_next {
            parts.push("***".to_owned());
            skip_next = false;
        } else if is_sensitive_flag(arg) {
            parts.push(arg.clone());
            skip_next = true;
        } else if let Some((key, _)) = split_sensitive_assignment(arg) {
            parts.push(format!("{key}=***"));
        } else {
            parts.push(arg.clone());
        }
    }
    parts.join(" ")
}

fn is_sensitive_flag(arg: &str) -> bool {
    const FLAGS: &[&str] = &[
        "/N",
        "-N",
        "/P",
        "-P",
        "--user",
        "--database-user",
        "--db-user",
        "--target-database-user",
        "--target-db-user",
        "--password",
        "--database-password",
        "--db-pwd",
        "--target-database-password",
        "--target-db-pwd",
    ];

    FLAGS.iter().any(|flag| arg.eq_ignore_ascii_case(flag))
}

fn split_sensitive_assignment(arg: &str) -> Option<(&str, &str)> {
    const FLAGS: &[&str] = &[
        "/N",
        "-N",
        "/P",
        "-P",
        "--user",
        "--database-user",
        "--db-user",
        "--target-database-user",
        "--target-db-user",
        "--password",
        "--database-password",
        "--db-pwd",
        "--target-database-password",
        "--target-db-pwd",
    ];

    let (key, value) = arg.split_once('=')?;
    if FLAGS.iter().any(|flag| key.eq_ignore_ascii_case(flag)) {
        Some((key, value))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        render_command, ManagedSpawnMode, ProcessError, ProcessExecutionPolicy, ProcessExecutor,
        ProcessInterruptionAction, ProcessInterruptionReason, ProcessInterruptionSafety,
        ProcessRequest, ProcessRunner,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
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
            .run(&ProcessRequest {
                program: script,
                args: vec![],
                workdir: None,
                stdout_log_path: Some(stdout_log.clone()),
                stderr_log_path: Some(stderr_log.clone()),
                startup_probe: None,
            })
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

    #[test]
    fn render_command_keeps_infobase_connection_string_visible() {
        let request = ProcessRequest {
            program: PathBuf::from("1cv8c"),
            args: vec![
                "/IBConnectionString".to_owned(),
                "Srvr=host;Ref=base;Usr=alice;Pwd=secret".to_owned(),
            ],
            workdir: None,
            stdout_log_path: None,
            stderr_log_path: None,
            startup_probe: None,
        };

        let rendered = render_command(&request);

        assert!(rendered.contains("/IBConnectionString Srvr=host;Ref=base;Usr=alice;Pwd=secret"));
    }

    #[test]
    fn render_command_keeps_infobase_connection_string_assignment_visible() {
        let request = ProcessRequest {
            program: PathBuf::from("1cv8c"),
            args: vec!["/IBConnectionString=File=/tmp/ib;usr=alice;PWD=secret".to_owned()],
            workdir: None,
            stdout_log_path: None,
            stderr_log_path: None,
            startup_probe: None,
        };

        let rendered = render_command(&request);

        assert!(rendered.contains("/IBConnectionString=File=/tmp/ib;usr=alice;PWD=secret"));
    }

    #[test]
    fn render_command_keeps_combined_infobase_connection_token_visible() {
        let request = ProcessRequest {
            program: PathBuf::from("1cv8c"),
            args: vec!["/IBConnectionStringSrvr=host;Ref=base;Usr=alice;Pwd=secret".to_owned()],
            workdir: None,
            stdout_log_path: None,
            stderr_log_path: None,
            startup_probe: None,
        };

        let rendered = render_command(&request);

        assert!(rendered.contains("/IBConnectionStringSrvr=host;Ref=base;Usr=alice;Pwd=secret"));
    }

    #[test]
    fn render_command_keeps_quoted_infobase_connection_values_visible() {
        let request = ProcessRequest {
            program: PathBuf::from("1cv8c"),
            args: vec![
                "/IBConnectionString".to_owned(),
                "File=/tmp/ib;Usr=alice;Pwd=\"sec;ret\";Ref=base".to_owned(),
            ],
            workdir: None,
            stdout_log_path: None,
            stderr_log_path: None,
            startup_probe: None,
        };

        let rendered = render_command(&request);

        assert!(rendered
            .contains("/IBConnectionString File=/tmp/ib;Usr=alice;Pwd=\"sec;ret\";Ref=base"));
    }

    #[cfg(unix)]
    #[test]
    fn spawn_returns_pid_and_binary_without_waiting() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("sleep.sh");
        write_script(&script, "sleep 0.1");

        let runner = ProcessExecutor;
        let result = runner
            .spawn(&ProcessRequest {
                program: script.clone(),
                args: vec![],
                workdir: None,
                stdout_log_path: None,
                stderr_log_path: None,
                startup_probe: None,
            })
            .expect("spawn");

        assert!(result.pid > 0);
        assert_eq!(result.binary, script);
    }

    #[cfg(unix)]
    #[test]
    fn spawn_detects_immediate_exit_when_probe_is_requested() {
        let false_binary = PathBuf::from("/usr/bin/false");
        assert!(false_binary.exists(), "/usr/bin/false must exist on Unix");

        let runner = ProcessExecutor;
        let err = runner
            .spawn(&ProcessRequest {
                program: false_binary,
                args: vec![],
                workdir: None,
                stdout_log_path: None,
                stderr_log_path: None,
                startup_probe: Some(Duration::from_millis(250)),
            })
            .expect_err("expected early exit");

        assert!(matches!(
            err,
            ProcessError::ExitedEarly { exit_code: 1, .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn spawn_managed_cleans_process_group_when_startup_probe_detects_early_exit() {
        let dir = tempdir().expect("tempdir");
        let script = dir.path().join("fork-and-exit.sh");
        let child_pid_path = dir.path().join("child.pid");
        write_script(
            &script,
            &format!(
                "sleep 5 &\nprintf '%s' \"$!\" > '{}'\nexit 0",
                child_pid_path.display()
            ),
        );

        let runner = ProcessExecutor;
        let err = match runner.spawn_managed(
            &ProcessRequest {
                program: script,
                args: vec![],
                workdir: None,
                stdout_log_path: None,
                stderr_log_path: None,
                startup_probe: Some(Duration::from_millis(100)),
            },
            ManagedSpawnMode::Detached,
        ) {
            Ok(managed) => {
                managed.terminate();
                panic!("expected managed startup probe to detect early exit");
            }
            Err(error) => error,
        };

        assert!(matches!(err, ProcessError::ExitedEarly { .. }));
        let child_pid = read_pid(&child_pid_path);
        if process_exists(child_pid) {
            unsafe {
                let _ = libc::kill(child_pid, libc::SIGKILL);
            }
            panic!("managed startup failure should terminate process group child {child_pid}");
        }
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
            )
            .expect("spawn managed");

        let child_pid = read_pid(&child_pid_path);
        managed.terminate();
        if !wait_for_process_exit(child_pid, Duration::from_secs(2)) {
            terminate_windows_process_tree_for_test(child_pid);
            panic!("managed termination should terminate Windows job child {child_pid}");
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
            terminate_windows_process_tree_for_test(child_pid);
            panic!("managed startup failure should terminate Windows job child {child_pid}");
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
            .run(&ProcessRequest {
                program: script,
                args: vec![],
                workdir: None,
                stdout_log_path: Some(dir.path().join("missing").join("stdout.log")),
                stderr_log_path: None,
                startup_probe: None,
            })
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
            .run_with_timeout(
                &ProcessRequest {
                    program: script,
                    args: vec![],
                    workdir: None,
                    stdout_log_path: None,
                    stderr_log_path: None,
                    startup_probe: None,
                },
                Duration::from_millis(100),
            )
            .expect_err("expected timeout");

        assert!(matches!(err, ProcessError::TimedOut { .. }));
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
            .run(&ProcessRequest {
                program: script,
                args: vec![],
                workdir: None,
                stdout_log_path: None,
                stderr_log_path: None,
                startup_probe: None,
            })
            .expect("run");

        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("line19999"));
    }

    #[cfg(unix)]
    fn read_pid(path: &Path) -> i32 {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < deadline {
            if let Ok(pid) = fs::read_to_string(path) {
                return pid.trim().parse().expect("child pid");
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("child pid file was not written: {}", path.display());
    }

    #[cfg(unix)]
    fn process_exists(pid: i32) -> bool {
        unsafe {
            if libc::kill(pid, 0) == 0 {
                return true;
            }
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
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
        std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"
                ),
            ])
            .status()
            .is_ok_and(|status| status.success())
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
    fn terminate_windows_process_tree_for_test(pid: u32) {
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    #[cfg(windows)]
    fn powershell_literal(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "''"))
    }
}
