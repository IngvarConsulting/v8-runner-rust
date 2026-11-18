//! Locate and prove the cluster administration endpoint before handing it to a command.

use std::net::{IpAddr, Ipv4Addr, TcpListener, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::model::{AppConfig, ClusterAdministration};
use crate::platform::connection::V8Connection;
use crate::platform::locator::{platform_version_of, UtilityType};
use crate::platform::process::{ManagedSpawnResult, ProcessRunner};
use crate::platform::rac;
use crate::platform::ragent::ras_address;
use crate::platform::service;
use crate::platform::service_record::running_ras;
use crate::platform::utilities::PlatformUtilities;
use crate::support::authority::{host_and_port_of_authority, host_of_authority, Host};
use crate::support::error::{AppError, EnvironmentLayer};
use crate::use_cases::context::{ExecutionContext, ExecutionTransport, InterruptionSafetyClass};
use crate::use_cases::interruption::interruption_before_safe_point;

const CLUSTER_MANAGER_PORT: u16 = 1541;
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const LISTENER_TIMEOUT: Duration = Duration::from_secs(2);
const LISTENER_POLL: Duration = Duration::from_millis(30);

/// A ready endpoint and the lifetime of its transient listener, which remains owned by the
/// calling command. Both normal return and cancellation drop the handle. The kernel removes
/// the child if the runner dies before it can drop the handle.
pub(crate) struct ClusterAdmin {
    rac: PathBuf,
    address: String,
    cluster: String,
    process: Option<ManagedSpawnResult>,
}

impl Drop for ClusterAdmin {
    fn drop(&mut self) {
        if let Some(process) = self.process.take() {
            process.remove();
        }
    }
}

impl ClusterAdmin {
    pub(crate) fn connect(
        context: &ExecutionContext,
        config: &AppConfig,
        runner: &dyn ProcessRunner,
    ) -> Result<Self, AppError> {
        check_cancelled(context)?;
        let connection = config.v8_connection();
        if let Some(address) = config
            .infobase
            .cluster
            .as_ref()
            .and_then(|cluster| cluster.ras.as_deref())
        {
            let candidates = candidates(config)?;
            return connect_to_ras(context, runner, &connection, address, candidates);
        }
        if config
            .infobase
            .cluster
            .as_ref()
            .is_some_and(|cluster| cluster.managed.is_some())
        {
            let (program, address) = recorded_ras(config)?;
            return connect_to_ras(context, runner, &connection, &address, vec![program]);
        }
        let ClusterAdministration::ManagedRasForAgent { host, port } = config
            .infobase
            .cluster_administration()
            .map_err(|error| AppError::Validation(error.to_string()))?
        else {
            unreachable!("declared RAS returns above");
        };
        if !cfg!(any(windows, target_os = "linux")) {
            return Err(environment("command-managed ras requires Windows or Linux for crash-safe cleanup; declare infobase.cluster.ras"));
        }
        if context.transport() != ExecutionTransport::Cli {
            return Err(environment("command-managed ras requires a dedicated CLI process; declare infobase.cluster.ras"));
        }
        let candidates = candidates(config)?;
        connect_via_agent(
            context,
            runner,
            &connection,
            &format!("{host}:{port}"),
            candidates,
        )
    }

    pub(crate) fn find_infobase(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        runner: &dyn ProcessRunner,
        reference: &str,
    ) -> Result<Option<uuid::Uuid>, AppError> {
        let auth = cluster_auth(config);
        let command = rac::with_cluster_auth(
            rac::infobase_list(self.rac(), self.address(), self.cluster()),
            auth,
        );
        let policy = context.process_policy(
            InterruptionSafetyClass::GracefulThenKill,
            Some(Duration::from_secs(15)),
        );
        let output = rac::probe_infobases(runner, &command, &policy)?;
        if output.outcome().is_err() {
            let operation = format!(
                "rac infobase summary list exited with code {}",
                output.exit_code
            );
            return Err(if auth.user.is_none() {
                AppError::environment(format!(
                    "{operation}; if this server requires authentication, {CLUSTER_CREDENTIALS}"
                ))
            } else {
                AppError::Platform(operation)
            });
        }
        rac::checked_infobase_id(&output.stdout, reference).ok_or_else(|| {
            AppError::InvalidOutput(
                "the cluster returned an incomplete or ambiguous infobase list".to_owned(),
            )
        })
    }

    pub(crate) fn rac(&self) -> &Path {
        &self.rac
    }

    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    pub(crate) fn cluster(&self) -> &str {
        &self.cluster
    }

    pub(crate) fn managed_ras(&self) -> bool {
        self.process.is_some()
    }
}

pub(crate) const CLUSTER_CREDENTIALS: &str =
    "cluster administrator level: declare infobase.cluster.user and infobase.cluster.password";

pub(crate) fn cluster_auth(config: &AppConfig) -> rac::Credentials<'_> {
    let cluster = config.infobase.cluster.as_ref();
    rac::Credentials {
        user: cluster
            .and_then(|cluster| cluster.user.as_deref())
            .filter(|user| !user.trim().is_empty()),
        password: cluster.and_then(|cluster| cluster.password.as_deref()),
    }
}

fn candidates(config: &AppConfig) -> Result<Vec<PathBuf>, AppError> {
    let mut utilities = PlatformUtilities::from_config(config);
    utilities.rac_candidates().map_err(|error| {
        let installed = utilities.installations().iter().map(|entry| entry.version.to_string())
            .collect::<Vec<_>>().join(", ");
        environment(format!("{error}; installed platform versions: {}; install matching server tools or declare infobase.cluster.ras", shown(&installed)))
    })
}

fn recorded_ras(config: &AppConfig) -> Result<(PathBuf, String), AppError> {
    let fail = || {
        environment("the runner-managed cluster has no live RAS record for this base; run cluster start or declare infobase.cluster.ras")
    };
    let local = config.local_dir();
    let base = config.infobase_name.as_deref().ok_or_else(fail)?;
    let record = running_ras(&local, base).ok_or_else(fail)?;
    let client = record
        .bin
        .with_file_name(UtilityType::Rac.executable_name());
    Ok((client, ras_address(record.port)))
}

fn connect_to_ras(
    context: &ExecutionContext,
    runner: &dyn ProcessRunner,
    connection: &V8Connection,
    address: &str,
    candidates: Vec<PathBuf>,
) -> Result<ClusterAdmin, AppError> {
    for program in &candidates {
        let Some(cluster) = probe(context, runner, connection, program, address)? else {
            continue;
        };
        return Ok(ClusterAdmin {
            rac: program.clone(),
            address: address.to_owned(),
            cluster,
            process: None,
        });
    }
    Err(environment(format!("RAS at {address} did not return a valid cluster list using installed rac versions: {}; check infobase.cluster.ras and install the matching administration client", versions(&candidates))))
}

fn connect_via_agent(
    context: &ExecutionContext,
    runner: &dyn ProcessRunner,
    connection: &V8Connection,
    agent: &str,
    candidates: Vec<PathBuf>,
) -> Result<ClusterAdmin, AppError> {
    for program in &candidates {
        check_cancelled(context)?;
        let Some(ras) = matching_ras(program) else {
            continue;
        };
        let port = free_port()?;
        let address = ras_address(port);
        let mut process =
            match crate::platform::ragent::spawn_command_ras(runner, &ras, port, agent) {
                Ok(process) => process,
                Err(error) => {
                    tracing::debug!(%error, "could not start this RAS candidate");
                    continue;
                }
            };
        match prove_command_ras(
            context,
            runner,
            connection,
            program,
            &address,
            port,
            &mut process,
        ) {
            Ok(Some(cluster)) => {
                return Ok(ClusterAdmin {
                    rac: program.clone(),
                    address,
                    cluster,
                    process: Some(process),
                });
            }
            result => {
                process.remove();
                result?;
            }
        }
    }
    Err(environment(format!("no installed RAS returned a valid cluster list for agent {agent}; installed rac versions: {}; install the server tools of the cluster's exact version or declare infobase.cluster.ras", versions(&candidates))))
}

fn prove_command_ras(
    context: &ExecutionContext,
    runner: &dyn ProcessRunner,
    connection: &V8Connection,
    program: &Path,
    address: &str,
    port: u16,
    process: &mut ManagedSpawnResult,
) -> Result<Option<String>, AppError> {
    if !wait_for_own_listener(context, port, process)? {
        return Ok(None);
    }
    let evidence = probe(context, runner, connection, program, address)?;
    if process.exited().is_some() || !service::listens_on(process.pid(), port) {
        return Ok(None);
    }
    Ok(evidence)
}

fn matching_ras(rac: &Path) -> Option<PathBuf> {
    let ras = rac.with_file_name(format!("ras{}", std::env::consts::EXE_SUFFIX));
    let resolved = crate::support::path::normalize_windows_verbatim_path(&ras.canonicalize().ok()?);
    // Use the paired executable in the same canonical directory. Runtime compatibility is
    // established by the cluster-list probe, not by either executable's own version output.
    (resolved.is_file() && resolved.parent() == rac.parent()).then_some(resolved)
}

fn free_port() -> Result<u16, AppError> {
    TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|socket| socket.local_addr())
        .map(|address| address.port())
        .map_err(|error| environment(format!("could not choose a free RAS port: {error}")))
}

#[derive(Debug, PartialEq, Eq)]
enum Listener {
    Pending,
    Ready,
    Foreign,
}

fn classify_listener(pid: u32, port: u16) -> Listener {
    if service::listens_on(pid, port) {
        return Listener::Ready;
    }
    if !service::port_answers(port) {
        return Listener::Pending;
    }
    // The child can bind between the first ownership query and the connection.
    if service::listens_on(pid, port) {
        Listener::Ready
    } else {
        Listener::Foreign
    }
}

fn wait_for_own_listener(
    context: &ExecutionContext,
    port: u16,
    process: &mut ManagedSpawnResult,
) -> Result<bool, AppError> {
    let deadline = Instant::now() + LISTENER_TIMEOUT;
    loop {
        check_cancelled(context)?;
        if process.exited().is_some() || Instant::now() >= deadline {
            return Ok(false);
        }
        let pid = process.pid();
        // The only process whose listener can be trusted is the owned child; a port that
        // someone else took is evidence to abandon this candidate without sending RAC traffic.
        match classify_listener(pid, port) {
            Listener::Ready => return Ok(true),
            Listener::Foreign => return Ok(false),
            Listener::Pending => std::thread::sleep(LISTENER_POLL),
        }
    }
}

fn probe(
    context: &ExecutionContext,
    runner: &dyn ProcessRunner,
    connection: &V8Connection,
    program: &Path,
    address: &str,
) -> Result<Option<String>, AppError> {
    check_cancelled(context)?;
    let policy = context.process_policy(
        InterruptionSafetyClass::GracefulThenKill,
        Some(PROBE_TIMEOUT),
    );
    let result = rac::probe_clusters(runner, program, address, &policy);
    check_cancelled(context)?;
    match result {
        Ok(clusters) if !clusters.is_empty() => select_cluster(&clusters, connection).map(Some),
        _ => Ok(None),
    }
}

fn select_cluster(
    clusters: &[rac::Cluster],
    connection: &V8Connection,
) -> Result<String, AppError> {
    let hosts = connection.cluster_hosts();
    let (target, port) = hosts
        .iter()
        .find(|host| !host.is_empty())
        .and_then(|host| host_and_port_of_authority(host))
        .ok_or_else(|| {
            AppError::Validation(
                "infobase.connection does not name a cluster manager in Srvr= or /S".to_owned(),
            )
        })?;
    let port = port.unwrap_or(CLUSTER_MANAGER_PORT);
    let mut matches = clusters.iter().filter(|cluster| {
        cluster.port == port
            && host_of_authority(&cluster.host)
                .is_some_and(|candidate| same_manager_host(&target, &candidate))
    });
    match (matches.next(), matches.next()) {
        (Some(cluster), None) => Ok(cluster.id.clone()),
        (first, _) => {
            let reason = if first.is_some() {
                "more than one cluster"
            } else {
                "no cluster"
            };
            Err(environment(format!("RAS lists {reason} for cluster manager {target}:{port}; check Srvr= and the RAS endpoint")))
        }
    }
}

fn same_manager_host(left: &Host, right: &Host) -> bool {
    if left == right || (left.is_loopback() && right.is_loopback()) {
        return true;
    }
    // RAC reports the machine name even when the target uses a local loopback address.
    let local = crate::support::machine::host_name().and_then(|name| host_of_authority(&name));
    let left = if left.is_loopback() {
        local.as_ref().unwrap_or(left)
    } else {
        left
    };
    let right = if right.is_loopback() {
        local.as_ref().unwrap_or(right)
    } else {
        right
    };
    if left == right {
        return true;
    }
    let left = manager_addresses(left);
    manager_addresses(right)
        .iter()
        .any(|address| left.contains(address))
}

fn manager_addresses(host: &Host) -> Vec<IpAddr> {
    match host {
        Host::Address(address) => vec![address.to_canonical()],
        Host::Name(host) => (host.as_str(), 0)
            .to_socket_addrs()
            .map(|addresses| {
                addresses
                    .map(|address| address.ip().to_canonical())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn versions(candidates: &[PathBuf]) -> String {
    let mut versions = Vec::new();
    for program in candidates {
        let version = platform_version_of(UtilityType::Rac, program).map_or_else(
            || program.display().to_string(),
            |version| version.to_string(),
        );
        if !versions.contains(&version) {
            versions.push(version);
        }
    }
    shown(&versions.join(", ")).to_owned()
}

fn shown(versions: &str) -> &str {
    if versions.is_empty() {
        "none"
    } else {
        versions
    }
}

fn environment(message: impl Into<String>) -> AppError {
    AppError::environment_at(EnvironmentLayer::Platform, message)
}

fn check_cancelled(context: &ExecutionContext) -> Result<(), AppError> {
    interruption_before_safe_point(context, "cluster administration readiness").map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cluster_is_selected_by_server_and_manager_port() {
        let clusters = [
            rac::Cluster {
                id: "first".into(),
                host: "192.0.2.10".into(),
                port: 1541,
            },
            rac::Cluster {
                id: "second".into(),
                host: "192.0.2.10".into(),
                port: 2541,
            },
            rac::Cluster {
                id: "third".into(),
                host: "192.0.2.20".into(),
                port: 2541,
            },
        ];
        let target = V8Connection::from_connection_string("Srvr=192.0.2.10:2541;Ref=base");
        assert_eq!(select_cluster(&clusters, &target).unwrap(), "second");
        let target = V8Connection::from_connection_string("Srvr=192.0.2.30:2541;Ref=base");
        assert!(select_cluster(&clusters, &target).is_err());
        assert!(select_cluster(&clusters[..1], &target).is_err());
        let target = V8Connection::from_connection_string("Srvr=192.0.2.10:2541;Ref=base");
        assert!(select_cluster(&clusters[..1], &target).is_err());
    }

    #[test]
    fn a_loopback_target_matches_the_reported_local_machine_name() {
        let Some(host) = crate::support::machine::host_name() else {
            return;
        };
        let clusters = [rac::Cluster {
            id: "owned".into(),
            host,
            port: 1541,
        }];
        let target = V8Connection::from_connection_string("Srvr=127.0.0.1;Ref=base");
        assert_eq!(select_cluster(&clusters, &target).unwrap(), "owned");
    }

    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    fn a_foreign_listener_prevents_the_rac_probe() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert_eq!(classify_listener(std::process::id(), port), Listener::Ready);
        assert_eq!(classify_listener(u32::MAX, port), Listener::Foreign);
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Mutex;

        use super::*;
        use crate::platform::process::{
            ManagedSpawnMode, ProcessError, ProcessExecutionPolicy, ProcessExecutor,
            ProcessRequest, ProcessResult, SpawnResult, WorkGiven,
        };
        use crate::use_cases::context::CommandName;

        #[test]
        fn ras_fixture() {
            let Ok(port) = std::env::var("V8TR_TEST_RAS_PORT") else {
                return;
            };
            let _listener =
                TcpListener::bind((Ipv4Addr::UNSPECIFIED, port.parse().unwrap())).unwrap();
            loop {
                std::thread::park();
            }
        }

        struct Fixture {
            _dir: tempfile::TempDir,
            candidates: Vec<PathBuf>,
            children: Mutex<Vec<(u32, u16)>>,
            cancel_probe: bool,
        }

        impl Fixture {
            fn new(cancel_probe: bool) -> Self {
                let dir = tempfile::tempdir().unwrap();
                let this = std::env::current_exe().unwrap();
                let mut candidates = Vec::new();
                for version in ["8.3.27.2342", "8.3.27.2074"] {
                    let bin = dir.path().join(version).join("bin");
                    fs::create_dir_all(&bin).unwrap();
                    let ras = bin.join("ras");
                    fs::write(&ras, format!(
                        "#!/bin/sh\nexport V8TR_TEST_RAS_PORT=\"${{2#--port=}}\"\nexec '{}' --exact use_cases::cluster_admin::tests::linux::ras_fixture\n",
                        this.display()
                    )).unwrap();
                    fs::set_permissions(&ras, fs::Permissions::from_mode(0o755)).unwrap();
                    let rac = bin.join("rac");
                    fs::write(&rac, "stub routed by Fixture").unwrap();
                    candidates.push(rac);
                }
                Self {
                    _dir: dir,
                    candidates,
                    children: Mutex::new(Vec::new()),
                    cancel_probe,
                }
            }

            fn launch(&self, context: &ExecutionContext) -> Result<ClusterAdmin, AppError> {
                connect_via_agent(
                    context,
                    self,
                    &V8Connection::from_connection_string("Srvr=host;Ref=base"),
                    "host:1540",
                    self.candidates.clone(),
                )
            }
        }

        impl ProcessRunner for Fixture {
            fn spawn(
                &self,
                _: &ProcessRequest,
                _: &WorkGiven,
            ) -> Result<SpawnResult, ProcessError> {
                unreachable!("command RAS requires retained ownership");
            }

            fn spawn_managed(
                &self,
                request: &ProcessRequest,
                mode: ManagedSpawnMode,
                work: Option<&WorkGiven>,
            ) -> Result<ManagedSpawnResult, ProcessError> {
                assert_eq!(mode, ManagedSpawnMode::Tied);
                assert!(work.is_none());
                let port = request.args[1]
                    .strip_prefix("--port=")
                    .unwrap()
                    .parse()
                    .unwrap();
                let child = ProcessExecutor.spawn_managed(request, mode, work)?;
                self.children.lock().unwrap().push((child.pid(), port));
                Ok(child)
            }

            fn run_with_policy(
                &self,
                request: &ProcessRequest,
                policy: &ProcessExecutionPolicy,
            ) -> Result<ProcessResult, ProcessError> {
                let children = self.children.lock().unwrap();
                let (pid, port) = *children.last().unwrap();
                assert!(service::listens_on(pid, port));
                assert_eq!(request.args, ["cluster", "list", &ras_address(port)]);
                assert!(children[..children.len() - 1]
                    .iter()
                    .all(|(previous, _)| service::process_facts(*previous).is_none()));
                if self.cancel_probe {
                    policy.cancellation.cancel();
                }
                let matched = request.program == self.candidates[1];
                Ok(ProcessResult {
                    exit_code: if matched { 0 } else { 255 },
                    stdout: if matched {
                        "cluster : be73edc5-b760-4520-95ec-2be1502e1e8d\nhost : host\nport : 1541\n"
                            .into()
                    } else {
                        String::new()
                    },
                    stderr: String::new(),
                    interruption: None,
                })
            }
        }

        #[test]
        fn command_ras_discards_a_mismatched_candidate_before_trying_the_exact_version() {
            let fixture = Fixture::new(false);
            let context = ExecutionContext::cli(CommandName::Apply);
            let admin = fixture.launch(&context).unwrap();
            assert!(admin.managed_ras());
            assert_eq!(admin.rac(), &fixture.candidates[1]);
            assert_eq!(admin.cluster(), "be73edc5-b760-4520-95ec-2be1502e1e8d");
            assert!(!context.work().given());
            let children = fixture.children.lock().unwrap();
            assert_eq!(children.len(), 2);
            let (pid, port) = children[1];
            assert!(service::listens_on(pid, port));
            drop(admin);
            assert!(service::process_facts(pid).is_none());
            assert!(!service::port_answers(port));
        }

        #[test]
        fn cancellation_during_ras_readiness_removes_the_child_before_return() {
            let fixture = Fixture::new(true);
            let context = ExecutionContext::cli(CommandName::Apply);
            let Err(error) = fixture.launch(&context) else {
                panic!("readiness ignored cancellation");
            };
            assert!(error.cancellation().is_some(), "{error}");
            assert!(!context.work().given());
            let children = fixture.children.lock().unwrap();
            assert_eq!(children.len(), 1);
            assert!(service::process_facts(children[0].0).is_none());
            assert!(!service::port_answers(children[0].1));
        }
    }
}
