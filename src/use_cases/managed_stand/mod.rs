//! Управляемый стенд: единственный владелец записи о службе, различия своего и чужого, порядка
//! подъёма и отмены. Команды `dbms` здесь; `cluster` и `standalone` лягут на тот же механизм.

mod record;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::config::model::AppConfig;
use crate::domain::dbms::{DbmsAction, DbmsResult, DbmsState};
use crate::domain::execution::ExecutionInterruptionPhase;
use crate::platform::postgres::{self, PostgresBuild};
use crate::platform::process::{
    ProcessExecutionPolicy, ProcessExecutor, ProcessRequest, ProcessRunner, WorkGiven,
};
use crate::platform::service::{self, ProcessFacts, ServiceRequest, ServiceSpawnError};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::interruption::{
    deferred_process_interruption, interruption_before_safe_point,
};
use crate::use_cases::result::{
    stamp_dispatch, UseCaseError, UseCaseErrorKind, UseCaseFailure, UseCaseResult,
};
use record::{
    machine_fingerprint, read_json, write_json, PinnedPorts, RecordState, ServicePaths,
    ServiceRecord,
};

/// Сколько ждать готовности и конца процесса.
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const STOP_TIMEOUT: Duration = Duration::from_secs(70);
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(200);
/// Порты вне динамического диапазона Windows (49152 и выше) и вне первых тысяч.
const PORT_BASE: u32 = 20_000;
const PORT_SPAN: u32 = 29_000;
const PORT_TRIES: u32 = 64;
const PASSWORD_LENGTH: usize = 24;
/// Службы сервера над этой СУБД: их запись делает `dbms stop` отказом.
const SERVERS: [(&str, &str); 2] = [
    ("cluster", "cluster stop"),
    ("standalone", "standalone stop"),
];

/// Что о машине и её процессах нужно сценарию. Подменяется в тестах.
pub(crate) trait ServiceHost {
    fn supported(&self) -> bool;
    fn spawn(&self, request: &ServiceRequest, work: &WorkGiven) -> Result<u32, ServiceSpawnError>;
    fn facts(&self, pid: u32) -> Option<ProcessFacts>;
    fn machine(&self) -> Option<String>;
    fn listening_pid(&self, port: u16) -> Option<u32>;
    fn port_answers(&self, port: u16) -> bool;
    fn hard_stop(&self, runner: &dyn ProcessRunner, pid: u32, context: &ExecutionContext);
    fn sleep(&self, duration: Duration);
    fn ready_timeout(&self) -> Duration {
        READY_TIMEOUT
    }
    fn stop_timeout(&self) -> Duration {
        STOP_TIMEOUT
    }
    fn identify_timeout(&self) -> Duration {
        IDENTIFY_TIMEOUT
    }
}

struct RealHost;

impl ServiceHost for RealHost {
    fn supported(&self) -> bool {
        cfg!(windows)
    }
    fn spawn(&self, request: &ServiceRequest, work: &WorkGiven) -> Result<u32, ServiceSpawnError> {
        ProcessExecutor.spawn_service(request, work)
    }
    fn facts(&self, pid: u32) -> Option<ProcessFacts> {
        service::process_facts(pid)
    }
    fn machine(&self) -> Option<String> {
        machine_fingerprint()
    }
    fn listening_pid(&self, port: u16) -> Option<u32> {
        service::listening_pid(port)
    }
    fn port_answers(&self, port: u16) -> bool {
        service::port_answers(port)
    }
    fn hard_stop(&self, runner: &dyn ProcessRunner, pid: u32, context: &ExecutionContext) {
        let request = ProcessRequest {
            program: PathBuf::from("taskkill"),
            args: vec![
                "/PID".to_owned(),
                pid.to_string(),
                "/T".to_owned(),
                "/F".to_owned(),
            ],
            workdir: None,
            stdout_log_path: None,
            stderr_log_path: None,
            startup_probe: None,
            windows_verbatim_args: Vec::new(),
        };
        let _ = runner.run_with_policy(&request, &policy(context));
    }
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

fn policy(context: &ExecutionContext) -> ProcessExecutionPolicy {
    context.process_policy(InterruptionSafetyClass::GracefulThenKill, None)
}

fn validation(message: impl Into<String>) -> UseCaseError {
    UseCaseError::new(UseCaseErrorKind::Validation, message)
}

fn environment(message: impl Into<String>) -> UseCaseError {
    UseCaseError::new(UseCaseErrorKind::Environment, message)
}

fn io_failure(what: &str, path: &Path, error: &std::io::Error) -> UseCaseError {
    UseCaseError::new(
        UseCaseErrorKind::Runtime,
        format!("{what} '{}': {error}", path.display()),
    )
}

fn not_on_this_system() -> UseCaseError {
    environment(
        "managed services run on Windows only: on this system the runner starts no PostgreSQL",
    )
}

/// База команды: объявленная, с `dbms.managed`. Строка соединения — отказ
/// (`INV.CLI.SERVICE-COMMANDS-TAKE-A-DECLARED-BASE`).
fn declared_base(config: &AppConfig) -> Result<String, UseCaseError> {
    config.infobase_name.clone().ok_or_else(|| {
        validation("dbms takes a declared infobase: a connection string has neither a record nor a managed section")
    })
}

struct Stand<'a> {
    config: &'a AppConfig,
    context: &'a ExecutionContext,
    runner: &'a dyn ProcessRunner,
    host: &'a dyn ServiceHost,
    infobase: String,
    paths: ServicePaths,
    started_at: Instant,
}

/// Служба, как её видит запись и система.
enum Observed {
    Nothing,
    /// Запись есть, но процесса из неё нет.
    Stale,
    Starting(ServiceRecord),
    Own(ServiceRecord),
}

impl<'a> Stand<'a> {
    fn new(
        config: &'a AppConfig,
        context: &'a ExecutionContext,
        runner: &'a dyn ProcessRunner,
        host: &'a dyn ServiceHost,
    ) -> Result<Self, UseCaseError> {
        let infobase = declared_base(config)?;
        let paths = ServicePaths::new(&config.work_path, &infobase, "postgres");
        Ok(Self {
            config,
            context,
            runner,
            host,
            infobase,
            paths,
            started_at: Instant::now(),
        })
    }

    fn observe(&self) -> Observed {
        let Some(record) = read_json::<ServiceRecord>(&self.paths.record()) else {
            return Observed::Nothing;
        };
        if record.state == RecordState::Starting {
            return self.observe_starting(record);
        }
        let facts = record.pid.and_then(|pid| self.host.facts(pid));
        if record.is_own(facts.as_ref(), self.host.machine().as_deref()) {
            Observed::Own(record)
        } else {
            Observed::Stale
        }
    }

    /// Запись `starting` верна, пока жив записавший её раннер. Он упал — служба либо поднялась и
    /// опознаётся, либо её нет. `postmaster.pid` переживает перезагрузку, поэтому pid из него
    /// свой, только если этот процесс — `postgres` и слушает порт записи.
    fn observe_starting(&self, mut record: ServiceRecord) -> Observed {
        let starter_alive = record.starter.is_some_and(|(pid, started)| {
            self.host
                .facts(pid)
                .is_some_and(|facts| facts.started == started)
        });
        if starter_alive {
            return Observed::Starting(record);
        }
        if record.machine != self.host.machine() {
            return Observed::Stale;
        }
        let identified = self.identified_postgres(record.port);
        match identified {
            Some((pid, facts)) => {
                record.state = RecordState::Running;
                record.pid = Some(pid);
                record.started = Some(facts.started);
                record.image = Some(facts.image);
                Observed::Own(record)
            }
            None => Observed::Stale,
        }
    }

    /// Живой `postgres` из `postmaster.pid`. Файл переживает перезагрузку, поэтому pid из него свой,
    /// только если этот процесс — `postgres` и слушает порт записи.
    fn identified_postgres(&self, port: u16) -> Option<(u32, ProcessFacts)> {
        let pid = postgres::postmaster_pid(&self.paths.data())?;
        let facts = self.host.facts(pid)?;
        (facts.image.starts_with("postgres") && self.host.listening_pid(port) == Some(pid))
            .then_some((pid, facts))
    }

    fn pinned_port(&self) -> Option<u16> {
        read_json::<PinnedPorts>(&self.paths.pins()).map(|pins| pins.port)
    }

    fn result(&self, state: DbmsState, action: DbmsAction) -> DbmsResult {
        DbmsResult {
            ok: true,
            provider_dispatched: false,
            infobase: self.infobase.clone(),
            state,
            action,
            pid: None,
            version: None,
            address: None,
            user: None,
            password_file: None,
            data_dir: None,
            log_path: None,
            explicit: None,
            ready: false,
            stale_record: false,
            occupant_pid: None,
            stopped_pid: None,
            warnings: Vec::new(),
            duration_ms: u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
        }
    }

    /// Форма отказа: что осталось после неудачи и где искать причину.
    fn failed(&self) -> DbmsResult {
        let mut result = self.status();
        result.ok = false;
        result.log_path = Some(self.paths.log());
        result
    }

    fn describe(&self, mut result: DbmsResult, record: &ServiceRecord) -> DbmsResult {
        result.pid = record.pid;
        result.version.clone_from(&record.version);
        result.address = Some(postgres::server_address(record.port));
        result.user = Some(postgres::SUPERUSER.to_owned());
        result.password_file = Some(self.paths.password());
        result.data_dir = Some(self.paths.data());
        result.log_path = Some(self.paths.log());
        result.explicit = Some(record.explicit);
        result
    }

    /// Порт на первый запуск выводится из пути копии, чтобы копии не мешали друг другу; занятый
    /// заменяется при этом запуске (`INV.PLATFORM.FIRST-PORTS-COME-FROM-THE-COPY-PATH`).
    fn choose_port(&self) -> Result<u16, UseCaseError> {
        let digest = Sha256::digest(self.paths.root().to_string_lossy().as_bytes());
        let seed = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]);
        for attempt in 0..PORT_TRIES {
            let offset = seed.wrapping_add(attempt) % PORT_SPAN;
            let port = u16::try_from(PORT_BASE + offset).expect("the span stays below 49152");
            if !self.host.port_answers(port) && self.host.listening_pid(port).is_none() {
                return Ok(port);
            }
        }
        Err(environment(format!(
            "no free port for PostgreSQL after {PORT_TRIES} tries"
        )))
    }

    fn occupant(&self, port: u16) -> Option<u32> {
        self.host
            .listening_pid(port)
            .or_else(|| self.host.port_answers(port).then_some(0))
    }

    fn occupied(&self, port: u16, pid: u32) -> UseCaseError {
        let who = if pid == 0 {
            "another process".to_owned()
        } else {
            format!("process {pid}")
        };
        UseCaseError::new(
            UseCaseErrorKind::WorkspaceBusy,
            format!(
                "port {port} of the PostgreSQL of '{}' is taken by {who}, which this runner did not start; it is left running: free the port and repeat",
                self.infobase
            ),
        )
    }

    fn start(&self, dry_run: bool) -> Result<DbmsResult, UseCaseError> {
        if dry_run {
            return self.plan_start();
        }
        if !self.host.supported() {
            return Err(not_on_this_system());
        }
        // Без идентификатора машины свою службу не опознать и потому не остановить.
        let Some(machine) = self.host.machine() else {
            return Err(environment(
                "this system gives no machine id: the runner could not tell its PostgreSQL from a foreign one",
            ));
        };
        let stale = match self.observe() {
            Observed::Own(mut record) => {
                record.explicit = true;
                self.write_record(&record)?;
                let mut result =
                    self.result(DbmsState::StartedByThisCopy, DbmsAction::AlreadyRunning);
                result.ready = self.host.port_answers(record.port);
                return Ok(self.describe(result, &record));
            }
            Observed::Starting(_) => return Err(self.being_started()),
            Observed::Stale => true,
            Observed::Nothing => false,
        };
        let build = self.build()?;
        self.paths
            .prepare()
            .map_err(|error| io_failure("cannot prepare", self.paths.root(), &error))?;
        let port = match self.pinned_port() {
            Some(port) => {
                if let Some(pid) = self.occupant(port) {
                    return Err(self.occupied(port, pid));
                }
                port
            }
            None => self.choose_port()?,
        };
        write_json(&self.paths.pins(), &PinnedPorts { port })
            .map_err(|error| io_failure("cannot write", &self.paths.pins(), &error))?;
        self.initialize(&build)?;
        let version = self.version(&build);
        let mut record = ServiceRecord {
            state: RecordState::Starting,
            pid: None,
            started: None,
            image: None,
            machine: Some(machine),
            starter: self.starter(),
            port,
            explicit: true,
            bin: build.bin().to_path_buf(),
            version,
        };
        self.write_record(&record)?;
        let request = build.start(&self.paths.data(), &self.paths.log(), port);
        self.host
            .spawn(&request, self.context.work())
            .map_err(|error| {
                self.forget_record();
                if let ServiceSpawnError::Io(_) = error {
                    UseCaseError::new(
                        UseCaseErrorKind::Platform,
                        format!("pg_ctl did not start: {error}"),
                    )
                } else {
                    environment(error.to_string())
                }
            })?;
        self.await_ready(&build, port)?;
        let pid = postgres::postmaster_pid(&self.paths.data());
        let facts = pid.and_then(|pid| self.host.facts(pid));
        let Some(facts) = facts else {
            self.stop_started(&build, port);
            return Err(self.not_ready("the PostgreSQL process could not be identified"));
        };
        record.state = RecordState::Running;
        record.pid = pid;
        record.started = Some(facts.started);
        record.image = Some(facts.image);
        record.starter = None;
        if let Err(error) = self.write_record(&record) {
            self.stop_started(&build, port);
            return Err(error);
        }
        let mut result = self.result(DbmsState::StartedByThisCopy, DbmsAction::Started);
        result.ready = true;
        result.stale_record = stale;
        Ok(self.describe(result, &record))
    }

    fn plan_start(&self) -> Result<DbmsResult, UseCaseError> {
        let mut result = self.result(DbmsState::NotRunning, DbmsAction::Planned);
        result.data_dir = Some(self.paths.data());
        result.log_path = Some(self.paths.log());
        result.password_file = Some(self.paths.password());
        result.address = self.pinned_port().map(postgres::server_address);
        match self.observe() {
            Observed::Own(record) => {
                result.state = DbmsState::StartedByThisCopy;
                result.pid = record.pid;
            }
            Observed::Starting(_) => result.state = DbmsState::Starting,
            Observed::Stale => result.stale_record = true,
            Observed::Nothing => {}
        }
        Ok(result)
    }

    fn build(&self) -> Result<PostgresBuild, UseCaseError> {
        let Some(path) = self.config.tools.postgres.path.as_ref() else {
            return Err(environment(
                "the PostgreSQL build was not found: declare tools.postgres.path",
            ));
        };
        PostgresBuild::locate(path).ok_or_else(|| {
            environment(format!(
                "the PostgreSQL build was not found in '{}': tools.postgres.path names a directory with pg_ctl",
                path.display()
            ))
        })
    }

    fn version(&self, build: &PostgresBuild) -> Option<String> {
        let answer = self
            .runner
            .run_with_policy(&build.version(), &policy(self.context))
            .ok()?;
        postgres::parse_version(&answer.stdout)
    }

    /// Первый запуск создаёт каталог данных и пароль, второй берёт готовые. `initdb` работает во
    /// временном каталоге: прерванный запуск не оставит полусобранных данных.
    fn initialize(&self, build: &PostgresBuild) -> Result<(), UseCaseError> {
        let data = self.paths.data();
        if postgres::data_dir_is_initialized(&data) {
            return Ok(());
        }
        let password = self.paths.password();
        if !password.is_file() {
            write_password(&password)
                .map_err(|error| io_failure("cannot write", &password, &error))?;
        }
        let staged = self.paths.staged_data();
        let _ = std::fs::remove_dir_all(&staged);
        let request = build.initdb(&staged, &password);
        let outcome = self.runner.run_with_policy(&request, &policy(self.context));
        let failure = match outcome {
            Ok(answer) if answer.outcome().is_ok() => None,
            Ok(answer) => Some(format!("exit code {}", answer.exit_code)),
            Err(error) => Some(error.to_string()),
        };
        if let Some(failure) = failure {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(UseCaseError::new(
                UseCaseErrorKind::Platform,
                format!("initdb failed: {failure}"),
            ));
        }
        std::fs::rename(&staged, &data).map_err(|error| io_failure("cannot move", &staged, &error))
    }

    fn await_ready(&self, build: &PostgresBuild, port: u16) -> Result<(), UseCaseError> {
        let deadline = Instant::now() + self.host.ready_timeout();
        loop {
            if let Some(error) =
                interruption_before_safe_point(self.context, "waiting for PostgreSQL")
            {
                self.stop_started(build, port);
                return Err(error.into());
            }
            let answer = self
                .runner
                .run_with_policy(&build.is_ready(port), &policy(self.context));
            if answer.is_ok_and(|answer| answer.outcome().is_ok()) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                self.stop_started(build, port);
                return Err(self.not_ready("PostgreSQL did not accept connections in time"));
            }
            self.host.sleep(POLL);
        }
    }

    fn not_ready(&self, why: &str) -> UseCaseError {
        UseCaseError::new(
            UseCaseErrorKind::Platform,
            format!("{why}; log: {}", self.paths.log().display()),
        )
    }

    /// Гасит только то, что подняла эта команда. Процесс появляется не сразу, а `postmaster.pid`
    /// может остаться от прошлого запуска, поэтому ждут опознанный `postgres`. Не дождались или не
    /// подтвердили его конец — запись остаётся: когда этого раннера не станет, её разберёт следующая
    /// команда (`observe_starting`).
    fn stop_started(&self, build: &PostgresBuild, port: u16) {
        let deadline = Instant::now() + self.host.identify_timeout();
        let mut identified = self.identified_postgres(port);
        while identified.is_none() && Instant::now() < deadline {
            self.host.sleep(POLL);
            identified = self.identified_postgres(port);
        }
        let Some((pid, facts)) = identified else {
            return;
        };
        self.stop_tool(build);
        if self.await_gone(pid, Some(facts.started), self.host.stop_timeout()) {
            self.forget_record();
        }
    }

    /// `pg_ctl stop`. Сам отмене не подлежит: иначе останется поднятое. Отмена, отложенная на это
    /// время, возвращается предупреждением.
    fn stop_tool(&self, build: &PostgresBuild) -> Option<String> {
        let policy = self
            .context
            .process_policy(InterruptionSafetyClass::CriticalNonAbortable, None);
        let answer = self
            .runner
            .run_with_policy(&build.stop(&self.paths.data()), &policy)
            .ok()?;
        deferred_process_interruption(
            ExecutionInterruptionPhase::ProviderCommand,
            "pg_ctl stop",
            answer.interruption,
        )
        .map(|(warning, _details)| warning)
    }

    fn write_record(&self, record: &ServiceRecord) -> Result<(), UseCaseError> {
        write_json(&self.paths.record(), record)
            .map_err(|error| io_failure("cannot write", &self.paths.record(), &error))
    }

    fn forget_record(&self) {
        let _ = std::fs::remove_file(self.paths.record());
    }

    /// Свой сервер над этой СУБД: пока он работает, её не останавливают.
    fn running_server(&self) -> Option<(&'static str, &'static str)> {
        let machine = self.host.machine();
        SERVERS.into_iter().find(|(service, _)| {
            read_json::<ServiceRecord>(&self.paths.sibling_record(service)).is_some_and(|record| {
                let facts = record.pid.and_then(|pid| self.host.facts(pid));
                record.is_own(facts.as_ref(), machine.as_deref())
            })
        })
    }

    fn refuse_under_a_server(&self) -> Result<(), UseCaseError> {
        match self.running_server() {
            Some((service, next)) => Err(validation(format!(
                "the {service} of '{}' is still running on this PostgreSQL; run {next} first",
                self.infobase
            ))),
            None => Ok(()),
        }
    }

    fn stop(&self, dry_run: bool) -> Result<DbmsResult, UseCaseError> {
        self.refuse_under_a_server()?;
        if dry_run {
            let observed = self.observe();
            let mut result = self.result(DbmsState::NotRunning, DbmsAction::Planned);
            if let Observed::Own(record) = observed {
                result.state = DbmsState::StartedByThisCopy;
                result = self.describe(result, &record);
            }
            return Ok(result);
        }
        let record = match self.observe() {
            Observed::Own(record) => record,
            Observed::Starting(_) => return Err(self.being_started()),
            Observed::Nothing => return Ok(self.nothing_to_stop(false)),
            Observed::Stale => {
                self.forget_record();
                return Ok(self.nothing_to_stop(true));
            }
        };
        let build = PostgresBuild::at(record.bin.clone());
        let deferred = self.stop_tool(&build);
        let pid = record.pid.expect("an own service has a pid");
        if !self.await_gone(pid, record.started, self.host.stop_timeout()) {
            self.host.hard_stop(self.runner, pid, self.context);
            if !self.await_gone(pid, record.started, self.host.stop_timeout()) {
                return Err(UseCaseError::new(
                    UseCaseErrorKind::Platform,
                    format!("PostgreSQL pid {pid} did not stop; repeat dbms stop"),
                ));
            }
        }
        self.forget_record();
        let mut result = self.result(DbmsState::NotRunning, DbmsAction::Stopped);
        result.stopped_pid = Some(pid);
        result.warnings.extend(deferred);
        Ok(result)
    }

    /// Конец процесса: pid исчез или занят процессом с другим временем старта.
    fn await_gone(&self, pid: u32, started: Option<u64>, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let same = self
                .host
                .facts(pid)
                .is_some_and(|facts| Some(facts.started) == started);
            if !same {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            self.host.sleep(POLL);
        }
    }

    fn being_started(&self) -> UseCaseError {
        UseCaseError::new(
            UseCaseErrorKind::WorkspaceBusy,
            format!(
                "the PostgreSQL of '{}' is being started by another running command",
                self.infobase
            ),
        )
    }

    /// Этот раннер: запись `starting` верна, пока он жив.
    fn starter(&self) -> Option<(u32, u64)> {
        let pid = std::process::id();
        self.host.facts(pid).map(|facts| (pid, facts.started))
    }

    fn nothing_to_stop(&self, stale: bool) -> DbmsResult {
        let mut result = self.result(DbmsState::NotRunning, DbmsAction::NothingToStop);
        result.stale_record = stale;
        if let Some(port) = self.pinned_port() {
            if let Some(pid) = self.occupant(port) {
                result.state = DbmsState::Foreign;
                result.occupant_pid = (pid != 0).then_some(pid);
            }
        }
        result
    }

    fn status(&self) -> DbmsResult {
        match self.observe() {
            Observed::Own(record) => {
                let mut result = self.result(DbmsState::StartedByThisCopy, DbmsAction::Reported);
                result.ready = self.host.port_answers(record.port);
                self.describe(result, &record)
            }
            Observed::Starting(record) => {
                let result = self.result(DbmsState::Starting, DbmsAction::Reported);
                self.describe(result, &record)
            }
            observed => {
                let stale = matches!(observed, Observed::Stale);
                let mut result = self.result(DbmsState::NotRunning, DbmsAction::Reported);
                result.stale_record = stale;
                if let Some(port) = self.pinned_port() {
                    result.address = Some(postgres::server_address(port));
                    if let Some(pid) = self.occupant(port) {
                        result.state = DbmsState::Foreign;
                        result.occupant_pid = (pid != 0).then_some(pid);
                    }
                }
                result
            }
        }
    }

    fn restart(&self, dry_run: bool) -> Result<DbmsResult, UseCaseError> {
        self.refuse_under_a_server()?;
        if dry_run {
            return self.plan_start();
        }
        if !self.host.supported() {
            return Err(not_on_this_system());
        }
        let stopped = self.stop(false)?;
        if let Some(error) =
            interruption_before_safe_point(self.context, "starting PostgreSQL again")
        {
            return Err(error.into());
        }
        let mut started = self.start(false)?;
        started.stopped_pid = stopped.stopped_pid;
        started.warnings.extend(stopped.warnings);
        Ok(started)
    }
}

/// Пароль из `getrandom`: буквы и цифры, чтобы ни одна утилита не разбирала его по-своему.
fn write_password(path: &Path) -> std::io::Result<()> {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = [0u8; PASSWORD_LENGTH];
    getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
    let text: String = bytes
        .iter()
        .map(|byte| char::from(ALPHABET[usize::from(*byte) % ALPHABET.len()]))
        .collect();
    crate::support::fs::write_file_atomically(path, |file| file.write_all(text.as_bytes()))
}

/// Какая из команд `dbms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbmsCommand {
    Start,
    Status,
    Stop,
    Restart,
}

/// `dbms start|stop|restart`: команда записи, под замком `workPath`.
pub fn execute_dbms(
    context: &ExecutionContext,
    config: &AppConfig,
    command: DbmsCommand,
    dry_run: bool,
) -> UseCaseResult<DbmsResult> {
    stamp_dispatch(
        run_dbms(
            context,
            config,
            command,
            dry_run,
            &ProcessExecutor,
            &RealHost,
        ),
        context.work(),
    )
}

/// `dbms status`: читает запись без замка, чтобы отвечать и пока идёт другая команда
/// (`INV.USE-CASES.A-SERVICE-STATUS-TAKES-NO-LOCK`).
pub fn dbms_status(context: &ExecutionContext, config: &AppConfig) -> UseCaseResult<DbmsResult> {
    stamp_dispatch(
        run_dbms(
            context,
            config,
            DbmsCommand::Status,
            false,
            &ProcessExecutor,
            &RealHost,
        ),
        context.work(),
    )
}

fn run_dbms(
    context: &ExecutionContext,
    config: &AppConfig,
    command: DbmsCommand,
    dry_run: bool,
    runner: &dyn ProcessRunner,
    host: &dyn ServiceHost,
) -> UseCaseResult<DbmsResult> {
    let stand =
        Stand::new(config, context, runner, host).map_err(UseCaseFailure::without_payload)?;
    let outcome = match command {
        DbmsCommand::Start => stand.start(dry_run),
        DbmsCommand::Status => Ok(stand.status()),
        DbmsCommand::Stop => stand.stop(dry_run),
        DbmsCommand::Restart => stand.restart(dry_run),
    };
    outcome.map_err(|error| UseCaseFailure::with_payload(error, stand.failed()))
}

/// Сценарий `dbms` с подделкой утилит и машины: файлы настоящие, процессы нет.
#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::config::model::{
        InfobaseClusterConfig, InfobaseConfig, InfobaseDbmsConfig, ManagedDbmsConfig,
        ManagedServerConfig,
    };
    use crate::platform::process::{ProcessError, ProcessResult, SpawnResult};
    use crate::use_cases::context::CommandName;

    const PID: u32 = 40112;
    const STARTED: u64 = 1000;
    const MACHINE: &str = "machine";

    /// Утилиты: `initdb` создаёт каталог данных, `pg_ctl stop` гасит подделанный процесс.
    struct FakeRunner<'a> {
        host: &'a FakeHost,
        calls: RefCell<Vec<String>>,
        initdb_fails: Cell<bool>,
        ready: Cell<bool>,
        stop_works: Cell<bool>,
    }

    impl<'a> FakeRunner<'a> {
        fn new(host: &'a FakeHost) -> Self {
            Self {
                host,
                calls: RefCell::default(),
                initdb_fails: Cell::new(false),
                ready: Cell::new(true),
                stop_works: Cell::new(true),
            }
        }

        fn called(&self, tool: &str) -> usize {
            self.calls
                .borrow()
                .iter()
                .filter(|call| call.starts_with(tool))
                .count()
        }
    }

    impl ProcessRunner for FakeRunner<'_> {
        fn run_with_policy(
            &self,
            request: &ProcessRequest,
            _policy: &ProcessExecutionPolicy,
        ) -> Result<ProcessResult, ProcessError> {
            let tool =
                service::image_name(&request.program).replace(std::env::consts::EXE_SUFFIX, "");
            let line = format!("{tool} {}", request.args.join(" "));
            self.calls.borrow_mut().push(line);
            let mut exit_code = 0;
            let mut stdout = String::new();
            match tool.as_str() {
                "pg_ctl" if request.args[0] == "--version" => {
                    stdout = "pg_ctl (PostgreSQL) 17.11\n".to_owned();
                }
                "pg_ctl" if request.args[0] == "stop" && self.stop_works.get() => {
                    self.host.processes.borrow_mut().remove(&PID);
                    self.host.listeners.borrow_mut().clear();
                }
                "initdb" => {
                    if self.initdb_fails.get() {
                        exit_code = 1;
                    } else {
                        let data = PathBuf::from(&request.args[1]);
                        std::fs::create_dir_all(&data).unwrap();
                        std::fs::write(data.join("PG_VERSION"), "17\n").unwrap();
                        std::fs::write(data.join("postgresql.conf"), "# conf\n").unwrap();
                    }
                }
                "pg_isready" if !self.ready.get() => exit_code = 2,
                _ => {}
            }
            Ok(ProcessResult {
                exit_code,
                stdout,
                stderr: String::new(),
                interruption: None,
            })
        }

        fn spawn(
            &self,
            _request: &ProcessRequest,
            _work: &WorkGiven,
        ) -> Result<SpawnResult, ProcessError> {
            unreachable!("a service is started through the host")
        }
    }

    /// Машина: какие процессы живы и кто слушает порты.
    #[derive(Default)]
    struct FakeHost {
        unsupported: Cell<bool>,
        no_machine_id: Cell<bool>,
        would_not_survive: Cell<bool>,
        processes: RefCell<HashMap<u32, ProcessFacts>>,
        listeners: RefCell<HashMap<u16, u32>>,
        spawned: Cell<u32>,
        hard_stops: RefCell<HashSet<u32>>,
        start_args: RefCell<Vec<String>>,
        spawn_is_slow: Cell<bool>,
    }

    use tokio_util::sync::CancellationToken;

    impl ServiceHost for FakeHost {
        fn supported(&self) -> bool {
            !self.unsupported.get()
        }
        fn spawn(
            &self,
            request: &ServiceRequest,
            _work: &WorkGiven,
        ) -> Result<u32, ServiceSpawnError> {
            if self.would_not_survive.get() {
                return Err(ServiceSpawnError::WouldNotSurvive);
            }
            self.spawned.set(self.spawned.get() + 1);
            let data = PathBuf::from(&request.args[3]);
            if self.spawn_is_slow.get() {
                return Ok(7);
            }
            std::fs::write(data.join("postmaster.pid"), format!("{PID}\n")).unwrap();
            self.start_args.replace(request.args.clone());
            let port = request
                .args
                .last()
                .and_then(|options| options.split_whitespace().nth(1))
                .and_then(|port| port.parse().ok())
                .expect("the port of the service");
            self.listeners.borrow_mut().insert(port, PID);
            self.processes.borrow_mut().insert(
                PID,
                ProcessFacts {
                    started: STARTED,
                    image: "postgres.exe".to_owned(),
                },
            );
            Ok(7)
        }
        fn facts(&self, pid: u32) -> Option<ProcessFacts> {
            self.processes.borrow().get(&pid).cloned()
        }
        fn machine(&self) -> Option<String> {
            (!self.no_machine_id.get()).then(|| MACHINE.to_owned())
        }
        fn listening_pid(&self, port: u16) -> Option<u32> {
            self.listeners.borrow().get(&port).copied()
        }
        fn port_answers(&self, port: u16) -> bool {
            self.listeners.borrow().contains_key(&port)
        }
        fn hard_stop(&self, _runner: &dyn ProcessRunner, pid: u32, _context: &ExecutionContext) {
            self.hard_stops.borrow_mut().insert(pid);
            self.processes.borrow_mut().remove(&pid);
        }
        fn ready_timeout(&self) -> Duration {
            Duration::ZERO
        }
        fn stop_timeout(&self) -> Duration {
            Duration::ZERO
        }
        fn identify_timeout(&self) -> Duration {
            Duration::ZERO
        }
        fn sleep(&self, _duration: Duration) {}
    }

    struct Setup {
        root: tempfile::TempDir,
        build: PathBuf,
        config: AppConfig,
    }

    fn setup() -> Setup {
        let root = tempfile::tempdir().unwrap();
        let build = root.path().join("pg");
        std::fs::create_dir_all(build.join("bin")).unwrap();
        std::fs::write(
            build
                .join("bin")
                .join(format!("pg_ctl{}", std::env::consts::EXE_SUFFIX)),
            "",
        )
        .unwrap();
        let mut infobase = InfobaseConfig::file(String::new());
        infobase.cluster = Some(InfobaseClusterConfig {
            managed: Some(ManagedServerConfig::default()),
            ..Default::default()
        });
        infobase.dbms = Some(InfobaseDbmsConfig {
            managed: Some(ManagedDbmsConfig::default()),
            ..Default::default()
        });
        let mut config = AppConfig {
            base_path: root.path().to_path_buf(),
            work_path: root.path().join("work"),
            format: crate::config::model::SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase,
            infobases: Default::default(),
            infobase_name: Some("origin".to_owned()),
            working_copy: None,
            source_sets: Vec::new(),
            tools: Default::default(),
            mcp: Default::default(),
            tests: Default::default(),
        };
        config.tools.postgres.path = Some(build.clone());
        Setup {
            root,
            build,
            config,
        }
    }

    impl Setup {
        fn paths(&self) -> ServicePaths {
            ServicePaths::new(&self.config.work_path, "origin", "postgres")
        }

        fn run(
            &self,
            command: DbmsCommand,
            dry_run: bool,
            runner: &FakeRunner<'_>,
            host: &FakeHost,
        ) -> Result<DbmsResult, UseCaseError> {
            self.run_in(
                &ExecutionContext::cli(CommandName::DbmsStart),
                command,
                dry_run,
                runner,
                host,
            )
        }

        fn run_in(
            &self,
            context: &ExecutionContext,
            command: DbmsCommand,
            dry_run: bool,
            runner: &FakeRunner<'_>,
            host: &FakeHost,
        ) -> Result<DbmsResult, UseCaseError> {
            run_dbms(context, &self.config, command, dry_run, runner, host)
                .map_err(|failure| failure.error)
        }
    }

    fn own_process(host: &FakeHost) {
        host.processes.borrow_mut().insert(
            PID,
            ProcessFacts {
                started: STARTED,
                image: "postgres.exe".to_owned(),
            },
        );
    }

    #[test]
    fn the_first_start_creates_the_data_directory_and_the_second_reuses_it() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        let started = setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        assert_eq!(started.action, DbmsAction::Started);
        assert_eq!(started.pid, Some(PID));
        assert_eq!(started.version.as_deref(), Some("17.11"));
        assert!(started.ready);
        assert_eq!(runner.called("initdb"), 1);
        assert!(
            host.start_args
                .borrow()
                .iter()
                .any(|arg| arg.contains("listen_addresses=127.0.0.1")),
            "{:?}",
            host.start_args.borrow()
        );
        assert!(!setup.paths().staged_data().exists());

        setup.run(DbmsCommand::Stop, false, &runner, &host).unwrap();
        let again = setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        assert_eq!(again.action, DbmsAction::Started);
        assert_eq!(runner.called("initdb"), 1, "the data directory is reused");
        assert_eq!(again.address, started.address, "the port stays pinned");
    }

    #[test]
    fn a_running_service_is_named_and_no_new_process_starts() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        let second = setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        assert_eq!(second.action, DbmsAction::AlreadyRunning);
        assert_eq!(second.pid, Some(PID));
        assert_eq!(host.spawned.get(), 1);
    }

    #[test]
    fn a_foreign_process_on_the_pinned_port_is_named_and_left_alone() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        let started = setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        setup.run(DbmsCommand::Stop, false, &runner, &host).unwrap();
        let port = read_json::<PinnedPorts>(&setup.paths().pins())
            .unwrap()
            .port;
        host.listeners.borrow_mut().insert(port, 555);

        let refused = setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::WorkspaceBusy);
        assert!(refused.to_string().contains("process 555"), "{refused}");
        assert_eq!(host.spawned.get(), 1, "{started:?}");

        let status = setup
            .run(DbmsCommand::Status, false, &runner, &host)
            .unwrap();
        assert_eq!(status.state, DbmsState::Foreign);
        assert_eq!(status.occupant_pid, Some(555));

        let stop = setup.run(DbmsCommand::Stop, false, &runner, &host).unwrap();
        assert_eq!(stop.action, DbmsAction::NothingToStop);
        assert_eq!(stop.occupant_pid, Some(555));
        assert!(host.hard_stops.borrow().is_empty());
    }

    #[test]
    fn a_first_port_taken_by_someone_is_chosen_again() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        let context = ExecutionContext::cli(CommandName::DbmsStart);
        let stand = Stand::new(&setup.config, &context, &runner, &host).unwrap();
        let first = stand.choose_port().unwrap();
        assert!((20_000..49_000).contains(&first), "{first}");
        host.listeners.borrow_mut().insert(first, 9);
        assert_ne!(stand.choose_port().unwrap(), first);
    }

    #[test]
    fn a_cancelled_start_stops_what_it_started() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        runner.ready.set(false);
        let token = CancellationToken::new();
        token.cancel();
        let context = ExecutionContext::cli(CommandName::DbmsStart).with_cancellation(token);
        let error = setup
            .run_in(&context, DbmsCommand::Start, false, &runner, &host)
            .unwrap_err();
        assert!(
            matches!(error.kind(), UseCaseErrorKind::Cancelled(_)),
            "{error}"
        );
        assert!(runner
            .calls
            .borrow()
            .iter()
            .any(|call| call.starts_with("pg_ctl stop")));
        assert!(!setup.paths().record().exists());
        assert!(
            host.processes.borrow().is_empty(),
            "what it started is stopped"
        );
    }

    #[test]
    fn a_probe_that_never_passes_names_the_log_and_stops_what_it_started() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        runner.ready.set(false);
        let error = setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap_err();
        assert_eq!(error.kind(), UseCaseErrorKind::Platform);
        assert!(error.to_string().contains("postgres.log"), "{error}");
        assert!(runner
            .calls
            .borrow()
            .iter()
            .any(|call| call.starts_with("pg_ctl stop")));
        assert!(!setup.paths().record().exists());
    }

    #[test]
    fn a_stale_record_is_replaced_and_named_and_the_data_stays() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        // Перезагрузка: процесса из записи больше нет, данные и порт на месте.
        host.processes.borrow_mut().clear();
        host.listeners.borrow_mut().clear();
        let status = setup
            .run(DbmsCommand::Status, false, &runner, &host)
            .unwrap();
        assert_eq!(status.state, DbmsState::NotRunning);
        assert!(status.stale_record);

        let again = setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        assert_eq!(again.action, DbmsAction::Started);
        assert!(again.stale_record);
        assert_eq!(runner.called("initdb"), 1);
    }

    #[test]
    fn a_recycled_pid_is_not_taken_for_the_service() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        host.processes.borrow_mut().insert(
            PID,
            ProcessFacts {
                started: STARTED + 1,
                image: "postgres.exe".to_owned(),
            },
        );
        let stop = setup.run(DbmsCommand::Stop, false, &runner, &host).unwrap();
        assert_eq!(stop.action, DbmsAction::NothingToStop);
        assert!(stop.stale_record);
        assert_eq!(runner.called("pg_ctl stop"), 0);
        assert!(
            host.processes.borrow().contains_key(&PID),
            "a foreign process stays"
        );
    }

    #[test]
    fn stop_refuses_under_a_running_own_server_and_names_it() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        let mut server = read_json::<ServiceRecord>(&setup.paths().record()).unwrap();
        server.pid = Some(777);
        write_json(&setup.paths().sibling_record("cluster"), &server).unwrap();
        host.processes.borrow_mut().insert(
            777,
            ProcessFacts {
                started: STARTED,
                image: "postgres.exe".to_owned(),
            },
        );
        for command in [DbmsCommand::Stop, DbmsCommand::Restart] {
            let refused = setup.run(command, false, &runner, &host).unwrap_err();
            assert_eq!(refused.kind(), UseCaseErrorKind::Validation);
            assert!(refused.to_string().contains("cluster stop"), "{refused}");
        }
        assert!(host.processes.borrow().contains_key(&PID));
    }

    #[test]
    fn a_dead_server_record_does_not_hold_the_dbms() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        let mut server = read_json::<ServiceRecord>(&setup.paths().record()).unwrap();
        server.pid = Some(777);
        write_json(&setup.paths().sibling_record("standalone"), &server).unwrap();
        assert_eq!(
            setup
                .run(DbmsCommand::Stop, false, &runner, &host)
                .unwrap()
                .action,
            DbmsAction::Stopped
        );
    }

    #[test]
    fn stop_that_failed_keeps_the_record_and_a_repeat_goes_on() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        runner.stop_works.set(false);
        let context = ExecutionContext::cli(CommandName::DbmsStop);
        let stand = Stand::new(&setup.config, &context, &runner, &host).unwrap();
        assert!(!stand.await_gone(PID, Some(STARTED), Duration::ZERO));
        assert!(
            stand.await_gone(PID, Some(STARTED + 1), Duration::ZERO),
            "a recycled pid is gone"
        );
        assert!(setup.paths().record().exists());
        let stopped = setup.run(DbmsCommand::Stop, false, &runner, &host).unwrap();
        assert_eq!(stopped.stopped_pid, Some(PID));
        assert!(
            host.hard_stops.borrow().contains(&PID),
            "soft stop did not end it"
        );
        assert!(!setup.paths().record().exists());
    }

    #[test]
    fn status_and_stop_work_from_the_record_when_the_tool_key_is_gone() {
        let mut setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        setup.config.tools.postgres.path = None;
        assert_eq!(
            setup
                .run(DbmsCommand::Status, false, &runner, &host)
                .unwrap()
                .state,
            DbmsState::StartedByThisCopy
        );
        assert_eq!(
            setup
                .run(DbmsCommand::Stop, false, &runner, &host)
                .unwrap()
                .action,
            DbmsAction::Stopped
        );
    }

    #[test]
    fn a_start_without_the_build_names_the_key() {
        let mut setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup.config.tools.postgres.path = None;
        let refused = setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::Environment);
        assert!(
            refused.to_string().contains("tools.postgres.path"),
            "{refused}"
        );
        setup.config.tools.postgres.path = Some(setup.root.path().join("no-such"));
        assert_eq!(
            setup
                .run(DbmsCommand::Start, false, &runner, &host)
                .unwrap_err()
                .kind(),
            UseCaseErrorKind::Environment
        );
        let _ = setup.build;
    }

    #[test]
    fn other_systems_refuse_start_and_restart_without_starting_anything() {
        let setup = setup();
        let host = FakeHost::default();
        host.unsupported.set(true);
        let runner = FakeRunner::new(&host);
        for command in [DbmsCommand::Start, DbmsCommand::Restart] {
            let refused = setup.run(command, false, &runner, &host).unwrap_err();
            assert_eq!(refused.kind(), UseCaseErrorKind::Environment);
        }
        assert!(runner.calls.borrow().is_empty());
        assert_eq!(host.spawned.get(), 0);
    }

    #[test]
    fn a_service_that_would_not_survive_is_refused_as_the_environment() {
        let setup = setup();
        let host = FakeHost::default();
        host.would_not_survive.set(true);
        let runner = FakeRunner::new(&host);
        let refused = setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::Environment);
        assert!(!setup.paths().record().exists());
    }

    #[test]
    fn a_failed_initdb_leaves_no_half_made_data_directory() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        runner.initdb_fails.set(true);
        let refused = setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::Platform);
        assert!(!setup.paths().data().exists());
        assert!(!setup.paths().staged_data().exists());
        assert_eq!(host.spawned.get(), 0);
    }

    #[test]
    fn a_preview_changes_nothing() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        for command in [DbmsCommand::Start, DbmsCommand::Stop, DbmsCommand::Restart] {
            let plan = setup.run(command, true, &runner, &host).unwrap();
            assert_eq!(plan.action, DbmsAction::Planned);
            assert!(!plan.provider_dispatched);
        }
        assert!(runner.calls.borrow().is_empty());
        assert!(!setup.config.work_path.exists());
    }

    #[test]
    fn the_password_stays_out_of_the_answer_and_the_arguments() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        let started = setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        let password = std::fs::read_to_string(setup.paths().password()).unwrap();
        assert_eq!(password.len(), PASSWORD_LENGTH);
        let answer = serde_json::to_string(&started).unwrap();
        assert!(!answer.contains(&password), "{answer}");
        assert!(answer.contains("password"), "the file is named: {answer}");
        assert!(!runner
            .calls
            .borrow()
            .iter()
            .any(|call| call.contains(&password)));
        let record = std::fs::read_to_string(setup.paths().record()).unwrap();
        assert!(!record.contains(&password));
    }

    #[test]
    fn an_already_running_service_started_elsewhere_becomes_explicit() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        let mut record = read_json::<ServiceRecord>(&setup.paths().record()).unwrap();
        record.explicit = false;
        write_json(&setup.paths().record(), &record).unwrap();
        own_process(&host);
        let answer = setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        assert_eq!(answer.explicit, Some(true));
        assert!(
            read_json::<ServiceRecord>(&setup.paths().record())
                .unwrap()
                .explicit
        );
    }

    #[test]
    fn a_machine_without_an_id_treats_the_recorded_process_as_foreign() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        host.no_machine_id.set(true);
        let refused = setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::Environment);
        let status = setup
            .run(DbmsCommand::Status, false, &runner, &host)
            .unwrap();
        assert_eq!(status.state, DbmsState::Foreign);
        assert!(status.stale_record);
    }

    #[test]
    fn an_ad_hoc_connection_is_refused() {
        let mut setup = setup();
        setup.config.infobase_name = None;
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        let refused = setup
            .run(DbmsCommand::Status, false, &runner, &host)
            .unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::Validation);
    }

    /// Запись `starting` от раннера с этим pid.
    fn starting_record(setup: &Setup, starter: Option<(u32, u64)>) {
        setup.paths().prepare().unwrap();
        std::fs::create_dir_all(setup.paths().data()).unwrap();
        write_json(
            &setup.paths().record(),
            &ServiceRecord {
                state: RecordState::Starting,
                pid: None,
                started: None,
                image: None,
                machine: Some(MACHINE.to_owned()),
                starter,
                port: 24432,
                explicit: true,
                bin: PathBuf::from("bin"),
                version: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn a_start_in_progress_is_reported_and_refused() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        host.processes.borrow_mut().insert(
            900,
            ProcessFacts {
                started: 5,
                image: "v8-runner.exe".to_owned(),
            },
        );
        starting_record(&setup, Some((900, 5)));
        let status = setup
            .run(DbmsCommand::Status, false, &runner, &host)
            .unwrap();
        assert_eq!(status.state, DbmsState::Starting);
        for command in [DbmsCommand::Start, DbmsCommand::Stop] {
            let refused = setup.run(command, false, &runner, &host).unwrap_err();
            assert_eq!(refused.kind(), UseCaseErrorKind::WorkspaceBusy, "{refused}");
        }
    }

    #[test]
    fn a_crashed_start_with_a_live_postgres_is_taken_over() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        starting_record(&setup, Some((900, 5)));
        std::fs::write(
            setup.paths().data().join("postmaster.pid"),
            format!("{PID}\n"),
        )
        .unwrap();
        own_process(&host);
        host.listeners.borrow_mut().insert(24432, PID);
        assert_eq!(
            setup
                .run(DbmsCommand::Status, false, &runner, &host)
                .unwrap()
                .state,
            DbmsState::StartedByThisCopy
        );
        let stopped = setup.run(DbmsCommand::Stop, false, &runner, &host).unwrap();
        assert_eq!(stopped.stopped_pid, Some(PID));
    }

    #[test]
    fn a_leftover_pid_file_after_a_reboot_never_names_a_foreign_process() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        starting_record(&setup, Some((900, 5)));
        std::fs::write(setup.paths().data().join("postmaster.pid"), "777\n").unwrap();
        host.processes.borrow_mut().insert(
            777,
            ProcessFacts {
                started: 1,
                image: "other.exe".to_owned(),
            },
        );
        let status = setup
            .run(DbmsCommand::Status, false, &runner, &host)
            .unwrap();
        assert_eq!(status.state, DbmsState::NotRunning);
        assert!(status.stale_record);
        let stop = setup.run(DbmsCommand::Stop, false, &runner, &host).unwrap();
        assert_eq!(stop.action, DbmsAction::NothingToStop);
        assert!(host.processes.borrow().contains_key(&777));
        assert!(host.hard_stops.borrow().is_empty());
    }

    #[test]
    fn a_cancelled_start_whose_service_would_not_stop_keeps_the_record() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        runner.stop_works.set(false);
        let token = CancellationToken::new();
        token.cancel();
        let context = ExecutionContext::cli(CommandName::DbmsStart).with_cancellation(token);
        setup
            .run_in(&context, DbmsCommand::Start, false, &runner, &host)
            .unwrap_err();
        assert!(setup.paths().record().exists());
    }

    #[test]
    fn stop_goes_through_pg_ctl_in_fast_mode() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        setup.run(DbmsCommand::Stop, false, &runner, &host).unwrap();
        assert!(runner
            .calls
            .borrow()
            .iter()
            .any(|call| call.starts_with("pg_ctl stop") && call.contains("-m fast")));
    }

    #[test]
    fn status_of_a_running_service_runs_no_utility() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        runner.calls.borrow_mut().clear();
        let status = setup
            .run(DbmsCommand::Status, false, &runner, &host)
            .unwrap();
        assert_eq!(status.state, DbmsState::StartedByThisCopy);
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn restart_names_what_it_stopped() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        let restarted = setup
            .run(DbmsCommand::Restart, false, &runner, &host)
            .unwrap();
        assert_eq!(restarted.action, DbmsAction::Started);
        assert_eq!(restarted.stopped_pid, Some(PID));
    }

    #[test]
    fn a_cancellation_before_postgres_appears_keeps_the_record_despite_an_old_pid_file() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        setup
            .run(DbmsCommand::Start, false, &runner, &host)
            .unwrap();
        setup.run(DbmsCommand::Stop, false, &runner, &host).unwrap();
        // После перезагрузки файл остался, процесса нет, новый ещё не поднялся.
        std::fs::write(
            setup.paths().data().join("postmaster.pid"),
            format!("{PID}\n"),
        )
        .unwrap();
        host.spawn_is_slow.set(true);
        let token = CancellationToken::new();
        token.cancel();
        let context = ExecutionContext::cli(CommandName::DbmsStart).with_cancellation(token);
        setup
            .run_in(&context, DbmsCommand::Start, false, &runner, &host)
            .unwrap_err();
        assert!(
            setup.paths().record().exists(),
            "the next command sorts it out"
        );
    }

    #[test]
    fn a_record_of_another_machine_is_not_taken_over() {
        let setup = setup();
        let host = FakeHost::default();
        let runner = FakeRunner::new(&host);
        starting_record(&setup, Some((900, 5)));
        let mut record = read_json::<ServiceRecord>(&setup.paths().record()).unwrap();
        record.machine = Some("other".to_owned());
        write_json(&setup.paths().record(), &record).unwrap();
        std::fs::write(
            setup.paths().data().join("postmaster.pid"),
            format!("{PID}\n"),
        )
        .unwrap();
        own_process(&host);
        host.listeners.borrow_mut().insert(24432, PID);
        let status = setup
            .run(DbmsCommand::Status, false, &runner, &host)
            .unwrap();
        assert_eq!(status.state, DbmsState::NotRunning);
        assert!(status.stale_record);
    }
}
