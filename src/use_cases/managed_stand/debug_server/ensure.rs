//! Own the lifecycle of a file debug session requested by an integration.

use std::io::{ErrorKind, Result as IoResult};

use crate::support::fs::{try_acquire_advisory_lock, AdvisoryLockGuard};
use crate::use_cases::interruption::interruption_before_safe_point;

use super::*;

const ENSURED: &str = "ensure-scope.json";

#[derive(serde::Deserialize, serde::Serialize)]
pub(super) struct EnsureScope {
    key: DebugSession,
    work_path: PathBuf,
}

impl DebugServer<'_> {
    pub(in crate::use_cases::managed_stand) fn ensure(
        &self,
        request: DebugServerStart,
        dry_run: bool,
    ) -> Result<DebugServerResult, UseCaseError> {
        use DebugServerStatusReason as Reason;

        let started = Instant::now();
        let file = crate::use_cases::infobase_owner::file_infobase_dir(self.config)
            .ok_or_else(|| refuse("debug-server start --ensure requires a file infobase"))?;
        let owner = request.owner_pid.ok_or_else(|| {
            UseCaseError::new(
                UseCaseErrorKind::Validation,
                "--ensure requires --owner-pid",
            )
        })?;
        self.check_owner(Some(owner))?;
        let owner_started = self.host.facts(owner).ok_or_else(owner_exited)?.started;
        let resolution = self.debug_resolution()?;
        if dry_run {
            return Ok(file_result(request, resolution));
        }
        if !self.host.can_confirm_debug_endpoint() {
            return Err(UseCaseError::from(AppError::environment_at(
                EnvironmentLayer::Platform,
                "automatic debug-server recovery requires an operating system that reports the owner of the debug connection (Windows or macOS)",
            )));
        }
        let machine = self.host.machine().ok_or_else(|| {
            UseCaseError::from(AppError::environment(
                "cannot identify this machine to recover its debug server",
            ))
        })?;
        let work_path = self.scope_work_path()?;
        let mut digest = Sha256::new();
        digest.update(owner.to_le_bytes());
        digest.update(owner_started.to_le_bytes());
        digest.update(machine.as_bytes());
        digest.update([0]);
        digest.update(file.as_os_str().as_encoded_bytes());
        digest.update([0]);
        digest.update(work_path.as_os_str().as_encoded_bytes());
        let scope: DebugSession = format!("{:x}", digest.finalize())[..32]
            .parse()
            .expect("scope is lowercase hex");
        self.sweep_ensured_sessions();
        let _guard = self.acquire_scope(scope)?;
        let intent = self.scope_file(scope, "json");
        if !self.owner_is_current(owner, owner_started) {
            return Err(owner_exited());
        }

        if let Some(session) = read_state_file::<DebugSession>(&intent)? {
            let paths = session_paths(self.config, session);
            if checked_session_directory(&paths)? && self.read_scope(&paths)? != Some(scope) {
                return Err(UseCaseError::new(
                    UseCaseErrorKind::Platform,
                    format!(
                        "the recorded debug session does not match this owner; check {}",
                        intent.display()
                    ),
                ));
            }
            let observed = match self.confirm_endpoint(&paths) {
                Err(Reason::ListenerUnverified | Reason::RdbgUnavailable) => {
                    if let Some(error) =
                        interruption_before_safe_point(self.context, "retrying the debug server")
                    {
                        return Err(error.into());
                    }
                    self.host.sleep(super::super::POLL);
                    self.confirm_endpoint(&paths)
                }
                observed => observed,
            };
            if !self.owner_is_current(owner, owner_started) {
                return Err(owner_exited());
            }
            if let Some(error) =
                interruption_before_safe_point(self.context, "returning the debug server")
            {
                return Err(error.into());
            }
            if let Ok(Some((pid, port))) = observed {
                let record = self.ensured_record(&paths)?;
                if record.bin == resolution.path && record.version == resolution.version {
                    let mut result = file_result(request, resolution);
                    complete_file_result(&mut result, &paths, session, pid, port);
                    result.action = DebugServerAction::Reused;
                    result.duration_ms = crate::support::time::elapsed_ms(started);
                    return Ok(result);
                }
            }
            match observed {
                Err(Reason::RecordChanged | Reason::UnsupportedOwner) => {
                    return Err(UseCaseError::new(
                        UseCaseErrorKind::Platform,
                        format!(
                            "the recorded debug session changed during recovery; record: {}",
                            paths.record().display()
                        ),
                    ));
                }
                Ok(None) => {}
                _ => {
                    self.stop_ensured_session(&paths)?;
                }
            }
        }

        if !self.owner_is_current(owner, owner_started) {
            return Err(owner_exited());
        }
        if let Some(error) =
            interruption_before_safe_point(self.context, "starting the debug server")
        {
            return Err(error.into());
        }
        let session = DebugSession::new();
        let paths = session_paths(self.config, session);
        let mut result = self.start_file_server(
            request,
            false,
            resolution,
            session,
            Some(EnsureScope {
                key: scope,
                work_path,
            }),
        )?;
        let mut observation = self.confirm_endpoint(&paths);
        if let Ok(Some((pid, port))) = observation {
            if (Some(pid), Some(port)) != (result.pid, result.port) {
                observation = Err(Reason::RecordChanged);
            }
        }
        let owner_alive = self.owner_is_current(owner, owner_started);
        let cancelled = interruption_before_safe_point(self.context, "returning the debug server");
        if let (Ok(Some(_)), true, None) = (&observation, owner_alive, &cancelled) {
            result.duration_ms = crate::support::time::elapsed_ms(started);
            return Ok(result);
        }
        let reason = if let Some(error) = cancelled {
            error.into()
        } else if !owner_alive {
            owner_exited()
        } else {
            let explanation = match observation {
                Err(Reason::RdbgUnavailable) => "the new 1C debug server did not answer the rdbg request",
                Err(Reason::ListenerUnverified) => "the operating system could not attribute the new debug connection to the 1C debug server",
                Err(Reason::ProcessChanged | Reason::ProcessGone) => "the new 1C debug server exited during startup",
                _ => "the runner could not verify the startup record of the new 1C debug server",
            };
            UseCaseError::new(UseCaseErrorKind::Platform, explanation)
        };
        if matches!(
            observation,
            Err(Reason::RecordChanged | Reason::RecordUnavailable | Reason::UnsupportedOwner)
        ) {
            return Err(reason.followed_by(format!("record: {}", paths.record().display())));
        }
        let log = self.keep_log(&paths).unwrap_or_else(|| paths.log());
        match self.stop_ensured_session(&paths) {
            Ok(_) => Err(reason.followed_by(format!("log: {}", log.display()))),
            Err(cleanup) => Err(reason.followed_by(format!("{cleanup}; log: {}", log.display()))),
        }
    }

    pub(super) fn record_debug_intent(
        &self,
        paths: &ServicePaths,
        scope: EnsureScope,
        session: DebugSession,
    ) -> Result<(), UseCaseError> {
        // Publish before spawn, so a successful launch can be reused even if
        // its caller never received the reply.
        let intent = self.scope_file(scope.key, "json");
        let result = write_json(&paths.root().join(ENSURED), &scope)
            .and_then(|()| write_json(&intent, &session));
        if let Err(error) = result {
            forget(paths);
            return Err(super::super::io_failure("cannot record", &intent, &error));
        }
        Ok(())
    }

    fn owner_is_current(&self, owner: u32, started: u64) -> bool {
        self.host
            .facts(owner)
            .is_some_and(|facts| facts.started == started)
    }

    fn scope_file(&self, scope: DebugSession, extension: &str) -> PathBuf {
        sessions_root(self.config)
            .join("ensure")
            .join(format!("{scope}.{extension}"))
    }

    fn scope_work_path(&self) -> Result<PathBuf, UseCaseError> {
        let path = &self.config.work_path;
        crate::support::path::nearest_existing_canonical_path(path)
            .map_err(|error| super::super::io_failure("cannot resolve", path, &error))
    }

    pub(super) fn read_scope(
        &self,
        paths: &ServicePaths,
    ) -> Result<Option<DebugSession>, UseCaseError> {
        let marker = paths.root().join(ENSURED);
        let Some(scope) = read_state_file::<EnsureScope>(&marker)? else {
            return Ok(None);
        };
        if scope.work_path != self.scope_work_path()? {
            return Err(invalid_state_path(
                &marker,
                "debug session from another workPath",
            ));
        }
        Ok(Some(scope.key))
    }

    fn acquire_scope(&self, scope: DebugSession) -> Result<AdvisoryLockGuard, UseCaseError> {
        let lock = self.scope_file(scope, "lock");
        loop {
            if let Some(error) =
                interruption_before_safe_point(self.context, "waiting for the debug session")
            {
                return Err(error.into());
            }
            match try_acquire_advisory_lock(&lock) {
                Ok(guard) => return Ok(guard),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    self.host.sleep(super::super::POLL)
                }
                Err(error) => return Err(super::super::io_failure("cannot lock", &lock, &error)),
            }
        }
    }

    pub(super) fn lock_ensured_session(
        &self,
        paths: &ServicePaths,
        dry_run: bool,
    ) -> Result<Option<AdvisoryLockGuard>, UseCaseError> {
        if dry_run {
            return Ok(None);
        }
        if !checked_session_directory(paths)? {
            return Ok(None);
        }
        self.read_scope(paths)?
            .map(|scope| self.acquire_scope(scope))
            .transpose()
    }

    pub(super) fn sweep_ensured_sessions(&self) {
        let folder = sessions_root(self.config).join("ensure");
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(scope) = path
                .file_stem()
                .and_then(|name| name.to_str())
                .and_then(|name| name.parse::<DebugSession>().ok())
                .filter(|_| path.extension().is_some_and(|ext| ext == "json"))
            else {
                continue;
            };
            let Ok(_guard) = try_acquire_advisory_lock(&self.scope_file(scope, "lock")) else {
                continue;
            };
            let Ok(Some(session)) = read_state_file::<DebugSession>(&path) else {
                continue;
            };
            let paths = session_paths(self.config, session);
            match checked_session_directory(&paths) {
                Ok(false) => {
                    let _ = std::fs::remove_file(path);
                    continue;
                }
                Ok(true) => {}
                Err(_) => continue,
            }
            if !self
                .read_scope(&paths)
                .is_ok_and(|found| found == Some(scope))
            {
                continue;
            }
            if let Ok(record) = self.ensured_record(&paths) {
                if self
                    .host
                    .machine()
                    .is_some_and(|machine| record.machine.as_deref() == Some(&machine))
                    && record.pid.is_some_and(|pid| {
                        super::super::debug_process_ended(self.host, &record, pid)
                    })
                {
                    forget(&paths);
                    if matches!(checked_session_directory(&paths), Ok(false)) {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
        }
    }

    fn ensured_record(&self, paths: &ServicePaths) -> Result<ServiceRecord, UseCaseError> {
        let path = paths.record();
        read_state_file(&path)?.ok_or_else(|| {
            UseCaseError::new(
                UseCaseErrorKind::Platform,
                format!("cannot read the recorded 1C debug process at {}; restart the debugger owner to release it safely", paths.record().display()),
            )
        })
    }

    pub(super) fn stop_ensured_session(
        &self,
        paths: &ServicePaths,
    ) -> Result<DebugServerResult, UseCaseError> {
        let mut result = empty(DebugServerAction::NothingToStop);
        if !checked_session_directory(paths)? {
            result.known_session = Some(false);
            return Ok(result);
        }
        result.known_session = Some(true);
        result.owner = Some(DebugServerOwner::Runner);
        let marker = paths.root().join(STAND_SESSION);
        match std::fs::symlink_metadata(&marker) {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(super::super::io_failure("cannot read", &marker, &error)),
            Ok(_) => return Err(invalid_state_path(&marker, "debug session owner")),
        }
        let record_path = paths.record();
        let record = self.ensured_record(paths)?;
        let machine = self.host.machine();
        let pid = record.pid.filter(|pid| *pid > 0).ok_or_else(|| {
            UseCaseError::new(
                UseCaseErrorKind::Platform,
                format!("the previous dbgs launch did not record its PID at {}; restart the debugger owner to release it safely", record_path.display()),
            )
        })?;
        if machine.is_some()
            && record.machine == machine
            && record.started.is_some()
            && record.image.as_deref() == Some(dbgs::executable_name())
        {
            if super::super::debug_process_ended(self.host, &record, pid) {
                forget(paths);
                result.stale_record = true;
                return Ok(result);
            }
            if record.is_own(self.host.facts(pid).as_ref(), machine.as_deref())
                && self.stop_process(paths, &record)
            {
                result.action = DebugServerAction::Stopped;
                result.stopped_pid = Some(pid);
                result.port = Some(record.port);
                result.url = Some(dbgs::url(record.port));
                return Ok(result);
            }
        }
        Err(UseCaseError::new(
            UseCaseErrorKind::Platform,
            format!("cannot safely stop recorded dbgs pid {pid}; restart the debugger owner to release it safely; record: {}", record_path.display()),
        ))
    }

    pub(super) fn requires_confirmed_process_end(&self, paths: &ServicePaths) -> bool {
        !matches!(std::fs::symlink_metadata(paths.root().join(ENSURED)),
            Err(error) if error.kind() == ErrorKind::NotFound)
    }
}

pub(super) fn read_state_file<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<Option<T>, UseCaseError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(invalid_state_path(path, "debug session state")),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(super::super::io_failure("cannot read", path, &error)),
    }
    let read = || -> IoResult<_> {
        serde_json::from_slice(&std::fs::read(path)?)
            .map(Some)
            .map_err(std::io::Error::other)
    };
    read().map_err(|error| super::super::io_failure("cannot read", path, &error))
}

pub(super) fn checked_session_directory(paths: &ServicePaths) -> Result<bool, UseCaseError> {
    let path = paths.root();
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(invalid_state_path(path, "debug session directory")),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(super::super::io_failure("cannot read", path, &error)),
    }
}

fn invalid_state_path(path: &Path, what: &str) -> UseCaseError {
    UseCaseError::new(
        UseCaseErrorKind::Platform,
        format!(
            "invalid {what} at {}; restore the runner's access to its workPath",
            path.display()
        ),
    )
}

fn owner_exited() -> UseCaseError {
    UseCaseError::new(
        UseCaseErrorKind::Validation,
        "the debugger owner process exited; start debugging from the running client",
    )
}
