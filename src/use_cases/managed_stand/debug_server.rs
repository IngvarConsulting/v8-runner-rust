//! `debug-server start` и `debug-server stop`: сервер отладки 1С для отладчика.
//!
//! У файловой базы сервер отладки поднимает раннер: свой `dbgs` из установки тонкого клиента, по
//! сеансу на каждый `start`. Запись сеанса лежит в `workPath/debug-server/<сеанс>/` — сеанс
//! одноразовый и восстановимый. У базы на управляемом кластере сервер отладки поднимает сам
//! кластер (`cluster start --debug`); сеанс тогда только называет его адрес, а `stop` его не
//! трогает.
//!
//! Команды не берут замок `workPath`: `stop` отвечает и пока идёт другая команда
//! (`INV.USE-CASES.DEBUG-SERVER-COMMANDS-TAKE-NO-LOCK`). Гонку двух команд одного сеанса решает
//! запись: её `starting` при живом запускающем — отказ «запускается».

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::config::model::AppConfig;
use crate::domain::capability::TargetKind;
use crate::domain::dbms::ServiceAction;
use crate::domain::debug_server::{DebugServerOwner, DebugServerResult};
use crate::domain::launch::PlatformResolution;
use crate::platform::dbgs::{self, DebugPort};
use crate::platform::process::ProcessRunner;
use crate::platform::service_record::{
    adopt_recorded_pid, observe_service, read_json, write_json, Observed, RecordState,
    ServicePaths, ServiceRecord,
};
use crate::support::error::{AppError, CapabilityReason, EnvironmentLayer};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::interruption::interruption_before_safe_point;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

use super::{await_gone, occupant_name, occupant_of, ServiceHost, POLL};

/// Каталог сеансов под `workPath`.
const SESSIONS: &str = "debug-server";
/// Файл, куда `dbgs` пишет свой адрес.
const NOTIFY: &str = "notify";
/// Метка сеанса, который называет сервер отладки кластера: процесса раннера у него нет.
const CLUSTER_SESSION: &str = "cluster.json";
/// Сколько ждать, пока `dbgs` назовёт адрес и ответит по `rdbg`; сервер кластера поднимается
/// лениво, вместе с первым рабочим процессом (замер «Сервер отладки `dbgs`»).
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// Полоса портов сервера отладки: ниже полосы закреплённых портов стенда (20000 и выше), выше
/// портов сервера 1С по умолчанию (1540–1591) и вне динамического диапазона Windows.
const PORT_BASE: u32 = 10_000;
const PORT_WINDOW: u32 = 32;
const PORT_WINDOWS: u32 = 312;
const PORT_TRIES: u32 = 64;

/// Что попросили у `debug-server start`.
#[derive(Debug, Clone, Copy, Default)]
pub struct DebugServerStart {
    /// Этот порт, а не первый свободный из окна копии.
    pub port: Option<u16>,
    /// Процесс отладчика: с его концом `dbgs` выходит сам.
    pub owner_pid: Option<u32>,
}

/// Чей сервер отладки у базы проекта.
enum Server {
    /// Файловая база: свой `dbgs` раннера.
    Runner,
    /// База на кластере раннера: сервер отладки поднимает кластер.
    Cluster,
}

fn server_of(config: &AppConfig) -> Result<Server, UseCaseError> {
    match config.target_kind() {
        TargetKind::File => Ok(Server::Runner),
        TargetKind::Cluster
            if config
                .infobase
                .cluster
                .as_ref()
                .is_some_and(|cluster| cluster.managed.is_some()) =>
        {
            Ok(Server::Cluster)
        }
        TargetKind::Cluster => Err(refuse(
            CapabilityReason::Target,
            "the infobase is on a cluster the runner did not start: the address of its debug server is known only to its administrator; a file infobase or cluster.managed is needed",
        )),
        TargetKind::Standalone => Err(refuse(
            CapabilityReason::Target,
            "debugging an infobase on a standalone server is not supported yet: the runner starts ibsrv without a debug server",
        )),
    }
}

fn refuse(reason: CapabilityReason, message: &str) -> UseCaseError {
    UseCaseError::from(AppError::capability_for(reason, message))
}

/// Каталог сеансов этой рабочей копии.
fn sessions_root(config: &AppConfig) -> PathBuf {
    config.work_path.join(SESSIONS)
}

/// Сеанс — 32 шестнадцатеричных знака: он становится именем каталога, поэтому иное — отказ, а
/// не путь.
fn is_session(session: &str) -> bool {
    session.len() == 32 && session.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn new_session() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn session_paths(config: &AppConfig, session: &str) -> ServicePaths {
    ServicePaths::under(&sessions_root(config), session)
}

/// Окна портов копии: первое выводится из пути `workPath`, следующие идут за ним по кругу
/// полосы.
fn port_windows(work_path: &Path) -> impl Iterator<Item = (u16, u16)> {
    let digest = Sha256::digest(work_path.to_string_lossy().as_bytes());
    let seed = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]);
    (0..PORT_TRIES).map(move |attempt| {
        let window = seed.wrapping_add(attempt) % PORT_WINDOWS;
        let first = PORT_BASE + window * PORT_WINDOW;
        let first = u16::try_from(first).expect("the band stays below 20000");
        (first, first + (PORT_WINDOW as u16) - 1)
    })
}

/// Сценарий команд над записью сеанса и системой.
pub(super) struct DebugServer<'a> {
    pub context: &'a ExecutionContext,
    pub config: &'a AppConfig,
    pub runner: &'a dyn ProcessRunner,
    pub host: &'a dyn ServiceHost,
    /// Тонкий клиент, из установки которого берётся `dbgs`.
    pub locate_client: &'a dyn Fn() -> Result<PlatformResolution, UseCaseError>,
    /// Порт сервера отладки кластера этой базы, когда он отвечает.
    pub cluster_port: &'a dyn Fn() -> Result<u16, UseCaseError>,
}

impl DebugServer<'_> {
    pub fn start(
        &self,
        request: DebugServerStart,
        dry_run: bool,
    ) -> Result<DebugServerResult, UseCaseError> {
        let started = Instant::now();
        let server = server_of(self.config)?;
        if let Some(owner) = request.owner_pid {
            if self.host.facts(owner).is_none() {
                return Err(UseCaseError::new(
                    UseCaseErrorKind::Validation,
                    format!("--owner-pid {owner}: there is no such process"),
                ));
            }
        }
        let mut result = match server {
            Server::Cluster => self.cluster(request, dry_run)?,
            Server::Runner => self.runner_server(request, dry_run)?,
        };
        result.duration_ms = elapsed_ms(started);
        Ok(result)
    }

    /// Сервер отладки кластера: процесса раннера нет, сеанс — метка с портом.
    fn cluster(
        &self,
        request: DebugServerStart,
        dry_run: bool,
    ) -> Result<DebugServerResult, UseCaseError> {
        let mut result = empty(ServiceAction::Planned);
        result.owner = Some(DebugServerOwner::Cluster);
        if request.owner_pid.is_some() {
            result.warnings.push(
                "--owner-pid is ignored: the debug server of the cluster lives as long as the cluster".to_owned(),
            );
        }
        if request.port.is_some() {
            result.warnings.push(
                "--port is ignored: the debug server of the cluster listens on the port of the cluster".to_owned(),
            );
        }
        if dry_run {
            return Ok(result);
        }
        let port = (self.cluster_port)()?;
        self.sweep();
        let session = new_session();
        let paths = session_paths(self.config, &session);
        write_json(
            &paths.root().join(CLUSTER_SESSION),
            &ClusterSession { port },
        )
        .map_err(|error| super::io_failure("cannot write", paths.root(), &error))?;
        result.action = ServiceAction::Started;
        result.session = Some(session);
        result.port = Some(port);
        result.url = Some(dbgs::url(port));
        Ok(result)
    }

    fn runner_server(
        &self,
        request: DebugServerStart,
        dry_run: bool,
    ) -> Result<DebugServerResult, UseCaseError> {
        let client = (self.locate_client)()?;
        let program = dbgs::beside(&client.path);
        if !program.is_file() {
            return Err(UseCaseError::from(AppError::environment_at(
                EnvironmentLayer::Platform,
                format!(
                    "there is no {} in the installation {} of the thin client {}: the debug server comes with the client it debugs",
                    dbgs::executable_name(),
                    client.version.as_deref().unwrap_or("of unknown version"),
                    client.path.display()
                ),
            )));
        }
        let resolution = PlatformResolution {
            path: program.clone(),
            ..client
        };
        let port = self.choose_port(request.port)?;
        let mut result = empty(ServiceAction::Planned);
        result.owner = Some(DebugServerOwner::Runner);
        result.owner_pid = request.owner_pid;
        result.platform_resolution = Some(resolution);
        if let DebugPort::Exact(port) = port {
            result.port = Some(port);
        }
        if dry_run {
            return Ok(result);
        }
        let Some(machine) = self.host.machine() else {
            return Err(UseCaseError::from(AppError::environment(
                "the machine has no identifier: the runner could not tell its debug server from another process later",
            )));
        };
        self.sweep();
        let session = new_session();
        let paths = session_paths(self.config, &session);
        let notify = paths.root().join(NOTIFY);
        let starter = {
            let pid = std::process::id();
            self.host.facts(pid).map(|facts| (pid, facts.started))
        };
        let (first, explicit) = match port {
            DebugPort::Exact(port) => (port, true),
            DebugPort::Window { first, .. } => (first, false),
        };
        let mut record = ServiceRecord {
            state: RecordState::Starting,
            pid: None,
            started: None,
            image: None,
            machine: Some(machine),
            starter,
            port: first,
            explicit,
            bin: program.clone(),
            version: result
                .platform_resolution
                .as_ref()
                .and_then(|resolution| resolution.version.clone()),
            agent: None,
            location: None,
        };
        self.write_record(&paths, &record)?;
        let spawn = dbgs::start_request(&program, port, &notify, request.owner_pid, &paths.log());
        let pid = match self.host.spawn(&spawn, self.context.work()) {
            Ok(pid) => pid,
            Err(error) => {
                forget(&paths);
                return Err(UseCaseError::new(
                    UseCaseErrorKind::Platform,
                    format!("dbgs did not start: {error}"),
                ));
            }
        };
        let Some(facts) = self.host.facts(pid) else {
            forget(&paths);
            return Err(self.exited(&paths, port, "dbgs exited as soon as it started"));
        };
        record.pid = Some(pid);
        record.started = Some(facts.started);
        record.image = Some(facts.image);
        if let Err(error) = self.write_record(&paths, &record) {
            self.stop_process(&paths, &record);
            return Err(error);
        }
        let actual = self.await_ready(&paths, &record, &notify, port)?;
        record.port = actual;
        record.state = RecordState::Running;
        record.starter = None;
        if let Err(error) = self.write_record(&paths, &record) {
            self.stop_process(&paths, &record);
            return Err(error);
        }
        result.action = ServiceAction::Started;
        result.session = Some(session);
        result.port = Some(actual);
        result.url = Some(dbgs::url(actual));
        result.pid = Some(pid);
        result.log_path = Some(paths.log());
        Ok(result)
    }

    /// Явный порт занят — отказ до запуска: слушающий процесс остаётся работать. Иначе первое
    /// окно копии, где есть свободный порт; `dbgs` сам займёт свободный из него.
    fn choose_port(&self, explicit: Option<u16>) -> Result<DebugPort, UseCaseError> {
        if let Some(port) = explicit {
            if let Some(pid) = occupant_of(self.host, port) {
                return Err(port_taken(port, pid));
            }
            return Ok(DebugPort::Exact(port));
        }
        port_windows(&self.config.work_path)
            .find(|(first, last)| {
                (*first..=*last).any(|port| occupant_of(self.host, port).is_none())
            })
            .map(|(first, last)| DebugPort::Window { first, last })
            .ok_or_else(|| {
                UseCaseError::from(AppError::environment(
                    "every port window of the debug server is taken: pass a free port with --port",
                ))
            })
    }

    /// Готов — назвал адрес и ответил по `rdbg`. Вышел, отмена или срок — процесс попытки
    /// гасится, запись снимается.
    fn await_ready(
        &self,
        paths: &ServicePaths,
        record: &ServiceRecord,
        notify: &Path,
        port: DebugPort,
    ) -> Result<u16, UseCaseError> {
        let deadline = Instant::now() + self.host.patience(READY_TIMEOUT);
        let pid = record.pid.expect("a started server has a pid");
        loop {
            if let Some(error) =
                interruption_before_safe_point(self.context, "waiting for the debug server")
            {
                self.stop_process(paths, record);
                return Err(error.into());
            }
            if let Some(actual) = dbgs::notified_port(notify) {
                if self.host.rdbg_answers(actual) {
                    return Ok(actual);
                }
            }
            let gone = self
                .host
                .facts(pid)
                .is_none_or(|facts| Some(facts.started) != record.started);
            if gone {
                forget(paths);
                return Err(self.exited(
                    paths,
                    port,
                    "dbgs exited before it answered as a debug server",
                ));
            }
            if Instant::now() >= deadline {
                self.stop_process(paths, record);
                return Err(UseCaseError::new(
                    UseCaseErrorKind::TimedOut,
                    format!(
                        "dbgs did not answer as a debug server in {} s; log: {}",
                        READY_TIMEOUT.as_secs(),
                        paths.log().display()
                    ),
                ));
            }
            self.host.sleep(POLL);
        }
    }

    /// `dbgs` вышел до готовности. Явный порт заняли между проверкой и запуском — он теперь
    /// отвечает: это отказ о занятом порте, а не сбой. Текст журнала не разбирается.
    fn exited(&self, paths: &ServicePaths, port: DebugPort, why: &str) -> UseCaseError {
        if let DebugPort::Exact(port) = port {
            if let Some(occupant) = occupant_of(self.host, port) {
                return port_taken(port, occupant);
            }
        }
        not_ready(paths, why)
    }

    pub fn stop(&self, session: &str, dry_run: bool) -> Result<DebugServerResult, UseCaseError> {
        let started = Instant::now();
        if !is_session(session) {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Validation,
                format!("'{session}' is not a debug server session: it is the 32 hex digits `debug-server start` answered"),
            ));
        }
        let paths = session_paths(self.config, session);
        let mut result = self.stop_session(&paths, dry_run)?;
        result.session = Some(session.to_owned());
        result.duration_ms = elapsed_ms(started);
        Ok(result)
    }

    fn stop_session(
        &self,
        paths: &ServicePaths,
        dry_run: bool,
    ) -> Result<DebugServerResult, UseCaseError> {
        if !paths.root().is_dir() {
            let mut result = empty(ServiceAction::NothingToStop);
            result.known_session = Some(false);
            return Ok(result);
        }
        let mut result = empty(if dry_run {
            ServiceAction::Planned
        } else {
            ServiceAction::NothingToStop
        });
        result.known_session = Some(true);
        if let Some(cluster) = read_json::<ClusterSession>(&paths.root().join(CLUSTER_SESSION)) {
            result.owner = Some(DebugServerOwner::Cluster);
            result.port = Some(cluster.port);
            result.url = Some(dbgs::url(cluster.port));
            if !dry_run {
                forget(paths);
            }
            return Ok(result);
        }
        result.owner = Some(DebugServerOwner::Runner);
        let record =
            match self.observe(paths) {
                Observed::Own(record) => record,
                Observed::Starting(_) => return Err(UseCaseError::new(
                    UseCaseErrorKind::WorkspaceBusy,
                    "the debug server of this session is being started by another running command",
                )),
                Observed::Stale => {
                    result.stale_record = true;
                    if !dry_run {
                        forget(paths);
                    }
                    return Ok(result);
                }
                Observed::Nothing => {
                    if !dry_run {
                        forget(paths);
                    }
                    return Ok(result);
                }
            };
        result.pid = record.pid;
        result.port = Some(record.port);
        result.url = Some(dbgs::url(record.port));
        if dry_run {
            return Ok(result);
        }
        if !self.stop_process(paths, &record) {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Platform,
                format!(
                    "dbgs pid {} did not stop; repeat debug-server stop",
                    record.pid.unwrap_or_default()
                ),
            ));
        }
        result.action = ServiceAction::Stopped;
        result.stopped_pid = record.pid;
        result.pid = None;
        Ok(result)
    }

    fn observe(&self, paths: &ServicePaths) -> Observed {
        let machine = self.host.machine();
        let facts = |pid| self.host.facts(pid);
        observe_service(paths, machine.as_deref(), &facts, &|record| {
            adopt_recorded_pid(record, &facts, machine.as_deref())
        })
    }

    /// Убирает сеансы, чей сервер уже вышел: с концом владельца `dbgs` выходит сам, а каталог
    /// остаётся. Живые и запускаемые сеансы не трогает.
    fn sweep(&self) {
        let Ok(entries) = std::fs::read_dir(sessions_root(self.config)) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(session) = name.to_str().filter(|name| is_session(name)) else {
                continue;
            };
            let paths = session_paths(self.config, session);
            if paths.root().join(CLUSTER_SESSION).is_file() {
                continue;
            }
            if matches!(self.observe(&paths), Observed::Stale | Observed::Nothing) {
                forget(&paths);
            }
        }
    }

    /// Мягко, с подтверждением конца; затем принудительно. Сеанс снимает только подтверждённый
    /// конец (`INV.PLATFORM.A-SERVICE-STOP-IS-CONFIRMED`).
    fn stop_process(&self, paths: &ServicePaths, record: &ServiceRecord) -> bool {
        let Some(pid) = record.pid else {
            forget(paths);
            return true;
        };
        let patience = self.host.patience(STOP_TIMEOUT);
        self.host.soft_stop(self.runner, pid, self.context);
        let mut gone = await_gone(self.host, pid, record.started, patience);
        if !gone {
            self.host.hard_stop(self.runner, pid, self.context);
            gone = await_gone(self.host, pid, record.started, patience);
        }
        if gone {
            forget(paths);
        }
        gone
    }

    fn write_record(
        &self,
        paths: &ServicePaths,
        record: &ServiceRecord,
    ) -> Result<(), UseCaseError> {
        super::write_record(paths, record)
    }
}

/// Сеанс сервера отладки кластера: только его порт.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ClusterSession {
    port: u16,
}

/// Снимает сеанс целиком: запись, файл адреса и журнал.
fn forget(paths: &ServicePaths) {
    let _ = std::fs::remove_dir_all(paths.root());
}

fn empty(action: ServiceAction) -> DebugServerResult {
    DebugServerResult {
        ok: true,
        action,
        session: None,
        owner: None,
        url: None,
        port: None,
        pid: None,
        owner_pid: None,
        platform_resolution: None,
        log_path: None,
        known_session: None,
        stale_record: false,
        stopped_pid: None,
        duration_ms: 0,
        warnings: Vec::new(),
    }
}

fn port_taken(port: u16, pid: u32) -> UseCaseError {
    UseCaseError::from(AppError::environment(format!(
        "port {port} is taken by {}: the debug server did not start; pass another --port or none",
        occupant_name(pid)
    )))
}

fn not_ready(paths: &ServicePaths, why: &str) -> UseCaseError {
    UseCaseError::new(
        UseCaseErrorKind::Platform,
        format!("{why}; log: {}", paths.log().display()),
    )
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Сценарий `debug-server` над подделкой системы: запуск, готовность, остановка и уборка без
/// настоящего `dbgs`.
#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::config::model::{InfobaseClusterConfig, InfobaseConfig, ManagedServerConfig};
    use crate::domain::launch::PlatformResolutionSource;
    use crate::platform::process::{
        ProcessError, ProcessExecutionPolicy, ProcessRequest, ProcessResult, SpawnResult, WorkGiven,
    };
    use crate::platform::service::{ProcessFacts, ServiceRequest, ServiceSpawnError};
    use crate::use_cases::context::CommandName;
    use crate::use_cases::result::EnvironmentReason;

    const PID: u32 = 8001;
    const OWNER: u32 = 8002;
    const STARTED: u64 = 21;
    const MACHINE: &str = "machine";

    struct Runner;

    impl ProcessRunner for Runner {
        fn run_with_policy(
            &self,
            _request: &ProcessRequest,
            _policy: &ProcessExecutionPolicy,
        ) -> Result<ProcessResult, ProcessError> {
            unreachable!("the debug server is stopped through the host")
        }
        fn spawn(
            &self,
            _request: &ProcessRequest,
            _work: &WorkGiven,
        ) -> Result<SpawnResult, ProcessError> {
            unreachable!("the debug server is started through the host")
        }
    }

    #[derive(Default)]
    struct Host {
        processes: RefCell<HashMap<u32, ProcessFacts>>,
        listeners: RefCell<HashMap<u16, u32>>,
        spawned: Cell<u32>,
        exits_at_once: Cell<bool>,
        /// Вышел, не назвав адрес.
        exits_before_ready: Cell<bool>,
        /// Назвал адрес, но не отвечает по `rdbg`.
        never_answers: Cell<bool>,
        /// Порт, который захватывает чужой процесс между проверкой и запуском.
        stolen: Cell<Option<u16>>,
        cancel_on_spawn: RefCell<Option<tokio_util::sync::CancellationToken>>,
        soft_stops: RefCell<HashSet<u32>>,
        hard_stops: RefCell<HashSet<u32>>,
        last_args: RefCell<Vec<String>>,
    }

    impl Host {
        fn alive(&self, pid: u32) -> bool {
            self.processes.borrow().contains_key(&pid)
        }
        fn with_owner() -> Self {
            let host = Self::default();
            host.processes.borrow_mut().insert(
                OWNER,
                ProcessFacts {
                    started: 1,
                    image: "unica.exe".to_owned(),
                },
            );
            host
        }
    }

    fn arg<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
        args.iter().find_map(|arg| arg.strip_prefix(key))
    }

    impl ServiceHost for Host {
        fn supported(&self) -> bool {
            false
        }
        fn spawn(
            &self,
            request: &ServiceRequest,
            _work: &WorkGiven,
        ) -> Result<u32, ServiceSpawnError> {
            self.spawned.set(self.spawned.get() + 1);
            self.last_args.replace(request.args.clone());
            if let Some(token) = self.cancel_on_spawn.borrow().as_ref() {
                token.cancel();
            }
            if self.exits_at_once.get() {
                return Ok(PID);
            }
            if let Some(port) = self.stolen.get() {
                // Чужой занял порт раньше: `dbgs` вышел с кодом 1.
                self.listeners.borrow_mut().insert(port, 4242);
                return Ok(PID);
            }
            self.processes.borrow_mut().insert(
                PID,
                ProcessFacts {
                    started: STARTED,
                    image: dbgs::executable_name().to_owned(),
                },
            );
            if self.exits_before_ready.get() {
                self.processes.borrow_mut().remove(&PID);
                return Ok(PID);
            }
            let port = match (
                arg(&request.args, "--port="),
                arg(&request.args, "--portRange="),
            ) {
                (Some(port), _) => port.parse().unwrap(),
                (None, Some(range)) => {
                    let mut bounds = range.split(':').map(|bound| bound.parse::<u16>().unwrap());
                    let (first, last) = (bounds.next().unwrap(), bounds.next().unwrap());
                    (first..=last)
                        .find(|port| !self.listeners.borrow().contains_key(port))
                        .unwrap()
                }
                (None, None) => panic!("no port in {:?}", request.args),
            };
            self.listeners.borrow_mut().insert(port, PID);
            let notify = PathBuf::from(arg(&request.args, "--notify=").unwrap());
            std::fs::create_dir_all(notify.parent().unwrap()).unwrap();
            std::fs::write(&notify, format!("127.0.0.1:{port}")).unwrap();
            Ok(PID)
        }
        fn facts(&self, pid: u32) -> Option<ProcessFacts> {
            if pid == std::process::id() {
                return Some(ProcessFacts {
                    started: 5,
                    image: "v8-runner.exe".to_owned(),
                });
            }
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
            self.processes.borrow_mut().remove(&pid);
            self.listeners.borrow_mut().retain(|_, held| *held != pid);
        }
        fn hard_stop(&self, _runner: &dyn ProcessRunner, pid: u32, _context: &ExecutionContext) {
            self.hard_stops.borrow_mut().insert(pid);
            self.processes.borrow_mut().remove(&pid);
            self.listeners.borrow_mut().retain(|_, held| *held != pid);
        }
        fn sleep(&self, _duration: Duration) {}
        fn patience(&self, _default: Duration) -> Duration {
            Duration::ZERO
        }
        fn rdbg_answers(&self, port: u16) -> bool {
            !self.never_answers.get() && self.listeners.borrow().get(&port) == Some(&PID)
        }
    }

    struct Setup {
        root: tempfile::TempDir,
        config: AppConfig,
        client: PathBuf,
    }

    fn setup() -> Setup {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("1cv8").join("8.3.27.2342").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let client = bin.join("1cv8c");
        std::fs::write(&client, "").unwrap();
        std::fs::write(bin.join(dbgs::executable_name()), "").unwrap();
        let infobase = InfobaseConfig::file(format!("File={}", root.path().join("ib").display()));
        let config = AppConfig {
            local_path: root.path().join(".local"),
            base_path: root.path().to_path_buf(),
            work_path: root.path().join("work"),
            format: crate::config::model::SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase,
            infobases: Default::default(),
            infobase_name: Some("origin".to_owned()),
            working_copy: None,
            project_file: crate::config::model::default_project_file(),
            source_sets: Vec::new(),
            tools: Default::default(),
            mcp: Default::default(),
            tests: Default::default(),
        };
        Setup {
            root,
            config,
            client,
        }
    }

    impl Setup {
        fn on_managed_cluster(mut self) -> Self {
            self.config.infobase = InfobaseConfig::file("Srvr=127.0.0.1:24001;Ref=demo".to_owned());
            self.config.infobase.cluster = Some(InfobaseClusterConfig {
                managed: Some(ManagedServerConfig {
                    name: Some("demo".to_owned()),
                    db_path: None,
                }),
                ..Default::default()
            });
            self
        }

        fn server<'a>(
            &'a self,
            context: &'a ExecutionContext,
            host: &'a Host,
            locate: &'a dyn Fn() -> Result<PlatformResolution, UseCaseError>,
            cluster_port: &'a dyn Fn() -> Result<u16, UseCaseError>,
        ) -> DebugServer<'a> {
            DebugServer {
                context,
                config: &self.config,
                runner: &Runner,
                host,
                locate_client: locate,
                cluster_port,
            }
        }

        fn resolution(&self) -> PlatformResolution {
            PlatformResolution {
                path: self.client.clone(),
                version: Some("8.3.27.2342".to_owned()),
                source: PlatformResolutionSource::Explicit,
                installation_root: self
                    .client
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .to_path_buf(),
            }
        }

        fn start_in(
            &self,
            context: &ExecutionContext,
            request: DebugServerStart,
            host: &Host,
        ) -> Result<DebugServerResult, UseCaseError> {
            let locate = || Ok(self.resolution());
            let cluster_port = || Ok(24018);
            self.server(context, host, &locate, &cluster_port)
                .start(request, false)
        }

        fn start(
            &self,
            request: DebugServerStart,
            host: &Host,
        ) -> Result<DebugServerResult, UseCaseError> {
            self.start_in(&context(), request, host)
        }

        fn stop(&self, session: &str, host: &Host) -> Result<DebugServerResult, UseCaseError> {
            let locate = || Ok(self.resolution());
            let cluster_port = || unreachable!("stop does not ask the cluster");
            self.server(&context(), host, &locate, &cluster_port)
                .stop(session, false)
        }

        fn sessions(&self) -> Vec<String> {
            std::fs::read_dir(sessions_root(&self.config))
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default()
        }
    }

    fn context() -> ExecutionContext {
        ExecutionContext::cli(CommandName::DebugServerStart)
    }

    /// Приёмка 1: успех — адрес, который ответил по `rdbg`, из установки клиента.
    #[test]
    fn start_answers_with_the_address_that_answered_as_a_debug_server() {
        let setup = setup();
        let host = Host::with_owner();
        let started = setup
            .start(
                DebugServerStart {
                    port: None,
                    owner_pid: Some(OWNER),
                },
                &host,
            )
            .unwrap();
        assert_eq!(started.action, ServiceAction::Started);
        assert_eq!(started.owner, Some(DebugServerOwner::Runner));
        let port = started.port.unwrap();
        assert_eq!(started.url.as_deref(), Some(dbgs::url(port).as_str()));
        assert!((10_000..20_000).contains(&port), "{port}");
        assert_eq!(started.pid, Some(PID));
        let resolution = started.platform_resolution.unwrap();
        assert_eq!(
            resolution.path,
            setup.client.with_file_name(dbgs::executable_name())
        );
        assert_eq!(resolution.version.as_deref(), Some("8.3.27.2342"));
        let args = host.last_args.borrow();
        assert!(args.contains(&"--addr=127.0.0.1".to_owned()), "{args:?}");
        assert!(args.contains(&format!("--ownerPID={OWNER}")), "{args:?}");
        // Сервер пережил команду: процесс жив, сеанс записан.
        assert!(host.alive(PID));
        assert_eq!(setup.sessions(), [started.session.unwrap()]);
    }

    /// Приёмка 1: нет `dbgs` в установке клиента — отказ окружения с версией и местом, без
    /// процесса и сеанса.
    #[test]
    fn a_missing_debug_server_is_an_environment_refusal_naming_the_installation() {
        let setup = setup();
        std::fs::remove_file(setup.client.with_file_name(dbgs::executable_name())).unwrap();
        let host = Host::default();
        let error = setup.start(DebugServerStart::default(), &host).unwrap_err();
        assert_eq!(
            error.kind(),
            UseCaseErrorKind::Environment(EnvironmentReason::Unavailable)
        );
        let message = error.to_string();
        assert!(message.contains("8.3.27.2342"), "{message}");
        assert!(
            message.contains(&setup.client.display().to_string()),
            "{message}"
        );
        assert_eq!(host.spawned.get(), 0);
        assert!(setup.sessions().is_empty());
    }

    /// Приёмка 1: занятый явный порт — отказ до запуска; слушающий процесс остаётся.
    #[test]
    fn a_taken_explicit_port_is_refused_before_anything_starts() {
        let setup = setup();
        let host = Host::default();
        host.listeners.borrow_mut().insert(1560, 777);
        let error = setup
            .start(
                DebugServerStart {
                    port: Some(1560),
                    owner_pid: None,
                },
                &host,
            )
            .unwrap_err();
        assert_eq!(
            error.kind(),
            UseCaseErrorKind::Environment(EnvironmentReason::Unavailable)
        );
        assert!(error.to_string().contains("process 777"), "{error}");
        assert_eq!(host.spawned.get(), 0);
        assert_eq!(host.listening_pid(1560), Some(777));
        assert!(setup.sessions().is_empty());
    }

    /// Порт заняли между проверкой и запуском: `dbgs` вышел — отказ о занятом порте, а не сбой.
    #[test]
    fn a_port_taken_during_the_start_is_named_as_taken() {
        let setup = setup();
        let host = Host::default();
        host.stolen.set(Some(1570));
        let error = setup
            .start(
                DebugServerStart {
                    port: Some(1570),
                    owner_pid: None,
                },
                &host,
            )
            .unwrap_err();
        assert!(error.to_string().contains("port 1570 is taken"), "{error}");
        assert!(setup.sessions().is_empty());
    }

    /// Приёмка 1: процесс вышел до готовности — сбой платформы со ссылкой на журнал, без сеанса.
    #[test]
    fn an_exit_before_readiness_leaves_no_session() {
        for exit in [Host::default(), Host::default()].iter().enumerate() {
            let (index, host) = exit;
            if index == 0 {
                host.exits_at_once.set(true);
            } else {
                host.exits_before_ready.set(true);
            }
            let setup = setup();
            let error = setup.start(DebugServerStart::default(), host).unwrap_err();
            assert_eq!(error.kind(), UseCaseErrorKind::Platform, "{error}");
            assert!(setup.sessions().is_empty());
            assert!(!host.alive(PID));
        }
    }

    /// Приёмка 1: срок готовности вышел — процесс попытки снят, сеанса нет.
    #[test]
    fn a_readiness_timeout_stops_the_attempt() {
        let setup = setup();
        let host = Host::default();
        host.never_answers.set(true);
        let error = setup.start(DebugServerStart::default(), &host).unwrap_err();
        assert_eq!(error.kind(), UseCaseErrorKind::TimedOut);
        assert!(host.soft_stops.borrow().contains(&PID));
        assert!(!host.alive(PID));
        assert!(setup.sessions().is_empty());
    }

    /// Приёмка 1: отмена во время ожидания — процесс попытки снят, сеанса нет
    /// (`INV.USE-CASES.OPERATIONS-DECLARE-AN-INTERRUPTION-CLASS`: ожидание прерывается в точке,
    /// остановка идёт под политикой очистки).
    #[test]
    fn a_cancellation_stops_the_attempt() {
        let setup = setup();
        let host = Host::default();
        host.never_answers.set(true);
        let token = tokio_util::sync::CancellationToken::new();
        host.cancel_on_spawn.replace(Some(token.clone()));
        let context = context().with_cancellation(token);
        let error = setup
            .start_in(&context, DebugServerStart::default(), &host)
            .unwrap_err();
        assert!(
            matches!(error.kind(), UseCaseErrorKind::Cancelled(_)),
            "{error}"
        );
        assert!(!host.alive(PID));
        assert!(setup.sessions().is_empty());
    }

    /// Мёртвый `--owner-pid` — отказ до запуска: иначе сервер вышел бы сразу после ответа.
    #[test]
    fn a_dead_owner_is_refused_before_anything_starts() {
        let setup = setup();
        let host = Host::default();
        let error = setup
            .start(
                DebugServerStart {
                    port: None,
                    owner_pid: Some(OWNER),
                },
                &host,
            )
            .unwrap_err();
        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert_eq!(host.spawned.get(), 0);
    }

    /// Приёмка 2: сервер жив после `start`; `stop` гасит его; повторный — безопасен.
    #[test]
    fn stop_stops_the_server_of_its_session_and_repeats_safely() {
        let setup = setup();
        let host = Host::default();
        let session = setup
            .start(DebugServerStart::default(), &host)
            .unwrap()
            .session
            .unwrap();
        assert!(host.alive(PID));
        let stopped = setup.stop(&session, &host).unwrap();
        assert_eq!(stopped.action, ServiceAction::Stopped);
        assert_eq!(stopped.stopped_pid, Some(PID));
        assert_eq!(stopped.known_session, Some(true));
        assert!(!host.alive(PID));
        assert!(setup.sessions().is_empty());
        let again = setup.stop(&session, &host).unwrap();
        assert_eq!(again.action, ServiceAction::NothingToStop);
        assert_eq!(again.known_session, Some(false));
    }

    /// Приёмка 2: неверный сеанс не трогает чужой процесс.
    #[test]
    fn a_wrong_session_leaves_the_server_alone() {
        let setup = setup();
        let host = Host::default();
        setup.start(DebugServerStart::default(), &host).unwrap();
        let other = setup
            .stop("0123456789abcdef0123456789abcdef", &host)
            .unwrap();
        assert_eq!(other.action, ServiceAction::NothingToStop);
        assert_eq!(other.known_session, Some(false));
        assert!(host.alive(PID));
        let error = setup.stop("../../etc", &host).unwrap_err();
        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert!(host.alive(PID));
    }

    /// Приёмка 2: устаревший сеанс — pid занят другим процессом — не трогает этот процесс.
    #[test]
    fn a_stale_session_leaves_a_process_that_took_its_pid_alone() {
        let setup = setup();
        let host = Host::default();
        let session = setup
            .start(DebugServerStart::default(), &host)
            .unwrap()
            .session
            .unwrap();
        // `dbgs` вышел, его pid занял другой процесс.
        host.processes.borrow_mut().insert(
            PID,
            ProcessFacts {
                started: STARTED + 1,
                image: "notepad.exe".to_owned(),
            },
        );
        let stopped = setup.stop(&session, &host).unwrap();
        assert_eq!(stopped.action, ServiceAction::NothingToStop);
        assert!(stopped.stale_record);
        assert!(host.soft_stops.borrow().is_empty());
        assert!(host.hard_stops.borrow().is_empty());
        assert!(host.alive(PID));
        assert!(setup.sessions().is_empty());
    }

    /// Сеансы, чей сервер вышел сам (с концом владельца), убирает следующий `start`.
    #[test]
    fn start_sweeps_sessions_whose_server_is_gone() {
        let setup = setup();
        let host = Host::default();
        let first = setup
            .start(DebugServerStart::default(), &host)
            .unwrap()
            .session
            .unwrap();
        host.processes.borrow_mut().remove(&PID);
        host.listeners.borrow_mut().clear();
        let second = setup
            .start(DebugServerStart::default(), &host)
            .unwrap()
            .session
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(setup.sessions(), [second]);
    }

    /// Предварительный просмотр ничего не запускает и не пишет.
    #[test]
    fn a_preview_starts_and_writes_nothing() {
        let setup = setup();
        let host = Host::default();
        let locate = || Ok(setup.resolution());
        let cluster_port = || Ok(24018);
        let context = context();
        let planned = setup
            .server(&context, &host, &locate, &cluster_port)
            .start(DebugServerStart::default(), true)
            .unwrap();
        assert_eq!(planned.action, ServiceAction::Planned);
        assert!(planned.session.is_none());
        assert_eq!(host.spawned.get(), 0);
        assert!(!sessions_root(&setup.config).exists());
    }

    /// База на кластере раннера: адрес сервера отладки кластера, процесса раннера нет, `stop` его
    /// не трогает.
    #[test]
    fn a_cluster_infobase_gets_the_debug_server_of_the_cluster() {
        let setup = setup().on_managed_cluster();
        let host = Host::default();
        let started = setup.start(DebugServerStart::default(), &host).unwrap();
        assert_eq!(started.owner, Some(DebugServerOwner::Cluster));
        assert_eq!(started.url.as_deref(), Some("http://127.0.0.1:24018"));
        assert_eq!(started.pid, None);
        assert_eq!(host.spawned.get(), 0);
        let stopped = setup.stop(&started.session.unwrap(), &host).unwrap();
        assert_eq!(stopped.action, ServiceAction::NothingToStop);
        assert_eq!(stopped.owner, Some(DebugServerOwner::Cluster));
        assert!(host.soft_stops.borrow().is_empty());
        assert!(setup.sessions().is_empty());
    }

    /// База на чужом кластере или на автономном сервере — отказ «не для этой цели».
    #[test]
    fn a_foreign_server_infobase_is_refused() {
        let mut setup = setup();
        setup.config.infobase = InfobaseConfig::file("Srvr=srv;Ref=demo".to_owned());
        let host = Host::default();
        let error = setup.start(DebugServerStart::default(), &host).unwrap_err();
        assert!(
            matches!(error.kind(), UseCaseErrorKind::Capability(_)),
            "{error}"
        );
        assert_eq!(host.spawned.get(), 0);
    }

    /// Окна портов не задевают полосу закреплённых портов стенда (20000 и выше), порты сервера 1С
    /// по умолчанию (1540–1591) и динамический диапазон.
    #[test]
    fn port_windows_stay_in_their_own_band() {
        for root in ["/w/a", "C:/work", "/home/u/проект/work"] {
            for (first, last) in port_windows(Path::new(root)) {
                assert!(first >= 10_000 && last < 20_000, "{first}..{last}");
                assert_eq!(last - first + 1, PORT_WINDOW as u16);
            }
        }
        let _ = setup().root;
    }
}
