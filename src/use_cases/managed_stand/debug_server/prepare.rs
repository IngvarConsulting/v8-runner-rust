//! Resolve a debug route and lease only processes started by prepare.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::model::InfobaseDebugConfig;
use crate::domain::debug_server::{
    DebugServerPrepareOwner as Owner, DebugServerPrepareResult as Receipt,
    DebugServerPrepareScope as Scope, DebugServerRelease,
};
use crate::support::fs::{try_acquire_advisory_lock, AdvisoryLockGuard};
use crate::support::path::{
    nearest_existing_canonical_path, normalize_windows_verbatim_path, stable_path_identity,
};
use crate::use_cases::interruption::interruption_before_safe_point;
use crate::use_cases::result::EnvironmentReason;

use super::super::{
    await_service_end, cluster, debug_process_ended, environment, io_failure,
    spawn_recorded_debugger, stop_with_confirmation, DebuggerJournal, POLL,
};
use super::{
    await_rdbg, dbgs, empty, refuse, write_json, AppConfig, DebugServer, DebugServerAction,
    DebugServerOwner, DebugServerResult, DebugSession, DebuggerLaunch, DebuggerSpawn, RdbgWait,
    RecordState, ServicePaths, ServiceRecord, SpawnedDebugger, TargetKind, UseCaseError,
    UseCaseErrorKind, FILE_ALIAS, NOTIFY, SESSIONS,
};

const PREPARED: &str = "prepared-v1";

enum Route<'a> {
    ManagedCluster,
    External {
        config: &'a InfobaseDebugConfig,
        file: bool,
    },
    Owned(Target),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
enum Target {
    File { path: String },
    Cluster { server: String, reference: String },
}

impl Target {
    fn scope(&self) -> Scope {
        match self {
            Self::File { .. } => Scope::ClientAndServer,
            Self::Cluster { .. } => Scope::Client,
        }
    }

    fn alias(&self) -> &str {
        match self {
            Self::File { .. } => FILE_ALIAS,
            Self::Cluster { reference, .. } => reference,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
struct ProcessOwner {
    pid: u32,
    started: u64,
}

#[derive(Deserialize, Serialize)]
struct Run {
    work: String,
    project: String,
    id: DebugSession,
    target: Target,
    owner: ProcessOwner,
    process: ServiceRecord,
    leases: Vec<DebugSession>,
    stopping: bool,
}

impl Run {
    fn valid(&self) -> bool {
        let mut seen = HashSet::new();
        self.owner.pid != 0
            && self
                .process
                .machine
                .as_ref()
                .is_some_and(|machine| !machine.is_empty())
            && self.process.port != 0
            && self
                .leases
                .iter()
                .all(|lease| lease.is_prepare() && seen.insert(*lease))
            && match self.process.state {
                RecordState::Starting => self.leases.is_empty() && !self.stopping,
                RecordState::Running => {
                    self.process.pid.is_some_and(|pid| pid != 0)
                        && self.process.started.is_some()
                        && self.process.image.as_deref() == Some(dbgs::executable_name())
                        && !self.leases.is_empty()
                        && (!self.stopping || self.leases.len() == 1)
                }
            }
    }
}

struct Store {
    root: PathBuf,
    work: String,
    project: String,
}

impl Store {
    fn new(config: &AppConfig) -> Result<Self, UseCaseError> {
        let work = canonical(&config.work_path)?;
        let project = stable_path_identity(&canonical(&config.base_path)?);
        let sessions = work.join(SESSIONS);
        let prepared = sessions.join(PREPARED);
        let root = prepared.join(&project);
        for directory in [&sessions, &prepared, &root] {
            checked_directory(directory)?;
        }
        Ok(Self {
            root,
            work: stable_path_identity(&work),
            project,
        })
    }

    fn file_for(&self, target: &Target, owner: ProcessOwner, machine: &str) -> PathBuf {
        let scope = (&self.work, &self.project, target, owner, machine);
        let encoded =
            serde_json::to_vec(&scope).expect("debug scope contains only strings and numbers");
        self.root
            .join(format!("{:x}.json", Sha256::digest(encoded)))
    }

    fn read(&self, file: &Path, allow_copied: bool) -> Result<Option<Run>, UseCaseError> {
        let Some(run) = super::ensure::read_state_file::<Run>(file)? else {
            return Ok(None);
        };
        let same_location = run.work == self.work && run.project == self.project;
        if same_location && !run.valid() {
            return Err(platform(format!(
                "invalid prepared debug state: {}",
                file.display()
            )));
        }
        let same_scope = run
            .process
            .machine
            .as_deref()
            .is_some_and(|machine| self.file_for(&run.target, run.owner, machine) == file);
        if !same_location || !same_scope {
            if allow_copied {
                return Ok(None);
            }
            return Err(platform(format!(
                "prepared debug state belongs to a different scope: {}",
                file.display()
            )));
        }
        Ok(Some(run))
    }

    // A directory scan only locates a token. Its scope is read again under that scope's lock.
    fn find_lease(&self, session: DebugSession) -> Result<Option<PathBuf>, UseCaseError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_failure("cannot read", &self.root, &error)),
        };
        let mut found = None;
        let mut unresolved = None;
        for entry in entries {
            let entry = entry.map_err(|error| io_failure("cannot read", &self.root, &error))?;
            let file = entry.path();
            let is_scope_file = file.extension().is_some_and(|ext| ext == "json")
                && file
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
                    });
            if !is_scope_file {
                continue;
            }
            match self.read(&file, true) {
                Ok(Some(run)) if run.leases.contains(&session) => {
                    if found.replace(file).is_some() {
                        return Err(platform(
                            "the prepared debug lease has more than one recorded process",
                        ));
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    unresolved.get_or_insert(error);
                }
            }
        }
        match (found, unresolved) {
            (None, Some(error)) => Err(platform(format!(
                "the requested lease could not be resolved: {error}"
            ))),
            (found, _) => Ok(found),
        }
    }

    fn write(&self, file: &Path, run: &Run) -> Result<(), UseCaseError> {
        write_json(file, run).map_err(|error| io_failure("cannot write", file, &error))
    }

    fn forget(&self, file: &Path) -> Result<(), UseCaseError> {
        match fs::remove_file(file) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_failure("cannot remove", file, &error)),
        }
    }

    fn lock(
        &self,
        file: &Path,
        server: &DebugServer<'_>,
    ) -> Result<AdvisoryLockGuard, UseCaseError> {
        let path = file.with_extension("lock");
        loop {
            server.prepare_safe_point()?;
            match try_acquire_advisory_lock(&path) {
                Ok(guard) => return Ok(guard),
                Err(error) if error.kind() == ErrorKind::WouldBlock => server.host.sleep(POLL),
                Err(error) => return Err(io_failure("cannot lock", &path, &error)),
            }
        }
    }

    fn run_paths(&self, id: DebugSession) -> ServicePaths {
        ServicePaths::under(&self.root, &format!("run-{id}"))
    }

    fn remove_run_files(&self, id: DebugSession) {
        let paths = self.run_paths(id);
        if checked_directory(paths.root()).is_ok() {
            let _ = fs::remove_file(paths.root().join(NOTIFY));
            let _ = fs::remove_file(paths.log());
            let _ = fs::remove_dir(paths.root());
        }
    }
}

impl DebugServer<'_> {
    pub(in crate::use_cases::managed_stand) fn prepare(
        &self,
        owner_pid: u32,
        dry_run: bool,
    ) -> Result<Receipt, UseCaseError> {
        let route = self.prepare_route()?;
        if dry_run {
            return Ok(Receipt::Planned);
        }
        self.prepare_safe_point()?;
        match route {
            Route::ManagedCluster => {
                let binding = cluster::prepare_debug_server(
                    self.context,
                    self.config,
                    self.runner,
                    self.host,
                );
                self.prepare_safe_point()?;
                let binding = binding?;
                Ok(receipt(
                    dbgs::url(binding.port),
                    binding.alias,
                    Scope::ClientAndServer,
                    Owner::Cluster,
                    None,
                ))
            }
            Route::External { config, file } => self.prepare_external(config, file),
            Route::Owned(target) => self.prepare_owned(target, owner_pid),
        }
    }

    fn prepare_route(&self) -> Result<Route<'_>, UseCaseError> {
        let external = self.config.infobase.debug.as_ref();
        match self.config.target_kind() {
            TargetKind::Cluster
                if self
                    .config
                    .infobase
                    .cluster
                    .as_ref()
                    .is_some_and(|cluster| cluster.managed.is_some()) =>
            {
                if external.is_some() {
                    return Err(UseCaseError::new(
                        UseCaseErrorKind::Validation,
                        "infobase.debug cannot override the debug endpoint of a managed cluster",
                    ));
                }
                Ok(Route::ManagedCluster)
            }
            TargetKind::Cluster => {
                let (server, reference) = self.config.v8_connection().cluster_address().ok_or_else(|| {
                    refuse("debug-server prepare requires a direct server infobase identified by its server and Ref")
                })?;
                if reference.trim().is_empty() || reference.chars().any(char::is_control) {
                    return Err(UseCaseError::new(
                        UseCaseErrorKind::Validation,
                        "the server infobase Ref is not a valid debug alias",
                    ));
                }
                Ok(match external {
                    Some(config) => Route::External {
                        config,
                        file: false,
                    },
                    None => Route::Owned(Target::Cluster {
                        server: server.to_ascii_lowercase(),
                        reference,
                    }),
                })
            }
            TargetKind::File => {
                let path = crate::use_cases::infobase_owner::file_infobase_dir(self.config)
                    .ok_or_else(|| {
                        refuse("debug-server prepare requires a resolved file infobase path")
                    })?;
                Ok(match external {
                    Some(config) => {
                        if config
                            .infobase_alias
                            .as_deref()
                            .is_some_and(|alias| alias != FILE_ALIAS)
                        {
                            return Err(UseCaseError::new(
                                UseCaseErrorKind::Validation,
                                "a file infobase requires debug alias DefAlias",
                            ));
                        }
                        Route::External { config, file: true }
                    }
                    None => Route::Owned(Target::File {
                        path: stable_path_identity(&canonical(&path)?),
                    }),
                })
            }
            TargetKind::Standalone => Err(environment(
                "debug-server prepare has no verified route for a standalone infobase",
            )),
        }
    }

    fn prepare_external(
        &self,
        config: &InfobaseDebugConfig,
        file: bool,
    ) -> Result<Receipt, UseCaseError> {
        let url = config.url.to_string();
        if !file {
            let proof = cluster::prepare_external_debug_server(
                self.context,
                self.config,
                self.runner,
                self.host,
                &config.url,
                config.infobase_alias.as_deref(),
            );
            self.prepare_safe_point()?;
            match proof {
                cluster::ExternalDebugProof::NoBinding => {}
                cluster::ExternalDebugProof::Unavailable => return Err(external_unavailable()),
                cluster::ExternalDebugProof::Unverified => return Err(external_unverified()),
                cluster::ExternalDebugProof::Confirmed(binding) => {
                    return Ok(receipt(
                        url,
                        binding.alias,
                        Scope::ClientAndServer,
                        Owner::External,
                        None,
                    ));
                }
            }
        }
        let answers = self.host.external_rdbg_answers(&url);
        self.prepare_safe_point()?;
        if !answers {
            return Err(external_unavailable());
        }
        if !file {
            return Err(external_unverified());
        }
        Ok(receipt(
            url,
            FILE_ALIAS.to_owned(),
            Scope::ClientAndServer,
            Owner::External,
            None,
        ))
    }

    fn prepare_owned(&self, target: Target, owner_pid: u32) -> Result<Receipt, UseCaseError> {
        self.check_owner(Some(owner_pid))?;
        let owner = ProcessOwner {
            pid: owner_pid,
            started: self.host.facts(owner_pid).ok_or_else(owner_exited)?.started,
        };
        if !self.host.can_confirm_debug_endpoint() {
            return Err(environment(
                "the operating system does not provide verified debug connections for prepare",
            ));
        }
        let machine = self
            .host
            .machine()
            .ok_or_else(|| platform("cannot identify this machine to lease a debug server"))?;
        let resolution = self.debug_resolution()?;
        let store = Store::new(self.config)?;
        self.prepare_safe_point()?;
        crate::support::local_dir::prepare_work_dir(&self.config.work_path)
            .map_err(|error| io_failure("cannot prepare", &self.config.work_path, &error))?;
        let file = store.file_for(&target, owner, &machine);
        let _guard = store.lock(&file, self)?;
        self.check_prepare_owner(owner)?;
        let lease = DebugSession::new_prepare();
        if let Some(mut run) = store.read(&file, false)? {
            if run.leases.is_empty() {
                // A crashed launch can be recovered only with definitive process evidence.
                self.end_prepare_process(&run.process)?;
            } else if !self.process_ended(&run.process) {
                if run.stopping
                    || run.process.bin != resolution.path
                    || run.process.version != resolution.version
                {
                    return Err(platform(
                        "the prepared debug server has active leases and cannot be replaced",
                    ));
                }
                let port = run.process.port;
                if !self.prepared_endpoint(&run.process, port) {
                    return Err(platform("the leased debug endpoint could not be confirmed; its existing leases were retained"));
                }
                self.check_prepare_owner(owner)?;
                self.prepare_safe_point()?;
                run.leases.push(lease);
                store.write(&file, &run)?;
                return Ok(owned_receipt(&target, port, owner.pid, lease));
            }
            store.forget(&file)?;
            store.remove_run_files(run.id);
        }

        let port = self.choose_port(None)?;
        self.prepare_safe_point()?;
        let run_id = DebugSession::new();
        let paths = store.run_paths(run_id);
        let notify = paths.root().join(NOTIFY);
        let platform_notify = normalize_windows_verbatim_path(&notify);
        let launch = DebuggerLaunch {
            paths: &paths,
            program: &resolution.path,
            port,
            notify: &platform_notify,
            owner: Some(owner_pid),
            machine,
            version: resolution.version,
            protect_unidentified: true,
        };
        let make_run = |process, leases| Run {
            work: store.work.clone(),
            project: store.project.clone(),
            id: run_id,
            target: target.clone(),
            owner,
            process,
            leases,
            stopping: false,
        };
        let journal = |entry: DebuggerJournal<'_>| match entry {
            DebuggerJournal::Save(record) | DebuggerJournal::Recover(record) => {
                store.write(&file, &make_run(record.clone(), Vec::new()))
            }
            DebuggerJournal::Forget => store.forget(&file),
        };
        let SpawnedDebugger { mut record, pid } =
            match spawn_recorded_debugger(self.host, self.runner, self.context, launch, journal) {
                DebuggerSpawn::Spawned(spawned) => *spawned,
                DebuggerSpawn::ExitedAtOnce => {
                    return Err(self.exited(
                        &paths.log(),
                        port,
                        "dbgs exited as soon as it started",
                    ))
                }
                DebuggerSpawn::Failed(error) | DebuggerSpawn::Uncertain(error) => {
                    return Err(error)
                }
            };

        let actual = match await_rdbg(
            self.host,
            self.context,
            "waiting for the prepared debug server",
            &|| dbgs::notified_port(&notify),
            &|| !debug_process_ended(self.host, &record, pid),
            &|port| self.prepared_endpoint(&record, port),
        ) {
            RdbgWait::Ready(port) => Ok(port),
            RdbgWait::Gone => {
                Err(self.exited(&paths.log(), port, "dbgs exited before it answered"))
            }
            RdbgWait::TimedOut => Err(UseCaseError::new(
                UseCaseErrorKind::TimedOut,
                "the prepared dbgs did not answer on a verified listener",
            )),
            RdbgWait::Interrupted(error) => Err(error),
        };
        let publish = actual.and_then(|port| {
            self.check_prepare_owner(owner)?;
            self.prepare_safe_point()?;
            record.port = port;
            record.state = RecordState::Running;
            record.starter = None;
            store.write(&file, &make_run(record.clone(), vec![lease]))?;
            Ok(owned_receipt(&target, port, owner.pid, lease))
        });
        match publish {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                // No receipt was sent. Confirm the process exit before reporting cancellation.
                if let Err(cleanup) = self.end_prepare_process(&record) {
                    return Err(cleanup.followed_by(error.to_string()));
                }
                store.forget(&file)?;
                if error.cancellation().is_some() {
                    store.remove_run_files(run_id);
                    Err(error)
                } else {
                    Err(error.followed_by(format!("log: {}", paths.log().display())))
                }
            }
        }
    }

    pub(super) fn stop_prepared(
        &self,
        session: DebugSession,
        dry_run: bool,
    ) -> Result<Option<DebugServerResult>, UseCaseError> {
        let store = Store::new(self.config)?;
        let Some(file) = store.find_lease(session)? else {
            return Ok(None);
        };
        let _guard = if dry_run {
            None
        } else {
            Some(store.lock(&file, self)?)
        };
        let Some(mut run) = store
            .read(&file, false)?
            .filter(|run| run.leases.contains(&session))
        else {
            return Ok(None);
        };
        if self.host.machine().as_deref() != run.process.machine.as_deref() {
            return Err(platform(
                "the prepared debug lease belongs to another machine",
            ));
        }
        let mut result = empty(DebugServerAction::Planned);
        result.known_session = Some(true);
        result.owner = Some(DebugServerOwner::Runner);
        if self.process_ended(&run.process) {
            result.stale_record = true;
            if !dry_run {
                store.forget(&file)?;
                store.remove_run_files(run.id);
                result.action = DebugServerAction::NothingToStop;
            }
            return Ok(Some(result));
        }
        let pid = self
            .confirmed_prepare_process(&run.process)
            .ok_or_else(|| {
                platform("the process behind the prepared debug lease could not be identified")
            })?;
        if dry_run {
            return Ok(Some(result));
        }
        if run.leases.len() > 1 {
            run.leases.retain(|lease| *lease != session);
            store.write(&file, &run)?;
            result.action = DebugServerAction::Released;
        } else {
            run.stopping = true;
            store.write(&file, &run)?;
            self.end_prepare_process(&run.process)?;
            store.forget(&file)?;
            store.remove_run_files(run.id);
            result.action = DebugServerAction::Stopped;
            result.stopped_pid = Some(pid);
        }
        Ok(Some(result))
    }

    fn confirmed_prepare_process(&self, record: &ServiceRecord) -> Option<u32> {
        let pid = record.pid?;
        (record.image.as_deref() == Some(dbgs::executable_name())
            && record.is_own(
                self.host.facts(pid).as_ref(),
                self.host.machine().as_deref(),
            ))
        .then_some(pid)
    }

    fn prepared_endpoint(&self, record: &ServiceRecord, port: u16) -> bool {
        record
            .pid
            .is_some_and(|pid| self.verify_owned_endpoint(record, pid, port).is_ok())
    }

    fn process_ended(&self, record: &ServiceRecord) -> bool {
        record.machine.as_deref() == self.host.machine().as_deref()
            && record
                .pid
                .is_some_and(|pid| debug_process_ended(self.host, record, pid))
    }

    fn end_prepare_process(&self, record: &ServiceRecord) -> Result<(), UseCaseError> {
        if self.process_ended(record) {
            return Ok(());
        }
        let pid = self.confirmed_prepare_process(record).ok_or_else(|| {
            platform("the previous prepared dbgs could not be identified; its recovery record was retained")
        })?;
        if stop_with_confirmation(
            self.host,
            self.runner,
            self.context,
            pid,
            record.started,
            Duration::ZERO,
            |timeout| await_service_end(self.host, timeout, || self.process_ended(record)),
        ) {
            Ok(())
        } else {
            Err(platform(format!(
                "prepared dbgs pid {pid} is not confirmed stopped; its recovery record was retained"
            )))
        }
    }

    fn check_prepare_owner(&self, owner: ProcessOwner) -> Result<(), UseCaseError> {
        self.host
            .facts(owner.pid)
            .is_some_and(|facts| facts.started == owner.started)
            .then_some(())
            .ok_or_else(owner_exited)
    }

    fn prepare_safe_point(&self) -> Result<(), UseCaseError> {
        match interruption_before_safe_point(self.context, "preparing or releasing a debug server")
        {
            Some(error) => Err(error.into()),
            None => Ok(()),
        }
    }
}

fn external_unavailable() -> UseCaseError {
    UseCaseError::new(
        UseCaseErrorKind::Environment(EnvironmentReason::DebugServerExternalUnavailable),
        "the configured debug endpoint did not answer as an RDBG server",
    )
}

fn external_unverified() -> UseCaseError {
    UseCaseError::new(
        UseCaseErrorKind::Environment(EnvironmentReason::DebugServerExternalTargetUnverified),
        "the configured RDBG endpoint could not be confirmed for the selected cluster infobase",
    )
}

fn canonical(path: &Path) -> Result<PathBuf, UseCaseError> {
    nearest_existing_canonical_path(path)
        .map_err(|error| io_failure("cannot resolve", path, &error))
}

fn checked_directory(path: &Path) -> Result<(), UseCaseError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && canonical(path)? == path => Ok(()),
        Ok(_) => Err(platform(format!(
            "invalid prepared debug state directory: {}",
            path.display()
        ))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_failure("cannot read", path, &error)),
    }
}

fn owner_exited() -> UseCaseError {
    UseCaseError::new(
        UseCaseErrorKind::Validation,
        "the debug server owner exited during prepare",
    )
}

fn platform(message: impl Into<String>) -> UseCaseError {
    UseCaseError::new(UseCaseErrorKind::Platform, message)
}

fn owned_receipt(target: &Target, port: u16, owner: u32, lease: DebugSession) -> Receipt {
    receipt(
        dbgs::url(port),
        target.alias().to_owned(),
        target.scope(),
        Owner::Runner,
        Some((owner, lease)),
    )
}

fn receipt(
    url: String,
    infobase_alias: String,
    scope: Scope,
    owner: Owner,
    lease: Option<(u32, DebugSession)>,
) -> Receipt {
    Receipt::Prepared {
        url,
        infobase_alias,
        scope,
        owner,
        owner_pid: lease.map(|(pid, _)| pid),
        release: lease.map(|(_, session)| DebugServerRelease::Stop { session }),
    }
}
