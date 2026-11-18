//! The cluster marks the lock. Local state records the previous scheduled-jobs setting so
//! allow can recover after a cancelled or interrupted deny.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io;
use std::path::Path;
use uuid::Uuid;

use super::{
    checked_output, cluster_auth, name, rac_call, Backend, Endpoint, Request, BASE_CREDENTIALS,
    CLUSTER_CREDENTIALS,
};
use crate::domain::sessions::SessionsLockResult;
use crate::platform::rac::{self, Credentials, InfobaseTarget, LockInfo, LockUpdate};
use crate::support::error::AppError;
use crate::use_cases::context::InterruptionSafetyClass;
use crate::use_cases::interruption::collecting_deferrals;

#[derive(Deserialize, Serialize)]
struct Intent {
    token: String,
    changed_jobs: bool,
    previous_jobs: bool,
    message_hash: String,
    code_hash: String,
    allowing: bool,
}

impl Intent {
    fn owns(&self, lock: &LockInfo) -> bool {
        lock.marker == self.token
            && !lock.scheduled
            && field_hash(&self.token, &lock.message) == self.message_hash
            && field_hash(&self.token, &lock.code) == self.code_hash
            && (!self.changed_jobs || lock.jobs_denied)
    }

    fn read(path: &Path) -> Result<Option<Self>, AppError> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice::<Self>(&bytes)
                .ok()
                .filter(|saved| Uuid::parse_str(&saved.token).is_ok())
                .map(Some)
                .ok_or_else(|| {
                    AppError::Runtime("the maintenance recovery record is invalid".to_owned())
                }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(state_error(error)),
        }
    }

    fn stage(&self, path: &Path) -> Result<(), AppError> {
        crate::support::fs::create_file_once(path, |file| {
            serde_json::to_writer(file, self).map_err(io::Error::other)
        })
        .map_err(state_error)?
        .then_some(())
        .ok_or_else(|| {
            AppError::Runtime("another command is preparing the maintenance lock".to_owned())
        })
    }

    fn prepare_allow(&mut self, path: &Path) -> Result<(), AppError> {
        self.allowing = true;
        crate::support::fs::write_file_atomically(path, |file| {
            serde_json::to_writer(file, self).map_err(io::Error::other)
        })
        .map_err(state_error)
    }
}

fn field_hash(token: &str, value: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(token);
    hash.update(value);
    format!("{:x}", hash.finalize())
}

fn has_foreign_settings(lock: &LockInfo) -> bool {
    lock.sessions_denied
        || lock.scheduled
        || !lock.marker.is_empty()
        || !lock.message.is_empty()
        || !lock.code.is_empty()
}

fn state_error(error: io::Error) -> AppError {
    AppError::Runtime(format!("maintenance recovery record: {error}"))
}

fn remove_intent(path: &Path) -> Result<(), AppError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(state_error(error)),
    }
}

fn foreign_lock() -> AppError {
    AppError::environment("the selected infobase has a foreign maintenance lock; verify it with the cluster administrator")
}

impl Backend<'_> {
    pub(super) fn change_lock(
        &self,
        operation: Request<'_>,
    ) -> Result<SessionsLockResult, AppError> {
        let Endpoint::Cluster { admin, base } = &self.endpoint else {
            unreachable!("maintenance only accepts a cluster endpoint");
        };
        let cluster_auth = cluster_auth(self.config);
        let connection = self.config.v8_connection();
        let user = connection
            .user
            .as_deref()
            .filter(|user| !user.trim().is_empty());
        if cluster_auth.user.is_none() || user.is_none() {
            let levels: Vec<_> = [
                cluster_auth.user.is_none().then_some(CLUSTER_CREDENTIALS),
                user.is_none().then_some(BASE_CREDENTIALS),
            ]
            .into_iter()
            .flatten()
            .collect();
            return Err(AppError::environment(levels.join(", ")));
        }
        let help = rac_call(
            self.context,
            self.config,
            self.runner,
            &rac::infobase_help(admin.rac()),
            "rac help infobase",
            None,
        )?;
        if !["--infobase-user=", "--infobase-pwd=", "--denied-parameter="]
            .iter()
            .all(|key| help.contains(key))
        {
            return Err(AppError::environment(
                "this rac version cannot authenticate the infobase user level for sessions deny or allow; use a version with --infobase-user and --infobase-pwd",
            ));
        }
        let target = InfobaseTarget {
            cluster: admin.cluster(),
            id: base,
            cluster_auth,
            infobase_auth: Credentials {
                user,
                password: connection.password.as_deref(),
            },
        };
        let read = |after_update: bool| {
            let command = rac::infobase_info(admin.rac(), admin.address(), &target);
            let mut policy = if after_update {
                self.context.cleanup_policy()
            } else {
                self.context.process_policy(
                    InterruptionSafetyClass::GracefulThenKill,
                    Some(super::TIMEOUT),
                )
            };
            policy.timeout = Some(super::TIMEOUT);
            let result = self.runner.run_with_policy(&command, &policy)?;
            if result.outcome().is_err() {
                // Properties are allowed to contain an existing permission code.
                return Err(AppError::Platform(format!(
                    "rac infobase info exited with code {}",
                    result.exit_code
                )));
            }
            rac::lock_info(&result.stdout, base).ok_or_else(|| {
                AppError::InvalidOutput(
                    "rac did not report the infobase maintenance state".to_owned(),
                )
            })
        };
        let ((observed, changed), warnings) = collecting_deferrals(|deferrals| {
            let mut update = |change: LockUpdate<'_>| {
                let code = Some(change.permission_code);
                let command = rac::infobase_update(admin.rac(), admin.address(), &target, change);
                // Killing the client does not cancel a dispatched server update.
                let policy = self
                    .context
                    .process_policy(InterruptionSafetyClass::CriticalNonAbortable, None);
                let result = self.runner.run_with_policy(&command, &policy)?;
                deferrals.note_outcome(
                    "rac infobase update",
                    &result.outcome(),
                    result.interruption,
                );
                checked_output(self.config, result, "rac infobase update", None, code)
            };
            let local = self.config.local_dir();
            let directory = local.path().join("maintenance");
            let path = directory.join(format!("{}-{base}.json", admin.cluster()));
            let before = read(false)?;
            let mut existing = Intent::read(&path)?;
            let owned = existing.as_ref().filter(|intent| intent.owns(&before));
            if before.sessions_denied && owned.is_none() {
                return Err(foreign_lock());
            }
            Ok(match operation {
                Request::Deny { .. } if before.sessions_denied => (before, false),
                Request::Allow if !before.sessions_denied && before.marker.is_empty() => {
                    if existing.as_ref().is_some_and(|intent| {
                        intent.allowing && intent.previous_jobs == before.jobs_denied
                    }) {
                        remove_intent(&path)?;
                    } else if existing.is_some() {
                        return Err(AppError::environment("an earlier maintenance update has an unconfirmed outcome; inspect the cluster before retrying sessions allow"));
                    }
                    (before, false)
                }
                Request::Deny {
                    message,
                    code,
                    jobs,
                } => {
                    if has_foreign_settings(&before) {
                        return Err(foreign_lock());
                    }
                    // The state must survive an uncertain result of the next platform request.
                    if existing.as_ref().is_some_and(|intent| {
                        intent.allowing && intent.previous_jobs == before.jobs_denied
                    }) {
                        remove_intent(&path)?;
                        existing = None;
                    }
                    let token = existing
                        .as_ref()
                        .map_or_else(|| Uuid::new_v4().to_string(), |intent| intent.token.clone());
                    let message = message.unwrap_or_default();
                    let code = code.unwrap_or_default();
                    for (option, value) in [("--message", message), ("--code", code)] {
                        if value.chars().any(char::is_control)
                            || value.contains('"')
                            || value != value.trim()
                        {
                            return Err(AppError::Validation(format!(
                            "{option} must be a single line without enclosing spaces or double quotes, so rac output can be verified"
                        )));
                        }
                    }
                    let intent = Intent {
                        message_hash: field_hash(&token, message),
                        code_hash: field_hash(&token, code),
                        token,
                        changed_jobs: jobs && !before.jobs_denied,
                        previous_jobs: before.jobs_denied,
                        allowing: false,
                    };
                    if let Some(previous) = existing {
                        if previous.allowing
                            || previous.previous_jobs != intent.previous_jobs
                            || previous.changed_jobs != intent.changed_jobs
                            || previous.message_hash != intent.message_hash
                            || previous.code_hash != intent.code_hash
                        {
                            return Err(AppError::environment("a prior maintenance update has an unconfirmed outcome; retry with its original settings or inspect the cluster state"));
                        }
                    } else {
                        local.prepare().map_err(state_error)?;
                        std::fs::create_dir_all(&directory).map_err(state_error)?;
                        intent.stage(&path)?;
                    }
                    let current = read(false)?;
                    if has_foreign_settings(&current) || current.jobs_denied != before.jobs_denied {
                        return Err(foreign_lock());
                    }
                    update(LockUpdate {
                        sessions_denied: true,
                        jobs_denied: intent.changed_jobs.then_some(true),
                        marker: &intent.token,
                        message,
                        permission_code: code,
                    })?;
                    let after = read(true)?;
                    if !after.sessions_denied
                        || !intent.owns(&after)
                        || after.jobs_denied != (jobs || before.jobs_denied)
                    {
                        return Err(AppError::Platform(
                            "the cluster did not confirm the intended maintenance lock".to_owned(),
                        ));
                    }
                    (after, true)
                }
                Request::Allow => {
                    let mut intent = existing
                        .filter(|intent| intent.owns(&before))
                        .ok_or_else(foreign_lock)?;
                    intent.prepare_allow(&path)?;
                    update(LockUpdate {
                        sessions_denied: false,
                        jobs_denied: intent.changed_jobs.then_some(intent.previous_jobs),
                        marker: "",
                        message: "",
                        permission_code: "",
                    })?;
                    let after = read(true)?;
                    if has_foreign_settings(&after)
                        || after.jobs_denied
                            != if intent.changed_jobs {
                                intent.previous_jobs
                            } else {
                                before.jobs_denied
                            }
                    {
                        return Err(AppError::Platform(
                            "the cluster did not confirm restoration of the maintenance state"
                                .to_owned(),
                        ));
                    }
                    remove_intent(&path)?;
                    (after, true)
                }
                Request::List | Request::Terminate { .. } => {
                    unreachable!("not a maintenance operation")
                }
            })
        })?;
        Ok(SessionsLockResult {
            infobase: name(self.config),
            managed_ras: self.managed_ras(),
            sessions_denied: observed.sessions_denied,
            jobs_denied: observed.jobs_denied,
            changed,
            warnings,
        })
    }
}
