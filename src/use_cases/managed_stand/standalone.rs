//! Автономный сервер, которого поднимает раннер: `standalone start|status|stop|restart`.
//! Адрес такой цели остальные команды берут из той же записи, её читает загрузчик настроек.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{
    await_gone, being_started, cluster_over, environment, forget_record, io_failure, occupant_of,
    owned_by_a_cluster, port_offsets, validation, write_record, Database, DbmsAccess,
    ServiceCommand, ServiceHost, Stand,
};
use crate::config::model::AppConfig;
use crate::config::model::{InfobaseConfig, InfobaseDbmsConfig};
use crate::domain::dbms::{ServiceAction, ServiceState};
use crate::domain::standalone::StandaloneResult;
use crate::platform::agent::write_service_host_key;
use crate::platform::ibcmd::{IbcmdConnection, IbcmdDsl, IbcmdInfobaseCreateStatus};
use crate::platform::ibsrv::{self, DbmsTarget, ServerPorts, ServerStorage};
use crate::platform::postgres;
use crate::platform::process::ProcessRunner;
use crate::platform::service::ServiceSpawnError;
use crate::platform::service_record::{
    adopt_recorded_pid, observe_service, read_json, write_json, Observed, RecordState,
    ServicePaths, ServiceRecord,
};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::context::InterruptionSafetyClass;
use crate::use_cases::interruption::interruption_before_safe_point;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind, UseCaseFailure, UseCaseResult};

/// Образ сервера в перечне процессов.
const IBSRV_IMAGE: &str = "ibsrv.exe";
const READY_TIMEOUT: Duration = Duration::from_secs(120);
const SOFT_STOP_TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(200);

/// Где лежит база сервера.
enum Storage {
    File(PathBuf),
    Dbms(Database),
}

/// То, что сценарий узнаёт о цели из настроек.
struct Declared {
    infobase: String,
    name: String,
    storage: Storage,
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
    let name = managed.reference(&infobase);
    let storage = match (&managed.db_path, &config.infobase.dbms) {
        (Some(db_path), None) => Storage::File(db_path.clone()),
        (None, Some(dbms)) => Storage::Dbms(Database::declared(dbms)?),
        _ => {
            return Err(validation(
                "infobase.standalone.managed names one storage: either db-path or the section infobase.dbms",
            ))
        }
    };
    Ok(Declared {
        infobase,
        name,
        storage,
    })
}

struct Server<'a> {
    config: &'a AppConfig,
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

    /// Файловая база сервера, если он над файловой базой.
    fn db_path(&self) -> Option<&std::path::Path> {
        match &self.declared.storage {
            Storage::File(path) => Some(path),
            _ => None,
        }
    }

    /// Чужой `ibsrv` над этой же базой: ключи запуска, которые её называют.
    fn foreign_server(&self) -> Option<u32> {
        let keys = match &self.declared.storage {
            Storage::File(path) => ibsrv::file_base_keys(path),
            Storage::Dbms(database) => {
                let (server, name) = database.address(self.stand().as_ref())?;
                ibsrv::database_keys(&server, &name)
            }
        };
        // Без перечня процессов чужой сервер не назвать: отказ по нему только сообщает, а запуск
        // упрётся в занятый порт.
        crate::platform::service::first_with_keys(
            &self.host.processes(&[IBSRV_IMAGE]).unwrap_or_default(),
            &keys,
        )
    }

    /// СУБД, которую поднимает раннер, как шаг этого сервера.
    fn stand(&self) -> Option<Stand<'_>> {
        matches!(
            self.declared.storage,
            Storage::Dbms(Database::Managed { .. })
        )
        .then(|| Stand::new(self.config, self.context, self.runner, self.host).ok())
        .flatten()
        .map(Stand::server_step)
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
            stopped_dbms_pid: None,
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
        if let Some(pid) = self.foreign_server() {
            return Err(UseCaseError::new(
                UseCaseErrorKind::WorkspaceBusy,
                format!(
                    "the infobase of '{}' is served by ibsrv pid {pid}, which this runner did not start; it is left running",
                    self.declared.infobase
                ),
            ));
        }
        if let Storage::Dbms(database) = &self.declared.storage {
            if let Some(owner) = cluster_over(self.config, self.host, database) {
                return Err(owned_by_a_cluster(&owner));
            }
        }
        if let Some(db_path) = self.db_path() {
            if !db_path.join("1Cv8.1CD").is_file() {
                return Err(validation(format!(
                    "there is no infobase in '{}': create it with `infobase create` before the server starts",
                    db_path.display()
                )));
            }
        }
        // Шаг сервера: поднятая здесь СУБД гаснет при любой неудаче дальше.
        let stand = self.stand();
        let raised = match &stand {
            Some(stand) => stand.start(false)?.action == ServiceAction::Started,
            None => false,
        };
        match self.start_process(stale, machine, stand.as_ref()) {
            Ok(result) => Ok(result),
            Err(error) => {
                if raised {
                    if let Some(stand) = &stand {
                        let _ = stand.stop_if_step();
                    }
                }
                Err(error)
            }
        }
    }

    fn start_process(
        &self,
        stale: bool,
        machine: String,
        stand: Option<&Stand<'_>>,
    ) -> Result<StandaloneResult, UseCaseError> {
        let storage = self.server_storage(stand)?;
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
        let request = ibsrv::start_server(
            &ibsrv,
            &self.paths.data(),
            &storage.view(),
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

    /// Значения для ключей `ibsrv`. У управляемой СУБД они из её записи, и БД должна уже быть.
    fn server_storage(&self, stand: Option<&Stand<'_>>) -> Result<OwnedStorage, UseCaseError> {
        match &self.declared.storage {
            Storage::File(path) => Ok(OwnedStorage::File(path.clone())),
            Storage::Dbms(database) => {
                let access = database.access(stand)?;
                if let (Database::Managed { name }, Some(stand)) = (database, stand) {
                    let (port, bin) = stand.running_record()?;
                    if !stand.database_exists(port, &bin, name)? {
                        return Err(validation(format!(
                            "there is no database '{name}' in the PostgreSQL of this infobase: create it with `infobase create` before the server starts"
                        )));
                    }
                }
                Ok(OwnedStorage::Dbms(access))
            }
        }
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
        let mut result = self.stop_server(dry_run)?;
        if !dry_run {
            // Сервер гаснет первым: СУБД нужна ему до конца. Шаг сервера гаснет вместе с ним, в том
            // числе если сам сервер уже упал.
            if let Some(stand) = self.stand() {
                result.stopped_dbms_pid = stand.stop_if_step()?;
            }
        }
        Ok(result)
    }

    fn stop_server(&self, dry_run: bool) -> Result<StandaloneResult, UseCaseError> {
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
        if let Some(pid) = by_port.or_else(|| self.foreign_server()) {
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
        // СУБД не трогает: перезапускается только сервер.
        let stopped = self.stop_server(false)?;
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
        config,
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

/// Значения для ключей `ibsrv`, владеющие своими строками.
enum OwnedStorage {
    File(PathBuf),
    Dbms(DbmsAccess),
}

impl OwnedStorage {
    fn view(&self) -> ServerStorage<'_> {
        match self {
            Self::File(path) => ServerStorage::File(path),
            Self::Dbms(access) => ServerStorage::Dbms(DbmsTarget {
                kind: &access.kind,
                server: &access.server,
                name: &access.name,
                user: access.user.as_deref(),
                password: access.password.as_deref(),
            }),
        }
    }
}

/// `infobase create` базы сервера в СУБД: через `ibcmd`, до запуска сервера. У PostgreSQL раннера
/// существующая БД — отказ до платформы; у вашей СУБД отвечает сам `ibcmd`.
pub(super) fn create_base(
    context: &ExecutionContext,
    config: &AppConfig,
    binary: PathBuf,
    dry_run: bool,
    runner: &dyn ProcessRunner,
    host: &dyn ServiceHost,
) -> Result<String, UseCaseError> {
    let declared = declared(config)?;
    let paths = ServicePaths::new(&config.work_path, &declared.infobase, "standalone");
    let database = match &declared.storage {
        Storage::File(_) => {
            return Err(validation("a file infobase is created by its own path"));
        }
        Storage::Dbms(database) => database,
    };
    // Раньше вопроса о БД: иначе отказ назвал бы не ту причину.
    if let Some(owner) = cluster_over(config, host, database) {
        return Err(owned_by_a_cluster(&owner));
    }
    let stand = match database {
        Database::Managed { .. } => Some(Stand::new(config, context, runner, host)?),
        Database::Own(_) => None,
    };
    if let (Database::Managed { name }, Some(stand)) = (database, &stand) {
        let Some((port, bin)) = stand.running_record().ok() else {
            return Err(validation(
                "the PostgreSQL of this infobase is not running: run `dbms start`",
            ));
        };
        if dry_run {
            return Ok(format!(
                "would create the database '{name}' on {}",
                postgres::server_address(port)
            ));
        }
        if stand.database_exists(port, &bin, name)? {
            return Err(validation(format!(
                "the database '{name}' already exists in the PostgreSQL of this infobase: name another with infobase.dbms.name, or drop it with PostgreSQL tools using the password file `dbms status` names"
            )));
        }
    }
    let DbmsAccess {
        kind,
        server,
        name,
        user,
        password,
    } = database.access(stand.as_ref())?;
    if dry_run {
        return Ok(format!("would create the database '{name}' on {server}"));
    }
    let dbms = InfobaseDbmsConfig {
        kind: Some(kind),
        server: Some(server.clone()),
        name: Some(name.clone()),
        user,
        password,
        locale: None,
        managed: None,
    };
    let target = InfobaseConfig::server(format!("Srvr=127.0.0.1;Ref={}", declared.name), dbms);
    let connection = IbcmdConnection::from_infobase(&target)
        .map_err(|error| UseCaseError::new(UseCaseErrorKind::Runtime, error.to_string()))?;
    std::fs::create_dir_all(paths.data())
        .map_err(|error| io_failure("cannot prepare", &paths.data(), &error))?;
    let locale = config
        .infobase
        .dbms
        .as_ref()
        .and_then(|dbms| dbms.locale.clone());
    let outcome = IbcmdDsl::new(
        binary,
        connection,
        runner,
        context.process_policy(InterruptionSafetyClass::GracefulThenKill, None),
    )
    .with_data_path(paths.data())
    .with_locale(locale)
    .ensure_infobase_create()
    .map_err(|error| UseCaseError::new(UseCaseErrorKind::Platform, error.to_string()))?;
    match outcome.status {
        IbcmdInfobaseCreateStatus::Created => Ok(format!(
            "the database '{name}' created on {server}; the first push loads every source-set in full"
        )),
        IbcmdInfobaseCreateStatus::AlreadyExists => Err(validation(format!(
            "the database '{name}' on {server} already holds an infobase: infobase create leaves it untouched"
        ))),
        _ => Err(UseCaseError::new(
            UseCaseErrorKind::Platform,
            format!(
                "ibcmd infobase create failed for the database '{name}': {}",
                outcome.result.process.stderr.trim()
            ),
        )),
    }
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
    use crate::use_cases::result::EnvironmentReason;

    const PID: u32 = 7001;
    const PG_PID: u32 = 7002;
    const STARTED: u64 = 11;
    const MACHINE: &str = "machine";

    /// Утилиты PostgreSQL: те же подделки, что у сценария `dbms`.
    struct Runner<'a> {
        host: &'a Host,
    }

    impl ProcessRunner for Runner<'_> {
        fn run_with_policy(
            &self,
            request: &ProcessRequest,
            _policy: &ProcessExecutionPolicy,
        ) -> Result<ProcessResult, ProcessError> {
            self.host.calls.borrow_mut().push(request.args.clone());
            let tool = request
                .program
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let mut stdout = String::new();
            match tool.as_str() {
                "initdb" => {
                    let data = PathBuf::from(&request.args[1]);
                    std::fs::create_dir_all(&data).unwrap();
                    std::fs::write(data.join("PG_VERSION"), "17").unwrap();
                }
                "pg_ctl" if request.args[0] == "stop" => {
                    self.host.processes.borrow_mut().remove(&PG_PID);
                    self.host
                        .listeners
                        .borrow_mut()
                        .retain(|_, pid| *pid != PG_PID);
                    self.host.pg_stops.set(self.host.pg_stops.get() + 1);
                }
                "psql" if self.host.database_exists.get() => {
                    stdout = "1".to_owned();
                }
                _ => {}
            }
            Ok(ProcessResult {
                exit_code: 0,
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

    #[derive(Default)]
    struct Host {
        unsupported: Cell<bool>,
        processes: RefCell<HashMap<u32, ProcessFacts>>,
        listeners: RefCell<HashMap<u16, u32>>,
        spawned: Cell<u32>,
        exits_at_once: Cell<bool>,
        never_ready: Cell<bool>,
        ignores_soft_stop: Cell<bool>,
        /// `ibsrv`, которого не снять ничем.
        immortal: Cell<bool>,
        /// Отмена команды, наступающая, как только поднят `ibsrv`.
        cancel_on_server: RefCell<Option<tokio_util::sync::CancellationToken>>,
        soft_stops: RefCell<HashSet<u32>>,
        hard_stops: RefCell<HashSet<u32>>,
        last_args: RefCell<Vec<String>>,
        /// Чужие `ibsrv`: pid и командная строка.
        foreign: RefCell<Vec<(u32, String)>>,
        database_exists: Cell<bool>,
        calls: RefCell<Vec<Vec<String>>>,
        pg_stops: Cell<u32>,
        last_stdin: RefCell<Option<String>>,
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
            if request.program.to_string_lossy().contains("pg_ctl") {
                let data = PathBuf::from(&request.args[3]);
                std::fs::write(data.join("postmaster.pid"), format!("{PG_PID}")).unwrap();
                self.processes.borrow_mut().insert(
                    PG_PID,
                    ProcessFacts {
                        started: STARTED,
                        image: "postgres.exe".to_owned(),
                    },
                );
                let port = request
                    .args
                    .last()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                self.listeners
                    .borrow_mut()
                    .insert(port.parse().unwrap(), PG_PID);
                return Ok(7);
            }
            self.spawned.set(self.spawned.get() + 1);
            if let Some(token) = self.cancel_on_server.borrow().as_ref() {
                token.cancel();
            }
            self.last_args.replace(request.args.clone());
            self.last_stdin.replace(request.stdin.clone());
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
                self.listeners.borrow_mut().retain(|_, held| *held != pid);
            }
        }
        fn hard_stop(&self, _runner: &dyn ProcessRunner, pid: u32, _context: &ExecutionContext) {
            self.hard_stops.borrow_mut().insert(pid);
            if pid == PID && self.immortal.get() {
                return;
            }
            self.processes.borrow_mut().remove(&pid);
            self.listeners.borrow_mut().retain(|_, held| *held != pid);
        }
        fn sleep(&self, _duration: Duration) {}
        fn patience(&self, _default: Duration) -> Duration {
            Duration::ZERO
        }
        fn processes(&self, images: &[&str]) -> Option<Vec<crate::platform::service::ProcessRow>> {
            assert_eq!(images, ["ibsrv.exe"]);
            Some(
                self.foreign
                    .borrow()
                    .iter()
                    .map(|(pid, command)| crate::platform::service::ProcessRow {
                        pid: *pid,
                        parent: 1,
                        command: command.clone(),
                    })
                    .collect(),
            )
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
                &Runner { host },
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
        assert_eq!(
            refused.kind(),
            UseCaseErrorKind::Environment(EnvironmentReason::Unavailable)
        );
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
        let db_path = setup
            .config
            .infobase
            .standalone
            .as_ref()
            .unwrap()
            .managed
            .as_ref()
            .unwrap()
            .db_path
            .clone()
            .unwrap();
        host.foreign.borrow_mut().extend([
            (
                30118,
                format!("ibsrv.exe \"--db-path={}x\"", db_path.display()),
            ),
            (
                30117,
                format!("ibsrv.exe \"--db-path={}\" --name=other", db_path.display()),
            ),
        ]);
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

    /// Сервер над базой в PostgreSQL, которого поднимает раннер.
    fn over_postgres() -> Setup {
        let mut setup = setup();
        let build = setup._root.path().join("pg");
        std::fs::create_dir_all(build.join("bin")).unwrap();
        std::fs::write(
            build
                .join("bin")
                .join(format!("pg_ctl{}", std::env::consts::EXE_SUFFIX)),
            "",
        )
        .unwrap();
        setup.config.tools.postgres.path = Some(build);
        setup.config.infobase.dbms = Some(crate::config::model::InfobaseDbmsConfig {
            managed: Some(crate::config::model::ManagedDbmsConfig::default()),
            name: Some("demo_data".to_owned()),
            ..Default::default()
        });
        let standalone = setup.config.infobase.standalone.as_mut().unwrap();
        standalone.managed.as_mut().unwrap().db_path = None;
        setup
    }

    fn dbms_record(setup: &Setup) -> ServiceRecord {
        let paths = ServicePaths::new(&setup.config.work_path, "origin", "postgres");
        read_json(&paths.record()).expect("the record of the PostgreSQL")
    }

    fn run_dbms(
        setup: &Setup,
        host: &Host,
        command: ServiceCommand,
    ) -> crate::domain::dbms::DbmsResult {
        super::super::run_dbms(
            &ExecutionContext::cli(CommandName::DbmsStart),
            &setup.config,
            command,
            false,
            &Runner { host },
            host,
        )
        .map_err(|failure| failure.error)
        .unwrap()
    }

    #[test]
    fn a_dbms_server_starts_over_its_database_with_the_password_on_stdin() {
        let setup = over_postgres();
        let host = Host::default();
        host.database_exists.set(true);
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        let args = host.last_args.borrow();
        assert!(
            args.iter().any(|arg| arg == "--dbms=PostgreSQL"),
            "{args:?}"
        );
        assert!(
            args.iter().any(|arg| arg == "--db-name=demo_data"),
            "{args:?}"
        );
        assert!(args.iter().any(|arg| arg == "-W"), "{args:?}");
        assert!(
            args.iter()
                .any(|arg| arg.starts_with("--data=") && arg.contains("standalone")),
            "{args:?}"
        );
        let stdin = host.last_stdin.borrow().clone().expect("the password");
        assert!(!args.iter().any(|arg| arg.contains(&stdin)), "{args:?}");
    }

    #[test]
    fn a_foreign_server_over_the_same_database_is_found_by_its_keys() {
        let setup = over_postgres();
        let host = Host::default();
        super::super::run_dbms(
            &ExecutionContext::cli(CommandName::DbmsStart),
            &setup.config,
            ServiceCommand::Start,
            false,
            &Runner { host: &host },
            &host,
        )
        .map_err(|failure| failure.error)
        .unwrap();
        let pins: crate::platform::service_record::PinnedPorts =
            read_json(&ServicePaths::new(&setup.config.work_path, "origin", "postgres").pins())
                .unwrap();
        host.foreign.borrow_mut().extend([
            (
                30118,
                format!("ibsrv.exe \"--db-server=127.0.0.1 port={}\" --db-name=other", pins.port),
            ),
            (
                30117,
                format!("ibsrv.exe --dbms=PostgreSQL \"--db-server=127.0.0.1 port={}\" --db-name=demo_data -W", pins.port),
            ),
        ]);
        let refused = setup.run(ServiceCommand::Start, false, &host).unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::WorkspaceBusy);
        assert!(refused.to_string().contains("30117"), "{refused}");
        assert_eq!(host.spawned.get(), 0, "no second server");
    }

    #[test]
    fn a_server_over_a_missing_database_names_infobase_create() {
        let setup = over_postgres();
        let host = Host::default();
        let refused = setup.run(ServiceCommand::Start, false, &host).unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::Validation);
        assert!(refused.to_string().contains("infobase create"), "{refused}");
        assert_eq!(host.spawned.get(), 0);
        assert!(
            !setup.paths().record().exists(),
            "a refusal leaves no record"
        );
        assert!(
            host.processes.borrow().is_empty(),
            "the step PostgreSQL is stopped"
        );
    }

    #[test]
    fn creating_over_an_existing_database_is_refused() {
        let setup = over_postgres();
        let host = Host::default();
        run_dbms(&setup, &host, ServiceCommand::Start);
        host.database_exists.set(true);
        let refused = create_base(
            &ExecutionContext::cli(CommandName::Init),
            &setup.config,
            PathBuf::from("ibcmd.exe"),
            false,
            &Runner { host: &host },
            &host,
        )
        .unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::Validation);
        assert!(refused.to_string().contains("already exists"), "{refused}");
        assert!(
            refused.to_string().contains("dbms status"),
            "the way out: {refused}"
        );
    }

    #[test]
    fn creating_without_the_postgres_names_dbms_start() {
        let setup = over_postgres();
        let host = Host::default();
        let refused = create_base(
            &ExecutionContext::cli(CommandName::Init),
            &setup.config,
            PathBuf::from("ibcmd.exe"),
            false,
            &Runner { host: &host },
            &host,
        )
        .unwrap_err();
        assert!(refused.to_string().contains("dbms start"), "{refused}");
    }

    #[test]
    fn a_server_step_postgres_stops_with_its_server() {
        let setup = over_postgres();
        let host = Host::default();
        host.database_exists.set(true);
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        assert!(!dbms_record(&setup).explicit);
        let stopped = setup.run(ServiceCommand::Stop, false, &host).unwrap();
        assert_eq!(stopped.stopped_pid, Some(PID));
        assert_eq!(stopped.stopped_dbms_pid, Some(PG_PID));
        assert!(host.processes.borrow().is_empty());
    }

    #[test]
    fn an_explicit_postgres_outlives_its_server() {
        let setup = over_postgres();
        let host = Host::default();
        host.database_exists.set(true);
        run_dbms(&setup, &host, ServiceCommand::Start);
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        let stopped = setup.run(ServiceCommand::Stop, false, &host).unwrap();
        assert_eq!(stopped.stopped_dbms_pid, None);
        assert!(host.processes.borrow().contains_key(&PG_PID));
    }

    #[test]
    fn a_later_dbms_start_makes_a_step_postgres_explicit() {
        let setup = over_postgres();
        let host = Host::default();
        host.database_exists.set(true);
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        run_dbms(&setup, &host, ServiceCommand::Start);
        assert!(dbms_record(&setup).explicit);
        setup.run(ServiceCommand::Stop, false, &host).unwrap();
        assert!(host.processes.borrow().contains_key(&PG_PID));
    }

    #[test]
    fn restart_leaves_the_postgres_alone() {
        let setup = over_postgres();
        let host = Host::default();
        host.database_exists.set(true);
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        setup.run(ServiceCommand::Restart, false, &host).unwrap();
        assert_eq!(host.pg_stops.get(), 0);
    }

    #[test]
    fn a_failed_server_stops_only_what_this_command_raised() {
        let setup = over_postgres();
        let host = Host::default();
        host.database_exists.set(true);
        host.exits_at_once.set(true);
        setup.run(ServiceCommand::Start, false, &host).unwrap_err();
        assert!(
            host.processes.borrow().is_empty(),
            "the step PostgreSQL is stopped"
        );

        let host = Host::default();
        host.database_exists.set(true);
        host.exits_at_once.set(true);
        run_dbms(&setup, &host, ServiceCommand::Start);
        setup.run(ServiceCommand::Start, false, &host).unwrap_err();
        assert!(
            host.processes.borrow().contains_key(&PG_PID),
            "the explicit one stays"
        );
    }

    #[test]
    fn a_crashed_server_still_takes_its_step_postgres_down() {
        let setup = over_postgres();
        let host = Host::default();
        host.database_exists.set(true);
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        host.processes.borrow_mut().remove(&PID);
        let stopped = setup.run(ServiceCommand::Stop, false, &host).unwrap();
        assert_eq!(stopped.action, ServiceAction::NothingToStop);
        assert_eq!(stopped.stopped_dbms_pid, Some(PG_PID));
    }

    #[test]
    fn a_fallen_postgres_under_a_live_server_is_named_and_comes_back_explicit_on_its_port() {
        let setup = over_postgres();
        let host = Host::default();
        host.database_exists.set(true);
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        let port = dbms_record(&setup).port;
        host.processes.borrow_mut().remove(&PG_PID);
        host.listeners.borrow_mut().retain(|_, pid| *pid != PG_PID);
        let status = run_dbms(&setup, &host, ServiceCommand::Status);
        assert_eq!(status.state, ServiceState::NotRunning);
        assert!(status.stale_record, "the fall is named");
        let started = run_dbms(&setup, &host, ServiceCommand::Start);
        assert_eq!(started.action, ServiceAction::Started);
        let record = dbms_record(&setup);
        assert_eq!(record.port, port, "the same port");
        assert!(record.explicit, "dbms start makes it explicit");
        let stopped = setup.run(ServiceCommand::Stop, false, &host).unwrap();
        assert_eq!(stopped.stopped_dbms_pid, None);
        assert!(host.processes.borrow().contains_key(&PG_PID));
    }

    #[test]
    fn after_a_reboot_the_server_starts_again_with_its_postgres_as_a_step() {
        let setup = over_postgres();
        let host = Host::default();
        host.database_exists.set(true);
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        let port = dbms_record(&setup).port;
        host.processes.borrow_mut().clear();
        host.listeners.borrow_mut().clear();
        let started = setup.run(ServiceCommand::Start, false, &host).unwrap();
        assert_eq!(started.action, ServiceAction::Started);
        assert!(started.stale_record);
        let record = dbms_record(&setup);
        assert_eq!(record.port, port);
        assert!(!record.explicit, "this start raised it as its step");
        let stopped = setup.run(ServiceCommand::Stop, false, &host).unwrap();
        assert_eq!(stopped.stopped_dbms_pid, Some(PG_PID));
    }

    #[test]
    fn after_a_reboot_an_explicit_postgres_raised_by_the_server_becomes_its_step() {
        let setup = over_postgres();
        let host = Host::default();
        host.database_exists.set(true);
        run_dbms(&setup, &host, ServiceCommand::Start);
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        assert!(dbms_record(&setup).explicit);
        host.processes.borrow_mut().clear();
        host.listeners.borrow_mut().clear();
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        assert!(!dbms_record(&setup).explicit, "standalone start raised it");
        let stopped = setup.run(ServiceCommand::Stop, false, &host).unwrap();
        assert_eq!(stopped.stopped_dbms_pid, Some(PG_PID));
    }

    #[test]
    fn a_cancelled_start_over_postgres_stops_the_server_and_the_postgres_it_raised() {
        let setup = over_postgres();
        let host = Host::default();
        host.database_exists.set(true);
        host.never_ready.set(true);
        let token = tokio_util::sync::CancellationToken::new();
        host.cancel_on_server.replace(Some(token.clone()));
        let context = ExecutionContext::cli(CommandName::StandaloneStart).with_cancellation(token);
        let error = setup
            .run_in(&context, ServiceCommand::Start, false, &host)
            .unwrap_err();
        assert!(
            matches!(error.kind(), UseCaseErrorKind::Cancelled(_)),
            "{error}"
        );
        assert!(
            host.processes.borrow().is_empty(),
            "{:?}",
            host.processes.borrow()
        );
    }

    #[test]
    fn a_server_that_does_not_stop_keeps_its_record_and_its_postgres_and_a_repeat_goes_on() {
        let setup = over_postgres();
        let host = Host::default();
        host.database_exists.set(true);
        setup.run(ServiceCommand::Start, false, &host).unwrap();
        host.ignores_soft_stop.set(true);
        host.immortal.set(true);
        let refused = setup.run(ServiceCommand::Stop, false, &host).unwrap_err();
        assert!(refused.to_string().contains(&PID.to_string()), "{refused}");
        assert!(
            setup.paths().record().exists(),
            "the record names the live server"
        );
        assert!(
            host.processes.borrow().contains_key(&PG_PID),
            "its database stays"
        );
        host.immortal.set(false);
        let stopped = setup.run(ServiceCommand::Stop, false, &host).unwrap();
        assert_eq!(stopped.stopped_pid, Some(PID));
        assert_eq!(stopped.stopped_dbms_pid, Some(PG_PID));
        assert!(host.processes.borrow().is_empty());
    }

    #[test]
    fn a_database_is_created_by_ibcmd_without_a_server_config() {
        let setup = over_postgres();
        let host = Host::default();
        super::super::run_dbms(
            &ExecutionContext::cli(CommandName::DbmsStart),
            &setup.config,
            ServiceCommand::Start,
            false,
            &Runner { host: &host },
            &host,
        )
        .map_err(|failure| failure.error)
        .unwrap();
        host.calls.borrow_mut().clear();
        let mut config = setup.config.clone();
        config.infobase.dbms.as_mut().unwrap().locale = Some("ru".to_owned());
        let message = create_base(
            &ExecutionContext::cli(CommandName::Init),
            &config,
            PathBuf::from("ibcmd.exe"),
            false,
            &Runner { host: &host },
            &host,
        )
        .unwrap();
        assert!(message.contains("demo_data"), "{message}");
        let calls = host.calls.borrow();
        let create = calls
            .iter()
            .find(|args| args.iter().any(|arg| arg == "--create-database"))
            .expect("infobase create");
        assert!(create.iter().any(|arg| arg == "--locale=ru"), "{create:?}");
        assert!(
            !calls.iter().flatten().any(|arg| arg == "init"),
            "{calls:?}"
        );
        assert!(!setup.paths().root().join("passfile").exists());
        assert!(
            !ServicePaths::new(&setup.config.work_path, "origin", "postgres")
                .passfile()
                .exists(),
            "the second copy of the password is gone"
        );
    }
}
