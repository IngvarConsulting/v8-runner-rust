use std::path::PathBuf;
use std::time::Duration;

use uuid::Uuid;

use crate::config::model::AppConfig;
use crate::domain::capability::TargetKind;
use crate::domain::sessions::{
    protected_application, SessionTerminationError, SessionsListResult, SessionsResult,
    SessionsTerminateResult,
};
use crate::platform::ibsrv::{self, SessionSource};
use crate::platform::process::{ProcessExecutor, ProcessRequest, ProcessResult, ProcessRunner};
use crate::platform::rac;
use crate::support::error::{AppError, CapabilityReason};
use crate::use_cases::cluster_admin::{cluster_auth, ClusterAdmin, CLUSTER_CREDENTIALS};
use crate::use_cases::context::{ExecutionContext, InterruptionSafetyClass};
use crate::use_cases::result::UseCaseFailure;

pub enum Terminate {
    All,
    One(Uuid),
}

pub enum Request<'a> {
    List,
    Terminate {
        selection: Terminate,
        message: Option<&'a str>,
    },
    Deny {
        message: Option<&'a str>,
        code: Option<&'a str>,
        jobs: bool,
    },
    Allow,
}

const TIMEOUT: Duration = Duration::from_secs(15);
const BASE_CREDENTIALS: &str = "infobase user level: declare infobase.user and infobase.password";

pub fn execute(
    context: &ExecutionContext,
    config: &AppConfig,
    request: Request<'_>,
) -> Result<SessionsResult, UseCaseFailure<SessionsResult>> {
    let backend = Backend::open(context, config, &ProcessExecutor, &request)
        .map_err(UseCaseFailure::without_payload)?;
    match request {
        Request::List => backend
            .list()
            .map(|sessions| {
                SessionsResult::List(SessionsListResult {
                    infobase: name(config),
                    managed_ras: backend.managed_ras(),
                    sessions: sessions.into_iter().map(rac::Session::into_entry).collect(),
                })
            })
            .map_err(UseCaseFailure::without_payload),
        Request::Terminate { selection, message } => {
            let (result, error) = backend
                .terminate(selection, message)
                .map_err(UseCaseFailure::without_payload)?;
            let data = SessionsResult::Terminate(result);
            match error {
                Some(error) => Err(UseCaseFailure::with_payload(error, data)),
                None => Ok(data),
            }
        }
        change @ (Request::Deny { .. } | Request::Allow) => backend
            .change_lock(change)
            .map(SessionsResult::Lock)
            .map_err(UseCaseFailure::without_payload),
    }
}

fn name(config: &AppConfig) -> String {
    config
        .infobase_name
        .clone()
        .or_else(|| {
            config
                .v8_connection()
                .cluster_address()
                .map(|(_, name)| name)
        })
        .unwrap_or_else(|| "selected infobase".to_owned())
}

enum Endpoint<'a> {
    Cluster {
        admin: ClusterAdmin,
        base: String,
    },
    Standalone {
        binary: PathBuf,
        source: SessionSource<'a>,
    },
}

struct Backend<'a> {
    context: &'a ExecutionContext,
    config: &'a AppConfig,
    runner: &'a dyn ProcessRunner,
    endpoint: Endpoint<'a>,
}

impl<'a> Backend<'a> {
    fn open(
        context: &'a ExecutionContext,
        config: &'a AppConfig,
        runner: &'a dyn ProcessRunner,
        request: &Request<'_>,
    ) -> Result<Self, AppError> {
        let lock = matches!(request, Request::Deny { .. } | Request::Allow);
        let endpoint = match config.target_kind() {
            TargetKind::File => return Err(AppError::capability_for(
                CapabilityReason::Target,
                "sessions requires a server infobase; for configuration updates use apply --sessions force to request the platform policy, or close the file infobase sessions manually",
            )),
            TargetKind::Standalone if lock => return Err(AppError::capability_for(
                CapabilityReason::Target,
                "sessions deny and allow require a cluster infobase; ibcmd exposes no maintenance lock for the standalone server",
            )),
            TargetKind::Standalone => {
                let (binary, source) = crate::use_cases::managed_stand::standalone_session_target(
                    context, config,
                ).ok_or_else(|| AppError::environment(
                    "sessions requires a running managed standalone server or infobase.standalone.gate and ibcmd",
                ))?;
                if matches!(source, SessionSource::Gate(_))
                    && config.infobase.standalone.as_ref().is_some_and(|server| server.host_fingerprint.is_some())
                {
                    return Err(AppError::environment(
                        "ibcmd cannot bind its SSH connection to infobase.standalone.host-fingerprint; use the managed server by local PID or the platform administration utilities with the declared key",
                    ));
                }
                Endpoint::Standalone { binary, source }
            }
            TargetKind::Cluster => {
                let admin = ClusterAdmin::connect(context, config, runner)?;
                let (_, reference) = config
                    .v8_connection()
                    .cluster_address()
                    .ok_or_else(|| AppError::Validation("the connection must name a cluster infobase (Ref)".to_owned()))?;
                let base = admin.find_infobase(context, config, runner, &reference)?
                    .ok_or_else(|| AppError::environment("the selected infobase is not registered in the cluster"))?
                    .to_string();
                Endpoint::Cluster { admin, base }
            }
        };
        Ok(Self {
            context,
            config,
            runner,
            endpoint,
        })
    }

    fn managed_ras(&self) -> bool {
        match &self.endpoint {
            Endpoint::Cluster { admin, .. } => admin.managed_ras(),
            Endpoint::Standalone { .. } => false,
        }
    }

    fn list(&self) -> Result<Vec<rac::Session>, AppError> {
        let answer = match &self.endpoint {
            Endpoint::Cluster { admin, base } => {
                let command =
                    rac::session_list(admin.rac(), admin.address(), admin.cluster(), base);
                rac_call(
                    self.context,
                    self.config,
                    self.runner,
                    &rac::with_cluster_auth(command, cluster_auth(self.config)),
                    "rac session list",
                    None,
                )?
            }
            Endpoint::Standalone { binary, source } => {
                self.ibcmd_call(&ibsrv::session_list(binary, *source), *source)?
            }
        };
        rac::checked_sessions(&answer).ok_or_else(|| {
            AppError::InvalidOutput("the platform returned an incomplete session list".to_owned())
        })
    }

    fn terminate(
        &self,
        selection: Terminate,
        message: Option<&str>,
    ) -> Result<(SessionsTerminateResult, Option<AppError>), AppError> {
        self.terminate_listed(selection, message, self.list()?)
    }

    fn terminate_listed(
        &self,
        selection: Terminate,
        message: Option<&str>,
        listed: Vec<rac::Session>,
    ) -> Result<(SessionsTerminateResult, Option<AppError>), AppError> {
        let requested = match selection {
            Terminate::All => "all".to_owned(),
            Terminate::One(id) => id.to_string(),
        };
        let mut data = SessionsTerminateResult {
            infobase: name(self.config),
            managed_ras: self.managed_ras(),
            requested,
            terminated_session_ids: Vec::new(),
            errors: Vec::new(),
        };
        let sessions = match selection {
            Terminate::All => listed
                .into_iter()
                .filter(|session| !protected_application(&session.application))
                .collect(),
            Terminate::One(_) => {
                let session = listed
                    .into_iter()
                    .find(|session| session.id == data.requested)
                    .ok_or_else(|| {
                        AppError::Validation(
                            "the session UUID is absent from the selected infobase".to_owned(),
                        )
                    })?;
                if matches!(session.application.as_str(), "RAS" | "SystemBackgroundJob") {
                    return Err(AppError::capability_for(
                        CapabilityReason::Target,
                        "the selected session belongs to the platform",
                    ));
                }
                vec![session]
            }
        };
        let mut pending = Vec::new();
        for session in sessions {
            let id = session.id;
            let answer = match &self.endpoint {
                Endpoint::Cluster { admin, .. } => rac_call(
                    self.context,
                    self.config,
                    self.runner,
                    &rac::session_terminate(
                        admin.rac(),
                        admin.address(),
                        admin.cluster(),
                        &id,
                        message,
                        cluster_auth(self.config),
                    ),
                    "rac session terminate",
                    None,
                ),
                Endpoint::Standalone { binary, source } => self.ibcmd_call(
                    &ibsrv::session_terminate(binary, *source, &id, message),
                    *source,
                ),
            };
            match answer {
                Ok(_) if matches!(self.endpoint, Endpoint::Cluster { .. }) => {
                    data.terminated_session_ids.push(id)
                }
                Ok(_) => pending.push(id),
                Err(error) if error.cancellation().is_some() => {
                    pending.push(id);
                    data.errors.extend(pending.into_iter().map(|session_id| {
                        SessionTerminationError {
                            session_id,
                            message: "termination could not be verified before cancellation"
                                .to_owned(),
                        }
                    }));
                    return Ok((data, Some(error)));
                }
                Err(error) => data.errors.push(SessionTerminationError {
                    session_id: id,
                    message: error.to_string(),
                }),
            }
        }
        if !pending.is_empty() {
            let listed = match self.list() {
                Ok(listed) => listed,
                Err(error) => {
                    for id in pending {
                        data.errors.push(SessionTerminationError {
                            session_id: id,
                            message: "termination could not be verified".to_owned(),
                        });
                    }
                    return Ok((data, Some(error)));
                }
            };
            for id in pending {
                if listed.iter().any(|session| session.id == id) {
                    data.errors.push(SessionTerminationError {
                        session_id: id,
                        message: "the session is still present in the server list".to_owned(),
                    });
                } else {
                    data.terminated_session_ids.push(id);
                }
            }
        }
        let error = (!data.errors.is_empty())
            .then(|| AppError::Platform("some sessions could not be terminated".to_owned()));
        Ok((data, error))
    }

    fn ibcmd_call(
        &self,
        command: &ProcessRequest,
        source: SessionSource<'_>,
    ) -> Result<String, AppError> {
        let policy = self
            .context
            .process_policy(InterruptionSafetyClass::GracefulThenKill, Some(TIMEOUT));
        let connection = self.config.v8_connection();
        let answer = ibsrv::run_session(
            self.runner,
            command,
            &policy,
            source,
            connection.user.as_deref(),
            connection.password.as_deref(),
        )?;
        let hint = connection.user.is_none().then_some(BASE_CREDENTIALS);
        let operation = match source {
            SessionSource::Gate(_) => {
                "ibcmd session (the SSH host key must already be trusted by ibcmd)"
            }
            SessionSource::Pid(_) => "ibcmd session",
        };
        checked_output(self.config, answer, operation, hint, None)
    }
}

pub(crate) fn terminate_for_apply(
    context: &ExecutionContext,
    config: &AppConfig,
    message: &str,
) -> Result<
    (
        Vec<crate::domain::sessions::SessionEntry>,
        bool,
        Option<AppError>,
    ),
    AppError,
> {
    let backend = Backend::open(context, config, &ProcessExecutor, &Request::List)?;
    let awake: Vec<_> = backend
        .list()?
        .into_iter()
        .filter(|session| !session.asleep)
        .collect();
    let (result, error) = backend.terminate_listed(Terminate::All, Some(message), awake.clone())?;
    let error = error.map(|error| {
        let details = result
            .errors
            .iter()
            .map(|failure| format!("{}: {}", failure.session_id, failure.message))
            .collect::<Vec<_>>()
            .join("; ");
        if details.is_empty() {
            error
        } else {
            error.with_context(details)
        }
    });
    let ended = awake
        .into_iter()
        .filter(|session| result.terminated_session_ids.contains(&session.id))
        .map(rac::Session::into_entry)
        .collect();
    Ok((ended, result.managed_ras, error))
}

fn rac_call(
    context: &ExecutionContext,
    config: &AppConfig,
    runner: &dyn ProcessRunner,
    command: &ProcessRequest,
    operation: &str,
    permission_code: Option<&str>,
) -> Result<String, AppError> {
    let policy = context.process_policy(InterruptionSafetyClass::GracefulThenKill, Some(TIMEOUT));
    let output = runner.run_with_policy(command, &policy)?;
    let hint = cluster_auth(config)
        .user
        .is_none()
        .then_some(CLUSTER_CREDENTIALS);
    checked_output(config, output, operation, hint, permission_code)
}

fn checked_output(
    config: &AppConfig,
    answer: ProcessResult,
    operation: &str,
    absent_credentials: Option<&str>,
    permission_code: Option<&str>,
) -> Result<String, AppError> {
    if answer.outcome().is_ok() {
        return Ok(answer.stdout);
    }
    let auth = cluster_auth(config);
    let connection = config.v8_connection();
    let secrets: Vec<_> = [
        auth.password,
        connection.password.as_deref(),
        permission_code,
    ]
    .into_iter()
    .flatten()
    .collect();
    // RAC may put infobase properties (including the server's permission code) on stdout.
    let diagnostics = crate::platform::secrets::mask_text(answer.stderr.trim(), &secrets);
    let diagnostics: String = diagnostics.chars().take(2048).collect();
    let message = format!(
        "{operation} exited with code {}: {diagnostics}",
        answer.exit_code
    );
    Err(match absent_credentials {
        Some(hint) => AppError::environment(format!(
            "{message}; if this server requires authentication, {hint}"
        )),
        None => AppError::Platform(message),
    })
}

mod maintenance;
