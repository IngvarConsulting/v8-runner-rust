//! Lifecycle of the private Apache belonging to a declared infobase.
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{
    await_gone, being_started, environment, forget_record, io_failure, locate, occupant_name,
    occupant_of, policy, port_offsets, refuse_a_stand_of_another_project, validation, write_record,
    RealHost, ServiceHost, IDENTIFY_TIMEOUT, POLL, STOP_TIMEOUT,
};
use crate::config::model::AppConfig;
use crate::domain::capability::TargetKind;
use crate::domain::dbms::{ServiceAction, ServiceState};
use crate::domain::web::WebResult;
use crate::platform::apache;
use crate::platform::locator::{platform_version_of, UtilityType};
use crate::platform::process::{ProcessExecutor, ProcessResult, ProcessRunner};
use crate::platform::service::{image_name, ProcessFacts};
use crate::platform::service_record::{
    observe_service, read_json, write_json, ClusterPins, Observed, PinnedPorts, RecordState,
    ServicePaths, ServiceRecord, StandService,
};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind, UseCaseFailure, UseCaseResult};

/// Системные сценарии управления Apache: они правят системную службу, а не частный экземпляр
/// (`INV.PLATFORM.A-MANAGED-APACHE-IS-PRIVATE`).
const SYSTEM_CONTROLS: [&str; 2] = ["apachectl", "apache2ctl"];

/// Команды записи `web`; `status` — отдельный вход без замка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebCommand {
    Start,
    Stop,
    Restart,
}

/// Кто поднимает Apache: явный экземпляр живёт до `web stop`, шаг кластера гаснет с ним.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ownership {
    Explicit,
    ClusterStep,
}

/// Что гасит остановка: `web stop` — свой экземпляр любого рода, `cluster stop` — только шаг.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StopScope {
    Any,
    ClusterStep,
}

/// Перезагрузка после `webinst`: обычная сверяет экземпляр со снятым до публикации,
/// восстановительная идёт после отката и не зависит от отмены команды.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reload {
    Activate,
    Recover,
}

/// Как экземпляр перечитывает конфиг: Linux — `SIGUSR1` и новые рабочие процессы,
/// Windows — остановка и запуск.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReloadStrategy {
    InPlace,
    Restart,
    Unsupported,
}

impl ReloadStrategy {
    const OF_THIS_SYSTEM: Self = if cfg!(windows) {
        Self::Restart
    } else if cfg!(target_os = "linux") {
        Self::InPlace
    } else {
        Self::Unsupported
    };
}

pub(super) struct Web<'a> {
    config: &'a AppConfig,
    context: &'a ExecutionContext,
    runner: &'a dyn ProcessRunner,
    host: &'a dyn ServiceHost,
    infobase: &'a str,
    paths: ServicePaths,
    started: Instant,
}

pub(super) fn declared(config: &AppConfig) -> bool {
    config.infobase.web.as_ref().is_some_and(|web| web.managed)
}

impl<'a> Web<'a> {
    pub(super) fn new(
        config: &'a AppConfig,
        context: &'a ExecutionContext,
        runner: &'a dyn ProcessRunner,
        host: &'a dyn ServiceHost,
    ) -> Result<Self, UseCaseError> {
        let infobase = config
            .infobase_name
            .as_deref()
            .ok_or_else(|| validation("web takes a declared infobase"))?;
        Ok(Self {
            config,
            context,
            runner,
            host,
            infobase,
            paths: ServicePaths::new(&config.local_dir(), infobase, StandService::Web),
            started: Instant::now(),
        })
    }

    pub(super) fn conf(&self) -> PathBuf {
        self.paths.root().join("httpd.conf")
    }

    fn pid_file(&self) -> PathBuf {
        self.paths.root().join("httpd.pid")
    }

    pub(super) fn observe(&self) -> Observed {
        let machine = self.host.machine();
        let facts = |pid| self.host.facts(pid);
        observe_service(&self.paths, machine.as_deref(), &facts, &|record| {
            if record.pid.is_some() {
                return crate::platform::service_record::adopt_recorded_pid(
                    record,
                    &facts,
                    machine.as_deref(),
                );
            }
            // Раннер упал между запуском Apache и записью PID: свой экземпляр узнаётся по
            // PID-файлу частного конфига, образу и порту записи.
            match self.identified_apache(record) {
                Some((pid, facts)) => {
                    record.pid = Some(pid);
                    record.started = Some(facts.started);
                    record.image = Some(facts.image);
                    true
                }
                None => false,
            }
        })
    }

    /// Живой Apache из PID-файла частного конфига: процесс с образом записи, начатый после
    /// упавшего раннера, слушает порт записи и запущен с этим конфигом.
    fn identified_apache(&self, record: &ServiceRecord) -> Option<(u32, ProcessFacts)> {
        let pid = std::fs::read_to_string(self.pid_file())
            .ok()?
            .trim()
            .parse()
            .ok()?;
        let facts = self.host.facts(pid)?;
        let image = image_name(&record.bin);
        let after_starter = record
            .starter
            .is_none_or(|(_, started)| facts.started >= started);
        let uses_config = self.host.processes(&[&image]).is_some_and(|rows| {
            crate::platform::service::process_using_config(&rows, &self.conf()) == Some(pid)
        });
        (facts.image == image
            && after_starter
            && self.host.listening_pid(record.port) == Some(pid)
            && uses_config)
            .then_some((pid, facts))
    }

    fn pinned_port(&self) -> Option<u16> {
        read_json::<PinnedPorts>(&self.paths.pins()).map(|pins| pins.port)
    }

    pub(super) fn result(&self, action: ServiceAction) -> WebResult {
        let mut result = WebResult {
            ok: true,
            infobase: self.infobase.to_owned(),
            state: ServiceState::NotRunning,
            action,
            ready: false,
            pid: None,
            explicit: None,
            address: None,
            program: None,
            config_path: self.conf(),
            log_path: self.paths.log(),
            occupant_pid: None,
            previous_port: None,
            publication_stale: self.publication_stale(),
            duration_ms: self.started.elapsed().as_millis() as u64,
        };
        match self.observe() {
            Observed::Own(record) => {
                result.state = ServiceState::StartedByThisCopy;
                result.pid = record.pid;
                result.explicit = Some(record.explicit);
                result.address = Some(address(record.port));
                result.program = Some(record.bin);
                result.ready =
                    record.pid.is_some() && self.host.listening_pid(record.port) == record.pid;
            }
            Observed::Starting(_) => result.state = ServiceState::Starting,
            Observed::Nothing | Observed::Stale => {
                if let Some(port) = self.pinned_port() {
                    result.address = Some(address(port));
                    if let Some(pid) = occupant_of(self.host, port) {
                        result.state = ServiceState::Foreign;
                        result.occupant_pid = (pid != 0).then_some(pid);
                    }
                }
            }
        }
        result
    }

    fn publication_stale(&self) -> bool {
        if self.config.target_kind() == TargetKind::File {
            return false;
        }
        let descriptor = self.paths.root().join("publication/default.vrd");
        let Ok(descriptor) = std::fs::read_to_string(descriptor) else {
            return false;
        };
        let cluster = self.paths.sibling(StandService::Cluster);
        let pins = read_json::<ClusterPins>(&cluster.pins());
        pins.is_some_and(|pins| {
            let authority = format!("127.0.0.1:{}", pins.ports.regport);
            !descriptor.match_indices(&authority).any(|(at, _)| {
                descriptor
                    .as_bytes()
                    .get(at + authority.len())
                    .is_some_and(|byte| matches!(byte, b';' | b'"' | b'&'))
            })
        })
    }

    fn program(&self) -> Result<PathBuf, UseCaseError> {
        let program = apache::locate(self.config.tools.apache.path.as_deref()).ok_or_else(|| environment(
            "Apache 2.4 was not found: set tools.apache.path to an installed httpd or apache2; use Apache Lounge on Windows or the Apache package of your Linux distribution",
        ))?;
        let stem = program
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if SYSTEM_CONTROLS.contains(&stem.as_str()) {
            return Err(validation(format!(
                "tools.apache.path names the system control script '{}': the runner starts its own Apache, so name the httpd or apache2 executable",
                program.display()
            )));
        }
        Ok(program)
    }

    fn check_module(&self, platform_tool: &Path) -> Result<(), UseCaseError> {
        let module = apache::platform_module(platform_tool);
        let expected = apache::module_directive(&module)
            .map_err(|err| io_failure("cannot represent", &module, &err))?;
        let path = self.conf();
        let conf =
            std::fs::read_to_string(&path).map_err(|err| io_failure("cannot read", &path, &err))?;
        if !conf.lines().any(|line| line == expected) {
            return Err(validation("running Apache uses a different 1C module: stop it and prepare the stand for the selected release"));
        }
        Ok(())
    }

    fn foreign_config(&self, program: &Path) -> Result<(), UseCaseError> {
        let image = image_name(program);
        let images = [image.as_str(), "apache2", "httpd", "httpd.exe"];
        let processes = self.host.processes(&images).ok_or_else(|| {
            UseCaseError::new(
                UseCaseErrorKind::WorkspaceBusy,
                "cannot inspect Apache processes to establish ownership",
            )
        })?;
        let config_user = crate::platform::service::process_using_config(&processes, &self.conf());
        let pidfile_user = std::fs::read_to_string(self.pid_file())
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .filter(|&pid| {
                self.host
                    .facts(pid)
                    .is_some_and(|facts| images.contains(&facts.image.as_str()))
            });
        if let Some(pid) = config_user.or(pidfile_user) {
            return Err(UseCaseError::new(
                UseCaseErrorKind::WorkspaceBusy,
                format!(
                    "Apache process {pid} is already using '{}'",
                    self.conf().display()
                ),
            ));
        }
        Ok(())
    }

    /// Превью запуска: программа и адрес, ничего не записано и не запущено.
    pub(super) fn plan_start(&self) -> Result<WebResult, UseCaseError> {
        self.refuse_undeclared()?;
        let mut result = self.result(ServiceAction::Planned);
        if result.program.is_none() {
            result.program = Some(self.program()?);
        }
        if result.address.is_none() {
            result.address = Some(address(self.choose_port()?));
        }
        Ok(result)
    }

    fn refuse_undeclared(&self) -> Result<(), UseCaseError> {
        if declared(self.config) {
            Ok(())
        } else {
            Err(validation(
                "declare infobase.web.managed for a local file infobase or managed cluster",
            ))
        }
    }

    pub(super) fn start(&self, ownership: Ownership) -> Result<WebResult, UseCaseError> {
        self.refuse_undeclared()?;
        let observed = self.observe();
        if !self.host.supported() || !self.host.can_start_as_current_user() {
            return Err(environment(
                "start managed Apache on Windows or as an unprivileged user on Linux",
            ));
        }
        if let Observed::Own(mut record) = observed {
            if self.config.target_kind() == TargetKind::File {
                self.check_module(&locate(self.config, UtilityType::Webinst)?)?;
            } else if let Some(cluster) = self.paths.sibling(StandService::Cluster).own_record() {
                self.check_module(&cluster.bin)?;
            }
            let mut result = self.result(ServiceAction::AlreadyRunning);
            if ownership == Ownership::Explicit {
                if !result.ready {
                    return Err(environment(
                        "the managed Apache is not listening on its pinned port",
                    ));
                }
                if !record.explicit {
                    record.explicit = true;
                    write_record(&self.paths, &record)?;
                    result.explicit = Some(true);
                }
            }
            return Ok(result);
        }
        if matches!(observed, Observed::Starting(_)) {
            return Err(being_started("Apache", self.infobase));
        }
        let machine = self
            .host
            .machine()
            .ok_or_else(|| environment("no machine identity for Apache"))?;
        let program = self.program()?;
        self.foreign_config(&program)?;
        let platform_tool = if self.config.target_kind() == TargetKind::File {
            locate(self.config, UtilityType::Webinst)?
        } else {
            let cluster = self.paths.sibling(StandService::Cluster);
            match cluster.own_record() {
                Some(record) => record.bin,
                None => locate(self.config, UtilityType::Ragent)?,
            }
        };
        let module = apache::platform_module(&platform_tool);
        if !module.is_file() {
            return Err(environment(format!(
                "1C Apache module is missing: {}",
                module.display()
            )));
        }
        let previous_port = self.pinned_port();
        let port = self.choose_port()?;
        self.paths
            .prepare()
            .map_err(|err| io_failure("cannot prepare", self.paths.root(), &err))?;
        let root = self.paths.root();
        let publication = root.join("publication");
        std::fs::create_dir_all(&publication)
            .map_err(|err| io_failure("cannot create", &publication, &err))?;
        let htdocs = root.join("htdocs");
        std::fs::create_dir_all(&htdocs)
            .map_err(|err| io_failure("cannot create", &htdocs, &err))?;
        let conf = self.conf();
        let rendered = apache::config(&program, &module, root, port)
            .map_err(|err| io_failure("cannot generate", &conf, &err))?;
        let previous = match std::fs::read_to_string(&conf) {
            Ok(previous) => Some(previous),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(io_failure("cannot read", &conf, &error)),
        };
        let content = match &previous {
            None => rendered,
            Some(previous) => apache::reconfigure(previous, &rendered)
                .map_err(|err| validation(err.to_string()))?,
        };
        self.replace_config(&conf, &content)?;
        let checked = self.check(&program).and_then(|()| {
            write_json(&self.paths.pins(), &PinnedPorts { port })
                .map_err(|err| io_failure("cannot save Apache port", &self.paths.pins(), &err))
        });
        if let Err(error) = checked {
            let rollback = match previous {
                Some(previous) => self.replace_config(&conf, &previous),
                None => std::fs::remove_file(&conf)
                    .map_err(|err| io_failure("cannot remove", &conf, &err)),
            };
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback) => error.followed_by(rollback.to_string()),
            });
        }
        let mut record = ServiceRecord {
            state: RecordState::Starting,
            pid: None,
            started: None,
            image: None,
            machine: Some(machine),
            starter: self
                .host
                .facts(std::process::id())
                .map(|facts| (std::process::id(), facts.started)),
            port,
            explicit: ownership == Ownership::Explicit,
            bin: program,
            version: None,
            agent: None,
            location: self.paths.location(),
        };
        write_record(&self.paths, &record)?;
        let request = apache::foreground(&record.bin, &conf, &self.paths.log());
        let pid = match self.host.spawn(&request, self.context.work()) {
            Ok(pid) => pid,
            Err(error) => {
                forget_record(&self.paths);
                return Err(environment(format!("cannot start Apache: {error}")));
            }
        };
        let deadline = Instant::now() + self.host.patience(IDENTIFY_TIMEOUT);
        let expected_image = image_name(&record.bin);
        let outcome = loop {
            let facts = self
                .host
                .facts(pid)
                .filter(|facts| facts.image == expected_image);
            if record.started.is_none() {
                if let Some(facts) = &facts {
                    record.pid = Some(pid);
                    record.started = Some(facts.started);
                    record.image = Some(facts.image.clone());
                    if let Err(error) = write_record(&self.paths, &record) {
                        break Err(error);
                    }
                }
            }
            if let Some(error) =
                super::interruption_before_safe_point(self.context, "starting Apache")
            {
                break Err(error.into());
            }
            if record.started.is_some()
                && facts.is_some_and(|facts| Some(facts.started) == record.started)
                && self.host.listening_pid(port) == Some(pid)
            {
                record.state = RecordState::Running;
                record.starter = None;
                break write_record(&self.paths, &record);
            }
            if Instant::now() >= deadline {
                break Err(environment(format!(
                    "Apache did not listen on 127.0.0.1:{port}; see {}",
                    self.paths.log().display()
                )));
            }
            self.host.sleep(POLL);
        };
        match outcome.and_then(|()| {
            let mut result = self.result(ServiceAction::Started);
            if ownership == Ownership::Explicit && !result.ready {
                return Err(environment(
                    "the managed Apache stopped listening during startup",
                ));
            }
            result.previous_port = previous_port.filter(|previous| *previous != port);
            Ok(result)
        }) {
            Ok(result) => Ok(result),
            Err(error) => {
                if record.started.is_some() {
                    self.host
                        .hard_stop(self.runner, pid, record.started, self.context);
                    if !await_gone(
                        self.host,
                        pid,
                        record.started,
                        self.host.patience(IDENTIFY_TIMEOUT),
                    ) {
                        record.state = RecordState::Running;
                        record.starter = None;
                        write_record(&self.paths, &record)?;
                        return Err(error.followed_by(format!("Apache PID {pid} did not stop")));
                    }
                }
                forget_record(&self.paths);
                Err(error)
            }
        }
    }

    fn replace_config(&self, path: &Path, contents: &str) -> Result<(), UseCaseError> {
        use std::io::Write as _;
        if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(validation(
                "private Apache config must not be a symbolic link",
            ));
        }
        crate::support::fs::write_file_atomically(path, |file| file.write_all(contents.as_bytes()))
            .map_err(|err| io_failure("cannot write", path, &err))
    }

    fn choose_port(&self) -> Result<u16, UseCaseError> {
        let pg = self.paths.sibling(StandService::Postgres);
        let pg_port = read_json::<PinnedPorts>(&pg.pins()).map(|value| value.port);
        let cluster = self.paths.sibling(StandService::Cluster);
        let cluster_ports = read_json::<ClusterPins>(&cluster.pins());
        self.pinned_port()
            .into_iter()
            .chain(port_offsets(self.paths.root()))
            .find(|&port| {
                Some(port) != pg_port
                    && !super::dynamic_ports().contains(&port)
                    && cluster_ports.as_ref().is_none_or(|pins| {
                        // Порт сервера отладки кластера (`cluster start --debug`) в блок не
                        // входит, но Apache его занимать не должен.
                        !pins.ports.all().any(|value| value == port) && pins.ports.debug() != port
                    })
                    && occupant_of(self.host, port).is_none()
            })
            .ok_or_else(|| environment("no free stand port for Apache"))
    }

    fn check(&self, program: &Path) -> Result<(), UseCaseError> {
        let request = apache::check(program, &self.conf());
        let answer = self
            .runner
            .run_with_policy(&request, &policy(self.context))
            .map_err(|err| environment(format!("cannot check Apache configuration: {err}")))?;
        if answer.outcome().is_err() {
            return Err(environment(format!(
                "invalid Apache configuration: {}",
                answer.stderr
            )));
        }
        Ok(())
    }

    /// Превью остановки: ничего не останавливается.
    pub(super) fn plan_stop(&self) -> WebResult {
        self.result(ServiceAction::Planned)
    }

    pub(super) fn stop(&self, scope: StopScope) -> Result<WebResult, UseCaseError> {
        let Observed::Own(record) = self.observe() else {
            let result = self.result(ServiceAction::NothingToStop);
            if scope == StopScope::Any && result.state == ServiceState::Foreign {
                return Err(UseCaseError::new(
                    UseCaseErrorKind::WorkspaceBusy,
                    format!(
                        "Apache port is held by {}",
                        occupant_name(result.occupant_pid.unwrap_or_default())
                    ),
                ));
            }
            return Ok(result);
        };
        if scope == StopScope::ClusterStep && record.explicit {
            return Ok(self.result(ServiceAction::Reported));
        }
        let pid = record
            .pid
            .ok_or_else(|| environment("Apache has no process identity"))?;
        self.host
            .soft_stop(self.runner, pid, record.started, self.context);
        if !await_gone(
            self.host,
            pid,
            record.started,
            self.host.patience(IDENTIFY_TIMEOUT),
        ) {
            self.host
                .hard_stop(self.runner, pid, record.started, self.context);
            if !await_gone(
                self.host,
                pid,
                record.started,
                self.host.patience(STOP_TIMEOUT),
            ) {
                return Err(environment(format!("Apache process {pid} did not stop")));
            }
        }
        forget_record(&self.paths);
        Ok(self.result(ServiceAction::Stopped))
    }

    fn restart(&self) -> Result<WebResult, UseCaseError> {
        let ownership = match self.observe() {
            Observed::Own(record) if !record.explicit => Ownership::ClusterStep,
            _ => Ownership::Explicit,
        };
        self.stop(StopScope::Any)?;
        self.start(ownership)
    }
}

fn address(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// `web status`: читает запись без замка (`INV.USE-CASES.A-SERVICE-STATUS-TAKES-NO-LOCK`).
pub fn web_status(context: &ExecutionContext, config: &AppConfig) -> UseCaseResult<WebResult> {
    refuse_a_stand_of_another_project(config).map_err(UseCaseFailure::without_payload)?;
    let web = Web::new(config, context, &ProcessExecutor, &RealHost::CURRENT)
        .map_err(UseCaseFailure::without_payload)?;
    Ok(web.result(ServiceAction::Reported))
}

/// `web start|stop|restart`: команда записи, под замком `workPath`.
pub fn execute_web(
    context: &ExecutionContext,
    config: &AppConfig,
    command: WebCommand,
    dry_run: bool,
) -> UseCaseResult<WebResult> {
    run_web(
        context,
        config,
        command,
        dry_run,
        &ProcessExecutor,
        &RealHost::CURRENT,
    )
}

fn run_web(
    context: &ExecutionContext,
    config: &AppConfig,
    command: WebCommand,
    dry_run: bool,
    runner: &dyn ProcessRunner,
    host: &dyn ServiceHost,
) -> UseCaseResult<WebResult> {
    refuse_a_stand_of_another_project(config).map_err(UseCaseFailure::without_payload)?;
    let web = Web::new(config, context, runner, host).map_err(UseCaseFailure::without_payload)?;
    let outcome = match (command, dry_run) {
        (WebCommand::Start | WebCommand::Restart, true) => web.plan_start(),
        (WebCommand::Stop, true) => Ok(web.plan_stop()),
        (WebCommand::Start, false) => web.start(Ownership::Explicit),
        (WebCommand::Stop, false) => web.stop(StopScope::Any),
        (WebCommand::Restart, false) => web.restart(),
    };
    outcome.map_err(|error| {
        let mut result = web.result(ServiceAction::Reported);
        result.ok = false;
        UseCaseFailure::with_payload(error, result)
    })
}

/// Публикация базы управляемого кластера в его частный Apache: куда, чем и по какому адресу.
pub(crate) struct ManagedPublication<'a> {
    web: Web<'a>,
    /// Экземпляр, снятый до публикации; в превью Apache может не работать.
    record: Option<ServiceRecord>,
    pub(crate) target: crate::config::model::InfobaseWebConfig,
    pub(crate) webinst: PathBuf,
    pub(crate) connection: String,
    pub(crate) log_root: PathBuf,
    /// `sudo`, которым поднимаются права одного `webinst`: на Linux он публикует только от
    /// root (замер #610). `None` — права не нужны.
    elevation: Option<PathBuf>,
}

/// Управляемая публикация объявленной базы; `None` — у базы своя секция `infobase.web`.
pub(crate) fn publication<'a>(
    context: &'a ExecutionContext,
    config: &'a AppConfig,
    action: crate::domain::publish::PublishAction,
    dry_run: bool,
) -> Result<Option<ManagedPublication<'a>>, UseCaseError> {
    publication_with(
        context,
        config,
        action,
        dry_run,
        &ProcessExecutor,
        &RealHost::CURRENT,
    )
}

pub(crate) fn publication_with<'a>(
    context: &'a ExecutionContext,
    config: &'a AppConfig,
    action: crate::domain::publish::PublishAction,
    dry_run: bool,
    runner: &'a dyn ProcessRunner,
    host: &'a dyn ServiceHost,
) -> Result<Option<ManagedPublication<'a>>, UseCaseError> {
    if !declared(config) {
        return Ok(None);
    }
    refuse_a_stand_of_another_project(config)?;
    // Публикация идёт от обычного пользователя: права поднимаются только у `webinst`, и файлы
    // под `localPath` root не достаются (`INV.PLATFORM.A-MANAGED-APACHE-IS-PRIVATE`).
    if !host.can_start_as_current_user() {
        return Err(environment(
            "run publish as an unprivileged user: on Linux the runner elevates only webinst, with sudo -n",
        ));
    }
    let web = Web::new(config, context, runner, host)?;
    let record = match web.observe() {
        Observed::Own(record)
            if record.pid.is_some() && host.listening_pid(record.port) == record.pid =>
        {
            Some(record)
        }
        Observed::Own(_) if !dry_run => {
            return Err(environment("the managed Apache is not ready"));
        }
        _ if !dry_run => {
            return Err(validation(
                "start the managed Apache with cluster start or web start",
            ));
        }
        _ => None,
    };
    let (webinst, connection) = if config.target_kind() == TargetKind::File {
        let path = config
            .v8_connection()
            .file_infobase_dir(&config.base_path)
            .ok_or_else(|| validation("missing file infobase path"))?;
        if path
            .to_str()
            .is_none_or(|value| crate::platform::connection::control_character(value).is_some())
        {
            return Err(validation("cannot represent file infobase address"));
        }
        let connection = crate::platform::connection::V8Connection::for_file(&path)
            .create_infobase_arg()
            .ok_or_else(|| validation("cannot represent file infobase address"))?;
        let webinst = locate(config, UtilityType::Webinst)?;
        if !dry_run {
            web.check_module(&webinst)?;
        }
        (webinst, connection)
    } else {
        cluster_publication_target(&web, action, dry_run)?
    };
    let log_root = web.paths.root().to_owned();
    let mut target = config
        .infobase
        .web
        .clone()
        .ok_or_else(|| validation("missing web configuration"))?;
    let wsdir = target
        .wsdir
        .as_deref()
        .ok_or_else(|| validation("infobase.web.wsdir is required"))?;
    let port = match &record {
        Some(record) => record.port,
        None => match web.pinned_port() {
            Some(port) => port,
            None => web.choose_port()?,
        },
    };
    target.dir = Some(web.paths.root().join("publication"));
    target.conf = Some(web.conf());
    target.url = Some(format!("{}/{wsdir}/", address(port)));
    target.server = Some(crate::config::model::WebServerKind::Apache24);
    let elevation = host.webinst_elevation()?;
    Ok(Some(ManagedPublication {
        web,
        record,
        target,
        webinst,
        connection,
        log_root,
        elevation,
    }))
}

fn cluster_publication_target(
    web: &Web<'_>,
    action: crate::domain::publish::PublishAction,
    dry_run: bool,
) -> Result<(PathBuf, String), UseCaseError> {
    let config = web.config;
    let host = web.host;
    let cluster = web.paths.sibling(StandService::Cluster);
    let cluster_record = if crate::platform::service_record::record_names_own_process(
        &cluster,
        host.machine().as_deref(),
        &|pid| host.facts(pid),
    ) {
        cluster.own_record()
    } else {
        None
    };
    if !dry_run {
        let running = cluster_record
            .as_ref()
            .ok_or_else(|| validation("start the managed cluster before publishing"))?;
        web.check_module(&running.bin)?;
    }
    let pins = read_json::<ClusterPins>(&cluster.pins())
        .filter(|pins| {
            pins.previous.is_none()
                && (pins.registered || action == crate::domain::publish::PublishAction::Delete)
        })
        .ok_or_else(|| {
            validation("create the infobase in the running cluster before publishing")
        })?;
    let managed = config
        .infobase
        .cluster
        .as_ref()
        .and_then(|cluster| cluster.managed.as_ref())
        .ok_or_else(|| validation("publish needs a managed cluster"))?;
    let connection = pins.ports.connection(&managed.reference(web.infobase));
    let webinst = cluster_webinst(config, cluster_record.as_ref())?;
    Ok((webinst, connection))
}

/// `webinst` берёт поиск утилит, а не запись кластера под `localPath`: путь из правимого
/// пользователем файла с поднятыми правами не запускается. Выпуск найденного обязан совпасть с
/// выпуском работающего кластера.
fn cluster_webinst(
    config: &AppConfig,
    cluster: Option<&ServiceRecord>,
) -> Result<PathBuf, UseCaseError> {
    let webinst = locate(config, UtilityType::Webinst)?;
    let Some(cluster) = cluster else {
        return Ok(webinst);
    };
    let same_release = match (
        platform_version_of(UtilityType::Webinst, &webinst),
        platform_version_of(UtilityType::Ragent, &cluster.bin),
    ) {
        (Some(found), Some(running)) => found == running,
        _ => {
            crate::support::path::compared_path(webinst.parent().unwrap_or(&webinst))
                == crate::support::path::compared_path(cluster.bin.parent().unwrap_or(&cluster.bin))
        }
    };
    if !same_release {
        return Err(environment(format!(
            "webinst '{}' is not from the release of the running cluster '{}': set tools.platform.version to the cluster release",
            webinst.display(),
            cluster.bin.display()
        )));
    }
    Ok(webinst)
}

pub(crate) struct PublicationSnapshot {
    conf: PathBuf,
    config: FileSnapshot,
    vrd: PathBuf,
    descriptor: Option<FileSnapshot>,
}

struct FileSnapshot {
    contents: Vec<u8>,
    permissions: std::fs::Permissions,
}

impl FileSnapshot {
    fn read(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(std::io::Error::other("publication requires a regular file"));
        }
        Ok(Self {
            contents: std::fs::read(path)?,
            permissions: metadata.permissions(),
        })
    }

    /// Возвращает содержимое и права. Файл пишет раннер, поэтому владелец — пользователь
    /// раннера, даже если `webinst` с поднятыми правами успел заменить файл своим.
    fn restore(self, path: &Path) -> std::io::Result<()> {
        use std::io::Write as _;
        crate::support::fs::write_file_atomically(path, |file| {
            file.set_permissions(self.permissions)?;
            file.write_all(&self.contents)
        })
    }
}

impl ManagedPublication<'_> {
    /// Исполнитель `webinst` и перезагрузки: тот же, что у стенда.
    pub(crate) fn runner(&self) -> &dyn ProcessRunner {
        self.web.runner
    }

    /// Программа и ключи запуска `webinst`: с `sudo -n` права поднимаются только у него.
    pub(crate) fn command(&self, args: Vec<String>) -> (PathBuf, Vec<String>) {
        match &self.elevation {
            Some(sudo) => {
                let mut elevated = vec![
                    "-n".to_owned(),
                    "--".to_owned(),
                    self.webinst.display().to_string(),
                ];
                elevated.extend(args);
                (sudo.clone(), elevated)
            }
            None => (self.webinst.clone(), args),
        }
    }

    /// Отказ `sudo -n` поднять права: `webinst` не запускался.
    pub(crate) fn elevation_refusal(&self, finished: &ProcessResult) -> Option<UseCaseError> {
        self.elevation.as_ref()?;
        (finished.outcome().is_err()
            && finished
                .stderr
                .lines()
                .any(|line| line.trim_start().starts_with("sudo:")))
        .then(|| {
            environment(format!(
                "sudo -n did not run webinst: on Linux webinst publishes only as root, so a sudoers rule for it is effectively root for this user (its -dir and -confpath lie under localPath, which the user controls); grant it only to a user who may have root anyway, e.g. on a personal dev machine, for exactly the command line that `publish --dry-run` shows after `sudo -n --` (`{}` with these arguments); sudo said: {}",
                self.webinst.display(),
                finished.stderr.trim()
            ))
        })
    }

    pub(crate) fn snapshot(&self) -> Result<PublicationSnapshot, UseCaseError> {
        let conf = self.web.conf();
        let dir = self.web.paths.root().join("publication");
        let backup = self.web.paths.root().join("httpd.conf.bak");
        for path in [self.web.paths.root(), &conf, &dir, &backup] {
            if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err(validation(format!(
                    "publication path must not be a symbolic link: {}",
                    path.display()
                )));
            }
        }
        if !dir.is_dir() {
            return Err(validation(
                "managed publication directory is missing: run web restart",
            ));
        }
        let vrd = dir.join("default.vrd");
        if std::fs::symlink_metadata(&vrd).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(validation("default.vrd must not be a symbolic link"));
        }
        let config =
            FileSnapshot::read(&conf).map_err(|err| io_failure("cannot read", &conf, &err))?;
        let descriptor = match FileSnapshot::read(&vrd) {
            Ok(bytes) => Some(bytes),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(io_failure("cannot read", &vrd, &err)),
        };
        Ok(PublicationSnapshot {
            conf,
            config,
            vrd,
            descriptor,
        })
    }

    /// Файлы, которые `webinst` с поднятыми правами записал под каталогом Apache, снова
    /// принадлежат пользователю раннера. Зовётся и после удачи, и после отката.
    pub(crate) fn reclaim_files(&self) -> Result<(), UseCaseError> {
        if self.elevation.is_none() {
            return Ok(());
        }
        let root = self.web.paths.root();
        reclaim(root, &|path| self.web.host.file_owner(path))
            .map_err(|err| io_failure("cannot take back", root, &err))
    }

    pub(crate) fn reload(&self, mode: Reload) -> Result<(), UseCaseError> {
        self.reload_with(mode, ReloadStrategy::OF_THIS_SYSTEM)
    }

    fn reload_with(&self, mode: Reload, strategy: ReloadStrategy) -> Result<(), UseCaseError> {
        match mode {
            Reload::Activate => self.reload_web(&self.web, mode, strategy),
            Reload::Recover => {
                // Откат идёт и после отмены команды: без перезагрузки Apache остался бы на
                // отменённом конфиге.
                let context = self
                    .web
                    .context
                    .clone()
                    .with_cancellation(tokio_util::sync::CancellationToken::new());
                let web = Web::new(self.web.config, &context, self.web.runner, self.web.host)?;
                self.reload_web(&web, mode, strategy)
            }
        }
    }

    fn reload_web(
        &self,
        web: &Web<'_>,
        mode: Reload,
        strategy: ReloadStrategy,
    ) -> Result<(), UseCaseError> {
        let explicit = self.record.as_ref().is_some_and(|record| record.explicit);
        let ownership = if explicit {
            Ownership::Explicit
        } else {
            Ownership::ClusterStep
        };
        let Observed::Own(current) = web.observe() else {
            if mode == Reload::Recover && strategy == ReloadStrategy::Restart {
                web.start(ownership)?;
                return Ok(());
            }
            return Err(environment(
                "Apache stopped before publication could be activated",
            ));
        };
        let unchanged = self
            .record
            .as_ref()
            .is_some_and(|record| current.pid == record.pid && current.started == record.started);
        if mode == Reload::Activate && !unchanged {
            return Err(environment(
                "Apache changed before publication could be activated",
            ));
        }
        web.check(&current.bin)?;
        match strategy {
            ReloadStrategy::InPlace => {
                let pid = current
                    .pid
                    .ok_or_else(|| environment("Apache has no process identity"))?;
                let image = image_name(&current.bin);
                let before = apache_workers(web.host, &image, pid)?;
                web.host
                    .graceful_reload(pid, current.started)
                    .map_err(|err| environment(format!("cannot reload Apache: {err}")))?;
                let deadline = Instant::now() + web.host.patience(IDENTIFY_TIMEOUT);
                loop {
                    if web
                        .host
                        .facts(pid)
                        .is_none_or(|facts| Some(facts.started) != current.started)
                    {
                        return Err(environment("Apache exited during reload"));
                    }
                    if apache_workers(web.host, &image, pid)?
                        .iter()
                        .any(|worker| !before.contains(worker))
                        && web.host.listening_pid(current.port) == Some(pid)
                    {
                        return Ok(());
                    }
                    if Instant::now() >= deadline {
                        return Err(environment(
                            "Apache did not activate the changed configuration",
                        ));
                    }
                    web.host.sleep(POLL);
                }
            }
            ReloadStrategy::Restart => {
                web.stop(StopScope::Any)?;
                web.start(if current.explicit {
                    Ownership::Explicit
                } else {
                    Ownership::ClusterStep
                })?;
                Ok(())
            }
            ReloadStrategy::Unsupported => Err(environment(
                "managed Apache reload is supported on Windows and Linux",
            )),
        }
    }
}

/// Рабочие процессы Apache: дети главного процесса с его образом.
fn apache_workers(
    host: &dyn ServiceHost,
    image: &str,
    parent: u32,
) -> Result<Vec<(u32, u64)>, UseCaseError> {
    let rows = host
        .processes(&[image])
        .ok_or_else(|| environment("cannot inspect the Apache children after reload"))?;
    Ok(rows
        .into_iter()
        .filter(|row| row.parent == parent)
        .filter_map(|row| host.facts(row.pid).map(|facts| (row.pid, facts.started)))
        .collect())
}

/// Файлы под каталогом Apache чужого владельца — их записал `webinst` с поднятыми правами —
/// снова принадлежат владельцу каталога: раннер переписывает их от своего имени с прежними
/// правами. Символические ссылки не трогаются. Каталог чужого владельца раннеру не переписать —
/// отказ называет путь и команду, которой его убрать. `owner` — владелец пути; `None` у корня —
/// система владельцев не называет, забирать нечего.
fn reclaim(root: &Path, owner: &dyn Fn(&Path) -> Option<u32>) -> std::io::Result<()> {
    use std::io::Write as _;
    let Some(ours) = owner(root) else {
        return Ok(());
    };
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let path = entry?.path();
            let meta = std::fs::symlink_metadata(&path)?;
            if meta.file_type().is_symlink() {
                continue;
            }
            let theirs = owner(&path);
            if meta.is_dir() {
                if theirs != Some(ours) {
                    let owner = theirs
                        .map_or_else(|| "another user".to_owned(), |uid| format!("uid {uid}"));
                    return Err(std::io::Error::other(format!(
                        "webinst left the directory '{}' owned by {owner}, which the runner cannot take back: remove it with `sudo rm -r {}` and publish again",
                        path.display(),
                        shell_quoted(&path)
                    )));
                }
                pending.push(path);
            } else if theirs != Some(ours) {
                let contents = std::fs::read(&path)?;
                crate::support::fs::write_file_atomically(&path, |file| file.write_all(&contents))?;
            }
        }
    }
    Ok(())
}

/// Путь в одинарных кавычках POSIX-оболочки: кавычка внутри закрывается, экранируется и
/// открывается снова.
fn shell_quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
}

impl PublicationSnapshot {
    pub(crate) fn restore(self) -> Result<(), UseCaseError> {
        let config = self
            .config
            .restore(&self.conf)
            .map_err(|err| io_failure("cannot restore", &self.conf, &err));
        let descriptor = match self.descriptor {
            Some(file) => file.restore(&self.vrd),
            None => match std::fs::remove_file(&self.vrd) {
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                result => result,
            },
        }
        .map_err(|err| io_failure("cannot restore", &self.vrd, &err));
        match (config, descriptor) {
            (Err(config), Err(descriptor)) => Err(config.followed_by(descriptor.to_string())),
            (config, descriptor) => config.and(descriptor),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Владелец в подделке: пути из `foreign` принадлежат root, остальное — пользователю.
    fn owners(foreign: &HashSet<PathBuf>) -> impl Fn(&Path) -> Option<u32> + '_ {
        move |path| Some(if foreign.contains(path) { 0 } else { 1000 })
    }

    #[test]
    fn reclaim_rewrites_a_foreign_file_as_the_runner_with_its_content() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("web");
        std::fs::create_dir_all(root.join("publication")).unwrap();
        let file = root.join("publication").join("default.vrd");
        std::fs::write(&file, "descriptor").unwrap();
        let ours = root.join("httpd.conf");
        std::fs::write(&ours, "config").unwrap();
        // Жёсткие ссылки вне каталога видят, заменён ли файл новым.
        let alias = directory.path().join("vrd.alias");
        std::fs::hard_link(&file, &alias).unwrap();
        let ours_alias = directory.path().join("conf.alias");
        std::fs::hard_link(&ours, &ours_alias).unwrap();
        let foreign = HashSet::from([file.clone()]);

        reclaim(&root, &owners(&foreign)).unwrap();

        std::fs::write(&alias, "changed").unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "descriptor",
            "the foreign file is replaced by the runner's copy"
        );
        std::fs::write(&ours_alias, "changed").unwrap();
        assert_eq!(
            std::fs::read_to_string(&ours).unwrap(),
            "changed",
            "the runner's own file is left as it is"
        );
    }

    #[test]
    fn reclaim_refuses_a_foreign_directory_and_names_the_command_to_remove_it() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("web");
        let foreign_dir = root.join("publication").join("cache");
        std::fs::create_dir_all(&foreign_dir).unwrap();
        let foreign = HashSet::from([foreign_dir.clone()]);

        let refused = reclaim(&root, &owners(&foreign)).unwrap_err().to_string();
        assert!(refused.contains("sudo rm -r"), "{refused}");
        assert!(
            refused.contains(&foreign_dir.display().to_string()),
            "{refused}"
        );
        assert!(foreign_dir.is_dir(), "nothing is removed");
    }

    #[test]
    fn a_path_for_the_shell_keeps_its_quotes_inside_single_quotes() {
        assert_eq!(shell_quoted(Path::new("/a b/c")), "'/a b/c'");
        assert_eq!(shell_quoted(Path::new("/it's")), r"'/it'\''s'");
    }

    #[test]
    fn reclaim_without_owners_changes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("file"), "text").unwrap();
        reclaim(directory.path(), &|_| None).unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.path().join("file")).unwrap(),
            "text"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reclaim_leaves_symbolic_links_alone() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("web");
        std::fs::create_dir_all(&root).unwrap();
        let outside = directory.path().join("outside");
        std::fs::write(&outside, "elsewhere").unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let foreign = HashSet::from([link.clone(), outside.clone()]);

        reclaim(&root, &owners(&foreign)).unwrap();

        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "elsewhere");
    }

    #[test]
    fn publication_rollback_restores_both_files() {
        let directory = tempfile::tempdir().unwrap();
        let conf = directory.path().join("httpd.conf");
        let vrd = directory.path().join("default.vrd");
        std::fs::write(&conf, "original config").unwrap();
        std::fs::write(&vrd, "original descriptor").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&vrd, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        let snapshot = PublicationSnapshot {
            config: FileSnapshot::read(&conf).unwrap(),
            descriptor: Some(FileSnapshot::read(&vrd).unwrap()),
            conf: conf.clone(),
            vrd: vrd.clone(),
        };
        std::fs::write(&conf, "changed config").unwrap();
        std::fs::remove_file(&vrd).unwrap();

        snapshot.restore().unwrap();

        assert_eq!(std::fs::read_to_string(&conf).unwrap(), "original config");
        assert_eq!(
            std::fs::read_to_string(&vrd).unwrap(),
            "original descriptor"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&vrd).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
    }
}

/// Частный Apache на подделках кластера: процессы и `webinst` подделаны, файлы настоящие.
#[cfg(test)]
mod stand_tests {
    use super::super::cluster::tests::{
        executable, release_bin, setup, with_web, Host, Runner, Setup, RELEASE, STARTED,
        WEBINST_LINE, WEB_PID,
    };
    use super::super::ServiceCommand;
    use super::*;
    use crate::domain::publish::{PublishAction, PublishResult};
    use crate::platform::service::ProcessRow;
    use crate::use_cases::context::CommandName;
    use crate::use_cases::publish_infobase::{execute_with, PublishRequest};
    use crate::use_cases::result::EnvironmentReason;

    /// Кластер с частным Apache, поднятым его шагом, и базой в реестре.
    fn running() -> (Setup, Host) {
        let mut stand = setup();
        with_web(&mut stand);
        let host = Host::default();
        stand.run(ServiceCommand::Start, &host).unwrap();
        assert!(host.alive(WEB_PID));
        let mut pins = stand.pins();
        pins.registered = true;
        write_json(&stand.agent().pins(), &pins).unwrap();
        (stand, host)
    }

    fn context(command: CommandName) -> ExecutionContext {
        ExecutionContext::cli(command)
    }

    fn web(
        stand: &Setup,
        command: WebCommand,
        dry_run: bool,
        host: &Host,
    ) -> UseCaseResult<WebResult> {
        run_web(
            &context(CommandName::WebStart),
            &stand.config,
            command,
            dry_run,
            &Runner { host },
            host,
        )
    }

    fn status(stand: &Setup, host: &Host) -> WebResult {
        let context = context(CommandName::WebStatus);
        Web::new(&stand.config, &context, &Runner { host }, host)
            .unwrap()
            .result(ServiceAction::Reported)
    }

    fn paths(stand: &Setup) -> ServicePaths {
        stand.agent().sibling(StandService::Web)
    }

    fn port_of(result: &WebResult) -> u16 {
        result
            .address
            .as_deref()
            .and_then(|address| address.rsplit(':').next())
            .and_then(|port| port.parse().ok())
            .expect("an address with a port")
    }

    /// Ответ проходит закреплённую форму своей команды (`CTR.WIRE.WEB-*-DATA`).
    fn assert_form(slug: &str, result: &WebResult) -> serde_json::Value {
        let form = crate::command_data::command_data_forms()
            .into_iter()
            .find(|form| form.slug == slug)
            .expect("a published form");
        let value = serde_json::to_value(result).unwrap();
        let validator = jsonschema::validator_for(&form.schema).expect("form compiles");
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .map(|error| error.to_string())
            .collect();
        assert!(errors.is_empty(), "{slug}: {errors:?}\n{value}");
        value
    }

    fn publish(
        stand: &Setup,
        host: &Host,
        dry_run: bool,
    ) -> Result<PublishResult, UseCaseFailure<PublishResult>> {
        let context = context(CommandName::Publish);
        let runner = Runner { host };
        let managed = publication_with(
            &context,
            &stand.config,
            PublishAction::Publish,
            dry_run,
            &runner,
            host,
        )
        .map_err(UseCaseFailure::without_payload)?;
        execute_with(
            &context,
            &stand.config,
            &PublishRequest {
                action: PublishAction::Publish,
                dry_run,
            },
            managed,
        )
    }

    fn webinst() -> String {
        format!("webinst{}", std::env::consts::EXE_SUFFIX)
    }

    fn httpd() -> String {
        format!("httpd{}", std::env::consts::EXE_SUFFIX)
    }

    fn conf(stand: &Setup) -> PathBuf {
        paths(stand).root().join("httpd.conf")
    }

    fn descriptor(stand: &Setup) -> PathBuf {
        paths(stand).root().join("publication").join("default.vrd")
    }

    fn address_of(port: u16) -> String {
        format!("http://127.0.0.1:{port}/demo/")
    }

    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    fn web_start_starts_only_apache_when_the_cluster_is_stopped() {
        let mut stand = setup();
        with_web(&mut stand);
        let host = Host::default();

        let result = web(&stand, WebCommand::Start, false, &host).unwrap();
        assert!(result.ready);
        assert_eq!(result.explicit, Some(true));
        assert!(host.alive(WEB_PID));
        assert_eq!(host.spawned.borrow().len(), 1);
        assert!(!stand.agent().root().exists());

        let repeated = web(&stand, WebCommand::Start, false, &host).unwrap();
        assert_eq!(repeated.action, ServiceAction::AlreadyRunning);
        assert_eq!(repeated.pid, result.pid);
        assert_eq!(host.spawned.borrow().len(), 1);
    }

    #[test]
    fn web_answers_take_the_forms_of_their_commands() {
        let (stand, host) = running();
        let step = status(&stand, &host);
        let value = assert_form("web-status", &step);
        assert_eq!(value["state"], "started_by_this_copy");
        assert_eq!(value["action"], "reported");
        assert_eq!(value["explicit"], false);
        assert_eq!(value["pid"], WEB_PID);
        assert_eq!(value["ready"], true);
        assert!(value["address"]
            .as_str()
            .unwrap()
            .starts_with("http://127.0.0.1:"));
        assert!(value.get("previous_port").is_none());
        assert!(value.get("publication_stale").is_none());

        let explicit = web(&stand, WebCommand::Start, false, &host).unwrap();
        let value = assert_form("web-start", &explicit);
        assert_eq!(value["action"], "already_running");
        assert_eq!(value["explicit"], true);

        let restarted = web(&stand, WebCommand::Restart, false, &host).unwrap();
        let value = assert_form("web-restart", &restarted);
        assert_eq!(value["action"], "started");
        assert_eq!(value["explicit"], true, "restart keeps the explicit start");
        assert_eq!(value["pid"], WEB_PID);

        let stopped = web(&stand, WebCommand::Stop, false, &host).unwrap();
        let value = assert_form("web-stop", &stopped);
        assert_eq!(value["action"], "stopped");
        assert_eq!(value["state"], "not_running");
        assert!(value.get("pid").is_none());
        assert!(value["config_path"]
            .as_str()
            .unwrap()
            .ends_with("httpd.conf"));
        assert!(!host.alive(WEB_PID));

        let nothing = web(&stand, WebCommand::Stop, false, &host).unwrap();
        assert_eq!(
            assert_form("web-stop", &nothing)["action"],
            "nothing_to_stop"
        );
        let planned = web(&stand, WebCommand::Restart, true, &host).unwrap();
        assert_eq!(assert_form("web-restart", &planned)["action"], "planned");
    }

    /// Порт сервера отладки кластера в его блок не входит, но Apache его не занимает: иначе
    /// `cluster start --debug` отказал бы из-за своего же Apache.
    #[test]
    fn apache_never_takes_the_debug_port_of_the_cluster() {
        let (stand, host) = running();
        web(&stand, WebCommand::Stop, false, &host).unwrap();
        let debug = stand.pins().ports.debug();
        write_json(
            &stand.agent().sibling(StandService::Web).pins(),
            &PinnedPorts { port: debug },
        )
        .unwrap();
        let started = web(&stand, WebCommand::Start, false, &host).unwrap();
        assert_ne!(port_of(&started), debug);
    }

    #[test]
    fn a_taken_port_moves_apache_and_the_answer_names_the_previous_one() {
        let (stand, host) = running();
        let first = port_of(&status(&stand, &host));
        web(&stand, WebCommand::Stop, false, &host).unwrap();
        host.listeners.borrow_mut().insert(first, 9001);
        host.live(9001, "nginx");

        let moved = web(&stand, WebCommand::Start, false, &host).unwrap();
        let value = assert_form("web-start", &moved);
        assert_eq!(value["action"], "started");
        assert_eq!(value["previous_port"], first);
        let second = port_of(&moved);
        assert_ne!(second, first);
        assert_eq!(host.listeners.borrow().get(&first), Some(&9001));
        assert!(host.alive(9001), "the foreign listener is left alone");
        assert_eq!(host.listeners.borrow().get(&second), Some(&WEB_PID));
        let text = std::fs::read_to_string(conf(&stand)).unwrap();
        assert!(
            text.contains(&format!("Listen 127.0.0.1:{second}")),
            "{text}"
        );
    }

    #[test]
    fn an_apache_holding_the_private_config_is_refused_with_its_pid() {
        let (stand, host) = running();
        web(&stand, WebCommand::Stop, false, &host).unwrap();
        host.live(9002, &httpd());
        host.web_rows.borrow_mut().push(ProcessRow {
            pid: 9002,
            parent: 1,
            command: format!("httpd\0-f\0{}\0", conf(&stand).display()),
        });
        let spawned = host.spawned.borrow().len();

        let refused = web(&stand, WebCommand::Start, false, &host).unwrap_err();
        assert_eq!(refused.error.kind(), UseCaseErrorKind::WorkspaceBusy);
        assert!(
            refused.error.message().contains("9002"),
            "{}",
            refused.error
        );
        assert_eq!(host.spawned.borrow().len(), spawned, "nothing is started");
        assert!(host.alive(9002));
    }

    #[test]
    fn the_runner_never_runs_the_system_apache_controls() {
        let (mut stand, host) = running();
        web(&stand, WebCommand::Restart, false, &host).unwrap();
        web(&stand, WebCommand::Stop, false, &host).unwrap();
        stand.run(ServiceCommand::Start, &host).unwrap();
        stand.run(ServiceCommand::Stop, &host).unwrap();
        for call in host.ran.borrow().iter() {
            let program = PathBuf::from(&call[0]);
            let stem = program
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_lowercase();
            assert!(
                ![
                    "apachectl",
                    "apache2ctl",
                    "systemctl",
                    "service",
                    "net",
                    "sc"
                ]
                .contains(&stem.as_str()),
                "{call:?}"
            );
            if stem == "httpd" {
                assert_eq!(call[1], "-t", "httpd is only asked to check: {call:?}");
                assert!(!call.iter().any(|arg| arg == "-k"), "{call:?}");
            }
        }
        assert!(!host.alive(WEB_PID), "the own Apache stops by its identity");

        let control = stand.config.base_path.join("apachectl");
        executable(&control);
        stand.config.tools.apache.path = Some(control);
        for dry_run in [true, false] {
            let refused = web(&stand, WebCommand::Start, dry_run, &host).unwrap_err();
            assert_eq!(refused.error.kind(), UseCaseErrorKind::Validation);
            assert!(
                refused.error.message().contains("control script"),
                "{}",
                refused.error
            );
        }
    }

    #[test]
    fn a_crashed_start_leaves_an_apache_that_status_and_stop_take_over() {
        let (stand, host) = running();
        let paths = paths(&stand);
        let mut record: ServiceRecord = read_json(&paths.record()).unwrap();
        record.state = RecordState::Starting;
        record.pid = None;
        record.started = None;
        record.image = None;
        record.starter = Some((424_242, 1));
        write_json(&paths.record(), &record).unwrap();

        let taken = status(&stand, &host);
        assert_eq!(taken.state, ServiceState::StartedByThisCopy);
        assert_eq!(taken.pid, Some(WEB_PID));
        let stopped = web(&stand, WebCommand::Stop, false, &host).unwrap();
        assert_eq!(stopped.action, ServiceAction::Stopped);
        assert!(!host.alive(WEB_PID));
    }

    #[test]
    // Перезагрузки Apache на macOS нет (`ReloadStrategy::Unsupported`): публикация там отказывает.
    #[cfg(any(windows, target_os = "linux"))]
    fn status_marks_the_publication_stale_after_the_regport_moves() {
        let (stand, host) = running();
        publish(&stand, &host, false).unwrap();
        assert!(!status(&stand, &host).publication_stale);

        let mut pins = stand.pins();
        pins.ports.regport += 100;
        write_json(&stand.agent().pins(), &pins).unwrap();
        let stale = status(&stand, &host);
        assert!(stale.publication_stale);
        assert_eq!(assert_form("web-status", &stale)["publication_stale"], true);
    }

    #[test]
    // Перезагрузки Apache на macOS нет (`ReloadStrategy::Unsupported`): публикация там отказывает.
    #[cfg(any(windows, target_os = "linux"))]
    fn publish_runs_the_located_webinst_on_the_private_config_and_reloads_apache() {
        let (stand, host) = running();
        let port = port_of(&status(&stand, &host));
        let published = publish(&stand, &host, false).unwrap();
        assert!(published.ok);
        assert_eq!(published.url, Some(address_of(port)));
        assert_eq!(published.dir, paths(&stand).root().join("publication"));

        let calls = host.ran_with("-confpath");
        assert_eq!(calls.len(), 1, "{calls:?}");
        let call = &calls[0];
        assert_eq!(
            PathBuf::from(&call[0]),
            release_bin(&stand.config).join(webinst())
        );
        let value = |key: &str| {
            let at = call.iter().position(|arg| arg == key).unwrap();
            call[at + 1].clone()
        };
        assert_eq!(PathBuf::from(value("-confpath")), conf(&stand));
        assert_eq!(
            PathBuf::from(value("-dir")),
            paths(&stand).root().join("publication")
        );
        assert_eq!(
            value("-connstr"),
            format!("Srvr=127.0.0.1:{};Ref=demo", stand.pins().ports.regport)
        );
        assert!(std::fs::read_to_string(conf(&stand))
            .unwrap()
            .contains(WEBINST_LINE));
        assert!(descriptor(&stand).is_file());
        assert!(host.alive(WEB_PID));
        match ReloadStrategy::OF_THIS_SYSTEM {
            ReloadStrategy::InPlace => assert_eq!(host.reloads.get(), 1),
            ReloadStrategy::Restart => {
                let httpd = host
                    .spawned
                    .borrow()
                    .iter()
                    .filter(|name| *name == "httpd")
                    .count();
                assert_eq!(httpd, 2, "the cluster step, then the restart");
            }
            ReloadStrategy::Unsupported => {}
        }
    }

    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    fn a_file_infobase_starts_and_publishes_with_webinst_without_a_cluster() {
        let mut stand = setup();
        with_web(&mut stand);
        stand.config.infobase.cluster = None;
        stand.config.infobase.dbms = None;
        stand.config.infobase.connection = "File='test base'".to_owned();
        std::fs::remove_file(
            release_bin(&stand.config).join(format!("ragent{}", std::env::consts::EXE_SUFFIX)),
        )
        .unwrap();
        let host = Host::default();
        let started = web(&stand, WebCommand::Start, false, &host).unwrap();
        assert!(started.ready);
        assert_eq!(host.spawned.borrow().as_slice(), &["httpd"]);
        assert!(!stand.agent().root().exists());

        let published = publish(&stand, &host, false).unwrap();
        assert!(published.ok);
        assert!(published.url.as_deref().unwrap().ends_with("/demo/"));
        let run = host.ran.borrow();
        let call = run
            .iter()
            .find(|call| call.iter().any(|arg| arg == "-connstr"))
            .unwrap();
        let position = call.iter().position(|arg| arg == "-connstr").unwrap();
        let base = stand
            .config
            .v8_connection()
            .file_infobase_dir(&stand.config.base_path)
            .unwrap();
        assert_eq!(call[position + 1], format!("File='{}'", base.display()));
        assert!(descriptor(&stand).is_file());
        assert!(!stand.agent().root().exists());
    }

    #[test]
    fn a_failed_webinst_restores_the_config_and_the_descriptor() {
        let (stand, host) = running();
        let before = std::fs::read_to_string(conf(&stand)).unwrap();
        host.webinst_fails.set(true);

        let failed = publish(&stand, &host, false).unwrap_err();
        assert_eq!(failed.error.kind(), UseCaseErrorKind::Platform);
        assert_eq!(failed.payload.map(|done| done.ok), Some(false));
        assert_eq!(std::fs::read_to_string(conf(&stand)).unwrap(), before);
        assert!(
            !descriptor(&stand).exists(),
            "no descriptor was there before"
        );
        assert!(host.alive(WEB_PID));
        assert_eq!(host.reloads.get(), 0);
    }

    #[test]
    fn a_publication_apache_rejects_is_rolled_back_and_apache_works_on_the_old_config() {
        let (stand, host) = running();
        let before = std::fs::read_to_string(conf(&stand)).unwrap();
        host.webinst_breaks_the_config.set(true);

        let failed = publish(&stand, &host, false).unwrap_err();
        assert!(
            failed
                .error
                .message()
                .contains("invalid Apache configuration"),
            "{}",
            failed.error
        );
        assert_eq!(std::fs::read_to_string(conf(&stand)).unwrap(), before);
        assert!(!descriptor(&stand).exists());
        assert!(host.alive(WEB_PID), "Apache works on the restored config");
    }

    #[test]
    fn both_reload_strategies_activate_the_same_apache() {
        for strategy in [ReloadStrategy::InPlace, ReloadStrategy::Restart] {
            let (stand, host) = running();
            let context = context(CommandName::Publish);
            let runner = Runner { host: &host };
            let publication = publication_with(
                &context,
                &stand.config,
                PublishAction::Publish,
                false,
                &runner,
                &host,
            )
            .unwrap()
            .unwrap();
            publication.reload_with(Reload::Activate, strategy).unwrap();
            let after = status(&stand, &host);
            assert_eq!(after.pid, Some(WEB_PID), "{strategy:?}");
            assert!(after.ready, "{strategy:?}");
            let expected_reloads = u32::from(strategy == ReloadStrategy::InPlace);
            assert_eq!(host.reloads.get(), expected_reloads, "{strategy:?}");
            // Экземпляр, который перезапустила другая команда, публикацию не принимает.
            host.live_since(WEB_PID, &httpd(), STARTED + 1);
            let record_path = paths(&stand).record();
            let mut record: ServiceRecord = read_json(&record_path).unwrap();
            record.started = Some(STARTED + 1);
            write_json(&record_path, &record).unwrap();
            let changed = publication
                .reload_with(Reload::Activate, strategy)
                .unwrap_err();
            assert!(changed.message().contains("changed"), "{changed}");
        }
    }

    #[test]
    // Перезагрузки Apache на macOS нет (`ReloadStrategy::Unsupported`): публикация там отказывает.
    #[cfg(any(windows, target_os = "linux"))]
    fn only_webinst_is_elevated_and_a_refusing_sudo_names_the_sudoers_rule() {
        let (stand, host) = running();
        let sudo = stand.config.base_path.join("sudo");
        host.elevation.replace(Some(sudo.clone()));
        publish(&stand, &host, false).unwrap();
        let located = release_bin(&stand.config).join(webinst());
        let elevated = host.ran_with("-confpath");
        assert_eq!(elevated.len(), 1);
        assert_eq!(PathBuf::from(&elevated[0][0]), sudo);
        assert_eq!(elevated[0][1..3], ["-n", "--"]);
        assert_eq!(PathBuf::from(&elevated[0][3]), located);
        for call in host.ran.borrow().iter() {
            if !call.iter().any(|arg| arg == "-confpath") {
                assert_ne!(
                    PathBuf::from(&call[0]),
                    sudo,
                    "only webinst is elevated: {call:?}"
                );
            }
        }

        host.sudo_refuses.set(true);
        let runs = host.webinst_runs.get();
        let refused = publish(&stand, &host, false).unwrap_err();
        assert_eq!(
            refused.error.kind(),
            UseCaseErrorKind::Environment(EnvironmentReason::Unavailable)
        );
        assert!(
            refused
                .error
                .message()
                .contains("effectively root for this user"),
            "{}",
            refused.error
        );
        assert!(
            refused
                .error
                .message()
                .contains(&located.display().to_string()),
            "{}",
            refused.error
        );
        assert_eq!(host.webinst_runs.get(), runs, "webinst did not run");
    }

    #[test]
    // Перезагрузки Apache на macOS нет (`ReloadStrategy::Unsupported`): публикация там отказывает.
    #[cfg(any(windows, target_os = "linux"))]
    fn files_webinst_leaves_as_root_are_taken_back_after_success_and_failure() {
        let (stand, host) = running();
        host.elevation
            .replace(Some(stand.config.base_path.join("sudo")));
        host.webinst_leaves_a_root_file.set(true);
        let web = paths(&stand).root().to_owned();
        let backup = web.join("httpd.conf.bak");
        let alias = web.parent().unwrap().join("httpd.conf.bak.alias");

        publish(&stand, &host, false).unwrap();
        std::fs::write(&alias, "changed").unwrap();
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "backup");

        // Каждый сбой оставляет файл root, который забрать может только откат: `webinst` не
        // дошёл до перезагрузки, а при отказе `httpd -t` файл появляется после того, как
        // активация свои файлы уже забрала.
        for breaks in [Breaks::Spawn, Breaks::Webinst, Breaks::Config] {
            host.webinst_leaves_a_root_file
                .set(breaks != Breaks::Config);
            host.webinst_cannot_start.set(breaks == Breaks::Spawn);
            host.webinst_fails.set(breaks == Breaks::Webinst);
            host.webinst_breaks_the_config.set(breaks == Breaks::Config);
            host.check_leaves_a_root_file.set(breaks == Breaks::Config);
            publish(&stand, &host, false).unwrap_err();
            std::fs::write(&alias, "changed").unwrap();
            assert_eq!(
                std::fs::read_to_string(&backup).unwrap(),
                "backup",
                "{breaks:?}: the rollback takes the root file back"
            );
        }

        host.webinst_leaves_a_root_file.set(true);
        host.webinst_cannot_start.set(false);
        host.webinst_breaks_the_config.set(false);
        host.check_leaves_a_root_file.set(false);
        host.webinst_fails.set(true);
        host.webinst_leaves_a_root_dir.set(true);
        let refused = publish(&stand, &host, false).unwrap_err();
        assert!(
            refused.error.message().contains("sudo rm -r"),
            "{}",
            refused.error
        );

        // Активация, которой не удалось забрать каталог root, не повторяет тот же отказ в откате.
        host.webinst_fails.set(false);
        let refused = publish(&stand, &host, false).unwrap_err();
        assert_eq!(
            refused.error.message().matches("sudo rm -r").count(),
            1,
            "{}",
            refused.error
        );
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Breaks {
        /// Запуск `webinst` не удался.
        Spawn,
        /// `webinst` вышел с ошибкой.
        Webinst,
        /// Apache отверг конфиг после `webinst`.
        Config,
    }

    #[test]
    fn publish_as_root_is_refused_before_anything_runs() {
        let (stand, host) = running();
        host.root.set(true);
        let ran = host.ran.borrow().len();
        for dry_run in [false, true] {
            let refused = publish(&stand, &host, dry_run).unwrap_err();
            assert_eq!(
                refused.error.kind(),
                UseCaseErrorKind::Environment(EnvironmentReason::Unavailable)
            );
        }
        assert_eq!(host.ran.borrow().len(), ran);
        assert!(!descriptor(&stand).exists());
    }

    #[test]
    // Перезагрузки Apache на macOS нет (`ReloadStrategy::Unsupported`): публикация там отказывает.
    #[cfg(any(windows, target_os = "linux"))]
    fn webinst_comes_from_the_platform_search_not_from_the_cluster_record() {
        let (stand, host) = running();
        let record_path = stand.agent().record();
        let mut record: ServiceRecord = read_json(&record_path).unwrap();
        let located = release_bin(&stand.config).join(webinst());

        // Тот же выпуск в другом каталоге: запись правит пользователь, программа — из поиска.
        let elsewhere = stand
            .config
            .base_path
            .join("copy")
            .join(RELEASE)
            .join("bin");
        executable(&elsewhere.join(webinst()));
        record.bin = elsewhere.join("ragent.exe");
        write_json(&record_path, &record).unwrap();
        let preview = publish(&stand, &host, true).unwrap();
        let plan = preview.plan.expect("a preview plan");
        assert_eq!(plan.program, located);

        // Другой выпуск в записи: найденный `webinst` кластеру не подходит.
        let other = stand
            .config
            .base_path
            .join("copy")
            .join("8.3.27.1000")
            .join("bin");
        executable(&other.join(webinst()));
        record.bin = other.join("ragent.exe");
        write_json(&record_path, &record).unwrap();
        let refused = publish(&stand, &host, true).unwrap_err();
        assert!(
            refused.error.message().contains("release"),
            "{}",
            refused.error
        );
        assert_eq!(host.webinst_runs.get(), 0);
    }

    #[test]
    fn a_preview_with_apache_stopped_names_the_pinned_address_and_writes_nothing() {
        let (stand, host) = running();
        let port = port_of(&status(&stand, &host));
        web(&stand, WebCommand::Stop, false, &host).unwrap();
        let before = std::fs::read_to_string(conf(&stand)).unwrap();

        let preview = publish(&stand, &host, true).unwrap();
        assert_eq!(preview.url, Some(address_of(port)));
        assert!(preview.plan.is_some());
        assert_eq!(std::fs::read_to_string(conf(&stand)).unwrap(), before);
        assert!(!descriptor(&stand).exists());
        assert_eq!(host.webinst_runs.get(), 0);
        assert!(!host.alive(WEB_PID));
    }
}
