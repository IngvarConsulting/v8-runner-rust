//! Кластер, которого поднимает раннер: `cluster start|status|stop|restart`. Агент `ragent` с его
//! `rmngr` и `rphost` и сервер администрирования `ras` — две службы со своими записями; адрес цели
//! остальные команды берут из записи агента и закреплённых портов, их читает загрузчик настроек.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{
    await_gone, being_started, environment, forget_record, io_failure, occupant_of,
    other_cluster_over, owned_by_a_cluster, port_offsets, validation, write_record, Database,
    ServiceHost, Stand,
};
use crate::config::model::{AppConfig, InfobaseDbmsConfig};
use crate::domain::cluster::ClusterResult;
use crate::domain::dbms::{ServiceAction, ServiceState};
use crate::platform::process::ProcessRunner;
use crate::platform::rac::{self, Registration};
use crate::platform::ragent::{self, ras_address, ClusterPorts};
use crate::platform::secrets::mask_text;
use crate::platform::service::{first_with_keys, ProcessRow, ServiceSpawnError};
use crate::platform::service_record::{
    adopt_recorded_pid, observe_service, read_json, record_names_own_process, write_json,
    ClusterPins, Observed, PreviousRegistry, RecordState, ServicePaths, ServiceRecord,
    StandService,
};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::interruption::interruption_before_safe_point;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind, UseCaseFailure, UseCaseResult};

const READY_TIMEOUT: Duration = Duration::from_secs(120);
/// Мягкая остановка `ragent` 8.3.24 идёт до 10 с (замер #534).
const SOFT_STOP_TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(200);
const AGENT_IMAGE: &str = "ragent.exe";
/// Процессы агента, которые переживают его конец (замер #534).
const REMNANT_IMAGES: [&str; 2] = ["rmngr.exe", "rphost.exe"];
/// Каталоги реестров под `managed/cluster/`.
const REGISTRY_PREFIX: &str = "registry-";

/// Команда кластера. Ключ `--new-regport` есть только у запуска.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterCommand {
    Start { new_regport: bool },
    Status,
    Stop,
    Restart,
}

/// Что сняла остановка агента.
struct AgentStop {
    pid: Option<u32>,
    remnants: Vec<u32>,
    stale: bool,
}

/// Почему закреплённые порты не годятся для запуска.
enum Busy {
    /// Чужой `ragent` работает над реестром этой копии: выхода нет, реестр живой.
    ForeignAgent(UseCaseError),
    /// Порт держит чужой процесс: выход — новый реестр.
    Port(UseCaseError),
    /// Порт занят, а перечня процессов нет: кто его держит, не сказать.
    Unlisted(UseCaseError),
}

impl Busy {
    fn into_error(self) -> UseCaseError {
        match self {
            Self::ForeignAgent(error) | Self::Port(error) | Self::Unlisted(error) => error,
        }
    }
}

/// То, что сценарий узнаёт о цели из настроек.
struct Declared {
    infobase: String,
    name: String,
    /// PostgreSQL раннера поднимается шагом кластера. Остальное о БД нужно только тому, кто
    /// регистрирует базу, и разбирается там (`Cluster::database`).
    dbms_managed: bool,
}

fn declared(config: &AppConfig) -> Result<Declared, UseCaseError> {
    let infobase = config.infobase_name.clone().ok_or_else(|| {
        validation("cluster takes a declared infobase: a connection string has neither a record nor a managed section")
    })?;
    let managed = config
        .infobase
        .cluster
        .as_ref()
        .and_then(|cluster| cluster.managed.as_ref())
        .ok_or_else(|| {
            validation("infobase.cluster.managed is not declared for this infobase: the cluster commands start only a cluster the runner owns")
        })?;
    let name = managed.reference(&infobase);
    let dbms_managed = config
        .infobase
        .dbms
        .as_ref()
        .is_some_and(|dbms| dbms.managed.is_some());
    Ok(Declared {
        infobase,
        name,
        dbms_managed,
    })
}

/// Утилита рядом с `ragent` записи: `ras` и `rac` лежат в том же `bin` платформы (замер #469).
fn sibling(ragent: &Path, name: &str) -> PathBuf {
    ragent
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// Версия платформы — имя каталога установки над `bin`.
fn platform_version(ragent: &Path) -> Option<String> {
    ragent
        .parent()?
        .parent()?
        .file_name()?
        .to_str()
        .map(str::to_owned)
}

struct Cluster<'a> {
    config: &'a AppConfig,
    context: &'a ExecutionContext,
    runner: &'a dyn ProcessRunner,
    host: &'a dyn ServiceHost,
    locate: &'a dyn Fn() -> Result<PathBuf, UseCaseError>,
    declared: Declared,
    agent: ServicePaths,
    ras: ServicePaths,
    started_at: Instant,
}

impl Cluster<'_> {
    fn observe(&self, paths: &ServicePaths) -> Observed {
        let host = self.host;
        let machine = host.machine();
        observe_service(
            paths,
            machine.as_deref(),
            &|pid| host.facts(pid),
            &|record| adopt_recorded_pid(record, &|pid| host.facts(pid), machine.as_deref()),
        )
    }

    fn pins(&self) -> Option<ClusterPins> {
        read_json(&self.agent.pins())
    }

    fn write_pins(&self, pins: &ClusterPins) -> Result<(), UseCaseError> {
        write_json(&self.agent.pins(), pins)
            .map_err(|error| io_failure("cannot write", &self.agent.pins(), &error))
    }

    fn registry(&self, pins: &ClusterPins) -> PathBuf {
        self.agent.root().join(&pins.registry)
    }

    /// БД базы кластера, как её объявили; без `dbms` кластер поднимается, но базы в нём не создать.
    fn database(&self) -> Result<Option<Database>, UseCaseError> {
        self.config
            .infobase
            .dbms
            .as_ref()
            .map(Database::declared)
            .transpose()
    }

    /// PostgreSQL раннера как шаг кластера.
    fn stand(&self) -> Option<Stand<'_>> {
        self.declared
            .dbms_managed
            .then(|| Stand::new(self.config, self.context, self.runner, self.host).ok())
            .flatten()
            .map(Stand::server_step)
    }

    fn result(&self, state: ServiceState, action: ServiceAction) -> ClusterResult {
        ClusterResult {
            ok: true,
            infobase: self.declared.infobase.clone(),
            state,
            action,
            pid: None,
            ras_pid: None,
            version: None,
            connection: None,
            agent: None,
            ras: None,
            worker_range: None,
            registry_dir: None,
            log_path: None,
            ready: false,
            registered: false,
            stale_record: false,
            remnant_pids: Vec::new(),
            occupant_pid: None,
            stopped_pid: None,
            stopped_ras_pid: None,
            stopped_remnant_pids: Vec::new(),
            stopped_dbms_pid: None,
            previous_regport: None,
            duration_ms: u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
        }
    }

    fn describe(&self, mut result: ClusterResult, agent: Option<&ServiceRecord>) -> ClusterResult {
        if let Some(pins) = self.pins() {
            result.connection = Some(pins.ports.connection(&self.declared.name));
            result.agent = Some(pins.ports.agent_address());
            result.worker_range = Some(pins.ports.worker_range());
            result.registry_dir = Some(self.registry(&pins));
            result.registered = pins.registered;
        }
        if let Some(agent) = agent {
            result.pid = agent.pid;
            result.version.clone_from(&agent.version);
        }
        if let Some(ras) = self.own_ras() {
            result.ras_pid = ras.pid;
            result.ras = Some(ras_address(ras.port));
        }
        result.log_path = Some(self.agent.log());
        result
    }

    fn own_ras(&self) -> Option<ServiceRecord> {
        match self.observe(&self.ras) {
            Observed::Own(record) => Some(record),
            _ => None,
        }
    }

    fn probe_policy(&self) -> crate::platform::process::ProcessExecutionPolicy {
        self.context.process_policy(
            InterruptionSafetyClass::GracefulThenKill,
            Some(PROBE_TIMEOUT),
        )
    }

    /// `rac cluster list`: идентификатор кластера, если он отвечает.
    fn probe(&self, ragent: &Path, ras_port: u16) -> Option<String> {
        let request = rac::cluster_list(&sibling(ragent, "rac"), &ras_address(ras_port));
        let answer = self
            .runner
            .run_with_policy(&request, &self.probe_policy())
            .ok()?;
        if answer.outcome().is_err() {
            return None;
        }
        rac::first_cluster(&answer.stdout)
    }

    /// Остатки упавшего агента: всё совпало по `INV.PLATFORM.A-CLUSTER-LEAVES-NO-REMNANTS`.
    /// `None` — перечня процессов нет, и остатки не опознать.
    fn remnants(&self, record: &ServiceRecord) -> Option<Vec<(u32, u64)>> {
        let (Some(agent), Some(since)) = (record.pid, record.started) else {
            return Some(Vec::new());
        };
        if self.host.machine().as_deref() != record.machine.as_deref() {
            return Some(Vec::new());
        }
        // pid агента может достаться новому процессу: его дети начались после него и не наши.
        let reused = match self.host.facts(agent) {
            Some(facts) if facts.started == since => return Some(Vec::new()),
            Some(facts) => Some(facts.started),
            None => None,
        };
        let rows = self.host.processes(&REMNANT_IMAGES)?;
        // Потомки агента: после его конца `rmngr` поднимает новые `rphost`, и их родитель — уже
        // остаток, а не агент (живой прогон #538).
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        let mut parents = VecDeque::from([agent]);
        while let Some(parent) = parents.pop_front() {
            for row in rows.iter().filter(|row| row.parent == parent) {
                if !seen.insert(row.pid) {
                    continue;
                }
                let Some(facts) = self.host.facts(row.pid) else {
                    continue;
                };
                let before_reuse =
                    parent != agent || reused.is_none_or(|reused| facts.started < reused);
                if REMNANT_IMAGES.contains(&facts.image.as_str())
                    && facts.started >= since
                    && before_reuse
                {
                    found.push((row.pid, facts.started));
                    parents.push_back(row.pid);
                }
            }
        }
        Some(found)
    }

    /// pid остатков для отчёта; без перечня процессов — ни одного.
    fn remnant_pids(&self) -> Vec<u32> {
        self.agent
            .own_record()
            .and_then(|record| self.remnants(&record))
            .map(|remnants| remnants.into_iter().map(|(pid, _)| pid).collect())
            .unwrap_or_default()
    }

    /// Снимает остатки по их pid: `taskkill /T` по мёртвому родителю их не находит (замер #534).
    /// Ошибка — какие-то остались или их не опознать: тогда запись агента нужна, чтобы узнать их
    /// в следующий раз.
    fn clear_remnants(&self, record: &ServiceRecord) -> Result<Vec<u32>, UseCaseError> {
        let remnants = self.remnants(record).ok_or_else(|| {
            platform("the process list did not answer, so the remnants of the fallen ragent cannot be told apart; repeat the command")
        })?;
        let (cleared, left): (Vec<_>, Vec<_>) = remnants.into_iter().partition(|(pid, started)| {
            self.host.hard_stop(self.runner, *pid, self.context);
            await_gone(
                self.host,
                *pid,
                Some(*started),
                self.host.patience(SOFT_STOP_TIMEOUT),
            )
        });
        if !left.is_empty() {
            let left: Vec<String> = left.iter().map(|(pid, _)| pid.to_string()).collect();
            return Err(platform(format!(
                "the remnants of the fallen ragent did not stop (pid {}); repeat cluster stop",
                left.join(", ")
            )));
        }
        Ok(cleared.into_iter().map(|(pid, _)| pid).collect())
    }

    /// Мягко, с проверкой конца процесса; затем принудительно (`INV.PLATFORM.A-SERVICE-STOP-IS-CONFIRMED`).
    fn stop_process(&self, record: &ServiceRecord) -> bool {
        let Some(pid) = record.pid else {
            return true;
        };
        self.host.soft_stop(self.runner, pid, self.context);
        let patience = self.host.patience(SOFT_STOP_TIMEOUT);
        if await_gone(self.host, pid, record.started, patience) {
            return true;
        }
        self.host.hard_stop(self.runner, pid, self.context);
        await_gone(self.host, pid, record.started, patience)
    }

    /// `ras` переживает конец агента, поэтому гаснет своим шагом (`INV.PLATFORM.RAS-STOPS-APART-FROM-RAGENT`).
    fn stop_ras(&self) -> Result<Option<u32>, UseCaseError> {
        match self.observe(&self.ras) {
            Observed::Own(record) => {
                if !self.stop_process(&record) {
                    return Err(UseCaseError::new(
                        UseCaseErrorKind::Platform,
                        format!(
                            "ras pid {} did not stop; repeat cluster stop",
                            record.pid.unwrap_or_default()
                        ),
                    ));
                }
                forget_record(&self.ras);
                Ok(record.pid)
            }
            Observed::Starting(_) => Err(being_started("the cluster", &self.declared.infobase)),
            Observed::Stale => {
                forget_record(&self.ras);
                Ok(None)
            }
            Observed::Nothing => Ok(None),
        }
    }

    /// Агент и его остатки. Запись агента живёт, пока по ней не сняты все остатки: только по ней их
    /// и узнать.
    fn stop_agent(&self) -> Result<AgentStop, UseCaseError> {
        match self.observe(&self.agent) {
            Observed::Own(record) => {
                if !self.stop_process(&record) {
                    return Err(platform(format!(
                        "ragent pid {} did not stop; repeat cluster stop",
                        record.pid.unwrap_or_default()
                    )));
                }
                let remnants = self.clear_remnants(&record)?;
                forget_record(&self.agent);
                Ok(AgentStop {
                    pid: record.pid,
                    remnants,
                    stale: false,
                })
            }
            Observed::Starting(_) => Err(being_started("the cluster", &self.declared.infobase)),
            Observed::Stale => {
                let remnants = self.clear_stale()?;
                Ok(AgentStop {
                    pid: None,
                    remnants,
                    stale: true,
                })
            }
            Observed::Nothing => Ok(AgentStop {
                pid: None,
                remnants: Vec::new(),
                stale: false,
            }),
        }
    }

    /// Устаревшая запись агента: снять остатки по ней, затем её саму.
    fn clear_stale(&self) -> Result<Vec<u32>, UseCaseError> {
        let remnants = match self.agent.own_record() {
            Some(record) => self.clear_remnants(&record)?,
            None => Vec::new(),
        };
        forget_record(&self.agent);
        Ok(remnants)
    }

    /// Свой процесс не поднялся до конца: снять его, записи и остатки. Что не снялось, остаётся в
    /// записях для следующей команды, а ошибка это называет.
    fn abandon(&self, agent: &ServiceRecord) -> Result<(), UseCaseError> {
        if let Some(ras) = self.own_ras() {
            if !self.stop_process(&ras) {
                return Err(platform(format!(
                    "ras pid {} did not stop; repeat cluster stop",
                    ras.pid.unwrap_or_default()
                )));
            }
            forget_record(&self.ras);
        }
        if !self.stop_process(agent) {
            return Err(platform(format!(
                "ragent pid {} did not stop; repeat cluster stop",
                agent.pid.unwrap_or_default()
            )));
        }
        self.clear_remnants(agent)?;
        forget_record(&self.agent);
        Ok(())
    }

    /// Неудача: снять поднятое здесь; что не снялось, дописывается к исходной ошибке.
    fn give_up(&self, agent: &ServiceRecord, error: UseCaseError) -> UseCaseError {
        match self.abandon(agent) {
            Ok(()) => error,
            Err(left) => error.followed_by(format!("not everything was taken down: {left}")),
        }
    }

    fn stop_cluster(&self, dry_run: bool) -> Result<ClusterResult, UseCaseError> {
        if dry_run {
            return Ok(self.plan());
        }
        let since = std::time::SystemTime::now();
        let ras = self.stop_ras()?;
        let agent = self.stop_agent()?;
        self.forget_stop_dumps(since);
        if agent.pid.is_none() && ras.is_none() && agent.remnants.is_empty() {
            let mut result = self.result(ServiceState::NotRunning, ServiceAction::NothingToStop);
            result.stale_record = agent.stale;
            self.name_occupant(&mut result);
            return Ok(result);
        }
        let mut result = self.result(ServiceState::NotRunning, ServiceAction::Stopped);
        result.stopped_pid = agent.pid;
        result.stopped_ras_pid = ras;
        result.stopped_remnant_pids = agent.remnants;
        result.stale_record = agent.stale;
        Ok(result)
    }

    /// Дампы, которые `ragent`, `rmngr` и `rphost` пишут в рабочий каталог, когда их останавливают
    /// (живой прогон #538). Дампы настоящих падений появились раньше остановки и остаются.
    fn forget_stop_dumps(&self, since: std::time::SystemTime) {
        let Ok(entries) = std::fs::read_dir(self.agent.root()) else {
            return;
        };
        // Время файла на Windows грубее часов: свежий дамп может оказаться чуть «старше» начала.
        let since = since - Duration::from_secs(1);
        for entry in entries.flatten() {
            let path = entry.path();
            let fresh = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .is_ok_and(|modified| modified >= since);
            if fresh
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("mdmp"))
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    fn stop(&self, dry_run: bool) -> Result<ClusterResult, UseCaseError> {
        let mut result = self.stop_cluster(dry_run)?;
        // Кластер гаснет первым: СУБД нужна ему до конца. Пока над ней работает свой автономный
        // сервер этой базы, она нужна ему.
        if !dry_run && !self.standalone_running() {
            if let Some(stand) = self.stand() {
                result.stopped_dbms_pid = stand.stop_if_step()?;
            }
        }
        Ok(result)
    }

    /// Чужой процесс на закреплённых портах; остатки своего упавшего агента чужими не считаются.
    fn name_occupant(&self, result: &mut ClusterResult) {
        let Some(pins) = self.pins() else {
            return;
        };
        let remnants = self.remnant_pids();
        if let Some(pid) = pins
            .ports
            .all()
            .filter_map(|port| occupant_of(self.host, port))
            .find(|pid| !remnants.contains(pid))
        {
            result.state = ServiceState::Foreign;
            result.occupant_pid = (pid != 0).then_some(pid);
        }
    }

    fn status(&self) -> ClusterResult {
        match self.observe(&self.agent) {
            Observed::Own(record) => {
                let mut result =
                    self.result(ServiceState::StartedByThisCopy, ServiceAction::Reported);
                result.ready = self
                    .own_ras()
                    .and_then(|ras| self.probe(&record.bin, ras.port))
                    .is_some();
                self.describe(result, Some(&record))
            }
            Observed::Starting(record) => {
                let result = self.result(ServiceState::Starting, ServiceAction::Reported);
                self.describe(result, Some(&record))
            }
            observed => {
                let mut result = self.result(ServiceState::NotRunning, ServiceAction::Reported);
                if matches!(observed, Observed::Stale) {
                    result.stale_record = true;
                    result.remnant_pids = self.remnant_pids();
                }
                self.name_occupant(&mut result);
                if let Some(pins) = self.pins() {
                    result.connection = Some(pins.ports.connection(&self.declared.name));
                    result.registered = pins.registered;
                }
                result
            }
        }
    }

    fn plan(&self) -> ClusterResult {
        let mut result = self.result(ServiceState::NotRunning, ServiceAction::Planned);
        let mut agent = None;
        match self.observe(&self.agent) {
            Observed::Own(record) => {
                result.state = ServiceState::StartedByThisCopy;
                agent = Some(record);
            }
            Observed::Starting(_) => result.state = ServiceState::Starting,
            Observed::Stale => result.stale_record = true,
            Observed::Nothing => {}
        }
        self.describe(result, agent.as_ref())
    }

    /// Свой автономный сервер той же базы: одна БД — один сервер.
    fn standalone_running(&self) -> bool {
        record_names_own_process(
            &self.agent.sibling(StandService::Standalone),
            self.host.machine().as_deref(),
            &|pid| self.host.facts(pid),
        )
    }

    fn start(&self, dry_run: bool, new_regport: bool) -> Result<ClusterResult, UseCaseError> {
        if dry_run {
            return Ok(self.plan());
        }
        if !self.host.supported() {
            return Err(environment(
                "managed services run on Windows only: on this system the runner starts no cluster",
            ));
        }
        let Some(machine) = self.host.machine() else {
            return Err(environment(
                "this system gives no machine id: the runner could not tell its cluster from a foreign one",
            ));
        };
        if self.standalone_running() {
            return Err(validation(format!(
                "the standalone server of '{}' is running over this infobase, and one server owns a database: run `standalone stop` first",
                self.declared.infobase
            )));
        }
        // Неполная секция `dbms` с чужой не сравнивается: регистрация базы её всё равно отвергнет.
        if let Some(owner) = self
            .database()
            .ok()
            .flatten()
            .and_then(|database| other_cluster_over(self.config, self.host, &database))
        {
            return Err(owned_by_a_cluster(&owner));
        }
        self.undo_interrupted_move()?;
        let mut stale = false;
        match self.observe(&self.agent) {
            Observed::Own(record) => {
                if new_regport {
                    return Err(validation(format!(
                        "the cluster of '{}' is running: --new-regport starts a new registry for a stopped cluster; run `cluster stop` first",
                        self.declared.infobase
                    )));
                }
                // Запись упавшего раннера становится рабочей: её читают остальные команды.
                write_record(&self.agent, &record)?;
                let ras = match self.own_ras() {
                    Some(ras) => ras,
                    None => {
                        forget_record(&self.ras);
                        let pins = self.pins().ok_or_else(|| {
                            validation("the ports of the running cluster are not pinned: run `cluster restart`")
                        })?;
                        self.raise_ras(&record, pins.ports, &machine)?
                    }
                };
                let mut result = self.result(
                    ServiceState::StartedByThisCopy,
                    ServiceAction::AlreadyRunning,
                );
                result.ready = self.probe(&record.bin, ras.port).is_some();
                return Ok(self.describe(result, Some(&record)));
            }
            Observed::Starting(_) => {
                return Err(being_started("the cluster", &self.declared.infobase))
            }
            Observed::Stale => {
                self.clear_stale()?;
                stale = true;
            }
            Observed::Nothing => {}
        }
        // Свой `ras` без своего агента больше никому не нужен.
        self.stop_ras()?;
        // Отказы, которым СУБД не нужна, — до её запуска.
        let pins = self.pins();
        match (&pins, new_regport) {
            (Some(pins), false) => {
                if let Some(busy) = self.busy(pins) {
                    return Err(busy.into_error());
                }
            }
            (Some(pins), true) => match self.busy(pins) {
                None => {
                    return Err(validation(format!(
                        "the ports of the cluster of '{}' are free: `cluster start` starts it on its registry; --new-regport is for ports taken by another process",
                        self.declared.infobase
                    )))
                }
                Some(Busy::Port(_)) => {}
                Some(refusal) => return Err(refusal.into_error()),
            },
            (None, true) => {
                return Err(validation(format!(
                    "the cluster of '{}' has no registry yet: `cluster start` makes it",
                    self.declared.infobase
                )))
            }
            (None, false) => {}
        }
        // Шаг кластера: поднятая здесь СУБД гаснет при любой неудаче дальше.
        let stand = self.stand();
        let raised = match &stand {
            Some(stand) => stand.start(false)?.action == ServiceAction::Started,
            None => false,
        };
        let outcome = match pins {
            Some(old) if new_regport => self.move_registry(&machine, stand.as_ref(), old),
            pins => self.start_on_pins(&machine, stand.as_ref(), pins),
        };
        match outcome {
            Ok(mut result) => {
                result.stale_record |= stale;
                Ok(result)
            }
            Err(error) => match stand.as_ref().filter(|_| raised).map(Stand::stop_if_step) {
                Some(Err(stop)) => Err(error.followed_by(format!(
                    "the PostgreSQL this command started did not stop: {stop}"
                ))),
                _ => Err(error),
            },
        }
    }

    /// Прерванный `--new-regport`: закреплённое ещё помнит прежний реестр. Поднятое по новому
    /// снимается, новый реестр удаляется, прежние порты возвращаются — и запуск идёт как обычно.
    fn undo_interrupted_move(&self) -> Result<(), UseCaseError> {
        let Some(pins) = self.pins() else {
            return Ok(());
        };
        let Some(previous) = pins.previous.clone() else {
            return Ok(());
        };
        self.stop_ras()?;
        self.stop_agent()?;
        let new_registry = self.registry(&pins);
        self.write_pins(&ClusterPins {
            ports: previous.ports,
            registry: previous.registry,
            registered: previous.registered,
            previous: None,
        })?;
        if self.registry_is_free(&new_registry) {
            let _ = std::fs::remove_dir_all(&new_registry);
        }
        Ok(())
    }

    /// Порт PostgreSQL раннера, чтобы блок кластера его не взял.
    fn dbms_port(stand: Option<&Stand<'_>>) -> Option<u16> {
        stand.and_then(|stand| stand.pinned_port())
    }

    /// Свободный блок портов на первый запуск или новый реестр.
    fn choose_ports(
        &self,
        stand: Option<&Stand<'_>>,
        except: Option<ClusterPorts>,
    ) -> Result<ClusterPorts, UseCaseError> {
        let dbms = Self::dbms_port(stand);
        port_offsets(self.agent.root())
            .map(ClusterPorts::from_first)
            .find(|ports| {
                except.is_none_or(|old| !old.all().any(|port| ports.all().any(|own| own == port)))
                    && ports
                        .all()
                        .all(|port| Some(port) != dbms && occupant_of(self.host, port).is_none())
            })
            .ok_or_else(|| environment("no free ports for the cluster"))
    }

    /// Порт `ras` — вне блока кластера и порта СУБД, свободный сейчас.
    fn choose_ras_port(&self, ports: ClusterPorts) -> Result<u16, UseCaseError> {
        let dbms = Self::dbms_port(self.stand().as_ref());
        port_offsets(self.ras.root())
            .find(|port| {
                !ports.all().any(|own| own == *port)
                    && Some(*port) != dbms
                    && occupant_of(self.host, *port).is_none()
            })
            .ok_or_else(|| environment("no free port for ras"))
    }

    /// Работающие `ragent` машины; `None` — перечня нет.
    fn agents(&self) -> Option<Vec<ProcessRow>> {
        self.host.processes(&[AGENT_IMAGE])
    }

    /// Над реестром не работает ни один `ragent`; без перечня процессов этого не сказать.
    fn registry_is_free(&self, registry: &Path) -> bool {
        self.agents().is_some_and(|agents| {
            first_with_keys(&agents, &ragent::registry_keys(registry)).is_none()
        })
    }

    /// Закреплённые порты заняты: чужой агент над нашим реестром — отказ без выхода, иначе выход —
    /// новый реестр (`INV.USE-CASES.A-BUSY-REGPORT-IS-REFUSED`). Перечень процессов нужен, только
    /// если порт занят.
    fn busy(&self, pins: &ClusterPins) -> Option<Busy> {
        let refusal = |message: String| UseCaseError::new(UseCaseErrorKind::WorkspaceBusy, message);
        let foreign = |agents: &[ProcessRow]| {
            first_with_keys(agents, &ragent::registry_keys(&self.registry(pins))).map(|agent| {
                Busy::ForeignAgent(refusal(format!(
                    "the registry of the cluster of '{}' is served by ragent pid {agent}, which this runner did not start; it is left running",
                    self.declared.infobase
                )))
            })
        };
        let Some((port, pid)) = pins
            .ports
            .all()
            .find_map(|port| occupant_of(self.host, port).map(|pid| (port, pid)))
        else {
            // Порты свободны, но чужой агент может работать над нашим реестром на других портах:
            // второй агент над тем же реестром его испортит. Без перечня запуск идёт как есть.
            return self.agents().and_then(|agents| foreign(&agents));
        };
        let Some(agents) = self.agents() else {
            return Some(Busy::Unlisted(refusal(format!(
                "port {port} of the cluster of '{}' is taken, and the process list did not answer to tell by whom; repeat the command",
                self.declared.infobase
            ))));
        };
        if let Some(busy) = foreign(&agents) {
            return Some(busy);
        }
        let who = if pid == 0 {
            "another process".to_owned()
        } else {
            format!("process {pid}")
        };
        Some(Busy::Port(refusal(format!(
            "port {port} of the cluster of '{}' is taken by {who}, which this runner did not start; it is left running: free the port and repeat, or start a new registry on other ports with `cluster start --new-regport`",
            self.declared.infobase
        ))))
    }

    /// Каталоги реестров, которых не называют закреплённые порты: остатки прерванного `--new-regport`.
    /// Без перечня процессов ничего не удаляется.
    fn sweep_registries(&self, keep: &str) {
        let Some(agents) = self.agents() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(self.agent.root()) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(REGISTRY_PREFIX)
                && name != keep
                && first_with_keys(&agents, &ragent::registry_keys(&entry.path())).is_none()
            {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    /// Запуск на закреплённых портах; их нет — на новых. Порты уже проверены.
    fn start_on_pins(
        &self,
        machine: &str,
        stand: Option<&Stand<'_>>,
        pins: Option<ClusterPins>,
    ) -> Result<ClusterResult, UseCaseError> {
        for paths in [&self.agent, &self.ras] {
            paths
                .prepare()
                .map_err(|error| io_failure("cannot prepare", paths.root(), &error))?;
        }
        let pins = match pins {
            // Реестры подметаются только при известных портах: если закреплённое потеряно, прежний
            // реестр может быть тем, где зарегистрирована база.
            Some(pins) => {
                self.sweep_registries(&pins.registry);
                pins
            }
            None => {
                let ports = self.choose_ports(stand, None)?;
                let pins = ClusterPins {
                    ports,
                    registry: format!("{REGISTRY_PREFIX}{}", ports.regport),
                    registered: false,
                    previous: None,
                };
                self.write_pins(&pins)?;
                pins
            }
        };
        let agent = self.raise(&pins, machine)?;
        let mut result = self.result(ServiceState::StartedByThisCopy, ServiceAction::Started);
        result.ready = true;
        Ok(self.describe(result, Some(&agent)))
    }

    /// Новый реестр на новых портах, если прежние заняты чужим процессом (это уже проверено).
    /// Закреплённое сначала запоминает прежний реестр: прерванный перенос откатит следующий запуск.
    /// Прежний реестр удаляется только после регистрации базы в новом: отказ ничего не теряет.
    fn move_registry(
        &self,
        machine: &str,
        stand: Option<&Stand<'_>>,
        old: ClusterPins,
    ) -> Result<ClusterResult, UseCaseError> {
        let old_registry = self.registry(&old);
        let declared = self.database()?;
        let existed = old.registered
            || match (&declared, stand) {
                (Some(Database::Managed { name }), Some(stand)) => {
                    let (port, bin) = stand.running_record()?;
                    stand.database_exists(port, &bin, name)?
                }
                _ => false,
            };
        let database = match (&declared, existed) {
            (Some(database), _) => Some(database),
            (None, true) => {
                return Err(validation(
                    "infobase.dbms is not declared: a new registry cannot register the infobase without its database",
                ))
            }
            (None, false) => None,
        };
        let ports = self.choose_ports(stand, Some(old.ports))?;
        let moving = ClusterPins {
            ports,
            registry: format!("{REGISTRY_PREFIX}{}", ports.regport),
            registered: false,
            previous: Some(PreviousRegistry {
                ports: old.ports,
                registry: old.registry.clone(),
                registered: old.registered,
            }),
        };
        for paths in [&self.agent, &self.ras] {
            paths
                .prepare()
                .map_err(|error| io_failure("cannot prepare", paths.root(), &error))?;
        }
        self.write_pins(&moving)?;
        let new_registry = self.registry(&moving);
        let roll_back = |error: UseCaseError, agent: Option<&ServiceRecord>| {
            let error = match agent {
                Some(agent) => self.give_up(agent, error),
                None => error,
            };
            // Агент нового реестра не снялся: закреплённое остаётся переносом, и следующий
            // `cluster start` откатит его, сняв агента.
            if !matches!(self.observe(&self.agent), Observed::Nothing) {
                return error.followed_by(
                    "the move to the new registry is left for the next cluster start to roll back",
                );
            }
            let error = match self.write_pins(&old) {
                Ok(()) => error,
                Err(restore) => error.followed_by(format!(
                    "the previous ports were not restored, the next cluster start restores them: {restore}"
                )),
            };
            let _ = std::fs::remove_dir_all(&new_registry);
            error
        };
        let agent = match self.raise(&moving, machine) {
            Ok(agent) => agent,
            Err(error) => return Err(roll_back(error, None)),
        };
        let registered = match (existed, database) {
            (true, Some(database)) => self.register(&agent, database, stand),
            _ => Ok(()),
        };
        let pins = ClusterPins {
            registered: existed,
            previous: None,
            ..moving
        };
        if let Err(error) = registered.and_then(|()| self.write_pins(&pins)) {
            return Err(roll_back(error, Some(&agent)));
        }
        let _ = std::fs::remove_dir_all(&old_registry);
        let mut result = self.result(ServiceState::StartedByThisCopy, ServiceAction::Started);
        result.ready = true;
        result.previous_regport = Some(old.ports.regport);
        Ok(self.describe(result, Some(&agent)))
    }

    /// База над прежней БД в новом реестре: `rac infobase create` без `--create-database`.
    fn register(
        &self,
        agent: &ServiceRecord,
        database: &Database,
        stand: Option<&Stand<'_>>,
    ) -> Result<(), UseCaseError> {
        let ras = self
            .own_ras()
            .ok_or_else(|| platform("ras of the new registry is not running"))?;
        let cluster = self
            .probe(&agent.bin, ras.port)
            .ok_or_else(|| platform("the new cluster did not name itself to rac cluster list"))?;
        let access = database.access(stand)?;
        let locale = self
            .config
            .infobase
            .dbms
            .as_ref()
            .and_then(|dbms| dbms.locale.clone());
        let rac_bin = sibling(&agent.bin, "rac");
        let address = ras_address(ras.port);
        let request = rac::register_infobase(
            &rac_bin,
            &address,
            &Registration {
                cluster: &cluster,
                name: &self.declared.name,
                kind: &access.kind,
                server: &access.server,
                database: &access.name,
                user: access.user.as_deref(),
                password: access.password.as_deref(),
                locale: locale.as_deref(),
            },
        );
        let secrets: Vec<&str> = access.password.as_deref().into_iter().collect();
        let answer = self
            .runner
            .run_with_policy(&request, &self.probe_policy())
            .map_err(|error| platform(format!("rac did not run: {error}")))?;
        if answer.outcome().is_err() {
            return Err(platform(format!(
                "rac could not register '{}' in the new registry: {}",
                self.declared.name,
                mask_text(answer.stderr.trim(), &secrets)
            )));
        }
        if !self.lists_infobase(&rac_bin, &address, &cluster) {
            return Err(platform(format!(
                "the new registry does not list the infobase '{}'",
                self.declared.name
            )));
        }
        Ok(())
    }

    fn lists_infobase(&self, rac_bin: &Path, address: &str, cluster: &str) -> bool {
        self.runner
            .run_with_policy(
                &rac::infobase_list(rac_bin, address, cluster),
                &self.probe_policy(),
            )
            .ok()
            .filter(|answer| answer.outcome().is_ok())
            .is_some_and(|answer| rac::lists_infobase(&answer.stdout, &self.declared.name))
    }

    /// Агент, затем `ras`, затем ответ `rac`. Неудача снимает всё поднятое здесь.
    fn raise(&self, pins: &ClusterPins, machine: &str) -> Result<ServiceRecord, UseCaseError> {
        let ragent_bin = (self.locate)()?;
        let registry = self.registry(pins);
        std::fs::create_dir_all(&registry)
            .map_err(|error| io_failure("cannot prepare", &registry, &error))?;
        let mut agent = ServiceRecord {
            state: RecordState::Starting,
            pid: None,
            started: None,
            image: None,
            machine: Some(machine.to_owned()),
            starter: self.starter(),
            port: pins.ports.agent,
            explicit: true,
            bin: ragent_bin.clone(),
            version: platform_version(&ragent_bin),
            agent: None,
            location: self.agent.location(),
        };
        write_record(&self.agent, &agent)?;
        let request = ragent::start_agent(&ragent_bin, &registry, pins.ports, &self.agent.log());
        let pid = self.spawn(&request, "ragent").inspect_err(|_| {
            forget_record(&self.agent);
        })?;
        let Some(facts) = self.host.facts(pid) else {
            forget_record(&self.agent);
            return Err(self.not_ready("ragent exited as soon as it started"));
        };
        agent.pid = Some(pid);
        agent.started = Some(facts.started);
        agent.image = Some(facts.image);
        if let Err(error) = write_record(&self.agent, &agent) {
            return Err(self.give_up(&agent, error));
        }
        if let Err(error) = self.raise_ras(&agent, pins.ports, machine) {
            return Err(self.give_up(&agent, error));
        }
        agent.state = RecordState::Running;
        agent.starter = None;
        if let Err(error) = write_record(&self.agent, &agent) {
            return Err(self.give_up(&agent, error));
        }
        Ok(agent)
    }

    /// `ras` к работающему агенту и ожидание ответа `rac`.
    fn raise_ras(
        &self,
        agent: &ServiceRecord,
        ports: ClusterPorts,
        machine: &str,
    ) -> Result<ServiceRecord, UseCaseError> {
        let port = self.choose_ras_port(ports)?;
        let ras_bin = sibling(&agent.bin, "ras");
        let mut ras = ServiceRecord {
            state: RecordState::Starting,
            pid: None,
            started: None,
            image: None,
            machine: Some(machine.to_owned()),
            starter: self.starter(),
            port,
            explicit: true,
            bin: ras_bin.clone(),
            version: agent.version.clone(),
            agent: None,
            location: self.ras.location(),
        };
        write_record(&self.ras, &ras)?;
        let request = ragent::start_ras(&ras_bin, port, ports, &self.ras.log());
        let pid = self.spawn(&request, "ras").inspect_err(|_| {
            forget_record(&self.ras);
        })?;
        let Some(facts) = self.host.facts(pid) else {
            forget_record(&self.ras);
            return Err(self.not_ready("ras exited as soon as it started"));
        };
        ras.pid = Some(pid);
        ras.started = Some(facts.started);
        ras.image = Some(facts.image);
        let ready = write_record(&self.ras, &ras).and_then(|()| self.await_ready(agent, &ras));
        if let Err(error) = ready {
            if self.stop_process(&ras) {
                forget_record(&self.ras);
            }
            return Err(error);
        }
        ras.state = RecordState::Running;
        ras.starter = None;
        write_record(&self.ras, &ras)?;
        Ok(ras)
    }

    fn await_ready(&self, agent: &ServiceRecord, ras: &ServiceRecord) -> Result<(), UseCaseError> {
        let deadline = Instant::now() + self.host.patience(READY_TIMEOUT);
        loop {
            if let Some(error) =
                interruption_before_safe_point(self.context, "waiting for the cluster")
            {
                return Err(error.into());
            }
            if self.probe(&agent.bin, ras.port).is_some() {
                return Ok(());
            }
            for (record, what) in [(agent, "ragent"), (ras, "ras")] {
                let gone = record.pid.is_none_or(|pid| {
                    self.host
                        .facts(pid)
                        .is_none_or(|facts| Some(facts.started) != record.started)
                });
                if gone {
                    return Err(self.not_ready(&format!(
                        "{what} exited before the cluster answered rac cluster list"
                    )));
                }
            }
            if Instant::now() >= deadline {
                return Err(self.not_ready("the cluster did not answer rac cluster list in time"));
            }
            self.host.sleep(POLL);
        }
    }

    fn spawn(
        &self,
        request: &crate::platform::service::ServiceRequest,
        what: &str,
    ) -> Result<u32, UseCaseError> {
        self.host
            .spawn(request, self.context.work())
            .map_err(|error| {
                if let ServiceSpawnError::Io(_) = error {
                    platform(format!("{what} did not start: {error}"))
                } else {
                    environment(error.to_string())
                }
            })
    }

    /// Этот раннер: запись `starting` верна, пока он жив.
    fn starter(&self) -> Option<(u32, u64)> {
        let pid = std::process::id();
        self.host.facts(pid).map(|facts| (pid, facts.started))
    }

    fn not_ready(&self, why: &str) -> UseCaseError {
        platform(format!("{why}; log: {}", self.agent.log().display()))
    }

    fn restart(&self, dry_run: bool) -> Result<ClusterResult, UseCaseError> {
        if dry_run {
            return Ok(self.plan());
        }
        // СУБД не трогает: перезапускается только кластер.
        let stopped = self.stop_cluster(false)?;
        if let Some(error) =
            interruption_before_safe_point(self.context, "starting the cluster again")
        {
            return Err(error.into());
        }
        let mut started = self.start(false, false)?;
        started.stopped_pid = stopped.stopped_pid;
        started.stopped_ras_pid = stopped.stopped_ras_pid;
        started.stopped_remnant_pids = stopped.stopped_remnant_pids;
        Ok(started)
    }

    fn failed(&self) -> ClusterResult {
        let mut result = self.status();
        result.ok = false;
        result.log_path = Some(self.agent.log());
        result
    }
}

fn platform(message: impl Into<String>) -> UseCaseError {
    UseCaseError::new(UseCaseErrorKind::Platform, message)
}

fn cluster<'a>(
    config: &'a AppConfig,
    context: &'a ExecutionContext,
    runner: &'a dyn ProcessRunner,
    host: &'a dyn ServiceHost,
    locate: &'a dyn Fn() -> Result<PathBuf, UseCaseError>,
) -> Result<Cluster<'a>, UseCaseError> {
    let declared = declared(config)?;
    let agent = ServicePaths::new(
        &config.local_dir(),
        &declared.infobase,
        StandService::Cluster,
    );
    let ras = ServicePaths::new(&config.local_dir(), &declared.infobase, StandService::Ras);
    Ok(Cluster {
        config,
        context,
        runner,
        host,
        locate,
        declared,
        agent,
        ras,
        started_at: Instant::now(),
    })
}

pub(super) fn run_cluster(
    context: &ExecutionContext,
    config: &AppConfig,
    command: ClusterCommand,
    dry_run: bool,
    runner: &dyn ProcessRunner,
    host: &dyn ServiceHost,
    locate: &dyn Fn() -> Result<PathBuf, UseCaseError>,
) -> UseCaseResult<ClusterResult> {
    super::refuse_a_stand_of_another_project(config).map_err(UseCaseFailure::without_payload)?;
    let cluster =
        cluster(config, context, runner, host, locate).map_err(UseCaseFailure::without_payload)?;
    let outcome = match command {
        ClusterCommand::Start { new_regport } => cluster.start(dry_run, new_regport),
        ClusterCommand::Status => Ok(cluster.status()),
        ClusterCommand::Stop => cluster.stop(dry_run),
        ClusterCommand::Restart => cluster.restart(dry_run),
    };
    outcome.map_err(|error| UseCaseFailure::with_payload(error, cluster.failed()))
}

/// Что `infobase create` делает с базой кластера раннера.
pub(crate) enum ClusterBase {
    /// Создать: цель с адресом кластера и доступом к СУБД для `CREATEINFOBASE`.
    Create(Box<AppConfig>),
    /// Кластер уже знает базу с этим именем: создавать нечего.
    AlreadyRegistered(String),
}

/// Цель `infobase create` в кластере раннера: кластер работает, база в СУБД, у PostgreSQL раннера
/// БД ещё нет. Память цели остаётся прежней: меняются только адрес и доступ к СУБД.
pub(super) fn cluster_base(
    context: &ExecutionContext,
    config: &AppConfig,
    dry_run: bool,
    runner: &dyn ProcessRunner,
    host: &dyn ServiceHost,
) -> Result<ClusterBase, UseCaseError> {
    super::refuse_a_stand_of_another_project(config)?;
    let no_locate = || Err(validation("a running cluster names its own utilities"));
    let cluster = cluster(config, context, runner, host, &no_locate)?;
    let not_running = || {
        validation(format!(
            "the cluster of '{}' is not running: run `cluster start`",
            cluster.declared.infobase
        ))
    };
    let Observed::Own(agent) = cluster.observe(&cluster.agent) else {
        return Err(not_running());
    };
    // Посреди переноса реестра базу не создают: следующий `cluster start` этот реестр откатит.
    let pins = cluster
        .pins()
        .filter(|pins| pins.previous.is_none())
        .ok_or_else(not_running)?;
    let database = cluster.database()?.ok_or_else(|| {
        validation("infobase.dbms is not declared: the infobase of a cluster lives in a DBMS")
    })?;
    let database = &database;
    if let Some(owner) = other_cluster_over(config, host, database) {
        return Err(owned_by_a_cluster(&owner));
    }
    // Базу уже знает кластер (её зарегистрировали и без раннера): создавать нечего.
    if let Some(ras) = cluster.own_ras() {
        if let Some(id) = cluster.probe(&agent.bin, ras.port) {
            if cluster.lists_infobase(&sibling(&agent.bin, "rac"), &ras_address(ras.port), &id) {
                if !dry_run {
                    mark_registered(&cluster, pins)?;
                }
                return Ok(ClusterBase::AlreadyRegistered(format!(
                    "the cluster of '{}' already holds the infobase '{}'",
                    cluster.declared.infobase, cluster.declared.name
                )));
            }
        }
    }
    let stand = match database {
        Database::Managed { .. } => Some(Stand::new(config, context, runner, host)?),
        Database::Own(_) => None,
    };
    if let (Database::Managed { name }, Some(stand)) = (database, &stand) {
        let (port, bin) = stand.running_record()?;
        if !dry_run && stand.database_exists(port, &bin, name)? {
            return Err(validation(format!(
                "the database '{name}' already exists in the PostgreSQL of this infobase: name another with infobase.dbms.name, or drop it with PostgreSQL tools using the password file `dbms status` names"
            )));
        }
    }
    let access = database.access(stand.as_ref())?;
    let mut target = config.clone();
    target.infobase.connection = pins.ports.connection(&cluster.declared.name);
    target.infobase.connection_projected = true;
    let locale = config
        .infobase
        .dbms
        .as_ref()
        .and_then(|dbms| dbms.locale.clone());
    target.infobase.dbms = Some(InfobaseDbmsConfig {
        kind: Some(access.kind),
        server: Some(access.server),
        name: Some(access.name),
        user: access.user,
        password: access.password,
        locale,
        managed: None,
    });
    Ok(ClusterBase::Create(Box::new(target)))
}

fn mark_registered(cluster: &Cluster<'_>, pins: ClusterPins) -> Result<(), UseCaseError> {
    cluster.write_pins(&ClusterPins {
        registered: true,
        ..pins
    })
}

/// После `CREATEINFOBASE`: база создана в текущем реестре.
pub(super) fn remember_registered(config: &AppConfig) -> Result<(), UseCaseError> {
    let declared = declared(config)?;
    let paths = ServicePaths::new(
        &config.local_dir(),
        &declared.infobase,
        StandService::Cluster,
    );
    let pins: ClusterPins = read_json(&paths.pins())
        .ok_or_else(|| validation("the ports of the cluster are not pinned"))?;
    write_json(
        &paths.pins(),
        &ClusterPins {
            registered: true,
            ..pins
        },
    )
    .map_err(|error| io_failure("cannot write", &paths.pins(), &error))
}

/// Сценарий `cluster` с подделкой утилит и машины: файлы настоящие, процессы нет.
#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, HashSet};

    use super::super::ServiceCommand;
    use super::*;
    use crate::config::model::{
        InfobaseClusterConfig, InfobaseConfig, ManagedDbmsConfig, ManagedServerConfig,
    };
    use crate::platform::postgres;
    use crate::platform::process::{
        ProcessError, ProcessExecutionPolicy, ProcessRequest, ProcessResult, SpawnResult, WorkGiven,
    };
    use crate::platform::service::{ProcessFacts, ProcessRow, ServiceRequest};
    use crate::use_cases::context::CommandName;
    use crate::use_cases::result::EnvironmentReason;

    const AGENT_PID: u32 = 7001;
    const RAS_PID: u32 = 7003;
    const RMNGR_PID: u32 = 7004;
    const RPHOST_PID: u32 = 7005;
    const PG_PID: u32 = 7002;
    const STARTED: u64 = 11;
    const MACHINE: &str = "machine";
    const CLUSTER_ID: &str = "6c5a8a62-3a8c-4e4b-b0a6-2f4c4b1f1d10";

    /// `rac` и утилиты PostgreSQL.
    struct Runner<'a> {
        host: &'a Host,
    }

    impl ProcessRunner for Runner<'_> {
        fn run_with_policy(
            &self,
            request: &ProcessRequest,
            _policy: &ProcessExecutionPolicy,
        ) -> Result<ProcessResult, ProcessError> {
            let host = self.host;
            let tool = request
                .program
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let mut stdout = String::new();
            let mut exit_code = 0;
            match tool.as_str() {
                "initdb" => {
                    let data = PathBuf::from(&request.args[1]);
                    std::fs::create_dir_all(&data).unwrap();
                    std::fs::write(data.join("PG_VERSION"), "17").unwrap();
                }
                "pg_ctl" if request.args[0] == "stop" => {
                    host.processes.borrow_mut().remove(&PG_PID);
                    host.listeners.borrow_mut().retain(|_, pid| *pid != PG_PID);
                    host.pg_stops.set(host.pg_stops.get() + 1);
                }
                "psql" if host.database_exists.get() => stdout = "1".to_owned(),
                "rac" => {
                    host.rac_calls.borrow_mut().push(request.args.clone());
                    let alive = |pid| host.processes.borrow().contains_key(&pid);
                    if !(alive(AGENT_PID) && alive(RAS_PID)) || host.cluster_silent.get() {
                        exit_code = 1;
                    } else if request.args[..2] == ["cluster", "list"] {
                        stdout = format!("cluster : {CLUSTER_ID}\nport    : 1\n");
                    } else if request.args[..3] == ["infobase", "summary", "list"] {
                        stdout = host
                            .registered
                            .borrow()
                            .iter()
                            .map(|name| format!("infobase : x\nname     : {name}\n\n"))
                            .collect();
                    } else if request.args[..2] == ["infobase", "create"] {
                        if host.registration_fails.get() {
                            exit_code = 1;
                        } else {
                            host.registered.borrow_mut().push("demo".to_owned());
                        }
                    }
                }
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

    #[derive(Default)]
    struct Host {
        unsupported: Cell<bool>,
        /// Отмена команды, наступающая, как только поднят `ragent`.
        cancel_on_agent: RefCell<Option<tokio_util::sync::CancellationToken>>,
        processes: RefCell<HashMap<u32, ProcessFacts>>,
        /// Родитель процесса, как его видит перечень.
        parents: RefCell<HashMap<u32, u32>>,
        listeners: RefCell<HashMap<u16, u32>>,
        spawned: RefCell<Vec<String>>,
        agent_args: RefCell<Vec<String>>,
        agent_exits_at_once: Cell<bool>,
        cluster_silent: Cell<bool>,
        hard_stops: RefCell<HashSet<u32>>,
        /// Процессы, которых не снять.
        immortal: RefCell<HashSet<u32>>,
        /// Перечень процессов не отвечает.
        unlisted: Cell<bool>,
        /// Чужие `ragent`: pid и командная строка.
        foreign_agents: RefCell<Vec<(u32, String)>>,
        database_exists: Cell<bool>,
        registered: RefCell<Vec<String>>,
        registration_fails: Cell<bool>,
        rac_calls: RefCell<Vec<Vec<String>>>,
        pg_stops: Cell<u32>,
    }

    impl Host {
        fn live(&self, pid: u32, image: &str) {
            self.processes.borrow_mut().insert(
                pid,
                ProcessFacts {
                    started: STARTED,
                    image: image.to_owned(),
                },
            );
        }
        /// Процесс, начавшийся в это время.
        fn live_since(&self, pid: u32, image: &str, started: u64) {
            self.processes.borrow_mut().insert(
                pid,
                ProcessFacts {
                    started,
                    image: image.to_owned(),
                },
            );
        }
        fn kill(&self, pid: u32) {
            if self.immortal.borrow().contains(&pid) {
                return;
            }
            self.processes.borrow_mut().remove(&pid);
            self.listeners.borrow_mut().retain(|_, held| *held != pid);
        }
        fn alive(&self, pid: u32) -> bool {
            self.processes.borrow().contains_key(&pid)
        }
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
            let program = request
                .program
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_string();
            self.spawned.borrow_mut().push(program.clone());
            match program.as_str() {
                "pg_ctl" => {
                    let data = PathBuf::from(&request.args[3]);
                    std::fs::write(data.join("postmaster.pid"), format!("{PG_PID}")).unwrap();
                    self.live(PG_PID, "postgres.exe");
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
                    Ok(7)
                }
                "ragent" => {
                    self.agent_args.replace(request.args.clone());
                    if let Some(token) = self.cancel_on_agent.borrow().as_ref() {
                        token.cancel();
                    }
                    if !self.agent_exits_at_once.get() {
                        self.live(AGENT_PID, "ragent.exe");
                        let port = |key: &str| -> u16 {
                            let at = request.args.iter().position(|arg| arg == key).unwrap();
                            request.args[at + 1].parse().unwrap()
                        };
                        self.listeners.borrow_mut().insert(port("/port"), AGENT_PID);
                        // `rmngr` и `rphost` — дети агента; `rmngr` держит `regport`.
                        self.live(RMNGR_PID, "rmngr.exe");
                        self.live(RPHOST_PID, "rphost.exe");
                        self.parents.borrow_mut().insert(RMNGR_PID, AGENT_PID);
                        self.parents.borrow_mut().insert(RPHOST_PID, AGENT_PID);
                        self.listeners
                            .borrow_mut()
                            .insert(port("/regport"), RMNGR_PID);
                    }
                    Ok(AGENT_PID)
                }
                "ras" => {
                    self.live(RAS_PID, "ras.exe");
                    Ok(RAS_PID)
                }
                other => panic!("unexpected service {other}"),
            }
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
            self.kill(pid);
            // Мягкий конец агента уносит его процессы (замер #534).
            if pid == AGENT_PID {
                self.kill(RMNGR_PID);
                self.kill(RPHOST_PID);
            }
        }
        fn hard_stop(&self, _runner: &dyn ProcessRunner, pid: u32, _context: &ExecutionContext) {
            self.hard_stops.borrow_mut().insert(pid);
            self.kill(pid);
        }
        fn sleep(&self, _duration: Duration) {}
        fn patience(&self, _default: Duration) -> Duration {
            Duration::ZERO
        }
        fn processes(&self, images: &[&str]) -> Option<Vec<ProcessRow>> {
            if self.unlisted.get() {
                return None;
            }
            if images == [AGENT_IMAGE] {
                return Some(
                    self.foreign_agents
                        .borrow()
                        .iter()
                        .map(|(pid, command)| ProcessRow {
                            pid: *pid,
                            parent: 1,
                            command: command.clone(),
                        })
                        .collect(),
                );
            }
            if images != REMNANT_IMAGES {
                return Some(Vec::new());
            }
            Some(
                self.parents
                    .borrow()
                    .iter()
                    .filter(|(pid, _)| self.alive(**pid))
                    .map(|(pid, parent)| ProcessRow {
                        pid: *pid,
                        parent: *parent,
                        command: String::new(),
                    })
                    .collect(),
            )
        }
    }

    struct Setup {
        _root: tempfile::TempDir,
        config: AppConfig,
    }

    /// Кластер над PostgreSQL раннера.
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
            managed: Some(ManagedServerConfig {
                name: Some("demo".to_owned()),
                db_path: None,
            }),
            ..Default::default()
        });
        infobase.dbms = Some(InfobaseDbmsConfig {
            managed: Some(ManagedDbmsConfig::default()),
            name: Some("demo_data".to_owned()),
            locale: Some("ru".to_owned()),
            ..Default::default()
        });
        let mut config = AppConfig {
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
            source_sets: Vec::new(),
            tools: Default::default(),
            mcp: Default::default(),
            tests: Default::default(),
        };
        config.tools.postgres.path = Some(build);
        Setup {
            _root: root,
            config,
        }
    }

    impl Setup {
        fn agent(&self) -> ServicePaths {
            ServicePaths::new(&self.config.local_dir(), "origin", StandService::Cluster)
        }
        fn ras(&self) -> ServicePaths {
            ServicePaths::new(&self.config.local_dir(), "origin", StandService::Ras)
        }
        fn pins(&self) -> ClusterPins {
            read_json(&self.agent().pins()).expect("pinned")
        }
        fn run(&self, command: ServiceCommand, host: &Host) -> Result<ClusterResult, UseCaseError> {
            self.run_with(command, false, host)
        }
        fn run_with(
            &self,
            command: ServiceCommand,
            new_regport: bool,
            host: &Host,
        ) -> Result<ClusterResult, UseCaseError> {
            self.run_in(
                &ExecutionContext::cli(CommandName::ClusterStart),
                command,
                new_regport,
                host,
            )
        }
        fn run_in(
            &self,
            context: &ExecutionContext,
            command: ServiceCommand,
            new_regport: bool,
            host: &Host,
        ) -> Result<ClusterResult, UseCaseError> {
            let locate = || Ok(PathBuf::from("C:/1cv8/8.5.4.2028/bin/ragent.exe"));
            let command = match command {
                ServiceCommand::Start => ClusterCommand::Start { new_regport },
                ServiceCommand::Status => ClusterCommand::Status,
                ServiceCommand::Stop => ClusterCommand::Stop,
                ServiceCommand::Restart => ClusterCommand::Restart,
            };
            run_cluster(
                context,
                &self.config,
                command,
                false,
                &Runner { host },
                host,
                &locate,
            )
            .map_err(|failure| failure.error)
        }
        fn dbms(&self, command: ServiceCommand, host: &Host) -> crate::domain::dbms::DbmsResult {
            super::super::run_dbms(
                &ExecutionContext::cli(CommandName::DbmsStart),
                &self.config,
                command,
                false,
                &Runner { host },
                host,
            )
            .map_err(|failure| failure.error)
            .unwrap()
        }
        fn dbms_record(&self) -> ServiceRecord {
            read_json(
                &ServicePaths::new(&self.config.local_dir(), "origin", StandService::Postgres)
                    .record(),
            )
            .unwrap()
        }
        fn base(&self, host: &Host) -> Result<ClusterBase, UseCaseError> {
            cluster_base(
                &ExecutionContext::cli(CommandName::Init),
                &self.config,
                false,
                &Runner { host },
                host,
            )
        }
    }

    #[test]
    fn start_raises_the_agent_then_ras_and_waits_for_rac() {
        let setup = setup();
        let host = Host::default();
        let started = setup.run(ServiceCommand::Start, &host).unwrap();
        assert_eq!(started.action, ServiceAction::Started);
        assert_eq!(started.pid, Some(AGENT_PID));
        assert_eq!(started.ras_pid, Some(RAS_PID));
        assert_eq!(started.version.as_deref(), Some("8.5.4.2028"));
        assert!(started.ready);
        assert!(!started.registered);
        let pins = setup.pins();
        assert_eq!(
            started.connection.as_deref(),
            Some(pins.ports.connection("demo").as_str())
        );
        assert_eq!(*host.spawned.borrow(), ["pg_ctl", "ragent", "ras"]);
        let args = host.agent_args.borrow();
        assert!(
            args.iter().any(|arg| arg.ends_with(&pins.registry)),
            "{args:?}"
        );
        let ras = read_json::<ServiceRecord>(&setup.ras().record()).unwrap();
        assert!(
            !pins.ports.all().any(|port| port == ras.port),
            "ras stays out of the pinned block"
        );
    }

    #[test]
    fn a_second_start_names_the_running_cluster_and_starts_nothing() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        let second = setup.run(ServiceCommand::Start, &host).unwrap();
        assert_eq!(second.action, ServiceAction::AlreadyRunning);
        assert!(second.ready);
        assert_eq!(host.spawned.borrow().len(), 3);
    }

    #[test]
    fn status_asks_rac_and_starts_nothing() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        host.rac_calls.borrow_mut().clear();
        let status = setup.run(ServiceCommand::Status, &host).unwrap();
        assert_eq!(status.state, ServiceState::StartedByThisCopy);
        assert!(status.ready);
        assert_eq!(status.ras_pid, Some(RAS_PID));
        assert_eq!(host.spawned.borrow().len(), 3, "nothing is started");
        assert_eq!(host.rac_calls.borrow()[0][..2], ["cluster", "list"]);
    }

    #[test]
    fn a_fallen_ras_is_raised_again_for_the_running_agent() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        host.kill(RAS_PID);
        let second = setup.run(ServiceCommand::Start, &host).unwrap();
        assert_eq!(second.action, ServiceAction::AlreadyRunning);
        assert!(second.ready);
        assert_eq!(
            host.spawned.borrow().last().map(String::as_str),
            Some("ras")
        );
    }

    #[test]
    fn the_ports_and_the_registry_stay_from_start_to_start() {
        let setup = setup();
        let host = Host::default();
        let first = setup.run(ServiceCommand::Start, &host).unwrap();
        setup.run(ServiceCommand::Stop, &host).unwrap();
        let second = setup.run(ServiceCommand::Start, &host).unwrap();
        assert_eq!(first.connection, second.connection);
        assert_eq!(first.registry_dir, second.registry_dir);
    }

    #[test]
    fn stop_takes_ras_then_the_agent_then_the_step_postgres() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        assert!(!setup.dbms_record().explicit);
        let stopped = setup.run(ServiceCommand::Stop, &host).unwrap();
        assert_eq!(stopped.action, ServiceAction::Stopped);
        assert_eq!(stopped.stopped_ras_pid, Some(RAS_PID));
        assert_eq!(stopped.stopped_pid, Some(AGENT_PID));
        assert_eq!(stopped.stopped_dbms_pid, Some(PG_PID));
        assert!(
            host.processes.borrow().is_empty(),
            "nothing of the stand is left"
        );
        assert!(!setup.agent().record().exists());
        assert!(!setup.ras().record().exists());
        let again = setup.run(ServiceCommand::Stop, &host).unwrap();
        assert_eq!(again.action, ServiceAction::NothingToStop);
    }

    #[test]
    fn an_explicit_postgres_outlives_its_cluster() {
        let setup = setup();
        let host = Host::default();
        setup.dbms(ServiceCommand::Start, &host);
        setup.run(ServiceCommand::Start, &host).unwrap();
        let stopped = setup.run(ServiceCommand::Stop, &host).unwrap();
        assert_eq!(stopped.stopped_dbms_pid, None);
        assert!(host.alive(PG_PID));
    }

    #[test]
    fn a_later_dbms_start_makes_a_step_postgres_explicit() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        setup.dbms(ServiceCommand::Start, &host);
        assert!(setup.dbms_record().explicit);
        setup.run(ServiceCommand::Stop, &host).unwrap();
        assert!(host.alive(PG_PID));
    }

    #[test]
    fn restart_leaves_the_postgres_alone() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        let restarted = setup.run(ServiceCommand::Restart, &host).unwrap();
        assert_eq!(restarted.stopped_pid, Some(AGENT_PID));
        assert_eq!(host.pg_stops.get(), 0);
        assert!(host.alive(PG_PID));
    }

    #[test]
    fn a_failed_cluster_stops_only_what_this_command_raised() {
        let setup = setup();
        let host = Host::default();
        host.agent_exits_at_once.set(true);
        setup.run(ServiceCommand::Start, &host).unwrap_err();
        assert!(!host.alive(PG_PID), "the step PostgreSQL is stopped");
        assert!(!setup.agent().record().exists());

        let host = Host::default();
        host.agent_exits_at_once.set(true);
        setup.dbms(ServiceCommand::Start, &host);
        setup.run(ServiceCommand::Start, &host).unwrap_err();
        assert!(host.alive(PG_PID), "the explicit one stays");
    }

    #[test]
    fn a_cluster_that_never_answers_is_taken_down() {
        let setup = setup();
        let host = Host::default();
        host.cluster_silent.set(true);
        let refused = setup.run(ServiceCommand::Start, &host).unwrap_err();
        assert!(
            refused.to_string().contains("rac cluster list"),
            "{refused}"
        );
        for pid in [AGENT_PID, RAS_PID, RMNGR_PID, RPHOST_PID, PG_PID] {
            assert!(!host.alive(pid), "pid {pid} is left");
        }
        assert!(!setup.agent().record().exists());
        assert!(!setup.ras().record().exists());
    }

    #[test]
    fn the_remnants_of_a_fallen_agent_are_stopped_by_their_pid() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        // `ragent` упал, `rmngr` и `rphost` живут и держат `regport`.
        host.kill(AGENT_PID);
        let status = setup.run(ServiceCommand::Status, &host).unwrap();
        assert!(status.stale_record);
        assert_eq!(
            status.state,
            ServiceState::NotRunning,
            "remnants are not foreign"
        );
        let mut remnants = status.remnant_pids.clone();
        remnants.sort_unstable();
        assert_eq!(remnants, [RMNGR_PID, RPHOST_PID]);
        let stopped = setup.run(ServiceCommand::Stop, &host).unwrap();
        assert_eq!(
            stopped.stopped_ras_pid,
            Some(RAS_PID),
            "ras outlives ragent"
        );
        let mut cleared = stopped.stopped_remnant_pids.clone();
        cleared.sort_unstable();
        assert_eq!(cleared, [RMNGR_PID, RPHOST_PID]);
        assert!(
            !host.hard_stops.borrow().contains(&AGENT_PID),
            "the dead parent is not asked"
        );
        assert!(host.processes.borrow().is_empty());
    }

    #[test]
    fn a_worker_raised_by_a_remnant_is_a_remnant_too() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        host.kill(AGENT_PID);
        // `rmngr` поднял новый `rphost`, и тот держит порт диапазона.
        host.live(7006, "rphost.exe");
        host.parents.borrow_mut().insert(7006, RMNGR_PID);
        let range = setup.pins().ports.range_start;
        host.listeners.borrow_mut().insert(range, 7006);
        let status = setup.run(ServiceCommand::Status, &host).unwrap();
        assert_eq!(status.state, ServiceState::NotRunning, "{status:?}");
        assert!(status.remnant_pids.contains(&7006));
        let started = setup.run(ServiceCommand::Start, &host).unwrap();
        assert_eq!(started.action, ServiceAction::Started);
        assert!(host.hard_stops.borrow().contains(&7006));
    }

    #[test]
    fn the_dumps_of_a_stop_are_removed_and_older_ones_stay() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        let old = setup.agent().root().join("ragent_old.mdmp");
        std::fs::write(&old, "crash").unwrap();
        let past = std::time::SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(past)
            .unwrap();
        // Дамп, который агент пишет при остановке, появляется во время `cluster stop`.
        let fresh = setup.agent().root().join("ragent_stop.mdmp");
        let context = ExecutionContext::cli(CommandName::ClusterStop);
        let runner = Runner { host: &host };
        let locate = || Ok(PathBuf::from("ragent.exe"));
        let cluster = cluster(&setup.config, &context, &runner, &host, &locate).unwrap();
        let since = std::time::SystemTime::now();
        std::fs::write(&fresh, "stop").unwrap();
        cluster.forget_stop_dumps(since);
        assert!(!fresh.exists());
        assert!(old.exists());
    }

    #[test]
    fn start_after_a_fallen_agent_clears_its_remnants_and_keeps_the_regport() {
        let setup = setup();
        let host = Host::default();
        let first = setup.run(ServiceCommand::Start, &host).unwrap();
        host.kill(AGENT_PID);
        host.kill(RAS_PID);
        let second = setup.run(ServiceCommand::Start, &host).unwrap();
        assert_eq!(second.action, ServiceAction::Started);
        assert!(second.stale_record);
        assert_eq!(first.connection, second.connection, "no new regport");
        assert!(host.hard_stops.borrow().contains(&RMNGR_PID));
    }

    #[test]
    fn a_process_with_another_parent_or_an_earlier_start_is_no_remnant() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        host.kill(AGENT_PID);
        host.parents.borrow_mut().insert(RPHOST_PID, 1);
        host.processes
            .borrow_mut()
            .get_mut(&RMNGR_PID)
            .unwrap()
            .started = STARTED - 1;
        let status = setup.run(ServiceCommand::Status, &host).unwrap();
        assert!(status.remnant_pids.is_empty(), "{:?}", status.remnant_pids);
        assert_eq!(
            status.state,
            ServiceState::Foreign,
            "the regport holder is foreign now"
        );
    }

    #[test]
    fn a_child_of_a_process_that_got_the_dead_agents_pid_is_no_remnant() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        host.kill(AGENT_PID);
        // Система отдала pid мёртвого агента новому `ragent` другой копии; у того свой `rmngr`.
        host.live_since(AGENT_PID, "ragent.exe", STARTED + 100);
        host.live_since(7020, "rmngr.exe", STARTED + 200);
        host.parents.borrow_mut().insert(7020, AGENT_PID);
        let status = setup.run(ServiceCommand::Status, &host).unwrap();
        let mut remnants = status.remnant_pids.clone();
        remnants.sort_unstable();
        assert_eq!(remnants, [RMNGR_PID, RPHOST_PID], "only the old children");
        setup.run(ServiceCommand::Stop, &host).unwrap();
        assert!(host.alive(7020), "the foreign rmngr is left alone");
        assert!(
            host.alive(AGENT_PID),
            "the new owner of the pid is left alone"
        );
        assert!(!host.alive(RMNGR_PID));
    }

    #[test]
    fn a_child_on_another_machine_or_of_another_image_is_no_remnant() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        host.kill(AGENT_PID);
        host.live(7021, "notepad.exe");
        host.parents.borrow_mut().insert(7021, AGENT_PID);
        let status = setup.run(ServiceCommand::Status, &host).unwrap();
        assert!(
            !status.remnant_pids.contains(&7021),
            "{:?}",
            status.remnant_pids
        );
        // Запись другой машины: её остатков здесь нет.
        let mut record: ServiceRecord = read_json(&setup.agent().record()).unwrap();
        record.machine = Some("another".to_owned());
        write_json(&setup.agent().record(), &record).unwrap();
        let status = setup.run(ServiceCommand::Status, &host).unwrap();
        assert!(status.remnant_pids.is_empty(), "{:?}", status.remnant_pids);
    }

    #[test]
    fn a_remnant_that_does_not_stop_keeps_the_agent_record() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        host.kill(AGENT_PID);
        host.immortal.borrow_mut().insert(RPHOST_PID);
        let refused = setup.run(ServiceCommand::Stop, &host).unwrap_err();
        assert!(
            refused.to_string().contains(&RPHOST_PID.to_string()),
            "{refused}"
        );
        assert!(
            setup.agent().record().exists(),
            "the record still names the remnants"
        );
        host.immortal.borrow_mut().clear();
        let stopped = setup.run(ServiceCommand::Stop, &host).unwrap();
        assert!(stopped.stopped_remnant_pids.contains(&RPHOST_PID));
        assert!(!setup.agent().record().exists());
    }

    #[test]
    fn without_the_process_list_remnants_are_not_guessed_and_the_record_stays() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        host.kill(AGENT_PID);
        host.unlisted.set(true);
        host.spawned.borrow_mut().clear();
        let refused = setup.run(ServiceCommand::Start, &host).unwrap_err();
        assert!(refused.to_string().contains("process list"), "{refused}");
        assert!(setup.agent().record().exists());
        assert!(
            host.hard_stops.borrow().is_empty(),
            "nothing is stopped blindly"
        );
        assert!(!host.spawned.borrow().iter().any(|tool| tool == "ragent"));
    }

    #[test]
    fn a_taken_port_without_the_process_list_is_refused_and_nothing_is_removed() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        setup.run(ServiceCommand::Stop, &host).unwrap();
        let pins = setup.pins();
        host.listeners.borrow_mut().insert(pins.ports.regport, 555);
        host.unlisted.set(true);
        let refused = setup.run(ServiceCommand::Start, &host).unwrap_err();
        assert!(refused.to_string().contains("process list"), "{refused}");
        assert!(setup.agent().root().join(&pins.registry).is_dir());
    }

    #[test]
    fn a_lost_pins_file_does_not_remove_the_old_registry() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        setup.run(ServiceCommand::Stop, &host).unwrap();
        let old = setup.pins();
        std::fs::remove_file(setup.agent().pins()).unwrap();
        setup.run(ServiceCommand::Start, &host).unwrap();
        assert!(
            setup.agent().root().join(&old.registry).is_dir(),
            "the registry that may hold the base stays"
        );
    }

    #[test]
    fn start_rolls_back_an_interrupted_move() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        super::remember_registered(&setup.config).unwrap();
        setup.run(ServiceCommand::Stop, &host).unwrap();
        let old = setup.pins();
        // Раннер умер посреди `--new-regport`: закреплённое называет новый реестр и помнит прежний.
        let moving = ClusterPins {
            ports: ClusterPorts::from_first(old.ports.agent + 100),
            registry: "registry-moving".to_owned(),
            registered: false,
            previous: Some(PreviousRegistry {
                ports: old.ports,
                registry: old.registry.clone(),
                registered: old.registered,
            }),
        };
        std::fs::create_dir_all(setup.agent().root().join(&moving.registry)).unwrap();
        write_json(&setup.agent().pins(), &moving).unwrap();
        let started = setup.run(ServiceCommand::Start, &host).unwrap();
        assert_eq!(started.action, ServiceAction::Started);
        assert_eq!(setup.pins(), old, "the previous registry is back");
        assert!(!setup.agent().root().join("registry-moving").exists());
    }

    #[test]
    fn a_busy_regport_is_refused_before_the_postgres_starts() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        setup.run(ServiceCommand::Stop, &host).unwrap();
        host.spawned.borrow_mut().clear();
        host.listeners
            .borrow_mut()
            .insert(setup.pins().ports.regport, 555);
        setup.run(ServiceCommand::Start, &host).unwrap_err();
        assert!(
            host.spawned.borrow().is_empty(),
            "{:?}",
            host.spawned.borrow()
        );
    }

    #[test]
    fn stop_leaves_the_step_postgres_to_the_running_standalone() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        // Над PostgreSQL шага работает и свой автономный сервер этой базы: СУБД нужна ему.
        own_standalone(&setup, &host);
        let stopped = setup.run(ServiceCommand::Stop, &host).unwrap();
        assert_eq!(stopped.stopped_pid, Some(AGENT_PID));
        assert_eq!(stopped.stopped_dbms_pid, None);
        assert!(host.alive(PG_PID));
    }

    #[test]
    fn a_busy_regport_is_refused_with_its_process_and_no_new_port_is_taken() {
        let setup = setup();
        let host = Host::default();
        let first = setup.run(ServiceCommand::Start, &host).unwrap();
        setup.run(ServiceCommand::Stop, &host).unwrap();
        let pins = setup.pins();
        host.listeners.borrow_mut().insert(pins.ports.regport, 555);
        let refused = setup.run(ServiceCommand::Start, &host).unwrap_err();
        assert_eq!(refused.kind(), UseCaseErrorKind::WorkspaceBusy);
        let text = refused.to_string();
        assert!(text.contains("process 555"), "{text}");
        assert!(text.contains("--new-regport"), "{text}");
        assert_eq!(setup.pins(), pins, "the pinned ports stay");
        let status = setup.run(ServiceCommand::Status, &host).unwrap();
        assert_eq!(status.state, ServiceState::Foreign);
        assert_eq!(status.occupant_pid, Some(555));
        assert_eq!(status.connection, first.connection);
        assert!(!host.hard_stops.borrow().contains(&555), "it is left alone");
    }

    #[test]
    fn a_foreign_agent_over_this_registry_is_refused_without_a_new_registry() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        setup.run(ServiceCommand::Stop, &host).unwrap();
        let pins = setup.pins();
        let registry = setup.agent().root().join(&pins.registry);
        host.foreign_agents.borrow_mut().push((
            9100,
            format!("ragent.exe /agent /d \"{}\"", registry.display()),
        ));
        host.listeners.borrow_mut().insert(pins.ports.regport, 9101);
        for new_regport in [false, true] {
            let refused = setup
                .run_with(ServiceCommand::Start, new_regport, &host)
                .unwrap_err();
            let text = refused.to_string();
            assert!(text.contains("ragent pid 9100"), "{text}");
            assert!(!text.contains("--new-regport"), "{text}");
        }
        assert!(registry.is_dir(), "the live registry stays");
    }

    #[test]
    fn new_regport_moves_the_base_to_a_new_registry_over_its_database() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        super::remember_registered(&setup.config).unwrap();
        setup.run(ServiceCommand::Stop, &host).unwrap();
        let old = setup.pins();
        let old_registry = setup.agent().root().join(&old.registry);
        host.listeners.borrow_mut().insert(old.ports.regport, 555);
        let moved = setup.run_with(ServiceCommand::Start, true, &host).unwrap();
        assert_eq!(moved.previous_regport, Some(old.ports.regport));
        let new = setup.pins();
        assert_ne!(new.ports.regport, old.ports.regport);
        assert!(new.registered);
        assert!(
            !old_registry.exists(),
            "the old registry is gone after the move"
        );
        assert!(setup.agent().root().join(&new.registry).is_dir());
        let calls = host.rac_calls.borrow();
        let create = calls
            .iter()
            .find(|args| args[..2] == ["infobase", "create"])
            .expect("the base is registered again");
        assert!(!create.iter().any(|arg| arg == "--create-database"));
        assert!(create.iter().any(|arg| arg == "--db-name=demo_data"));
        assert!(create.iter().any(|arg| arg == "--name=demo"));
    }

    #[test]
    fn a_failed_move_keeps_the_old_registry_and_its_ports() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        super::remember_registered(&setup.config).unwrap();
        setup.run(ServiceCommand::Stop, &host).unwrap();
        let old = setup.pins();
        host.listeners.borrow_mut().insert(old.ports.regport, 555);
        host.registration_fails.set(true);
        setup
            .run_with(ServiceCommand::Start, true, &host)
            .unwrap_err();
        assert_eq!(setup.pins(), old);
        assert!(setup.agent().root().join(&old.registry).is_dir());
        let registries = std::fs::read_dir(setup.agent().root())
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(REGISTRY_PREFIX)
            })
            .count();
        assert_eq!(registries, 1, "the new registry is removed");
        for pid in [AGENT_PID, RAS_PID, RMNGR_PID, PG_PID] {
            assert!(!host.alive(pid), "pid {pid} is left");
        }
    }

    #[test]
    fn new_regport_on_free_ports_is_refused() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        setup.run(ServiceCommand::Stop, &host).unwrap();
        let refused = setup
            .run_with(ServiceCommand::Start, true, &host)
            .unwrap_err();
        assert!(refused.to_string().contains("are free"), "{refused}");
    }

    fn own_standalone(setup: &Setup, host: &Host) {
        let standalone = ServicePaths::new(
            &setup.config.local_dir(),
            "origin",
            StandService::Standalone,
        );
        standalone.prepare().unwrap();
        host.live(8100, "ibsrv.exe");
        write_json(
            &standalone.record(),
            &ServiceRecord {
                state: RecordState::Running,
                pid: Some(8100),
                started: Some(STARTED),
                image: Some("ibsrv.exe".to_owned()),
                machine: Some(MACHINE.to_owned()),
                starter: None,
                port: 1,
                explicit: true,
                bin: PathBuf::from("ibsrv.exe"),
                version: None,
                agent: None,
                location: standalone.location(),
            },
        )
        .unwrap();
    }

    #[test]
    fn a_running_standalone_of_the_same_base_refuses_the_cluster() {
        let setup = setup();
        let host = Host::default();
        own_standalone(&setup, &host);
        let refused = setup.run(ServiceCommand::Start, &host).unwrap_err();
        assert!(refused.to_string().contains("standalone stop"), "{refused}");
        assert!(host.spawned.borrow().is_empty());
    }

    #[test]
    fn a_running_cluster_refuses_ibcmd_over_the_same_database() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        // Та же база переобъявлена автономным сервером над той же БД, и БД уже есть: отказ
        // называет кластер, а не существующую БД.
        let mut config = setup.config.clone();
        config.infobase.cluster = None;
        config.infobase.standalone = Some(crate::config::model::StandaloneConfig {
            managed: Some(ManagedServerConfig {
                name: Some("demo".to_owned()),
                db_path: None,
            }),
            gate: None,
            host_fingerprint: None,
            exchange: None,
        });
        host.database_exists.set(true);
        let refused = super::super::standalone::create_base(
            &ExecutionContext::cli(CommandName::Init),
            &config,
            PathBuf::from("ibcmd.exe"),
            false,
            &Runner { host: &host },
            &host,
        )
        .unwrap_err();
        assert!(refused.to_string().contains("cluster stop"), "{refused}");
    }

    #[test]
    fn infobase_create_asks_for_the_running_cluster() {
        let setup = setup();
        let host = Host::default();
        let refused = setup.base(&host).err().unwrap();
        assert!(refused.to_string().contains("cluster start"), "{refused}");
    }

    #[test]
    fn infobase_create_refuses_an_existing_database_before_the_platform() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        host.database_exists.set(true);
        let refused = setup.base(&host).err().unwrap();
        assert!(refused.to_string().contains("already exists"), "{refused}");
    }

    #[test]
    fn infobase_create_targets_the_running_cluster_with_the_dbms_access() {
        let mut setup = setup();
        let host = Host::default();
        let started = setup.run(ServiceCommand::Start, &host).unwrap();
        setup.config.infobase.managed_memory = Some("managed:origin".to_owned());
        let Ok(ClusterBase::Create(target)) = setup.base(&host) else {
            panic!("a target to create");
        };
        assert_eq!(Some(target.infobase.connection.clone()), started.connection);
        assert!(target.infobase.connection_projected);
        assert_eq!(
            target.infobase.managed_memory.as_deref(),
            Some("managed:origin"),
            "the memory follows the name"
        );
        let dbms = target.infobase.dbms.as_ref().unwrap();
        assert_eq!(dbms.user.as_deref(), Some(postgres::SUPERUSER));
        assert!(dbms
            .password
            .as_deref()
            .is_some_and(|password| !password.is_empty()));
        assert!(dbms
            .server
            .as_deref()
            .unwrap()
            .starts_with("127.0.0.1 port="));
        assert_eq!(dbms.locale.as_deref(), Some("ru"));
        assert!(dbms.managed.is_none());
    }

    #[test]
    fn infobase_create_over_a_base_the_cluster_knows_marks_it_created() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        host.registered.borrow_mut().push("demo".to_owned());
        let answer = setup.base(&host);
        assert!(matches!(answer, Ok(ClusterBase::AlreadyRegistered(_))));
        assert!(setup.pins().registered);
    }

    #[test]
    fn a_managed_postgres_without_a_database_name_still_starts_and_stops_with_the_cluster() {
        let mut setup = setup();
        let host = Host::default();
        setup.config.infobase.dbms.as_mut().unwrap().name = None;
        let started = setup.run(ServiceCommand::Start, &host).unwrap();
        assert_eq!(started.action, ServiceAction::Started);
        assert!(host.alive(PG_PID), "the step PostgreSQL is up");
        let stopped = setup.run(ServiceCommand::Stop, &host).unwrap();
        assert_eq!(stopped.stopped_dbms_pid, Some(PG_PID));
        // Создать базу без имени БД нельзя: отказ называет ключ.
        setup.run(ServiceCommand::Start, &host).unwrap();
        let refused = setup.base(&host).err().expect("refused");
        assert!(
            refused.to_string().contains("infobase.dbms.name"),
            "{refused}"
        );
    }

    #[test]
    fn a_crashed_cluster_still_takes_its_step_postgres_down() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        for pid in [AGENT_PID, RAS_PID, RMNGR_PID, RPHOST_PID] {
            host.kill(pid);
        }
        let stopped = setup.run(ServiceCommand::Stop, &host).unwrap();
        assert_eq!(stopped.action, ServiceAction::NothingToStop);
        assert!(stopped.stale_record);
        assert_eq!(stopped.stopped_dbms_pid, Some(PG_PID));
        assert!(!host.alive(PG_PID));
        assert!(
            !setup.agent().record().exists(),
            "the record of the fallen agent is gone"
        );
    }

    #[test]
    fn a_fallen_postgres_under_a_live_cluster_is_named_and_comes_back_explicit_on_its_port() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        let port = setup.dbms_record().port;
        host.kill(PG_PID);
        let status = setup.dbms(ServiceCommand::Status, &host);
        assert_eq!(status.state, ServiceState::NotRunning);
        assert!(status.stale_record, "the fall is named");
        let started = setup.dbms(ServiceCommand::Start, &host);
        assert_eq!(started.action, ServiceAction::Started);
        let record = setup.dbms_record();
        assert_eq!(record.port, port, "the same port");
        assert!(record.explicit, "dbms start makes it explicit");
        let stopped = setup.run(ServiceCommand::Stop, &host).unwrap();
        assert_eq!(stopped.stopped_dbms_pid, None);
        assert!(host.alive(PG_PID), "cluster stop leaves the explicit one");
    }

    #[test]
    fn a_cancelled_cluster_start_stops_the_postgres_it_raised() {
        let setup = setup();
        let host = Host::default();
        let token = tokio_util::sync::CancellationToken::new();
        host.cancel_on_agent.replace(Some(token.clone()));
        let context = ExecutionContext::cli(CommandName::ClusterStart).with_cancellation(token);
        let error = setup
            .run_in(&context, ServiceCommand::Start, false, &host)
            .unwrap_err();
        assert!(
            matches!(error.kind(), UseCaseErrorKind::Cancelled(_)),
            "{error}"
        );
        for pid in [AGENT_PID, RAS_PID, RMNGR_PID, RPHOST_PID, PG_PID] {
            assert!(!host.alive(pid), "pid {pid} is left");
        }

        // Явный PostgreSQL отмена кластера не трогает.
        let host = Host::default();
        setup.dbms(ServiceCommand::Start, &host);
        let token = tokio_util::sync::CancellationToken::new();
        host.cancel_on_agent.replace(Some(token.clone()));
        let context = ExecutionContext::cli(CommandName::ClusterStart).with_cancellation(token);
        let error = setup
            .run_in(&context, ServiceCommand::Start, false, &host)
            .unwrap_err();
        assert!(
            matches!(error.kind(), UseCaseErrorKind::Cancelled(_)),
            "{error}"
        );
        assert!(host.alive(PG_PID), "the explicit one stays");
    }

    #[test]
    fn after_a_reboot_the_cluster_starts_again_with_its_postgres_as_a_step() {
        let setup = setup();
        let host = Host::default();
        let first = setup.run(ServiceCommand::Start, &host).unwrap();
        let port = setup.dbms_record().port;
        // Перезагрузка: процессов нет, записи остались.
        host.processes.borrow_mut().clear();
        host.listeners.borrow_mut().clear();
        let status = setup.run(ServiceCommand::Status, &host).unwrap();
        assert_eq!(status.state, ServiceState::NotRunning);
        assert!(status.stale_record);
        let second = setup.run(ServiceCommand::Start, &host).unwrap();
        assert_eq!(second.action, ServiceAction::Started);
        assert!(second.stale_record);
        assert_eq!(second.connection, first.connection, "the same regport");
        let record = setup.dbms_record();
        assert_eq!(record.port, port);
        assert!(!record.explicit, "this start raised it as its step");
        let stopped = setup.run(ServiceCommand::Stop, &host).unwrap();
        assert_eq!(stopped.stopped_dbms_pid, Some(PG_PID));
    }

    /// Удаление `workPath` стенда не трогает: СУБД, кластер с реестром и признак созданной базы
    /// лежат под `localPath`. Запись цела, `dbms status` видит свою СУБД, а `cluster start` после
    /// перезагрузки поднимает тот же кластер над той же базой
    /// (`INV.USE-CASES.DELETING-THE-WORKPATH-KEEPS-STANDS-AND-BASES`).
    #[test]
    fn deleting_the_work_path_keeps_the_stand_and_its_base() {
        let setup = setup();
        let host = Host::default();
        setup.dbms(ServiceCommand::Start, &host);
        let first = setup.run(ServiceCommand::Start, &host).unwrap();
        host.registered.borrow_mut().push("demo".to_owned());
        assert!(matches!(
            setup.base(&host),
            Ok(ClusterBase::AlreadyRegistered(_))
        ));
        let port = setup.dbms_record().port;
        std::fs::create_dir_all(setup.config.work_path.join("infobases").join("origin")).unwrap();
        std::fs::remove_dir_all(&setup.config.work_path).unwrap();

        let status = setup.dbms(ServiceCommand::Status, &host);
        assert_eq!(status.state, ServiceState::StartedByThisCopy);
        assert!(setup.pins().registered, "the base is still known created");
        assert!(setup.agent().own_record().is_some());

        host.processes.borrow_mut().clear();
        host.listeners.borrow_mut().clear();
        let second = setup.run(ServiceCommand::Start, &host).unwrap();
        assert_eq!(second.action, ServiceAction::Started);
        assert_eq!(second.connection, first.connection, "the same regport");
        assert!(second.registered, "the same base");
        assert_eq!(setup.dbms_record().port, port);
        assert!(setup
            .config
            .local_path
            .join("infobases")
            .join("origin")
            .join("managed")
            .join("postgres")
            .join("data")
            .is_dir());
    }

    #[test]
    fn after_a_reboot_the_command_that_raises_the_postgres_decides_whether_it_is_explicit() {
        // Явный до перезагрузки, после неё поднят кластером — шаг кластера.
        let setup = setup();
        let host = Host::default();
        setup.dbms(ServiceCommand::Start, &host);
        setup.run(ServiceCommand::Start, &host).unwrap();
        assert!(setup.dbms_record().explicit);
        host.processes.borrow_mut().clear();
        host.listeners.borrow_mut().clear();
        setup.run(ServiceCommand::Start, &host).unwrap();
        assert!(!setup.dbms_record().explicit, "cluster start raised it");
        let stopped = setup.run(ServiceCommand::Stop, &host).unwrap();
        assert_eq!(stopped.stopped_dbms_pid, Some(PG_PID));

        // Шаг до перезагрузки, после неё поднят `dbms start` — явный.
        drop(setup);
        let setup = self::setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        assert!(!setup.dbms_record().explicit);
        host.processes.borrow_mut().clear();
        host.listeners.borrow_mut().clear();
        setup.dbms(ServiceCommand::Start, &host);
        assert!(setup.dbms_record().explicit, "dbms start raised it");
        setup.run(ServiceCommand::Start, &host).unwrap();
        let stopped = setup.run(ServiceCommand::Stop, &host).unwrap();
        assert_eq!(stopped.stopped_dbms_pid, None);
        assert!(host.alive(PG_PID));
    }

    #[test]
    fn other_systems_refuse_the_cluster() {
        let setup = setup();
        let host = Host::default();
        host.unsupported.set(true);
        let refused = setup.run(ServiceCommand::Start, &host).unwrap_err();
        assert_eq!(
            refused.kind(),
            UseCaseErrorKind::Environment(EnvironmentReason::Unavailable)
        );
    }

    /// Ваша СУБД вместо PostgreSQL раннера.
    fn own_dbms(server: &str) -> InfobaseDbmsConfig {
        InfobaseDbmsConfig {
            kind: Some("PostgreSQL".to_owned()),
            server: Some(server.to_owned()),
            name: Some("shared".to_owned()),
            ..Default::default()
        }
    }

    /// Работающий свой кластер другой базы.
    fn running_cluster_of(setup: &Setup, host: &Host, infobase: &str) {
        let paths = ServicePaths::new(&setup.config.local_dir(), infobase, StandService::Cluster);
        paths.prepare().unwrap();
        host.live(9200, "ragent.exe");
        write_json(
            &paths.record(),
            &ServiceRecord {
                state: RecordState::Running,
                pid: Some(9200),
                started: Some(STARTED),
                image: Some("ragent.exe".to_owned()),
                machine: Some(MACHINE.to_owned()),
                starter: None,
                port: 1,
                explicit: true,
                bin: PathBuf::from("ragent.exe"),
                version: None,
                agent: None,
                location: paths.location(),
            },
        )
        .unwrap();
    }

    #[test]
    fn the_cluster_of_another_base_over_the_same_database_owns_it() {
        let mut setup = setup();
        let host = Host::default();
        setup.config.infobase.dbms = Some(own_dbms("DbHost"));
        let mut other = setup.config.infobase.clone();
        // Регистр и пробелы адреса не различаются.
        other.dbms = Some(own_dbms(" dbhost "));
        setup.config.infobases.insert("other".to_owned(), other);
        // Кластер этой базы поднят раньше; потом над той же БД поднялся кластер другой базы.
        setup.run(ServiceCommand::Start, &host).unwrap();
        running_cluster_of(&setup, &host, "other");
        let database = Database::declared(setup.config.infobase.dbms.as_ref().unwrap()).unwrap();
        assert_eq!(
            super::super::other_cluster_over(&setup.config, &host, &database).as_deref(),
            Some("other")
        );
        // Базу над той же БД этому кластеру не создать.
        let refused = setup.base(&host).err().expect("refused");
        assert!(refused.to_string().contains("'other'"), "{refused}");
        assert!(refused.to_string().contains("cluster stop"), "{refused}");
        // И заново его не поднять.
        setup.run(ServiceCommand::Stop, &host).unwrap();
        host.spawned.borrow_mut().clear();
        let refused = setup.run(ServiceCommand::Start, &host).unwrap_err();
        assert!(refused.to_string().contains("'other'"), "{refused}");
        assert!(host.spawned.borrow().is_empty());
    }

    #[test]
    fn a_foreign_agent_over_this_registry_on_other_ports_is_refused() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        setup.run(ServiceCommand::Stop, &host).unwrap();
        let pins = setup.pins();
        let registry = setup.agent().root().join(&pins.registry);
        host.foreign_agents.borrow_mut().push((
            9100,
            format!(
                "ragent.exe /agent /port 30000 /d \"{}\"",
                registry.display()
            ),
        ));
        host.spawned.borrow_mut().clear();
        let refused = setup.run(ServiceCommand::Start, &host).unwrap_err();
        assert!(refused.to_string().contains("ragent pid 9100"), "{refused}");
        assert!(host.spawned.borrow().is_empty());
    }

    #[test]
    fn a_move_whose_new_agent_does_not_stop_is_left_to_the_next_start() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        super::remember_registered(&setup.config).unwrap();
        setup.run(ServiceCommand::Stop, &host).unwrap();
        let old = setup.pins();
        host.listeners.borrow_mut().insert(old.ports.regport, 555);
        host.registration_fails.set(true);
        host.immortal.borrow_mut().insert(AGENT_PID);
        let refused = setup
            .run_with(ServiceCommand::Start, true, &host)
            .unwrap_err();
        assert!(refused.to_string().contains("roll back"), "{refused}");
        let moving = setup.pins();
        assert!(moving.previous.is_some(), "the move is still recorded");
        assert!(setup.agent().root().join(&moving.registry).is_dir());
        // `infobase create` такой кластер работающим не считает.
        assert!(setup.base(&host).is_err());
        host.immortal.borrow_mut().clear();
        host.registration_fails.set(false);
        host.listeners.borrow_mut().remove(&old.ports.regport);
        setup.run(ServiceCommand::Start, &host).unwrap();
        assert_eq!(setup.pins(), old, "the next start rolled the move back");
    }

    #[test]
    fn localhost_and_the_loopback_address_are_different_databases() {
        let mut setup = setup();
        let host = Host::default();
        setup.config.infobase.dbms = Some(own_dbms("localhost"));
        let mut other = setup.config.infobase.clone();
        other.dbms = Some(own_dbms("127.0.0.1"));
        setup.config.infobases.insert("other".to_owned(), other);
        running_cluster_of(&setup, &host, "other");
        let database = Database::declared(setup.config.infobase.dbms.as_ref().unwrap()).unwrap();
        assert_eq!(
            super::super::cluster_over(&setup.config, &host, &database),
            None
        );
    }

    #[test]
    fn a_running_cluster_refuses_the_standalone_start_over_the_same_database() {
        let setup = setup();
        let host = Host::default();
        setup.run(ServiceCommand::Start, &host).unwrap();
        let mut config = setup.config.clone();
        config.infobase.cluster = None;
        config.infobase.standalone = Some(crate::config::model::StandaloneConfig {
            managed: Some(ManagedServerConfig {
                name: Some("demo".to_owned()),
                db_path: None,
            }),
            gate: None,
            host_fingerprint: None,
            exchange: None,
        });
        host.spawned.borrow_mut().clear();
        let refused = super::super::standalone::run_standalone(
            &ExecutionContext::cli(CommandName::StandaloneStart),
            &config,
            ServiceCommand::Start,
            false,
            &Runner { host: &host },
            &host,
            &|| Ok(PathBuf::from("ibsrv.exe")),
        )
        .map_err(|failure| failure.error)
        .unwrap_err();
        assert!(refused.to_string().contains("cluster stop"), "{refused}");
        assert!(
            !host.spawned.borrow().iter().any(|tool| tool == "ibsrv"),
            "{:?}",
            host.spawned.borrow()
        );
    }

    #[test]
    fn a_cluster_without_a_name_takes_the_name_of_its_infobase() {
        let mut setup = setup();
        let host = Host::default();
        setup
            .config
            .infobase
            .cluster
            .as_mut()
            .unwrap()
            .managed
            .as_mut()
            .unwrap()
            .name = None;
        let started = setup.run(ServiceCommand::Start, &host).unwrap();
        assert!(
            started
                .connection
                .as_deref()
                .unwrap()
                .ends_with(";Ref=origin"),
            "{:?}",
            started.connection
        );
    }

    /// Стенд базы в общем `localPath` принадлежит другому проекту: команды кластера и
    /// `infobase create` в нём отказывают `workspace_busy`, называя тот проект, и ничего не
    /// поднимают (`INV.PLATFORM.A-SERVICE-IS-TOLD-APART-BY-ITS-IDENTITY`).
    #[test]
    fn the_cluster_and_its_base_refuse_the_stand_of_another_project() {
        let first = setup();
        ServicePaths::new(&first.config.local_dir(), "origin", StandService::Cluster)
            .prepare()
            .unwrap();
        let mut second = setup();
        second.config.local_path = first.config.local_path.clone();
        let host = Host::default();
        let project = first.config.base_path.display().to_string();
        for command in [
            ServiceCommand::Start,
            ServiceCommand::Status,
            ServiceCommand::Stop,
            ServiceCommand::Restart,
        ] {
            let refused = second.run(command, &host).unwrap_err();
            assert_eq!(refused.kind(), UseCaseErrorKind::WorkspaceBusy, "{refused}");
            assert!(
                refused.to_string().contains(&project),
                "{command:?}: {refused}"
            );
        }
        let refused = second.base(&host).err().unwrap();
        assert_eq!(refused.kind(), UseCaseErrorKind::WorkspaceBusy, "{refused}");
        assert!(refused.to_string().contains(&project), "{refused}");
        assert!(host.spawned.borrow().is_empty());
        assert!(!first.agent().pins().exists());
    }
}
