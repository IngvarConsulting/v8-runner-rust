//! Select a verified debug endpoint. A runner-owned process is released by lease.
mod prepare;

use std::path::Path;
use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::config::model::AppConfig;
use crate::domain::debug_server::{
    DebugServerAction, DebugServerResult, DebugServerStatusReason, DebugSession,
};
use crate::domain::launch::PlatformResolution;
use crate::platform::dbgs::{self, DebugPort};
use crate::platform::process::ProcessRunner;
use crate::platform::service_record::ServiceRecord;
use crate::support::error::{AppError, CapabilityReason, EnvironmentLayer};
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

use super::{occupant_of, ServiceHost};

const SESSIONS: &str = "debug-server";
const NOTIFY: &str = "notify";
const FILE_ALIAS: &str = "DefAlias";
/// Полоса портов сервера отладки: ниже полосы закреплённых портов стенда (20000 и выше), выше
/// портов сервера 1С по умолчанию (1540–1591) и вне динамического диапазона Windows.
const PORT_BASE: u16 = 10_000;
const PORT_WINDOW: u16 = 32;
const PORT_WINDOWS: u16 = 312;
const PORT_TRIES: u16 = 64;
const _: () = assert!(PORT_BASE as u32 + PORT_WINDOWS as u32 * PORT_WINDOW as u32 <= 20_000);

fn refuse(message: &str) -> UseCaseError {
    UseCaseError::from(AppError::capability_for(CapabilityReason::Target, message))
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

/// Сценарий команд над записью сеанса и системой.
pub(super) struct DebugServer<'a> {
    pub context: &'a ExecutionContext,
    pub config: &'a AppConfig,
    pub runner: &'a dyn ProcessRunner,
    pub host: &'a dyn ServiceHost,
    /// Тонкий клиент, из установки которого берётся `dbgs`.
    pub locate_client: &'a dyn Fn() -> Result<PlatformResolution, UseCaseError>,
}

impl DebugServer<'_> {
    fn debug_resolution(&self) -> Result<PlatformResolution, UseCaseError> {
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
        Ok(PlatformResolution {
            path: program,
            ..client
        })
    }

    fn choose_port(&self) -> Result<DebugPort, UseCaseError> {
        port_windows(&self.config.work_path)
            .find(|(first, last)| {
                (*first..=*last).any(|port| occupant_of(self.host, port).is_none())
            })
            .map(|(first, last)| DebugPort::Window { first, last })
            .ok_or_else(|| {
                UseCaseError::from(AppError::environment(
                    "every port window of the debug server is taken",
                ))
            })
    }

    fn verify_owned_endpoint(
        &self,
        record: &ServiceRecord,
        pid: u32,
        port: u16,
    ) -> Result<(), DebugServerStatusReason> {
        use DebugServerStatusReason as Reason;

        let verify_process = || {
            let machine = self.host.machine().ok_or(Reason::ProcessUnverified)?;
            if record.machine.as_deref() != Some(machine.as_str()) {
                return Err(Reason::ProcessChanged);
            }
            match self.host.facts(pid) {
                Some(facts) if record.is_own(Some(&facts), Some(&machine)) => Ok(()),
                Some(_) => Err(Reason::ProcessChanged),
                None if self.host.debug_process_gone(pid) => Err(Reason::ProcessGone),
                None => Err(Reason::ProcessUnverified),
            }
        };
        let verify_listener = || {
            self.host
                .debug_loopback_owner(pid, port)
                .then_some(())
                .ok_or(Reason::ListenerUnverified)
        };

        verify_process()?;
        verify_listener()?;
        match self.host.debug_rdbg_answers(pid, port) {
            Some(true) => {}
            Some(false) => return Err(Reason::RdbgUnavailable),
            None => return Err(Reason::ListenerUnverified),
        }
        verify_listener()?;
        verify_process()
    }

    pub fn stop(
        &self,
        session: DebugSession,
        dry_run: bool,
    ) -> Result<DebugServerResult, UseCaseError> {
        let started = Instant::now();
        let released = if session.is_prepare() {
            self.stop_prepared(session, dry_run)?
        } else {
            None
        };
        let mut result = released.unwrap_or_else(|| {
            let mut result = empty(DebugServerAction::NothingToStop);
            result.known_session = Some(false);
            result
        });
        result.session = Some(session);
        result.duration_ms = crate::support::time::elapsed_ms(started);
        Ok(result)
    }
}

fn empty(action: DebugServerAction) -> DebugServerResult {
    DebugServerResult {
        ok: true,
        action,
        session: None,
        owner: None,
        known_session: None,
        stale_record: false,
        stopped_pid: None,
        duration_ms: 0,
        warnings: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;
    use crate::config::model::{InfobaseClusterConfig, InfobaseConfig, ManagedServerConfig};
    use crate::domain::debug_server::DebugServerStatus;
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
        /// Назвал адрес, но не отвечает по `rdbg`.
        never_answers: Cell<bool>,
        answers_external: Cell<bool>,
        external_probes: RefCell<Vec<String>>,
        replace_listener_during_probe: Cell<bool>,
        during_rdbg: RefCell<Option<Box<dyn FnOnce()>>>,
        exit_during_rdbg: Cell<bool>,
        process_inaccessible: Cell<bool>,
        proof_unavailable: Cell<bool>,
        only_prepared_endpoint_proof: Cell<bool>,
        exit_pending: Cell<bool>,
        /// Порт из файла адреса слушает чужой процесс.
        foreign_listener: Cell<bool>,
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
        fn can_confirm_debug_endpoint(&self) -> bool {
            !self.only_prepared_endpoint_proof.get()
        }

        fn can_confirm_owned_debug_endpoint(&self) -> bool {
            true
        }

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
            self.processes.borrow_mut().insert(
                PID,
                ProcessFacts {
                    started: STARTED,
                    image: dbgs::executable_name().to_owned(),
                },
            );
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
        fn debug_loopback_owner(&self, pid: u32, port: u16) -> bool {
            !self.proof_unavailable.get() && self.listening_pid(port) == Some(pid)
        }
        fn debug_process_gone(&self, pid: u32) -> bool {
            !self.process_inaccessible.get() && !self.exit_pending.get() && !self.alive(pid)
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
        fn sleep(&self, _duration: Duration) {
            self.exit_pending.set(false);
        }

        fn patience(&self, _default: Duration) -> Duration {
            if self.exit_pending.get() {
                Duration::from_secs(1)
            } else {
                Duration::ZERO
            }
        }
        fn rdbg_answers(&self, port: u16) -> bool {
            if let Some(callback) = self.during_rdbg.borrow_mut().take() {
                callback();
            }
            if self.replace_listener_during_probe.get() {
                self.listeners.borrow_mut().insert(port, OWNER);
            }
            if self.exit_during_rdbg.get() {
                self.processes.borrow_mut().remove(&PID);
                self.listeners.borrow_mut().remove(&port);
            }
            !self.never_answers.get() && self.listeners.borrow().contains_key(&port)
        }
        fn external_rdbg_answers(&self, address: &str) -> bool {
            self.external_probes.borrow_mut().push(address.to_owned());
            self.answers_external.get()
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

        fn server<'a>(
            &'a self,
            context: &'a ExecutionContext,
            host: &'a Host,
            locate: &'a dyn Fn() -> Result<PlatformResolution, UseCaseError>,
        ) -> DebugServer<'a> {
            DebugServer {
                context,
                config: &self.config,
                runner: &Runner,
                host,
                locate_client: locate,
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

        fn stop(
            &self,
            session: DebugSession,
            host: &Host,
        ) -> Result<DebugServerResult, UseCaseError> {
            let locate = || Ok(self.resolution());
            self.server(&context(), host, &locate).stop(session, false)
        }
    }

    fn sessions_root(config: &AppConfig) -> PathBuf {
        config.work_path.join(SESSIONS)
    }

    fn context() -> ExecutionContext {
        ExecutionContext::cli(CommandName::DebugServerStart)
    }

    fn state_file(config: &AppConfig) -> PathBuf {
        let prepared = sessions_root(config).join("prepared-v1");
        let scope = std::fs::read_dir(prepared)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        std::fs::read_dir(scope.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "json"))
            .unwrap()
    }

    #[test]
    fn a_missing_debug_server_is_an_environment_refusal_naming_the_installation() {
        let setup = setup();
        let host = Host::with_owner();
        std::fs::remove_file(dbgs::beside(&setup.client)).unwrap();
        let locate = || Ok(setup.resolution());
        let context = context();
        let server = setup.server(&context, &host, &locate);
        let error = server.start(OWNER, false).unwrap_err();
        assert!(matches!(error.kind(), UseCaseErrorKind::Environment(_)));
        assert!(error.to_string().contains("8.3.27.2342"));
        assert!(error.to_string().contains(dbgs::executable_name()));
        assert_eq!(host.spawned.get(), 0);
        assert!(!setup.config.work_path.exists());
    }

    #[test]
    fn debug_start_rejects_a_dead_owner_and_the_runner_itself_before_work() {
        let setup = setup();
        let host = Host::default();
        let locate = || panic!("invalid owner must be rejected before resolving the platform");
        let context = context();
        let server = setup.server(&context, &host, &locate);
        for owner in [OWNER, std::process::id(), 0] {
            let error = server.start(owner, false).unwrap_err();
            assert_eq!(error.kind(), UseCaseErrorKind::Validation);
        }
        assert_eq!(host.spawned.get(), 0);
        assert!(!setup.config.work_path.exists());
    }

    #[test]
    fn debug_start_does_not_publish_exited_unresponsive_or_foreign_processes() {
        for mode in [
            "early_exit",
            "exit_during_rdbg",
            "foreign_listener",
            "no_rdbg",
        ] {
            let setup = setup();
            let host = Host::with_owner();
            host.exits_at_once.set(mode == "early_exit");
            host.exit_during_rdbg.set(mode == "exit_during_rdbg");
            host.foreign_listener.set(mode == "foreign_listener");
            host.never_answers.set(mode == "no_rdbg");
            let locate = || Ok(setup.resolution());
            let context = context();
            let server = setup.server(&context, &host, &locate);
            let error = server.start(OWNER, false).unwrap_err();
            assert_eq!(
                error.kind(),
                if matches!(mode, "early_exit" | "exit_during_rdbg") {
                    UseCaseErrorKind::Platform
                } else {
                    UseCaseErrorKind::TimedOut
                },
                "{mode}: {error}"
            );
            assert!(error.to_string().contains("log:"), "{mode}: {error}");
            assert!(!host.alive(PID), "{mode}");
            assert!(!host.soft_stops.borrow().contains(&4343));
            assert!(!host.hard_stops.borrow().contains(&4343));
            assert_eq!(
                server.status(DebugSession::new_prepare()).unwrap().status,
                DebugServerStatus::Unknown,
                "{mode}"
            );
        }
    }

    #[test]
    fn status_confirms_only_the_recorded_listener_without_mutating_the_session() {
        let setup = setup();
        let host = Host::with_owner();
        let locate = || Ok(setup.resolution());
        let context = context();
        let server = setup.server(&context, &host, &locate);
        let lease = prepared_lease(server.start(OWNER, false).unwrap());
        assert_eq!(
            arg(&host.last_args.borrow(), "--ownerPID="),
            Some(OWNER.to_string().as_str())
        );
        let file = state_file(&setup.config);
        let original = std::fs::read(&file).unwrap();
        let ready = server.status(lease).unwrap();
        assert_eq!(ready.status, DebugServerStatus::Ready);
        assert_eq!(ready.session, lease);
        assert_eq!(ready.pid, Some(PID));
        let port = ready.port.unwrap();
        assert_eq!(ready.url, Some(dbgs::url(port)));
        assert_eq!(ready.infobase_alias.as_deref(), Some(FILE_ALIAS));
        assert_eq!(ready.reason, None);
        for other in [DebugSession::new(), DebugSession::new_prepare()] {
            assert_eq!(
                server.status(other).unwrap().status,
                DebugServerStatus::Unknown
            );
        }

        let assert_unready = |status, reason| {
            let denied = server.status(lease).unwrap();
            assert_eq!(denied.status, status);
            assert_eq!(denied.reason, Some(reason));
            assert_eq!(denied.pid, None);
            assert_eq!(denied.port, None);
            assert_eq!(denied.url, None);
            assert_eq!(denied.infobase_alias, None);
        };
        host.never_answers.set(true);
        assert_unready(
            DebugServerStatus::Unverified,
            DebugServerStatusReason::RdbgUnavailable,
        );
        host.never_answers.set(false);
        host.replace_listener_during_probe.set(true);
        assert_unready(
            DebugServerStatus::Unverified,
            DebugServerStatusReason::ListenerUnverified,
        );
        host.replace_listener_during_probe.set(false);
        host.listeners.borrow_mut().insert(port, PID);
        host.processes.borrow_mut().get_mut(&OWNER).unwrap().started += 1;
        assert_unready(
            DebugServerStatus::Stale,
            DebugServerStatusReason::ProcessChanged,
        );
        host.processes.borrow_mut().get_mut(&OWNER).unwrap().started -= 1;
        host.processes.borrow_mut().get_mut(&PID).unwrap().started += 1;
        assert_unready(
            DebugServerStatus::Stale,
            DebugServerStatusReason::ProcessChanged,
        );

        assert_eq!(std::fs::read(&file).unwrap(), original);
        assert_eq!(host.spawned.get(), 1);
        assert!(host.soft_stops.borrow().is_empty());
        assert!(host.hard_stops.borrow().is_empty());
    }

    #[test]
    fn status_refuses_a_lease_that_begins_stopping_during_rdbg() {
        let setup = setup();
        let host = Host::with_owner();
        let locate = || Ok(setup.resolution());
        let context = context();
        let server = setup.server(&context, &host, &locate);
        let lease = prepared_lease(server.start(OWNER, false).unwrap());
        let file = state_file(&setup.config);
        let changed = |path: &Path| {
            let mut run: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            run["stopping"] = serde_json::Value::Bool(true);
            serde_json::to_vec(&run).unwrap()
        };
        let replacement = changed(&file);
        let owned_path = file.clone();
        let bytes = replacement.clone();
        host.during_rdbg.replace(Some(Box::new(move || {
            std::fs::write(&owned_path, &bytes).unwrap();
        })));
        for _ in 0..2 {
            let status = server.status(lease).unwrap();
            assert_eq!(status.status, DebugServerStatus::Unverified);
            assert_eq!(status.reason, Some(DebugServerStatusReason::RecordChanged));
            assert_eq!(status.url, None);
        }
        assert_eq!(std::fs::read(&file).unwrap(), replacement);
        assert!(host.alive(PID));
        assert!(host.soft_stops.borrow().is_empty());
        assert!(host.hard_stops.borrow().is_empty());
    }

    #[test]
    fn status_refuses_live_lease_when_the_namespace_cannot_be_confirmed() {
        let setup = setup();
        let host = Host::with_owner();
        let locate = || Ok(setup.resolution());
        let context = context();
        let server = setup.server(&context, &host, &locate);
        let lease = prepared_lease(server.start(OWNER, false).unwrap());
        let file = state_file(&setup.config);
        let unresolved = file.with_file_name(format!("{}.json", "0".repeat(64)));
        std::fs::write(&unresolved, b"invalid record").unwrap();

        let status = server.status(lease).unwrap();
        assert_eq!(status.status, DebugServerStatus::Unverified);
        assert_eq!(
            status.reason,
            Some(DebugServerStatusReason::RecordUnavailable)
        );
        assert_eq!(status.url, None);
        assert_eq!(std::fs::read(&unresolved).unwrap(), b"invalid record");
        assert!(host.alive(PID));
    }

    fn prepared_lease(
        receipt: crate::domain::debug_server::DebugServerPrepareResult,
    ) -> DebugSession {
        use crate::domain::debug_server::{DebugServerPrepareResult, DebugServerRelease};
        let DebugServerPrepareResult::Prepared {
            release: Some(DebugServerRelease::Stop { session }),
            ..
        } = receipt
        else {
            panic!("expected an owned debug server with a release token");
        };
        session
    }

    #[test]
    fn prepared_leases_release_independently_even_after_target_selection_changes() {
        let mut setup = setup();
        let host = Host::with_owner();
        let (first, second) = {
            let ctx = context();
            let locate = || Ok(setup.resolution());
            let server = setup.server(&ctx, &host, &locate);
            (
                prepared_lease(server.start(OWNER, false).unwrap()),
                prepared_lease(server.start(OWNER, false).unwrap()),
            )
        };
        assert_ne!(first, second);
        assert_eq!(host.spawned.get(), 1);
        setup.config.infobase = InfobaseConfig::file("Srvr=other;Ref=another".to_owned());
        let locate = || Ok(setup.resolution());
        let ctx = context();
        let server = setup.server(&ctx, &host, &locate);
        let before = server.status(first).unwrap();
        assert_eq!(before.status, DebugServerStatus::Ready);
        assert_eq!(before.infobase_alias.as_deref(), Some(FILE_ALIAS));
        let release = setup.stop(first, &host).unwrap();
        assert_eq!(release.action, DebugServerAction::Released);
        assert_eq!(release.known_session, Some(true));
        assert_eq!(release.session, Some(first));
        assert_eq!(release.stopped_pid, None);
        assert!(host.alive(PID));
        assert_eq!(
            setup.stop(first, &host).unwrap().action,
            DebugServerAction::NothingToStop
        );
        assert_eq!(
            server.status(first).unwrap().status,
            DebugServerStatus::Unknown
        );
        assert_eq!(
            server.status(second).unwrap().status,
            DebugServerStatus::Ready
        );
        let last = setup.stop(second, &host).unwrap();
        assert_eq!(last.action, DebugServerAction::Stopped);
        assert_eq!(last.stopped_pid, Some(PID));
        assert!(!host.alive(PID));
    }

    #[test]
    fn prepare_preview_does_not_locate_resolve_owner_or_write_state() {
        use crate::domain::debug_server::DebugServerPrepareResult;
        let setup = setup();
        let host = Host::default();
        let ctx = context();
        let locate = || panic!("preview must not locate the client");
        let receipt = setup
            .server(&ctx, &host, &locate)
            .start(OWNER, true)
            .unwrap();
        assert!(matches!(receipt, DebugServerPrepareResult::Planned));
        assert!(!setup.config.work_path.exists());
        assert_eq!(host.spawned.get(), 0);
    }

    #[test]
    fn managed_prepare_refuses_rdbg_without_a_matching_cluster_binding() {
        let setup = setup().on_managed_cluster();
        let host = Host::with_owner();
        let ctx = context();
        let locate = || panic!("managed prepare must not locate a local dbgs");
        assert!(setup
            .server(&ctx, &host, &locate)
            .start(OWNER, false)
            .is_err());
        assert!(!setup.config.work_path.exists());
        assert_eq!(host.spawned.get(), 0);
    }

    #[test]
    fn prepared_stop_waits_for_death_after_identity_becomes_unavailable() {
        let setup = setup();
        let host = Host::with_owner();
        let context = context();
        let locate = || Ok(setup.resolution());
        let server = setup.server(&context, &host, &locate);
        let session = prepared_lease(server.start(OWNER, false).unwrap());
        host.exit_pending.set(true);

        let stopped = server.stop(session, false).unwrap();

        assert_eq!(stopped.action, DebugServerAction::Stopped);
        assert_eq!(stopped.stopped_pid, Some(PID));
        assert!(!host.exit_pending.get());
        assert!(host.hard_stops.borrow().is_empty());
    }

    #[test]
    fn prepared_stop_does_not_signal_a_recycled_pid() {
        let setup = setup();
        let host = Host::with_owner();
        let ctx = context();
        let locate = || Ok(setup.resolution());
        let server = setup.server(&ctx, &host, &locate);
        let session = prepared_lease(server.start(OWNER, false).unwrap());
        host.processes.borrow_mut().get_mut(&PID).unwrap().started += 1;
        let stale = server.stop(session, false).unwrap();
        assert_eq!(stale.action, DebugServerAction::NothingToStop);
        assert!(stale.stale_record);
        assert!(host.alive(PID));
        assert!(host.soft_stops.borrow().is_empty());
        assert!(host.hard_stops.borrow().is_empty());
    }

    #[test]
    fn prepare_distinguishes_unavailable_and_unverified_external_cluster() {
        use crate::config::model::InfobaseDebugConfig;
        use crate::domain::launch::DebuggerUrl;

        let mut setup = setup();
        setup.config.infobase = InfobaseConfig::file("Srvr=external;Ref=demo".to_owned());
        let address = "http://debug.example:1548";
        setup.config.infobase.debug = Some(InfobaseDebugConfig {
            url: DebuggerUrl::parse(address).unwrap(),
            infobase_alias: Some("demo".to_owned()),
        });
        let host = Host::default();
        let ctx = context();
        let locate = || panic!("configured external route must not locate a local dbgs");
        let server = setup.server(&ctx, &host, &locate);
        assert_eq!(
            server.start(OWNER, false).unwrap_err().kind(),
            UseCaseErrorKind::Environment(EnvironmentReason::DebugServerExternalUnavailable)
        );
        host.answers_external.set(true);
        assert_eq!(
            server.start(OWNER, false).unwrap_err().kind(),
            UseCaseErrorKind::Environment(EnvironmentReason::DebugServerExternalTargetUnverified)
        );
        assert_eq!(host.external_probes.borrow().as_slice(), [address, address]);
        assert!(!setup.config.work_path.exists());
        assert_eq!(host.spawned.get(), 0);
    }

    #[test]
    fn prepare_borrows_exact_explicit_file_endpoint_without_a_lease() {
        use crate::config::model::InfobaseDebugConfig;
        use crate::domain::debug_server::{
            DebugServerPrepareOwner, DebugServerPrepareResult, DebugServerPrepareScope,
        };
        use crate::domain::launch::DebuggerUrl;

        let mut setup = setup();
        setup.config.infobase.debug = Some(InfobaseDebugConfig {
            url: DebuggerUrl::parse("http://debug.example:1548").unwrap(),
            infobase_alias: None,
        });
        let host = Host::default();
        host.answers_external.set(true);
        let ctx = context();
        let locate = || panic!("external file route must not locate a local dbgs");
        let server = setup.server(&ctx, &host, &locate);
        assert!(matches!(
            server.start(OWNER, false).unwrap(),
            DebugServerPrepareResult::Prepared {
                url, infobase_alias, owner: DebugServerPrepareOwner::External,
                scope: DebugServerPrepareScope::ClientAndServer, owner_pid: None, release: None,
            } if url == "http://debug.example:1548" && infobase_alias == FILE_ALIAS
        ));
        assert!(!setup.config.work_path.exists());
        assert_eq!(host.spawned.get(), 0);
    }

    #[test]
    fn prepare_uses_client_only_local_debug_for_external_cluster_without_an_address() {
        use crate::domain::debug_server::{
            DebugServerPrepareOwner, DebugServerPrepareResult, DebugServerPrepareScope,
        };

        let mut setup = setup();
        setup.config.infobase = InfobaseConfig::file("Srvr=external;Ref=demo".to_owned());
        let host = Host::with_owner();
        host.only_prepared_endpoint_proof.set(true);
        let ctx = context();
        let locate = || Ok(setup.resolution());
        let receipt = setup
            .server(&ctx, &host, &locate)
            .start(OWNER, false)
            .unwrap();
        assert!(matches!(
            receipt,
            DebugServerPrepareResult::Prepared {
                url, infobase_alias, scope: DebugServerPrepareScope::Client,
                owner: DebugServerPrepareOwner::Runner, owner_pid: Some(OWNER), release: Some(_),
            } if url.starts_with("http://127.0.0.1:") && infobase_alias == "demo"
        ));
        assert!(host.external_probes.borrow().is_empty());
        assert_eq!(host.spawned.get(), 1);
    }

    #[test]
    fn prepare_reuses_only_with_verified_ownership() {
        let setup = setup();
        let host = Host::with_owner();
        host.only_prepared_endpoint_proof.set(true);
        let ctx = context();
        let locate = || Ok(setup.resolution());
        let server = setup.server(&ctx, &host, &locate);
        let first = prepared_lease(server.start(OWNER, false).unwrap());
        host.proof_unavailable.set(true);
        let failure = server.start(OWNER, false).unwrap_err();
        assert_eq!(failure.kind(), UseCaseErrorKind::Platform);
        assert_eq!(host.spawned.get(), 1);
        assert!(host.alive(PID));
        host.proof_unavailable.set(false);
        let second = prepared_lease(server.start(OWNER, false).unwrap());
        assert_ne!(first, second);
        assert_eq!(host.spawned.get(), 1);
        assert_eq!(
            server.stop(first, false).unwrap().action,
            DebugServerAction::Released
        );
        assert_eq!(server.stop(second, false).unwrap().stopped_pid, Some(PID));
        assert!(!host.alive(PID));
    }

    #[test]
    fn legacy_unknown_stop_ignores_an_unreadable_prepared_namespace() {
        let setup = setup();
        let host = Host::default();
        let sessions = sessions_root(&setup.config);
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("prepared-v1"), "invalid directory").unwrap();
        let old_token = DebugSession::new();
        let result = setup.stop(old_token, &host).unwrap();
        assert_eq!(result.action, DebugServerAction::NothingToStop);
        assert_eq!(result.known_session, Some(false));
    }

    #[test]
    fn unresolved_prepared_scope_cannot_confirm_absence_of_a_lease() {
        let setup = setup();
        let host = Host::with_owner();
        let ctx = context();
        let locate = || Ok(setup.resolution());
        let server = setup.server(&ctx, &host, &locate);
        let session = prepared_lease(server.start(OWNER, false).unwrap());
        let prepared = sessions_root(&setup.config).join("prepared-v1");
        let project = std::fs::read_dir(&prepared)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let invalid = project.join(format!("{}.json", "0".repeat(64)));
        std::fs::write(invalid, "broken state").unwrap();
        let unresolved = server.stop(DebugSession::new_prepare(), false).unwrap_err();
        assert_eq!(unresolved.kind(), UseCaseErrorKind::Platform);
        assert!(host.alive(PID));
        assert_eq!(server.stop(session, false).unwrap().stopped_pid, Some(PID));
        assert!(!host.alive(PID));
    }

    #[test]
    fn copied_prepared_scope_cannot_stop_the_original_debug_server() {
        let mut setup = setup();
        let host = Host::with_owner();
        let session = {
            let ctx = context();
            let locate = || Ok(setup.resolution());
            let server = setup.server(&ctx, &host, &locate);
            prepared_lease(server.start(OWNER, false).unwrap())
        };
        let original = setup.config.work_path.clone();
        let project = std::fs::read_dir(sessions_root(&setup.config).join("prepared-v1"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        setup.config.work_path = setup.config.base_path.join("copied-work");
        let copy = sessions_root(&setup.config)
            .join("prepared-v1")
            .join(project.file_name());
        std::fs::create_dir_all(&copy).unwrap();
        let record = std::fs::read_dir(project.path())
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .unwrap();
        std::fs::copy(record.path(), copy.join(record.file_name())).unwrap();

        assert_eq!(
            setup.stop(session, &host).unwrap().action,
            DebugServerAction::NothingToStop
        );
        assert!(host.alive(PID));
        setup.config.work_path = original;
        assert_eq!(setup.stop(session, &host).unwrap().stopped_pid, Some(PID));
    }

    #[test]
    fn prepare_cancellation_stops_only_the_unpublished_owned_attempt() {
        let setup = setup();
        let host = Host::with_owner();
        let token = tokio_util::sync::CancellationToken::new();
        host.cancel_on_spawn.replace(Some(token.clone()));
        let ctx = context().with_cancellation(token);
        let locate = || Ok(setup.resolution());
        let server = setup.server(&ctx, &host, &locate);
        assert!(matches!(
            server.start(OWNER, false).unwrap_err().kind(),
            UseCaseErrorKind::Cancelled(_)
        ));
        assert!(!host.alive(PID));
        assert_eq!(host.spawned.get(), 1);
        host.cancel_on_spawn.replace(None);
        let ctx = context();
        let server = setup.server(&ctx, &host, &locate);
        assert!(server.start(OWNER, false).is_ok());
        assert!(host.alive(PID));
        assert_eq!(host.spawned.get(), 2);
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
