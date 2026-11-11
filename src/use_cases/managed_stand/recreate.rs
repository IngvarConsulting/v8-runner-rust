//! `infobase create --recreate` у управляемого стенда над своим PostgreSQL: проверки до
//! изменений, откладывание прежней БД под временное имя и снятие регистрации в кластере,
//! удаление отложенного после удачи и возврат после неудачи.
//!
//! Только своя БД (`dbms.managed`): чужую СУБД раннер не трогает
//! (`INV.USE-CASES.A-MANAGED-RECREATE-DROPS-ONLY-ITS-OWN-DATABASE`).

use std::io::Write as _;
use std::path::PathBuf;

use super::cluster::{register_again, registration_for_recreate, Registered};
use super::{
    io_failure, policy, sessions_refusal, validation, ProcessExecutor, RealHost, ServiceHost, Stand,
};
use crate::config::model::AppConfig;
use crate::domain::capability::TargetKind;
use crate::domain::init::RemovedKind;
use crate::platform::postgres::{self, PostgresBuild};
use crate::platform::process::ProcessRunner;
use crate::use_cases::context::ExecutionContext;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

/// Окончание временного имени отложенной БД.
const ASIDE_SUFFIX: &str = "_v8r_recreate";
/// Длина идентификатора PostgreSQL: длиннее он молча обрезается.
const NAMEDATALEN: usize = 63;

/// Прежняя база управляемого стенда, проверенная и готовая к откладыванию.
pub(crate) struct OldDatabase {
    port: u16,
    bin: PathBuf,
    name: String,
    aside: String,
    server: String,
    database_exists: bool,
    registration: Option<Registered>,
}

/// Проверки `--recreate` у управляемого стенда, ничего не меняя: цель — стенд над своим
/// PostgreSQL, PostgreSQL работает, остатка прошлого пересоздания нет, у кластера — нет
/// сеансов базы.
pub(crate) fn old_database(
    context: &ExecutionContext,
    config: &AppConfig,
) -> Result<OldDatabase, UseCaseError> {
    old_database_with(context, config, &ProcessExecutor, &RealHost)
}

pub(super) fn old_database_with(
    context: &ExecutionContext,
    config: &AppConfig,
    runner: &dyn ProcessRunner,
    host: &dyn ServiceHost,
) -> Result<OldDatabase, UseCaseError> {
    let name = config
        .infobase
        .dbms
        .as_ref()
        .filter(|dbms| dbms.managed.is_some())
        .and_then(|dbms| dbms.name.clone())
        .ok_or_else(|| {
            validation(
                "infobase create --recreate replaces only an infobase over the runner's own PostgreSQL (infobase.dbms.managed): a DBMS the runner did not start is left untouched",
            )
        })?;
    let aside = format!("{name}{ASIDE_SUFFIX}");
    if aside.len() > NAMEDATALEN {
        return Err(validation(format!(
            "infobase.dbms.name '{name}' is too long to be set aside under '{aside}': PostgreSQL names are at most {NAMEDATALEN} bytes"
        )));
    }
    let stand = Stand::new(config, context, runner, host)?;
    let (port, bin) = stand.running_record()?;
    refuse_a_leftover(&stand, port, &bin, &name)?;
    let registration = match config.target_kind() {
        TargetKind::Cluster => Some(registration_for_recreate(context, config, runner, host)?),
        _ => None,
    };
    let database_exists = stand.database_exists(port, &bin, &name)?;
    Ok(OldDatabase {
        server: postgres::server_address(port),
        port,
        bin,
        name,
        aside,
        database_exists,
        registration,
    })
}

/// Остаток пересоздания у простого `infobase create`: PostgreSQL не работает — проверять
/// нечего, отказ о нём назовёт само создание.
pub(crate) fn refuse_a_leftover_database(
    context: &ExecutionContext,
    config: &AppConfig,
) -> Result<(), UseCaseError> {
    let Some(name) = config
        .infobase
        .dbms
        .as_ref()
        .filter(|dbms| dbms.managed.is_some())
        .and_then(|dbms| dbms.name.clone())
    else {
        return Ok(());
    };
    let stand = Stand::new(config, context, &ProcessExecutor, &RealHost)?;
    let Ok((port, bin)) = stand.running_record() else {
        return Ok(());
    };
    refuse_a_leftover(&stand, port, &bin, &name)
}

/// Остаток пересоздания, оборванного аварийным выходом раннера: БД под временным именем
/// держит прежнюю базу. Его называют и `infobase create`, и `--recreate`.
fn refuse_a_leftover(
    stand: &Stand<'_>,
    port: u16,
    bin: &std::path::Path,
    name: &str,
) -> Result<(), UseCaseError> {
    let aside = format!("{name}{ASIDE_SUFFIX}");
    if aside.len() <= NAMEDATALEN && stand.database_exists(port, bin, &aside)? {
        return Err(validation(format!(
            "the database '{aside}' in the PostgreSQL of this infobase is left by an earlier infobase create --recreate that did not finish: it holds the old infobase; rename it back to '{name}' or drop it with PostgreSQL tools using the password file `dbms status` names, then repeat"
        )));
    }
    Ok(())
}

impl OldDatabase {
    /// Что будет отложено: БД и регистрация, если они есть.
    pub(crate) fn parts(&self) -> Vec<(RemovedKind, String)> {
        let mut parts = Vec::new();
        if let Some(registration) = self
            .registration
            .as_ref()
            .filter(|registration| registration.infobase.is_some())
        {
            parts.push((
                RemovedKind::Registration,
                format!("the infobase '{}' in the cluster", registration.name),
            ));
        }
        if self.database_exists {
            parts.push((
                RemovedKind::Database,
                format!("the database '{}' on {}", self.name, self.server),
            ));
        }
        parts
    }

    /// Точка невозврата: регистрация снимается без БД, БД переименовывается во временное имя.
    /// Отказ PostgreSQL из-за подключений — отказ `infobase_sessions`, и регистрация
    /// возвращается.
    pub(crate) fn set_aside(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
    ) -> Result<(), UseCaseError> {
        self.set_aside_with(context, config, &ProcessExecutor, &RealHost)
    }

    pub(super) fn set_aside_with(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        runner: &dyn ProcessRunner,
        host: &dyn ServiceHost,
    ) -> Result<(), UseCaseError> {
        if let Some(registration) = &self.registration {
            // Снятие регистрации отмена не обрывает: оборванный `rac` мог уже снять её, и
            // база осталась бы без регистрации и без отката.
            registration.unregister(&uncancelled(context), config, runner)?;
        }
        if !self.database_exists {
            return Ok(());
        }
        let renamed = self.statement(
            context,
            config,
            runner,
            host,
            &postgres::rename_database(&self.name, &self.aside),
        );
        let error = match renamed {
            Ok(()) => return Ok(()),
            Err(Refused::InUse(_)) => sessions_refusal(
                config,
                format!(
                    "infobase create --recreate removes nothing: the database '{}' on {} has open connections, so PostgreSQL refused to set it aside; close the sessions of the infobase and repeat",
                    self.name, self.server
                ),
                None,
                crate::domain::sessions::SessionsEvidence::Executor,
            ),
            Err(Refused::Other(error)) => error,
        };
        let registered_again = match &self.registration {
            Some(registration) if registration.infobase.is_some() => {
                register_again(&uncancelled(context), config, runner, host).err()
            }
            _ => None,
        };
        let error = match registered_again {
            None => error,
            Some(again) => error.followed_by(format!(
                "the registration of the infobase was dropped and not restored: {}; register it again with `cluster restart`",
                again.message()
            )),
        };
        Err(error)
    }

    /// Удача: отложенная БД удаляется. Неудача — строка для ответа: БД осталась под временным
    /// именем.
    pub(crate) fn discard(&self, context: &ExecutionContext, config: &AppConfig) -> Option<String> {
        self.discard_with(context, config, &ProcessExecutor, &RealHost)
    }

    pub(super) fn discard_with(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        runner: &dyn ProcessRunner,
        host: &dyn ServiceHost,
    ) -> Option<String> {
        if !self.database_exists {
            return None;
        }
        let context = &uncancelled(context);
        self.statement(
            context,
            config,
            runner,
            host,
            &postgres::drop_database(&self.aside),
        )
        .err()
        .map(|refused| {
            let error = refused.into_error();
            format!(
                "the old database was not dropped and stays as '{}' on {}: {}; drop it with PostgreSQL tools using the password file `dbms status` names",
                self.aside,
                self.server,
                error.message()
            )
        })
    }

    /// Неудача создания: то, что оставило создание, снимается, прежняя БД получает своё имя и
    /// регистрацию. Ответ — что вернуть не вышло.
    pub(crate) fn restore(&self, context: &ExecutionContext, config: &AppConfig) -> Vec<String> {
        self.restore_with(context, config, &ProcessExecutor, &RealHost)
    }

    pub(super) fn restore_with(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        runner: &dyn ProcessRunner,
        host: &dyn ServiceHost,
    ) -> Vec<String> {
        // Откат идёт и после отмены команды: её отмена его не обрывает.
        let context = &uncancelled(context);
        let mut left = Vec::new();
        if let Some(registration) = &self.registration {
            if let Err(error) = registration.drop_new(context, runner) {
                left.push(format!(
                    "the registration left by the failed creation: {}",
                    error.message()
                ));
            }
        }
        if !self.database_exists {
            return left;
        }
        let stand = match Stand::new(config, context, runner, host) {
            Ok(stand) => stand,
            Err(error) => {
                left.push(self.left_aside(error.message()));
                return left;
            }
        };
        // База, которую успело создать неудачное создание, — новая: прежняя лежит под
        // временным именем.
        let created = stand.database_exists(self.port, &self.bin, &self.name);
        if matches!(created, Ok(true)) {
            if let Err(error) = self.statement(
                context,
                config,
                runner,
                host,
                &postgres::drop_database(&self.name),
            ) {
                let error = error.into_error();
                left.push(self.left_aside(&format!(
                    "the database '{}' left by the failed creation was not dropped: {}",
                    self.name,
                    error.message()
                )));
                return left;
            }
        }
        if let Err(error) = self.statement(
            context,
            config,
            runner,
            host,
            &postgres::rename_database(&self.aside, &self.name),
        ) {
            left.push(self.left_aside(error.into_error().message()));
            return left;
        }
        if self
            .registration
            .as_ref()
            .is_some_and(|registration| registration.infobase.is_some())
        {
            if let Err(error) = register_again(context, config, runner, host) {
                left.push(format!(
                    "the registration of '{}' was not restored: {}; register it again with `cluster restart`",
                    self.name,
                    error.message()
                ));
            }
        }
        left
    }

    fn left_aside(&self, why: &str) -> String {
        format!(
            "the old database stays as '{}' on {} ({why}); rename it back to '{}' with PostgreSQL tools using the password file `dbms status` names",
            self.aside, self.server, self.name
        )
    }

    /// Одна команда SQL к своему PostgreSQL с паролем во временном файле.
    fn statement(
        &self,
        context: &ExecutionContext,
        config: &AppConfig,
        runner: &dyn ProcessRunner,
        host: &dyn ServiceHost,
        sql: &str,
    ) -> Result<(), Refused> {
        let stand = Stand::new(config, context, runner, host).map_err(Refused::Other)?;
        let passfile = stand.paths.passfile();
        let password = stand.password().map_err(Refused::Other)?;
        crate::support::fs::write_file_atomically(&passfile, |file| {
            file.write_all(postgres::passfile_line(self.port, &password).as_bytes())
        })
        .map_err(|error| Refused::Other(io_failure("cannot write", &passfile, &error)))?;
        let answer = runner.run_with_policy(
            &PostgresBuild::at(self.bin.clone()).statement(self.port, &passfile, sql),
            &policy(context),
        );
        let _ = std::fs::remove_file(&passfile);
        let answer = answer.map_err(|error| {
            Refused::Other(UseCaseError::new(
                UseCaseErrorKind::Platform,
                format!("psql did not run: {error}"),
            ))
        })?;
        if answer.outcome().is_ok() {
            return Ok(());
        }
        let error = UseCaseError::new(
            UseCaseErrorKind::Platform,
            format!("psql refused '{sql}': {}", answer.stderr.trim()),
        );
        if postgres::refused_as_in_use(&answer.stderr) {
            return Err(Refused::InUse(error));
        }
        Err(Refused::Other(error))
    }
}

/// Тот же контекст с собственной отменой: удаление отложенного и откат идут до конца и после
/// отмены команды, иначе прежняя база осталась бы под временным именем.
fn uncancelled(context: &ExecutionContext) -> ExecutionContext {
    context
        .clone()
        .with_cancellation(tokio_util::sync::CancellationToken::new())
}

/// Отказ команды SQL: из-за подключений к БД или по другой причине.
enum Refused {
    InUse(UseCaseError),
    Other(UseCaseError),
}

impl Refused {
    fn into_error(self) -> UseCaseError {
        match self {
            Self::InUse(error) | Self::Other(error) => error,
        }
    }
}
