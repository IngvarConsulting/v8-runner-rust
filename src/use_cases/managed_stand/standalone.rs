//! Автономный сервер, которого поднимает раннер: `standalone start|status|stop|restart`.
//! Адрес такой цели остальные команды берут из той же записи, её читает загрузчик настроек.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{
    await_gone, being_started, environment, forget_record, io_failure, occupant_of, port_offsets,
    validation, write_record, ServiceCommand, ServiceHost,
};
use crate::config::model::AppConfig;
use crate::domain::dbms::{ServiceAction, ServiceState};
use crate::domain::standalone::StandaloneResult;
use crate::platform::agent::write_service_host_key;
use crate::platform::ibsrv::{self, ServerPorts};
use crate::platform::process::ProcessRunner;
use crate::platform::service::ServiceSpawnError;
use crate::platform::service_record::{
    adopt_recorded_pid, observe_service, read_json, write_json, Observed, RecordState,
    ServicePaths, ServiceRecord,
};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::interruption::interruption_before_safe_point;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind, UseCaseFailure, UseCaseResult};

const READY_TIMEOUT: Duration = Duration::from_secs(120);
const SOFT_STOP_TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(200);

/// То, что сценарий узнаёт о цели из настроек.
struct Declared {
    infobase: String,
    name: String,
    db_path: PathBuf,
}

fn declared(config: &AppConfig) -> Result<Declared, UseCaseError> {
    let infobase = config.infobase_name.clone().ok_or_else(|| {
        validation("standalone takes a declared infobase: a connection string has neither a record nor a managed section")
    })?;
    let managed = config
        .infobase
        .standalone
        .as_ref()
        .and_then(|standalone| standalone.managed.as_ref())
        .ok_or_else(|| {
            validation("infobase.standalone.managed is not declared for this infobase: the standalone commands start only a server the runner owns")
        })?;
    let name = managed
        .name
        .clone()
        .ok_or_else(|| validation("infobase.standalone.managed.name is not declared"))?;
    let db_path = managed
        .db_path
        .clone()
        .ok_or_else(|| validation("infobase.standalone.managed.db-path is not declared"))?;
    Ok(Declared {
        infobase,
        name,
        db_path,
    })
}

struct Server<'a> {
    context: &'a ExecutionContext,
    runner: &'a dyn ProcessRunner,
    host: &'a dyn ServiceHost,
    locate: &'a dyn Fn() -> Result<PathBuf, UseCaseError>,
    declared: Declared,
    paths: ServicePaths,
    started_at: Instant,
}

impl Server<'_> {
    fn observe(&self) -> Observed {
        let host = self.host;
        let machine = host.machine();
        observe_service(
            &self.paths,
            machine.as_deref(),
            &|pid| host.facts(pid),
            &|record| adopt_recorded_pid(record, &|pid| host.facts(pid), machine.as_deref()),
        )
    }

    fn pins(&self) -> Option<ServerPorts> {
        read_json(&self.paths.pins())
    }

    fn result(&self, state: ServiceState, action: ServiceAction) -> StandaloneResult {
        StandaloneResult {
            ok: true,
            infobase: self.declared.infobase.clone(),
            state,
            action,
            pid: None,
            connection: None,
            gate: None,
            http: None,
            data_dir: None,
            log_path: None,
            ready: false,
            stale_record: false,
            occupant_pid: None,
            stopped_pid: None,
            duration_ms: u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
        }
    }

    fn describe(&self, mut result: StandaloneResult, pid: Option<u32>) -> StandaloneResult {
        result.pid = pid;
        if let Some(ports) = self.pins() {
            result.connection = Some(ports.connection(&self.declared.name));
            result.gate = Some(ports.gate());
            result.http = Some(ports.http_url(&self.declared.name));
        }
        result.data_dir = Some(self.paths.data());
        result.log_path = Some(self.paths.log());
        result
    }

    fn occupant(&self, ports: ServerPorts) -> Option<u32> {
        ports.all().find_map(|port| occupant_of(self.host, port))
    }

    /// Блок портов на первый запуск; занятый заменяется при этом запуске.
    fn choose_ports(&self) -> Result<ServerPorts, UseCaseError> {
        port_offsets(self.paths.root())
            .map(ServerPorts::from_first)
            .find(|ports| self.occupant(*ports).is_none())
            .ok_or_else(|| environment("no free ports for ibsrv"))
    }

    fn write_record(&self, record: &ServiceRecord) -> Result<(), UseCaseError> {
        write_record(&self.paths, record)
    }

    fn forget_record(&self) {
        forget_record(&self.paths);
    }

    fn start(&self, dry_run: bool) -> Result<StandaloneResult, UseCaseError> {
        if dry_run {
            return Ok(self.plan());
        }
        if !self.host.supported() {
            return Err(environment(
                "managed services run on Windows only: on this system the runner starts no ibsrv",
            ));
        }
        let Some(machine) = self.host.machine() else {
            return Err(environment(
                "this system gives no machine id: the runner could not tell its server from a foreign one",
            ));
        };
        let stale = match self.observe() {
            Observed::Own(record) => {
                // Запись упавшего раннера становится рабочей: её читают остальные команды.
                self.write_record(&record)?;
                let mut result = self.result(
                    ServiceState::StartedByThisCopy,
                    ServiceAction::AlreadyRunning,
                );
                result.ready = self.host.port_answers(record.port);
                return Ok(self.describe(result, record.pid));
            }
            Observed::Starting(_) => {
                return Err(being_started("the server", &self.declared.infobase))
            }
            Observed::Stale => true,
            Observed::Nothing => false,
        };
        if let Some(pid) = self.host.server_over(&self.declared.db_path) {
            return Err(UseCaseError::new(
                UseCaseErrorKind::WorkspaceBusy,
                format!(
                    "the infobase '{}' is served by ibsrv pid {pid}, which this runner did not start; it is left running",
                    self.declared.db_path.display()
                ),
            ));
        }
        if !self.declared.db_path.join("1Cv8.1CD").is_file() {
            return Err(validation(format!(
                "there is no infobase in '{}': create it with `infobase create` before the server starts",
                self.declared.db_path.display()
            )));
        }
        let ibsrv = (self.locate)()?;
        self.paths
            .prepare()
            .map_err(|error| io_failure("cannot prepare", self.paths.root(), &error))?;
        let ports = match self.pins() {
            Some(ports) => {
                if let Some(pid) = self.occupant(ports) {
                    return Err(UseCaseError::new(
                        UseCaseErrorKind::WorkspaceBusy,
                        format!(
                            "a port of the server of '{}' is taken by process {pid}, which this runner did not start; it is left running: free the port and repeat",
                            self.declared.infobase
                        ),
                    ));
                }
                ports
            }
            None => self.choose_ports()?,
        };
        write_json(&self.paths.pins(), &ports)
            .map_err(|error| io_failure("cannot write", &self.paths.pins(), &error))?;
        let key = self.paths.host_key();
        if !key.is_file() {
            write_service_host_key(&key)
                .map_err(|error| UseCaseError::new(UseCaseErrorKind::Runtime, error.to_string()))?;
        }
        std::fs::create_dir_all(self.paths.data())
            .map_err(|error| io_failure("cannot prepare", &self.paths.data(), &error))?;
        let starter = {
            let pid = std::process::id();
            self.host.facts(pid).map(|facts| (pid, facts.started))
        };
        let mut record = ServiceRecord {
            state: RecordState::Starting,
            pid: None,
            started: None,
            image: None,
            machine: Some(machine),
            starter,
            port: ports.direct,
            explicit: true,
            bin: ibsrv.clone(),
            version: None,
        };
        self.write_record(&record)?;
        let request = ibsrv::start_over_file_base(
            &ibsrv,
            &self.paths.data(),
            &self.declared.db_path,
            &self.declared.name,
            ports,
            &key,
            &self.paths.log(),
        );
        let pid = self
            .host
            .spawn(&request, self.context.work())
            .map_err(|error| {
                self.forget_record();
                if let ServiceSpawnError::Io(_) = error {
                    UseCaseError::new(
                        UseCaseErrorKind::Platform,
                        format!("ibsrv did not start: {error}"),
                    )
                } else {
                    environment(error.to_string())
                }
            })?;
        let Some(facts) = self.host.facts(pid) else {
            self.forget_record();
            return Err(self.not_ready("ibsrv exited as soon as it started"));
        };
        record.pid = Some(pid);
        record.started = Some(facts.started);
        record.image = Some(facts.image);
        if let Err(error) = self.write_record(&record) {
            self.stop_process(&record);
            return Err(error);
        }
        self.await_ready(&record, ports)?;
        record.state = RecordState::Running;
        record.starter = None;
        if let Err(error) = self.write_record(&record) {
            self.stop_process(&record);
            return Err(error);
        }
        let mut result = self.result(ServiceState::StartedByThisCopy, ServiceAction::Started);
        result.ready = true;
        result.stale_record = stale;
        Ok(self.describe(result, record.pid))
    }

    fn await_ready(&self, record: &ServiceRecord, ports: ServerPorts) -> Result<(), UseCaseError> {
        let deadline = Instant::now() + self.host.patience(READY_TIMEOUT);
        let pid = record.pid.expect("a started server has a pid");
        loop {
            if let Some(error) = interruption_before_safe_point(self.context, "waiting for ibsrv") {
                self.stop_process(record);
                return Err(error.into());
            }
            if self.host.port_answers(ports.direct) {
                return Ok(());
            }
            let gone = self
                .host
                .facts(pid)
                .is_none_or(|facts| Some(facts.started) != record.started);
            if gone {
                self.forget_record();
                return Err(
                    self.not_ready("ibsrv exited before its direct gate accepted a connection")
                );
            }
            if Instant::now() >= deadline {
                self.stop_process(record);
                return Err(
                    self.not_ready("the direct gate of ibsrv did not accept a connection in time")
                );
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

    fn plan(&self) -> StandaloneResult {
        let mut result = self.result(ServiceState::NotRunning, ServiceAction::Planned);
        let mut pid = None;
        match self.observe() {
            Observed::Own(record) => {
                result.state = ServiceState::StartedByThisCopy;
                pid = record.pid;
            }
            Observed::Starting(_) => result.state = ServiceState::Starting,
            Observed::Stale => result.stale_record = true,
            Observed::Nothing => {}
        }
        self.describe(result, pid)
    }

    /// Мягко, с проверкой конца процесса; затем принудительно. Запись снимает только подтверждённый
    /// конец. Успех `taskkill` без `/F` ничего не доказывает.
    fn stop_process(&self, record: &ServiceRecord) -> bool {
        let Some(pid) = record.pid else {
            return false;
        };
        self.host.soft_stop(self.runner, pid, self.context);
        let mut gone = self.await_gone(pid, record.started, self.host.patience(SOFT_STOP_TIMEOUT));
        if !gone {
            self.host.hard_stop(self.runner, pid, self.context);
            gone = self.await_gone(pid, record.started, self.host.patience(SOFT_STOP_TIMEOUT));
        }
        if gone {
            self.forget_record();
        }
        gone
    }

    fn await_gone(&self, pid: u32, started: Option<u64>, timeout: Duration) -> bool {
        await_gone(self.host, pid, started, timeout)
    }

    fn stop(&self, dry_run: bool) -> Result<StandaloneResult, UseCaseError> {
        if dry_run {
            return Ok(self.plan());
        }
        let record = match self.observe() {
            Observed::Own(record) => record,
            Observed::Starting(_) => {
                return Err(being_started("the server", &self.declared.infobase))
            }
            Observed::Nothing => return Ok(self.nothing_to_stop(false)),
            Observed::Stale => {
                self.forget_record();
                return Ok(self.nothing_to_stop(true));
            }
        };
        if !self.stop_process(&record) {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Platform,
                format!(
                    "ibsrv pid {} did not stop; repeat standalone stop",
                    record.pid.unwrap_or_default()
                ),
            ));
        }
        let mut result = self.result(ServiceState::NotRunning, ServiceAction::Stopped);
        result.stopped_pid = record.pid;
        Ok(result)
    }

    fn nothing_to_stop(&self, stale: bool) -> StandaloneResult {
        let mut result = self.result(ServiceState::NotRunning, ServiceAction::NothingToStop);
        result.stale_record = stale;
        self.name_occupant(&mut result);
        result
    }

    fn name_occupant(&self, result: &mut StandaloneResult) {
        let by_port = self.pins().and_then(|ports| self.occupant(ports));
        if let Some(pid) = by_port.or_else(|| self.host.server_over(&self.declared.db_path)) {
            result.state = ServiceState::Foreign;
            result.occupant_pid = Some(pid);
        }
    }

    fn status(&self) -> StandaloneResult {
        match self.observe() {
            Observed::Own(record) => {
                let mut result =
                    self.result(ServiceState::StartedByThisCopy, ServiceAction::Reported);
                result.ready = self.host.port_answers(record.port);
                self.describe(result, record.pid)
            }
            Observed::Starting(record) => {
                let result = self.result(ServiceState::Starting, ServiceAction::Reported);
                self.describe(result, record.pid)
            }
            observed => {
                let mut result = self.result(ServiceState::NotRunning, ServiceAction::Reported);
                result.stale_record = matches!(observed, Observed::Stale);
                self.name_occupant(&mut result);
                result
            }
        }
    }

    fn restart(&self, dry_run: bool) -> Result<StandaloneResult, UseCaseError> {
        if dry_run {
            return Ok(self.plan());
        }
        let stopped = self.stop(false)?;
        if let Some(error) = interruption_before_safe_point(self.context, "starting ibsrv again") {
            return Err(error.into());
        }
        let mut started = self.start(false)?;
        started.stopped_pid = stopped.stopped_pid;
        Ok(started)
    }

    fn failed(&self) -> StandaloneResult {
        let mut result = self.status();
        result.ok = false;
        result.log_path = Some(self.paths.log());
        result
    }
}

pub(super) fn run_standalone(
    context: &ExecutionContext,
    config: &AppConfig,
    command: ServiceCommand,
    dry_run: bool,
    runner: &dyn ProcessRunner,
    host: &dyn ServiceHost,
    locate: &dyn Fn() -> Result<PathBuf, UseCaseError>,
) -> UseCaseResult<StandaloneResult> {
    let declared = declared(config).map_err(UseCaseFailure::without_payload)?;
    let paths = ServicePaths::new(&config.work_path, &declared.infobase, "standalone");
    let server = Server {
        context,
        runner,
        host,
        locate,
        declared,
        paths,
        started_at: Instant::now(),
    };
    let outcome = match command {
        ServiceCommand::Start => server.start(dry_run),
        ServiceCommand::Status => Ok(server.status()),
        ServiceCommand::Stop => server.stop(dry_run),
        ServiceCommand::Restart => server.restart(dry_run),
    };
    outcome.map_err(|error| UseCaseFailure::with_payload(error, server.failed()))
}

/// Имя образа сервера, как его сравнивает запись.
#[cfg(test)]
fn image() -> String {
    format!("ibsrv{}", std::env::consts::EXE_SUFFIX)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::config::model::{InfobaseConfig, ManagedServerConfig, StandaloneConfig};
    use crate::platform::process::{
        ProcessError, ProcessExecutionPolicy, ProcessRequest, ProcessResult, SpawnResult, WorkGiven,
    };
    use crate::platform::service::{ProcessFacts, ServiceRequest};
    use crate::use_cases::context::CommandName;
    use std::path::Path;

    const PID: u32 = 7001;
    const STARTED: u64 = 11;
    const MACHINE: &str = "machine";

    struct NoRunner;

    impl ProcessRunner for NoRunner {
        fn run_with_policy(
            &self,
            _request: &ProcessRequest,
            _policy: &ProcessExecutionPolicy,
        ) -> Result<ProcessResult, ProcessError> {
            unreachable!("the server stops through the host")
        }
        fn spawn(
            &self,
            _request: &ProcessRequest,
            _work: &WorkGiven,
        ) -> Result<SpawnResult, ProcessError> {
            unreachable!("a service is started through the host")
        }
    }

    #[derive(Default)]
    struct Host {
        unsupported: Cell<bool>,
        processes: RefCell<HashMap<u32, ProcessFacts>>,
        listeners: RefCell<HashMap<u16, u32>>,
        spawned: Cell<u32>,
        exits_at_once: Cell<bool>,
        never_ready: Cell<bool>,
        ignores_soft_stop: Cell<bool>,
        soft_stops: RefCell<HashSet<u32>>,
        hard_stops: RefCell<HashSet<u32>>,
        last_args: RefCell<Vec<String>>,
        foreign_server: Cell<Option<u32>>,
    }

    impl ServiceHost for Host {
        fn supported(&self) -> bool {
            !self.unsupported.get()
        }
        fn spawn(
            &self,
            request: &ServiceRequest,
            _work: &WorkGiven,
        ) -> Result<u32, ServiceSpawnError> {
            self.spawned.set(self.spawned.get() + 1);
            self.last_args.replace(request.args.clone());
            if !self.exits_at_once.get() {
                self.processes.borrow_mut().insert(
                    PID,
                    ProcessFacts {
                        started: STARTED,
                        image: image(),
                    },
                );
                if !self.never_ready.get() {
                    let port = request
                        .args
                        .iter()
                        .find_map(|arg| arg.strip_prefix("--direct-regport="))
                        .and_then(|port| port.parse().ok())
                        .expect("the direct port");
                    self.listeners.borrow_mut().insert(port, PID);
                }
            }
            Ok(PID)
        }
        fn facts(&self, pid: u32) -> Option<ProcessFacts> {
            self.processes.borrow().get(&pid).cloned()
        }
        fn machine(&self) -> Option<String> {
            Some(MACHINE.to_owned())
        }
        fn listening_pid(&self, port: u16) -> Option<u32> {
            self.listeners.borrow().get(&port).copied()
        }
        fn port_answers(&self, port: u16) -> bool {
            self.listeners.borrow().contains_key(&port)
        }
        fn soft_stop(&self, _runner: &dyn ProcessRunner, pid: u32, _context: &ExecutionContext) {
            self.soft_stops.borrow_mut().insert(pid);
            if !self.ignores_soft_stop.get() {
                self.processes.borrow_mut().remove(&pid);
                self.listeners.borrow_mut().clear();
            }
        }
        fn hard_stop(&self, _runner: &dyn ProcessRunner, pid: u32, _context: &ExecutionContext) {
            self.hard_stops.borrow_mut().insert(pid);
            self.processes.borrow_mut().remove(&pid);
            self.listeners.borrow_mut().clear();
        }
        fn sleep(&self, _duration: Duration) {}
        fn patience(&self, _default: Duration) -> Duration {
            Duration::ZERO
        }
        fn server_over(&self, _db_path: &Path) -> Option<u32> {
            self.foreign_server.get()
        }
    }

    struct Setup {
        _root: tempfile::TempDir,
        config: AppConfig,
    }

    fn setup() -> Setup {
        let root = tempfile::tempdir().unwrap();
        let db_path = root.path().join("ib");
        std::fs::create_dir_all(&db_path).unwrap();
        std::fs::write(db_path.join("1Cv8.1CD"), "").unwrap();
        let mut infobase = InfobaseConfig::file(String::new());
        infobase.standalone = Some(StandaloneConfig {
            managed: Some(ManagedServerConfig {
                name: Some("demo".to_owned()),
                db_path: Some(db_path),
            }),
            gate: None,
            host_fingerprint: None,
            exchange: None,
        });
        let config = AppConfig {
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
        Setup {
            _root: root,
            config,
        }
    }

    impl Setup {
        fn paths(&self) -> ServicePaths {
            ServicePaths::new(&self.config.work_path, "origin", "standalone")
        }
        fn run(
            &self,
            command: ServiceCommand,
            dry_run: bool,
            host: &Host,
        ) -> Result<StandaloneResult, UseCaseError> {
            self.run_in(
                &ExecutionContext::cli(CommandName::StandaloneStart),
                command,
                dry_run,
                host,
            )
        }
        fn run_in(
            &self,
            context: &ExecutionContext,
            command: ServiceCommand,
            dry_run: bool,
            host: &Host,
        ) -> Result<StandaloneResult, UseCaseError> {
            let locate = || Ok(PathBuf::from("ibsrv.exe"));
            run_standalone(
                context,
                &self.config,
                command,
                dry_run,
                &NoRunner,
                host,
                &locate,
            )
            .map_err(|failure| failure.error)
        }
    }

    #[test]
    fn start_runs_the_server_and_names_its_addresses() {
        let setup = setup();
        let host = Host::default();
        let started = setup.run(ServiceCommand::Start, false, &host).unwrap();
        assert_eq!(started.action, ServiceAction::Started);
        assert_eq!(started.pid, Some(PID));
        assert!(started.ready);
        let connection = started.connection.unwrap();
        assert!(connection.ends_with(";Ref=demo"), "{connection}");
        assert!(setup.paths().host_key().is_file(), "the gate key is made");
        assert!(host
            .last_args
            .borrow()
            .iter()
            .any(|arg| arg == "--enable-ssh-gate"));
    }

    #[test]
    fn a_second_start_names_the_running_server_and_starts_nothing() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        let second = setup.run(ServiceCommand::Start, false, &host).unwrap();
        assert_eq!(second.action, ServiceAction::AlreadyRunning);
        assert_eq!(host.spawned.get(), 1);
    }

    #[test]
    fn the_ports_and_the_key_stay_from_start_to_start() {
        let setup = setup();
        let host = Host::default();
        let first = setup.run(ServiceCommand::Start, false, &host).unwrap();
        let key = std::fs::read(setup.paths().host_key()).unwrap();
        setup.run(ServiceCommand::Stop, false, &host).unwrap();
        let second = setup.run(ServiceCommand::Start, false, &host).unwrap();
        assert_eq!(first.connection, second.connection);
        assert_eq!(key, std::fs::read(setup.paths().host_key()).unwrap());
    }

    #[test]
    fn start_without_an_infobase_names_the_command_that_creates_it() {
        let setup = setup();
        let host = Host::default();
        let managed = setup
            .config
            .infobase
            .standalone
            .as_ref()
            .unwrap()
            .managed
            .as_ref()
            .unwrap();
        std::fs::remove_file(managed.db_path.as_ref().unwrap().join("1Cv8.1CD")).unwrap();
        let refused = setup.run(ServiceCommand::Start, false, &host).unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::Validation);
        assert!(refused.to_string().contains("infobase create"), "{refused}");
        assert_eq!(host.spawned.get(), 0);
    }

    #[test]
    fn stop_is_soft_first_and_confirms_the_end_of_the_process() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        let stopped = setup.run(ServiceCommand::Stop, false, &host).unwrap();
        assert_eq!(stopped.stopped_pid, Some(PID));
        assert!(host.soft_stops.borrow().contains(&PID));
        assert!(host.hard_stops.borrow().is_empty());
        assert!(!setup.paths().record().exists());
    }

    #[test]
    fn a_server_that_ignores_the_soft_stop_is_stopped_hard() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        host.ignores_soft_stop.set(true);
        setup.run(ServiceCommand::Stop, false, &host).unwrap();
        assert!(host.hard_stops.borrow().contains(&PID));
        assert!(!setup.paths().record().exists());
    }

    #[test]
    fn a_foreign_process_on_a_pinned_port_is_named_and_left_alone() {
        let setup = setup();
        let host = Host::default();
        let started = setup.run(ServiceCommand::Start, false, &host).unwrap();
        setup.run(ServiceCommand::Stop, false, &host).unwrap();
        let port: u16 = started
            .connection
            .unwrap()
            .split(':')
            .nth(1)
            .and_then(|tail| tail.split(';').next())
            .and_then(|port| port.parse().ok())
            .unwrap();
        host.listeners.borrow_mut().insert(port, 555);
        let refused = setup.run(ServiceCommand::Start, false, &host).unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::WorkspaceBusy);
        assert!(refused.to_string().contains("process 555"), "{refused}");
        let status = setup.run(ServiceCommand::Status, false, &host).unwrap();
        assert_eq!(status.state, ServiceState::Foreign);
        assert_eq!(status.occupant_pid, Some(555));
        let stop = setup.run(ServiceCommand::Stop, false, &host).unwrap();
        assert_eq!(stop.action, ServiceAction::NothingToStop);
        assert!(
            host.soft_stops.borrow().len() == 1,
            "only our own was stopped"
        );
    }

    #[test]
    fn a_server_that_exits_at_once_answers_with_the_log() {
        let setup = setup();
        let host = Host::default();
        host.exits_at_once.set(true);
        let refused = setup.run(ServiceCommand::Start, false, &host).unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::Platform);
        assert!(refused.to_string().contains("standalone"), "{refused}");
        assert!(!setup.paths().record().exists());
    }

    #[test]
    fn a_gate_that_never_answers_stops_the_server() {
        let setup = setup();
        let host = Host::default();
        host.never_ready.set(true);
        let refused = setup.run(ServiceCommand::Start, false, &host).unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::Platform);
        assert!(host.soft_stops.borrow().contains(&PID));
        assert!(!setup.paths().record().exists());
    }

    #[test]
    fn a_cancelled_start_stops_the_server() {
        let setup = setup();
        let host = Host::default();
        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();
        let context = ExecutionContext::cli(CommandName::StandaloneStart).with_cancellation(token);
        let error = setup
            .run_in(&context, ServiceCommand::Start, false, &host)
            .unwrap_err();
        assert!(
            matches!(error.kind(), UseCaseErrorKind::Cancelled(_)),
            "{error}"
        );
        assert!(host.processes.borrow().is_empty());
    }

    #[test]
    fn a_stale_record_is_named_and_replaced() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        host.processes.borrow_mut().clear();
        host.listeners.borrow_mut().clear();
        let status = setup.run(ServiceCommand::Status, false, &host).unwrap();
        assert_eq!(status.state, ServiceState::NotRunning);
        assert!(status.stale_record);
        let started = setup.run(ServiceCommand::Start, false, &host).unwrap();
        assert_eq!(started.action, ServiceAction::Started);
        assert!(started.stale_record);
    }

    #[test]
    fn other_systems_refuse_and_a_preview_changes_nothing() {
        let setup = setup();
        let host = Host::default();
        host.unsupported.set(true);
        let refused = setup.run(ServiceCommand::Start, false, &host).unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::Environment);
        for command in [
            ServiceCommand::Start,
            ServiceCommand::Stop,
            ServiceCommand::Restart,
        ] {
            assert_eq!(
                setup.run(command, true, &host).unwrap().action,
                ServiceAction::Planned
            );
        }
        assert_eq!(host.spawned.get(), 0);
        assert!(!setup.config.work_path.exists());
    }

    #[test]
    fn a_base_without_a_managed_server_is_refused() {
        let mut setup = setup();
        setup.config.infobase.standalone = None;
        let host = Host::default();
        assert_eq!(
            setup
                .run(ServiceCommand::Status, false, &host)
                .unwrap_err()
                .kind(),
            UseCaseErrorKind::Validation
        );
    }

    #[test]
    fn a_foreign_server_over_the_same_base_is_named_and_left_alone() {
        let setup = setup();
        let host = Host::default();
        host.foreign_server.set(Some(30117));
        let refused = setup.run(ServiceCommand::Start, false, &host).unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::WorkspaceBusy);
        assert!(refused.to_string().contains("30117"), "{refused}");
        assert_eq!(host.spawned.get(), 0);
        let status = setup.run(ServiceCommand::Status, false, &host).unwrap();
        assert_eq!(status.state, ServiceState::Foreign);
        assert_eq!(status.occupant_pid, Some(30117));
        let stop = setup.run(ServiceCommand::Stop, false, &host).unwrap();
        assert_eq!(stop.action, ServiceAction::NothingToStop);
        assert!(host.soft_stops.borrow().is_empty());
    }

    #[test]
    fn a_crashed_start_with_a_live_server_is_taken_over_and_the_record_becomes_running() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        let mut record: ServiceRecord = read_json(&setup.paths().record()).unwrap();
        record.state = RecordState::Starting;
        record.starter = Some((900, 5));
        write_json(&setup.paths().record(), &record).unwrap();
        let again = setup.run(ServiceCommand::Start, false, &host).unwrap();
        assert_eq!(again.action, ServiceAction::AlreadyRunning);
        let written: ServiceRecord = read_json(&setup.paths().record()).unwrap();
        assert_eq!(written.state, RecordState::Running);
        assert_eq!(host.spawned.get(), 1);
    }

    #[test]
    fn restart_stops_and_starts_the_server_on_the_same_ports() {
        let setup = setup();
        let host = Host::default();
        let first = setup.run(ServiceCommand::Start, false, &host).unwrap();
        let again = setup.run(ServiceCommand::Restart, false, &host).unwrap();
        assert_eq!(again.action, ServiceAction::Started);
        assert_eq!(again.stopped_pid, Some(PID));
        assert_eq!(first.connection, again.connection);
        assert_eq!(host.spawned.get(), 2);
    }
}
