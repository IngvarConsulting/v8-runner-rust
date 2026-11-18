//! `debug-server start` и `debug-server stop`: сервер отладки 1С для отладчика.
//!
//! У файловой базы сервер отладки поднимает раннер: свой `dbgs` из установки тонкого клиента, по
//! сеансу на каждый `start`. Запись сеанса лежит в `workPath/debug-server/<сеанс>/` — сеанс
//! одноразовый и восстановимый. У базы на управляемом кластере сервер отладки поднимает сам
//! кластер (`cluster start --debug`); сеанс тогда только называет его адрес, а `stop` его не
//! трогает.
//!
//! Команды не берут замок `workPath`: `stop` отвечает и пока идёт другая команда
//! (`INV.USE-CASES.DEBUG-SERVER-COMMANDS-TAKE-NO-LOCK`). Гонку команд одного сеанса решает
//! запись: её `starting` при живом запускающем — отказ «запускается».

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use sha2::{Digest, Sha256};

use crate::config::model::AppConfig;
use crate::domain::capability::TargetKind;
use crate::domain::dbms::ServiceAction;
use crate::domain::debug_server::{DebugServerOwner, DebugServerResult, DebugSession};
use crate::domain::launch::PlatformResolution;
use crate::platform::dbgs::{self, DebugPort};
use crate::platform::process::ProcessRunner;
use crate::platform::service_record::{
    adopt_recorded_pid, observe_service, read_json, write_json, Observed, RecordState,
    ServicePaths, ServiceRecord,
};
use crate::support::error::{AppError, CapabilityReason, EnvironmentLayer};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

use super::{
    await_rdbg, occupant_name, occupant_of, spawn_debugger, stop_confirmed, write_record,
    DebuggerLaunch, DebuggerSpawn, RdbgWait, ServiceHost, SpawnedDebugger, DEBUG_READY_TIMEOUT,
};

/// Каталог сеансов под `workPath`.
const SESSIONS: &str = "debug-server";
/// Файл, куда `dbgs` пишет свой адрес.
const NOTIFY: &str = "notify";
/// Метка сеанса, который называет сервер отладки стенда: процесса раннера у него нет.
const STAND_SESSION: &str = "session.json";
/// Метка сеанса сервера кластера прежнего вида: читается как `owner: cluster`.
const CLUSTER_SESSION: &str = "cluster.json";
/// Имя файловой базы для отладчика (замер «Сервер отладки `dbgs`»).
const FILE_ALIAS: &str = "DefAlias";
/// Журнал неудачной попытки: каталог сеанса снят, а причина отказа остаётся читаемой.
const FAILED_LOG_PREFIX: &str = "failed-";
/// Каталог без записи моложе этого — его, возможно, прямо сейчас заводит другой `start`.
const FRESH: Duration = Duration::from_secs(60);
/// Сколько хранить журналы неудачных попыток и метки сеансов кластера, которые никто не
/// остановил.
const KEPT: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Полоса портов сервера отладки: ниже полосы закреплённых портов стенда (20000 и выше), выше
/// портов сервера 1С по умолчанию (1540–1591) и вне динамического диапазона Windows.
const PORT_BASE: u16 = 10_000;
const PORT_WINDOW: u16 = 32;
const PORT_WINDOWS: u16 = 312;
const PORT_TRIES: u16 = 64;
const _: () = assert!(PORT_BASE as u32 + PORT_WINDOWS as u32 * PORT_WINDOW as u32 <= 20_000);

/// Что попросили у `debug-server start`.
#[derive(Debug, Clone, Copy, Default)]
pub struct DebugServerStart {
    /// Этот порт, а не первый свободный из окна копии.
    pub port: Option<u16>,
    /// Процесс отладчика: с его концом `dbgs` выходит сам.
    pub owner_pid: Option<u32>,
}

/// Чей сервер отладки стенда называет сеанс: процесса раннера у него нет.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StandOwner {
    Cluster,
    Standalone,
}

impl StandOwner {
    const fn owner(self) -> DebugServerOwner {
        match self {
            Self::Cluster => DebugServerOwner::Cluster,
            Self::Standalone => DebugServerOwner::Standalone,
        }
    }

    const fn whose(self) -> &'static str {
        match self {
            Self::Cluster => "the cluster",
            Self::Standalone => "the standalone server",
        }
    }
}

/// Сервер отладки стенда: порт и имя базы для отладчика, если оно замерено.
pub(super) struct StandDebugServer {
    pub port: u16,
    pub infobase_alias: Option<String>,
}

/// Чей сервер отладки у базы проекта.
enum Server {
    /// Файловая база: свой `dbgs` раннера.
    Runner,
    /// База на кластере раннера: сервер отладки поднимает кластер.
    Cluster,
    /// База на автономном сервере раннера: его `dbgs` поднимает `standalone start --debug`.
    Standalone,
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
            "the infobase is on a cluster the runner did not start: the address of its debug server is known only to its administrator; a file infobase or cluster.managed is needed",
        )),
        TargetKind::Standalone
            if config
                .infobase
                .standalone
                .as_ref()
                .is_some_and(|standalone| standalone.managed.is_some()) =>
        {
            Ok(Server::Standalone)
        }
        TargetKind::Standalone => Err(refuse(
            "the infobase is on a standalone server the runner did not start: the address of its debug server is known only to its administrator; a file infobase, cluster.managed or standalone.managed is needed",
        )),
    }
}

fn refuse(message: &str) -> UseCaseError {
    UseCaseError::from(AppError::capability_for(CapabilityReason::Target, message))
}

/// Каталог сеансов этой рабочей копии.
fn sessions_root(config: &AppConfig) -> PathBuf {
    config.work_path.join(SESSIONS)
}

fn session_paths(config: &AppConfig, session: DebugSession) -> ServicePaths {
    ServicePaths::under(&sessions_root(config), &session.to_string())
}

/// Окна портов копии: первое выводится из пути `workPath`, следующие идут за ним по кругу
/// полосы.
fn port_windows(work_path: &Path) -> impl Iterator<Item = (u16, u16)> {
    let digest = Sha256::digest(work_path.to_string_lossy().as_bytes());
    let seed = u16::from_le_bytes([digest[0], digest[1]]);
    (0..PORT_TRIES).map(move |attempt| {
        let window = seed.wrapping_add(attempt) % PORT_WINDOWS;
        let first = PORT_BASE + window * PORT_WINDOW;
        (first, first + PORT_WINDOW - 1)
    })
}

/// Сколько прошло с последней правки файла или каталога; `None` — система не сказала.
fn age(path: &Path) -> Option<Duration> {
    let modified = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()?;
    SystemTime::now().duration_since(modified).ok()
}

/// Сценарий команд над записью сеанса и системой.
pub(super) struct DebugServer<'a> {
    pub context: &'a ExecutionContext,
    pub config: &'a AppConfig,
    pub runner: &'a dyn ProcessRunner,
    pub host: &'a dyn ServiceHost,
    /// Тонкий клиент, из установки которого берётся `dbgs`.
    pub locate_client: &'a dyn Fn() -> Result<PlatformResolution, UseCaseError>,
    /// Сервер отладки стенда этой базы, когда он отвечает: кластера или автономного сервера.
    pub stand_server: &'a dyn Fn(StandOwner) -> Result<StandDebugServer, UseCaseError>,
}

impl DebugServer<'_> {
    pub fn start(
        &self,
        request: DebugServerStart,
        dry_run: bool,
    ) -> Result<DebugServerResult, UseCaseError> {
        let started = Instant::now();
        let mut result = match server_of(self.config)? {
            Server::Cluster => self.stand(StandOwner::Cluster, request, dry_run)?,
            Server::Standalone => self.stand(StandOwner::Standalone, request, dry_run)?,
            Server::Runner => self.runner_server(request, dry_run)?,
        };
        result.duration_ms = crate::support::time::elapsed_ms(started);
        Ok(result)
    }

    /// Владелец должен быть живым процессом и не самим раннером: иначе `dbgs` вышел бы сразу
    /// после ответа.
    fn check_owner(&self, owner: Option<u32>) -> Result<(), UseCaseError> {
        let Some(owner) = owner else {
            return Ok(());
        };
        let why = if owner == std::process::id() {
            "it is this runner, which ends as soon as it answers"
        } else if self.host.facts(owner).is_none() {
            "there is no such process"
        } else {
            return Ok(());
        };
        Err(UseCaseError::new(
            UseCaseErrorKind::Validation,
            format!("--owner-pid {owner}: {why}"),
        ))
    }

    /// Сервер отладки стенда — кластера или автономного сервера: процесса раннера нет, сеанс —
    /// метка с владельцем.
    fn stand(
        &self,
        stand: StandOwner,
        request: DebugServerStart,
        dry_run: bool,
    ) -> Result<DebugServerResult, UseCaseError> {
        let owner = stand.owner();
        let mut result = empty(ServiceAction::Planned);
        result.owner = Some(owner);
        let whose = stand.whose();
        if request.owner_pid.is_some() {
            result.warnings.push(format!(
                "--owner-pid is ignored: the debug server of {whose} lives as long as {whose}"
            ));
        }
        if request.port.is_some() {
            result.warnings.push(format!(
                "--port is ignored: the debug server of {whose} listens on the port of {whose}"
            ));
        }
        if dry_run {
            return Ok(result);
        }
        let StandDebugServer {
            port,
            infobase_alias: alias,
        } = (self.stand_server)(stand)?;
        self.sweep();
        let session = DebugSession::new();
        let paths = session_paths(self.config, session);
        write_json(&paths.root().join(STAND_SESSION), &SessionMarker { owner })
            .map_err(|error| super::io_failure("cannot write", paths.root(), &error))?;
        result.action = ServiceAction::Started;
        result.session = Some(session);
        result.port = Some(port);
        result.url = Some(dbgs::url(port));
        result.infobase_alias = alias;
        Ok(result)
    }

    fn runner_server(
        &self,
        request: DebugServerStart,
        dry_run: bool,
    ) -> Result<DebugServerResult, UseCaseError> {
        self.check_owner(request.owner_pid)?;
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
        if let DebugPort::Exact(port) = port {
            result.port = Some(port);
        }
        let version = resolution.version.clone();
        result.platform_resolution = Some(resolution);
        if dry_run {
            return Ok(result);
        }
        let Some(machine) = self.host.machine() else {
            return Err(UseCaseError::from(AppError::environment(
                "the machine has no identifier: the runner could not tell its debug server from another process later",
            )));
        };
        self.sweep();
        let session = DebugSession::new();
        let paths = session_paths(self.config, session);
        let notify = paths.root().join(NOTIFY);
        let launch = DebuggerLaunch {
            paths: &paths,
            program: &program,
            port,
            notify: &notify,
            owner: request.owner_pid,
            machine,
            version,
        };
        let SpawnedDebugger { mut record, pid } =
            match spawn_debugger(self.host, self.runner, self.context, launch) {
                DebuggerSpawn::Spawned(spawned) => *spawned,
                DebuggerSpawn::ExitedAtOnce => {
                    let log = self.abandon(&paths);
                    return Err(self.exited(&log, port, "dbgs exited as soon as it started"));
                }
                DebuggerSpawn::Failed(error) => {
                    forget(&paths);
                    return Err(error);
                }
            };
        let actual = self.await_ready(&paths, &record, pid, session, &notify, port)?;
        record.port = actual;
        record.state = RecordState::Running;
        record.starter = None;
        if let Err(error) = write_record(&paths, &record) {
            return Err(self.give_up(&paths, &record, session, error));
        }
        result.action = ServiceAction::Started;
        result.session = Some(session);
        result.port = Some(actual);
        result.url = Some(dbgs::url(actual));
        result.pid = Some(pid);
        result.log_path = Some(paths.log());
        result.infobase_alias = Some(FILE_ALIAS.to_owned());
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

    /// Готов — назвал адрес, по нему ответил `rdbg`, и порт слушает этот процесс, а не чужой.
    /// Вышел, отмена или срок — процесс попытки гасится, запись снимается, журнал остаётся.
    fn await_ready(
        &self,
        paths: &ServicePaths,
        record: &ServiceRecord,
        pid: u32,
        session: DebugSession,
        notify: &Path,
        port: DebugPort,
    ) -> Result<u16, UseCaseError> {
        // Порт из файла адреса, если его слушает этот процесс, а не чужой (на Unix слушающего
        // система не называет).
        let named = || {
            dbgs::notified_port(notify).filter(|actual| {
                self.host
                    .listening_pid(*actual)
                    .is_none_or(|listener| listener == pid)
            })
        };
        let alive = || {
            self.host
                .facts(pid)
                .is_some_and(|facts| Some(facts.started) == record.started)
        };
        match await_rdbg(
            self.host,
            self.context,
            "waiting for the debug server",
            &named,
            &alive,
        ) {
            RdbgWait::Ready(actual) => Ok(actual),
            RdbgWait::Gone => {
                let log = self.abandon(paths);
                Err(self.exited(
                    &log,
                    port,
                    "dbgs exited before it answered as a debug server",
                ))
            }
            RdbgWait::TimedOut => {
                let timeout = UseCaseError::new(
                    UseCaseErrorKind::TimedOut,
                    format!(
                        "dbgs did not answer as a debug server in {} s",
                        DEBUG_READY_TIMEOUT.as_secs()
                    ),
                );
                Err(self.give_up(paths, record, session, timeout))
            }
            RdbgWait::Interrupted(error) => Err(self.give_up(paths, record, session, error)),
        }
    }

    /// Попытка не удалась после запуска: процесс гасится, журнал остаётся под своим именем.
    /// Не погас — ошибка говорит это и называет сеанс: его запись остаётся, и `stop` или
    /// следующий `start` снимут процесс.
    fn give_up(
        &self,
        paths: &ServicePaths,
        record: &ServiceRecord,
        session: DebugSession,
        error: UseCaseError,
    ) -> UseCaseError {
        let log = paths.log();
        let kept = self.keep_log(paths);
        if self.stop_process(paths, record) {
            return error.followed_by(format!("log: {}", kept.unwrap_or(log).display()));
        }
        error.followed_by(format!(
            "dbgs pid {} did not stop: run `debug-server stop {session}`; log: {}",
            record.pid.unwrap_or_default(),
            log.display()
        ))
    }

    /// Снимает сеанс попытки, оставив журнал; возвращает, где журнал теперь.
    fn abandon(&self, paths: &ServicePaths) -> PathBuf {
        let log = self.keep_log(paths).unwrap_or_else(|| paths.log());
        forget(paths);
        log
    }

    /// Копия журнала попытки рядом с каталогами сеансов: каталог сеанса снимается, а причину
    /// отказа должно быть где прочесть.
    fn keep_log(&self, paths: &ServicePaths) -> Option<PathBuf> {
        let name = paths.root().file_name()?.to_string_lossy().into_owned();
        let kept = sessions_root(self.config).join(format!("{FAILED_LOG_PREFIX}{name}.log"));
        std::fs::copy(paths.log(), &kept).ok().map(|_| kept)
    }

    /// `dbgs` вышел до готовности. Явный порт заняли между проверкой и запуском — он теперь
    /// отвечает: это отказ о занятом порте, а не сбой. Текст журнала не разбирается.
    fn exited(&self, log: &Path, port: DebugPort, why: &str) -> UseCaseError {
        if let DebugPort::Exact(port) = port {
            if let Some(occupant) = occupant_of(self.host, port) {
                return port_taken(port, occupant);
            }
        }
        UseCaseError::new(
            UseCaseErrorKind::Platform,
            format!("{why}; log: {}", log.display()),
        )
    }

    pub fn stop(
        &self,
        session: DebugSession,
        dry_run: bool,
    ) -> Result<DebugServerResult, UseCaseError> {
        let started = Instant::now();
        let paths = session_paths(self.config, session);
        let mut result = self.stop_session(&paths, dry_run)?;
        result.session = Some(session);
        result.duration_ms = crate::support::time::elapsed_ms(started);
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
        // Адрес сервера стенда сеанс не хранит: порты стенда могли смениться, а живой адрес
        // называет `cluster status` или `standalone status`.
        if let Some(owner) = stand_session(paths) {
            result.owner = Some(owner);
            if !dry_run {
                forget(paths);
            }
            return Ok(result);
        }
        result.owner = Some(DebugServerOwner::Runner);
        let (record, pid) =
            match self.observe(paths) {
                Observed::Own(record) => match record.pid {
                    Some(pid) => (record, pid),
                    None => {
                        if !dry_run {
                            forget(paths);
                        }
                        return Ok(result);
                    }
                },
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
        result.pid = Some(pid);
        result.port = Some(record.port);
        result.url = Some(dbgs::url(record.port));
        if dry_run {
            return Ok(result);
        }
        if !self.stop_process(paths, &record) {
            return Err(UseCaseError::new(
                UseCaseErrorKind::Platform,
                format!("dbgs pid {pid} did not stop; repeat debug-server stop"),
            ));
        }
        result.action = ServiceAction::Stopped;
        result.stopped_pid = Some(pid);
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

    /// Уборка перед новым сеансом:
    /// - сеанс, чей сервер вышел сам (с концом владельца), — каталог снимается;
    /// - сеанс оборванного `start` (запись `starting`, запускавший раннер мёртв) — его `dbgs`
    ///   никто не получил и не остановит, поэтому он гасится здесь;
    /// - каталог без записи моложе [`FRESH`] не трогается: его, возможно, заводит другой
    ///   `start`;
    /// - метки сеансов кластера и журналы неудачных попыток старше [`KEPT`] снимаются.
    fn sweep(&self) {
        let root = sessions_root(self.config);
        let Ok(entries) = std::fs::read_dir(&root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(FAILED_LOG_PREFIX) {
                if age(&path).is_some_and(|age| age > KEPT) {
                    let _ = std::fs::remove_file(&path);
                }
                continue;
            }
            let Ok(session) = name.parse::<DebugSession>() else {
                continue;
            };
            let paths = session_paths(self.config, session);
            if stand_session(&paths).is_some() {
                if age(paths.root()).is_some_and(|age| age > KEPT) {
                    forget(&paths);
                }
                continue;
            }
            let interrupted = read_json::<ServiceRecord>(&paths.record())
                .is_some_and(|record| record.state == RecordState::Starting);
            match self.observe(&paths) {
                Observed::Own(record) if interrupted => {
                    self.stop_process(&paths, &record);
                }
                Observed::Stale => forget(&paths),
                Observed::Nothing if age(paths.root()).is_some_and(|age| age > FRESH) => {
                    forget(&paths);
                }
                Observed::Own(_) | Observed::Starting(_) | Observed::Nothing => {}
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
        let gone = stop_confirmed(
            self.host,
            self.runner,
            self.context,
            pid,
            record.started,
            Duration::ZERO,
        );
        if gone {
            forget(paths);
        }
        gone
    }
}

/// Метка сеанса сервера отладки стенда: только чей он. Порта в ней нет: он принадлежит
/// закреплённому стенда и меняется вместе с ним.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SessionMarker {
    owner: DebugServerOwner,
}

/// Чей сеанс стенда лежит в каталоге; метка кластера прежнего вида — сеанс кластера.
fn stand_session(paths: &ServicePaths) -> Option<DebugServerOwner> {
    read_json::<SessionMarker>(&paths.root().join(STAND_SESSION))
        .map(|marker| marker.owner)
        .or_else(|| {
            paths
                .root()
                .join(CLUSTER_SESSION)
                .is_file()
                .then_some(DebugServerOwner::Cluster)
        })
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
        infobase_alias: None,
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
        /// Порт из файла адреса слушает чужой процесс.
        foreign_listener: Cell<bool>,
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
            std::fs::create_dir_all(request.log_path.parent().unwrap()).unwrap();
            std::fs::write(&request.log_path, "dbgs log").unwrap();
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
            let listener = if self.foreign_listener.get() {
                4343
            } else {
                PID
            };
            self.listeners.borrow_mut().insert(port, listener);
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
        fn soft_stop(
            &self,
            _runner: &dyn ProcessRunner,
            pid: u32,
            _started: Option<u64>,
            _context: &ExecutionContext,
        ) {
            self.soft_stops.borrow_mut().insert(pid);
            self.processes.borrow_mut().remove(&pid);
            self.listeners.borrow_mut().retain(|_, held| *held != pid);
        }
        fn hard_stop(
            &self,
            _runner: &dyn ProcessRunner,
            pid: u32,
            _started: Option<u64>,
            _context: &ExecutionContext,
        ) {
            self.hard_stops.borrow_mut().insert(pid);
            self.processes.borrow_mut().remove(&pid);
            self.listeners.borrow_mut().retain(|_, held| *held != pid);
        }
        fn sleep(&self, _duration: Duration) {}
        fn patience(&self, _default: Duration) -> Duration {
            Duration::ZERO
        }
        fn rdbg_answers(&self, port: u16) -> bool {
            !self.never_answers.get() && self.listeners.borrow().contains_key(&port)
        }
    }

    struct Setup {
        _root: tempfile::TempDir,
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
            _root: root,
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

        fn on_managed_standalone(mut self) -> Self {
            let db_path = self.config.base_path.join("ib");
            self.config.infobase = InfobaseConfig::file(String::new());
            self.config.infobase.standalone = Some(crate::config::model::StandaloneConfig {
                managed: Some(ManagedServerConfig {
                    name: Some("demo".to_owned()),
                    db_path: Some(db_path),
                }),
                gate: None,
                host_fingerprint: None,
                exchange: None,
            });
            self
        }

        fn server<'a>(
            &'a self,
            context: &'a ExecutionContext,
            host: &'a Host,
            locate: &'a dyn Fn() -> Result<PlatformResolution, UseCaseError>,
            stand: &'a dyn Fn(StandOwner) -> Result<StandDebugServer, UseCaseError>,
        ) -> DebugServer<'a> {
            DebugServer {
                context,
                config: &self.config,
                runner: &Runner,
                host,
                locate_client: locate,
                stand_server: stand,
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
            self.server(context, host, &locate, &fake_stand)
                .start(request, false)
        }

        fn start(
            &self,
            request: DebugServerStart,
            host: &Host,
        ) -> Result<DebugServerResult, UseCaseError> {
            self.start_in(&context(), request, host)
        }

        fn stop(
            &self,
            session: DebugSession,
            host: &Host,
        ) -> Result<DebugServerResult, UseCaseError> {
            let locate = || Ok(self.resolution());
            self.server(&context(), host, &locate, &fake_stand)
                .stop(session, false)
        }

        /// Каталоги сеансов, без журналов неудачных попыток.
        fn sessions(&self) -> Vec<String> {
            std::fs::read_dir(sessions_root(&self.config))
                .map(|entries| {
                    entries
                        .flatten()
                        .filter(|entry| entry.path().is_dir())
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default()
        }
    }

    /// Сервер отладки автономного сервера подделки: порт блока и имя базы в сервере.
    fn fake_stand(stand: StandOwner) -> Result<StandDebugServer, UseCaseError> {
        Ok(match stand {
            StandOwner::Cluster => StandDebugServer {
                port: 24018,
                infobase_alias: None,
            },
            StandOwner::Standalone => StandDebugServer {
                port: 24019,
                infobase_alias: Some("demo".to_owned()),
            },
        })
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
        assert_eq!(started.infobase_alias.as_deref(), Some("DefAlias"));
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
        assert_eq!(setup.sessions(), [started.session.unwrap().to_string()]);
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

    /// Журнал, который называет отказ, существует: каталог сеанса снят, а журнал остался.
    fn assert_names_a_kept_log(error: &UseCaseError) {
        let message = error.to_string();
        let log = message
            .rsplit("log: ")
            .next()
            .map(str::trim)
            .unwrap_or_default();
        assert!(Path::new(log).is_file(), "{message}");
    }

    fn assert_an_exit_leaves_no_session(host: &Host) {
        let setup = setup();
        let error = setup.start(DebugServerStart::default(), host).unwrap_err();
        assert_eq!(error.kind(), UseCaseErrorKind::Platform, "{error}");
        assert_names_a_kept_log(&error);
        assert!(setup.sessions().is_empty());
        assert!(!host.alive(PID));
    }

    /// Приёмка 1: процесс вышел сразу после запуска — сбой платформы с журналом, без сеанса.
    #[test]
    fn an_exit_at_once_leaves_no_session() {
        let host = Host::default();
        host.exits_at_once.set(true);
        assert_an_exit_leaves_no_session(&host);
    }

    /// Приёмка 1: процесс вышел до готовности — сбой платформы с журналом, без сеанса.
    #[test]
    fn an_exit_before_readiness_leaves_no_session() {
        let host = Host::default();
        host.exits_before_ready.set(true);
        assert_an_exit_leaves_no_session(&host);
    }

    /// Порт из файла адреса слушает чужой процесс: это не готовность этого `dbgs`.
    #[test]
    fn an_answer_from_another_process_is_not_readiness() {
        let setup = setup();
        let host = Host::default();
        host.foreign_listener.set(true);
        let error = setup.start(DebugServerStart::default(), &host).unwrap_err();
        assert_eq!(error.kind(), UseCaseErrorKind::TimedOut, "{error}");
        assert!(!host.alive(PID));
    }

    /// Сеанс оборванного `start` — запускавший раннер убит, его `dbgs` никто не получил —
    /// гасит следующий `start`.
    #[test]
    fn start_stops_the_server_of_an_interrupted_start() {
        let setup = setup();
        let host = Host::default();
        let orphan = DebugSession::new();
        let paths = session_paths(&setup.config, orphan);
        host.processes.borrow_mut().insert(
            7777,
            ProcessFacts {
                started: 3,
                image: dbgs::executable_name().to_owned(),
            },
        );
        write_json(
            &paths.record(),
            &ServiceRecord {
                state: RecordState::Starting,
                pid: Some(7777),
                started: Some(3),
                image: Some(dbgs::executable_name().to_owned()),
                machine: Some(MACHINE.to_owned()),
                // Раннер, которого больше нет.
                starter: Some((6666, 1)),
                port: 10_000,
                explicit: false,
                bin: PathBuf::from("dbgs"),
                version: None,
                agent: None,
                location: None,
            },
        )
        .unwrap();
        setup.start(DebugServerStart::default(), &host).unwrap();
        assert!(host.soft_stops.borrow().contains(&7777));
        assert!(!host.alive(7777));
        assert!(!setup.sessions().contains(&orphan.to_string()));
    }

    /// Владелец — сам раннер: `dbgs` вышел бы сразу после ответа. Отказ до запуска.
    #[test]
    fn the_runner_itself_is_no_owner() {
        let setup = setup();
        let host = Host::default();
        let error = setup
            .start(
                DebugServerStart {
                    port: None,
                    owner_pid: Some(std::process::id()),
                },
                &host,
            )
            .unwrap_err();
        assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        assert_eq!(host.spawned.get(), 0);
    }

    /// Приёмка 1: срок готовности вышел — процесс попытки снят, сеанса нет.
    #[test]
    fn a_readiness_timeout_stops_the_attempt() {
        let setup = setup();
        let host = Host::default();
        host.never_answers.set(true);
        let error = setup.start(DebugServerStart::default(), &host).unwrap_err();
        assert_eq!(error.kind(), UseCaseErrorKind::TimedOut);
        assert_names_a_kept_log(&error);
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
        let stopped = setup.stop(session, &host).unwrap();
        assert_eq!(stopped.action, ServiceAction::Stopped);
        assert_eq!(stopped.stopped_pid, Some(PID));
        assert_eq!(stopped.known_session, Some(true));
        assert!(!host.alive(PID));
        assert!(setup.sessions().is_empty());
        let again = setup.stop(session, &host).unwrap();
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
            .stop("0123456789abcdef0123456789abcdef".parse().unwrap(), &host)
            .unwrap();
        assert_eq!(other.action, ServiceAction::NothingToStop);
        assert_eq!(other.known_session, Some(false));
        assert!(host.alive(PID));
    }

    /// Сеанс — имя каталога: иное, чем 32 шестнадцатеричных знака, не разбирается; верхний
    /// регистр — тот же сеанс, что нижний.
    #[test]
    fn a_session_is_32_hex_digits_in_any_case() {
        assert!("../../etc".parse::<DebugSession>().is_err());
        assert!("0123456789abcdef0123456789abcde"
            .parse::<DebugSession>()
            .is_err());
        let lower: DebugSession = "0123456789abcdef0123456789abcdef".parse().unwrap();
        let upper: DebugSession = "0123456789ABCDEF0123456789ABCDEF".parse().unwrap();
        assert_eq!(lower, upper);
        assert_eq!(upper.to_string(), "0123456789abcdef0123456789abcdef");
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
        let stopped = setup.stop(session, &host).unwrap();
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
        assert_eq!(setup.sessions(), [second.to_string()]);
    }

    /// Предварительный просмотр ничего не запускает и не пишет.
    #[test]
    fn a_preview_starts_and_writes_nothing() {
        let setup = setup();
        let host = Host::default();
        let locate = || Ok(setup.resolution());

        let context = context();
        let planned = setup
            .server(&context, &host, &locate, &fake_stand)
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
        let stopped = setup.stop(started.session.unwrap(), &host).unwrap();
        assert_eq!(stopped.action, ServiceAction::NothingToStop);
        assert_eq!(stopped.owner, Some(DebugServerOwner::Cluster));
        assert_eq!(stopped.url, None, "the session keeps no stale address");
        assert!(host.soft_stops.borrow().is_empty());
        assert!(setup.sessions().is_empty());
    }

    /// База на автономном сервере раннера: адрес его `dbgs` и имя базы в сервере, процесса
    /// раннера нет, `stop` его не трогает.
    #[test]
    fn a_standalone_infobase_gets_the_debug_server_of_the_server() {
        let setup = setup().on_managed_standalone();
        let host = Host::default();
        let started = setup.start(DebugServerStart::default(), &host).unwrap();
        assert_eq!(started.owner, Some(DebugServerOwner::Standalone));
        assert_eq!(started.url.as_deref(), Some("http://127.0.0.1:24019"));
        assert_eq!(started.infobase_alias.as_deref(), Some("demo"));
        assert_eq!(host.spawned.get(), 0);
        let stopped = setup.stop(started.session.unwrap(), &host).unwrap();
        assert_eq!(stopped.owner, Some(DebugServerOwner::Standalone));
        assert_eq!(stopped.action, ServiceAction::NothingToStop);
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
                assert_eq!(last - first + 1, PORT_WINDOW);
            }
        }
    }
}
