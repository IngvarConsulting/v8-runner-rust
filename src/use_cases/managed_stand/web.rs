//! Lifecycle of the private Apache belonging to a declared cluster.
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::{
    await_gone, being_started, environment, forget_record, io_failure, locate, occupant_name,
    occupant_of, policy, port_offsets, refuse_a_stand_of_another_project, validation, write_record,
    RealHost, ServiceCommand, ServiceHost, IDENTIFY_TIMEOUT, POLL, STOP_TIMEOUT,
};
use crate::config::model::AppConfig;
use crate::domain::dbms::{ServiceAction, ServiceState};
use crate::platform::apache;
use crate::platform::locator::UtilityType;
use crate::platform::process::{ProcessExecutor, ProcessRunner};
use crate::platform::service::image_name;
use crate::platform::service_record::{
    observe_service, read_json, write_json, Observed, PinnedPorts, RecordState, ServicePaths,
    ServiceRecord, StandService,
};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind, UseCaseFailure, UseCaseResult};

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct WebResult {
    pub ok: bool,
    pub infobase: String,
    pub state: ServiceState,
    pub action: ServiceAction,
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explicit: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<PathBuf>,
    pub config_path: PathBuf,
    pub log_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occupant_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_port: Option<u16>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub publication_stale: bool,
    pub duration_ms: u64,
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

    pub(super) fn observe(&self) -> Observed {
        let machine = self.host.machine();
        let facts = |pid| self.host.facts(pid);
        observe_service(&self.paths, machine.as_deref(), &facts, &|record| {
            crate::platform::service_record::adopt_recorded_pid(record, &facts, machine.as_deref())
        })
    }

    fn pinned_port(&self) -> Option<u16> {
        read_json::<PinnedPorts>(&self.paths.pins()).map(|pins| pins.port)
    }

    fn result(&self, action: ServiceAction) -> WebResult {
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
                result.address = Some(format!("http://127.0.0.1:{}", record.port));
                result.program = Some(record.bin);
                result.ready =
                    record.pid.is_some() && self.host.listening_pid(record.port) == record.pid;
            }
            Observed::Starting(_) => result.state = ServiceState::Starting,
            Observed::Nothing | Observed::Stale => {
                if let Some(port) = self.pinned_port() {
                    result.address = Some(format!("http://127.0.0.1:{port}"));
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
        let descriptor = self.paths.root().join("publication/default.vrd");
        let Ok(descriptor) = std::fs::read_to_string(descriptor) else {
            return false;
        };
        let cluster = self.paths.sibling(StandService::Cluster);
        let pins = read_json::<crate::platform::service_record::ClusterPins>(&cluster.pins());
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
        apache::locate(self.config.tools.apache.path.as_deref()).ok_or_else(|| environment(
            "Apache 2.4 was not found: set tools.apache.path to an installed httpd or apache2; use Apache Lounge on Windows or the Apache package of your Linux distribution",
        ))
    }

    fn check_module(&self, ragent: &Path) -> Result<(), UseCaseError> {
        let module = apache::platform_module(ragent);
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
        let pidfile_user = std::fs::read_to_string(self.paths.root().join("httpd.pid"))
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

    pub(super) fn start(&self, explicit: bool, dry_run: bool) -> Result<WebResult, UseCaseError> {
        if !declared(self.config) {
            return Err(validation(
                "declare infobase.web.managed for a managed cluster",
            ));
        }
        let observed = self.observe();
        if dry_run {
            let mut result = self.result(ServiceAction::Planned);
            if result.program.is_none() {
                result.program = Some(self.program()?);
            }
            if result.address.is_none() {
                let port = self.choose_port()?;
                result.address = Some(format!("http://127.0.0.1:{port}"));
            }
            return Ok(result);
        }
        if !self.host.supported() || !self.host.can_start_as_current_user() {
            return Err(environment(
                "start managed Apache on Windows or as an unprivileged user on Linux",
            ));
        }
        if let Observed::Own(mut record) = observed {
            if let Some(cluster) = self.paths.sibling(StandService::Cluster).own_record() {
                self.check_module(&cluster.bin)?;
            }
            if explicit && !record.explicit {
                record.explicit = true;
                write_record(&self.paths, &record)?;
            }
            return Ok(self.result(ServiceAction::AlreadyRunning));
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
        let cluster = self.paths.sibling(StandService::Cluster);
        let ragent = match cluster.own_record() {
            Some(record) => record.bin,
            None => locate(self.config, UtilityType::Ragent)?,
        };
        let module = apache::platform_module(&ragent);
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
            Some(previous) => {
                let old = previous
                    .lines()
                    .find(|line| line.starts_with("Listen 127.0.0.1:"))
                    .ok_or_else(|| {
                        validation("private Apache config has no expected Listen directive")
                    })?;
                // The path anchors the persisted config even when the service is stopped.
                let local_pid = rendered.lines().find(|line| line.starts_with("PidFile "));
                if !previous.lines().any(|line| Some(line) == local_pid) {
                    return Err(validation(
                        "Apache config belongs to a different stand path",
                    ));
                }
                let prefix = "LoadModule _1cws_module ";
                let expected_module = rendered
                    .lines()
                    .find(|line| line.starts_with(prefix))
                    .ok_or_else(|| validation("generated Apache config has no 1C module"))?;
                let mut modules = previous.lines().filter(|line| line.starts_with(prefix));
                let old_module = modules
                    .next()
                    .filter(|_| modules.next().is_none())
                    .ok_or_else(|| validation("private Apache config must have one 1C module"))?;
                previous.replacen(old_module, expected_module, 1).replacen(
                    old,
                    &format!("Listen 127.0.0.1:{port}"),
                    1,
                )
            }
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
            explicit,
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
        if let Err(error) = outcome {
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
            return Err(error);
        }
        let mut result = self.result(ServiceAction::Started);
        result.previous_port = previous_port.filter(|previous| *previous != port);
        Ok(result)
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
        let cluster_ports =
            read_json::<crate::platform::service_record::ClusterPins>(&cluster.pins());
        self.pinned_port()
            .into_iter()
            .chain(port_offsets(self.paths.root()))
            .find(|&port| {
                Some(port) != pg_port
                    && !super::dynamic_ports().contains(&port)
                    && cluster_ports
                        .as_ref()
                        .is_none_or(|pins| !pins.ports.all().any(|value| value == port))
                    && occupant_of(self.host, port).is_none()
            })
            .ok_or_else(|| environment("no free stand port for Apache"))
    }

    fn check_publication(&self, program: &Path) -> Result<(), UseCaseError> {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let pid = match self.observe() {
                Observed::Own(record) => record.pid,
                _ => None,
            }
            .ok_or_else(|| environment("Apache is no longer running"))?;
            let meta = std::fs::metadata(format!("/proc/{pid}"))
                .map_err(|err| environment(format!("cannot read Apache owner: {err}")))?;
            if meta.uid() == 0 {
                return Err(environment(
                    "the managed Apache must run as an unprivileged user",
                ));
            }
            let mut request = apache::check(program, &self.conf());
            request.args.splice(
                ..0,
                [
                    format!("--reuid={}", meta.uid()),
                    format!("--regid={}", meta.gid()),
                    "--clear-groups".to_owned(),
                    "--".to_owned(),
                    program.display().to_string(),
                ],
            );
            request.program = PathBuf::from("/usr/bin/setpriv");
            self.check_request(&request)
        }
        #[cfg(not(target_os = "linux"))]
        self.check(program)
    }

    fn check(&self, program: &Path) -> Result<(), UseCaseError> {
        self.check_request(&apache::check(program, &self.conf()))
    }

    fn check_request(
        &self,
        request: &crate::platform::process::ProcessRequest,
    ) -> Result<(), UseCaseError> {
        let answer = self
            .runner
            .run_with_policy(request, &policy(self.context))
            .map_err(|err| environment(format!("cannot check Apache configuration: {err}")))?;
        if answer.outcome().is_err() {
            return Err(environment(format!(
                "invalid Apache configuration: {}",
                answer.stderr
            )));
        }
        Ok(())
    }

    pub(super) fn stop(&self, only_step: bool, dry_run: bool) -> Result<WebResult, UseCaseError> {
        if dry_run {
            return Ok(self.result(ServiceAction::Planned));
        }
        let Observed::Own(record) = self.observe() else {
            let result = self.result(ServiceAction::NothingToStop);
            if !only_step && result.state == ServiceState::Foreign {
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
        if only_step && record.explicit {
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
}

pub fn execute_web(
    context: &ExecutionContext,
    config: &AppConfig,
    command: ServiceCommand,
    dry_run: bool,
) -> UseCaseResult<WebResult> {
    refuse_a_stand_of_another_project(config).map_err(UseCaseFailure::without_payload)?;
    let web = Web::new(config, context, &ProcessExecutor, &RealHost)
        .map_err(UseCaseFailure::without_payload)?;
    let outcome = match command {
        ServiceCommand::Status => Ok(web.result(ServiceAction::Reported)),
        ServiceCommand::Start => web.start(true, dry_run),
        ServiceCommand::Stop => web.stop(false, dry_run),
        ServiceCommand::Restart if dry_run => web.start(true, true),
        ServiceCommand::Restart => {
            let explicit = !matches!(web.observe(), Observed::Own(record) if !record.explicit);
            web.stop(false, false)
                .and_then(|_| web.start(explicit, false))
        }
    };
    outcome.map_err(|error| {
        let mut result = web.result(ServiceAction::Reported);
        result.ok = false;
        UseCaseFailure::with_payload(error, result)
    })
}

pub(crate) struct ManagedPublication<'a> {
    web: Web<'a>,
    record: ServiceRecord,
    pub(crate) target: crate::config::model::InfobaseWebConfig,
    pub(crate) webinst: PathBuf,
    pub(crate) connection: String,
    pub(crate) log_root: PathBuf,
}

pub(crate) fn publication<'a>(
    context: &'a ExecutionContext,
    config: &'a AppConfig,
    action: crate::domain::publish::PublishAction,
    dry_run: bool,
) -> Result<Option<ManagedPublication<'a>>, UseCaseError> {
    if !declared(config) {
        return Ok(None);
    }
    refuse_a_stand_of_another_project(config)?;
    let web = Web::new(config, context, &ProcessExecutor, &RealHost)?;
    let Observed::Own(record) = web.observe() else {
        return Err(validation(
            "start the managed Apache with cluster start or web start",
        ));
    };
    if record.pid.is_none() || web.host.listening_pid(record.port) != record.pid {
        return Err(environment("the managed Apache is not ready"));
    }
    let cluster = web.paths.sibling(StandService::Cluster);
    if !crate::platform::service_record::record_names_own_process(
        &cluster,
        web.host.machine().as_deref(),
        &|pid| web.host.facts(pid),
    ) {
        return Err(validation("start the managed cluster before publishing"));
    }
    let cluster_record = cluster
        .own_record()
        .ok_or_else(|| validation("the managed cluster has no record"))?;
    web.check_module(&cluster_record.bin)?;
    let pins = read_json::<crate::platform::service_record::ClusterPins>(&cluster.pins())
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
    let dir = web.paths.root().join("publication");
    target.dir = Some(dir);
    target.conf = Some(web.conf());
    target.url = Some(format!("http://127.0.0.1:{}/{wsdir}/", record.port));
    target.server = Some(crate::config::model::WebServerKind::Apache24);
    #[cfg(not(target_os = "linux"))]
    let _ = dry_run;
    #[cfg(target_os = "linux")]
    // SAFETY: geteuid takes no pointers and has no side effects.
    if !dry_run && unsafe { libc::geteuid() } != 0 {
        return Err(environment("1C webinst requires root on Linux: invoke publish explicitly with sudo from a trusted project"));
    }
    let webinst = cluster_record
        .bin
        .with_file_name(format!("webinst{}", std::env::consts::EXE_SUFFIX));
    if !webinst.is_file() {
        return Err(environment(format!(
            "the cluster release has no webinst: {}",
            webinst.display()
        )));
    }
    Ok(Some(ManagedPublication {
        web,
        record,
        target,
        webinst,
        connection,
        log_root,
    }))
}

pub(crate) struct PublicationSnapshot {
    conf: PathBuf,
    config: FileSnapshot,
    vrd: PathBuf,
    descriptor: Option<FileSnapshot>,
}

struct FileSnapshot {
    contents: Vec<u8>,
    metadata: std::fs::Metadata,
}

impl FileSnapshot {
    fn read(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(std::io::Error::other("publication requires a regular file"));
        }
        Ok(Self {
            contents: std::fs::read(path)?,
            metadata,
        })
    }

    fn restore(self, path: &Path) -> std::io::Result<()> {
        use std::io::Write as _;
        crate::support::fs::write_file_atomically(path, |file| {
            #[cfg(target_os = "linux")]
            {
                use std::os::fd::AsRawFd as _;
                use std::os::unix::fs::MetadataExt as _;
                // SAFETY: the temporary file descriptor is owned and open.
                let temporary = file.metadata()?;
                if temporary.uid() != self.metadata.uid() || temporary.gid() != self.metadata.gid()
                {
                    let result = unsafe {
                        libc::fchown(file.as_raw_fd(), self.metadata.uid(), self.metadata.gid())
                    };
                    if result != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
            }
            file.set_permissions(self.metadata.permissions())?;
            file.write_all(&self.contents)
        })
    }
}

impl ManagedPublication<'_> {
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

    pub(crate) fn reload(&self, recovering: bool) -> Result<(), UseCaseError> {
        if recovering {
            let context = self
                .web
                .context
                .clone()
                .with_cancellation(tokio_util::sync::CancellationToken::new());
            let web = Web::new(self.web.config, &context, self.web.runner, self.web.host)?;
            return self.reload_web(&web, true);
        }
        self.reload_web(&self.web, false)
    }

    fn reload_web(&self, web: &Web<'_>, recovering: bool) -> Result<(), UseCaseError> {
        let Observed::Own(current) = web.observe() else {
            #[cfg(windows)]
            if recovering {
                web.start(self.record.explicit, false)?;
                return Ok(());
            }
            return Err(environment(
                "Apache stopped before publication could be activated",
            ));
        };
        if !recovering && (current.pid != self.record.pid || current.started != self.record.started)
        {
            return Err(environment(
                "Apache changed before publication could be activated",
            ));
        }
        web.check_publication(&current.bin)?;
        #[cfg(target_os = "linux")]
        {
            let pid = current
                .pid
                .ok_or_else(|| environment("Apache has no process identity"))?;
            let before = self.apache_workers(pid)?;
            crate::platform::service::signal_owned_process(pid, current.started, libc::SIGUSR1)
                .map_err(|err| environment(format!("cannot reload Apache: {err}")))?;
            let deadline = Instant::now() + IDENTIFY_TIMEOUT;
            loop {
                if self
                    .web
                    .host
                    .facts(pid)
                    .is_none_or(|facts| Some(facts.started) != current.started)
                {
                    return Err(environment("Apache exited during reload"));
                }
                if self
                    .apache_workers(pid)?
                    .iter()
                    .any(|worker| !before.contains(worker))
                    && web.host.listening_pid(current.port) == Some(pid)
                {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(environment(
                        "Apache did not activate the changed configuration",
                    ));
                }
                web.host.sleep(POLL);
            }
        }
        #[cfg(windows)]
        {
            web.stop(false, false)?;
            web.start(current.explicit, false)?;
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        return Err(environment(
            "managed Apache reload is supported on Windows and Linux",
        ));
        #[cfg(any(windows, target_os = "linux"))]
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn apache_workers(&self, parent: u32) -> Result<Vec<(u32, u64)>, UseCaseError> {
        let image = image_name(&self.record.bin);
        let rows = self
            .web
            .host
            .processes(&[&image])
            .ok_or_else(|| environment("cannot inspect the Apache children after reload"))?;
        Ok(rows
            .into_iter()
            .filter(|row| row.parent == parent)
            .filter_map(|row| {
                self.web
                    .host
                    .facts(row.pid)
                    .map(|facts| (row.pid, facts.started))
            })
            .collect())
    }
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
