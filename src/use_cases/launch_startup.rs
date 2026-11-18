//! Наблюдение входа клиента после `launch --observe-startup-ms`
//! (`INV.CLI.A-LAUNCH-OBSERVES-ITS-OWN-STARTUP`).
//!
//! PID в журнале регистрации нет (замер #777), поэтому попытку входа клиента узнают по
//! приложению, компьютеру, пользователю ОС и имени входа. Любая другая попытка входа в окне
//! делает исход неподтверждённым: две попытки одного вида в журнале не различить.

use std::time::{Duration, Instant};

use chrono::{DateTime, FixedOffset, Local, NaiveDateTime, TimeZone};
use tokio_util::sync::CancellationToken;

use crate::domain::launch::{LaunchStartup, StartupOutcome};
use crate::domain::sessions::ApplicationCode;
use crate::platform::event_log::{
    EventLogError, EventLogTail, LogEvent, LoginAttempt, ACCESS_DENIED, AUTHENTICATION,
    AUTHENTICATION_ERROR, LOGIN_EVENT_PREFIX, SESSION_FINISH, SESSION_START,
};
use crate::platform::process::ObservedSpawn;

/// Наибольшее окно наблюдения, мс.
pub const MAX_WINDOW_MS: u64 = 600_000;
/// Сколько клиент живёт после своего `Session.Start`, чтобы вход считался состоявшимся.
const LIFE_AFTER_START: Duration = Duration::from_secs(2);
/// Сколько журнал дочитывается после выхода клиента.
const LOG_AFTER_EXIT: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(200);

/// Какая попытка входа — попытка этого клиента.
#[derive(Debug, Clone)]
pub(crate) struct OurAttempt {
    /// `1CV8C` у тонкого клиента, `1CV8` у толстого и обычного приложения.
    pub application: ApplicationCode,
    /// `/N` из реквизитов базы; `None` — вход по пользователю ОС.
    pub infobase_user: Option<String>,
    /// Учётная запись ОС, от которой запущен клиент, без домена.
    pub os_user: String,
    /// Компьютер записи — эта машина.
    pub this_machine: fn(&str) -> bool,
}

impl OurAttempt {
    fn owns(&self, event: &LogEvent) -> bool {
        let same_os_user = |logged: &str| account_part(logged) == account_part(&self.os_user);
        crate::platform::session_list::application_of_rac(&event.application)
            == Some(self.application)
            && (self.this_machine)(&event.computer)
            && match (&event.login, &self.infobase_user) {
                (Some(LoginAttempt::ByName { user, os_user }), Some(name)) => {
                    user.to_lowercase() == name.to_lowercase() && same_os_user(os_user)
                }
                (Some(LoginAttempt::ByOsUser { os_user }), None) => same_os_user(os_user),
                _ => false,
            }
    }
}

/// Имя учётной записи без домена, без учёта регистра: `w11p-igor\Igor` — `igor`.
fn account_part(name: &str) -> String {
    name.rsplit('\\').next().unwrap_or(name).to_lowercase()
}

/// Почему исход этого запуска наблюдать нельзя — он сразу `unconfirmed`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Unobservable {
    #[error("the client opens the base by its web address, whose event log is not on this machine")]
    ViaWeb,
    #[error("a server base keeps its event log on the server, not on this machine")]
    ServerBase,
    #[error("--raw-key or tools.enterprise.additional-launch-keys names the base or the user, so the attempt cannot be told apart")]
    ConnectionKeys,
    #[error("the runner observes the sign-in only on Windows yet (https://github.com/IngvarConsulting/v8-runner-rust/issues/785)")]
    NotWindows,
    #[error("the OS account the runner runs as has no name the event log could be matched with")]
    UnknownOsUser,
    #[error("{0}")]
    Log(#[from] EventLogError),
}

/// Что делать с запуском: наблюдать по метке журнала или сразу ответить `unconfirmed`.
pub(crate) enum Plan {
    Observe {
        log: Box<EventLogTail>,
        attempt: OurAttempt,
    },
    Unobservable(Unobservable),
}

/// Что известно о запуске до старта клиента. Метка журнала ставится здесь, до запуска.
pub(crate) struct LaunchFacts<'a> {
    pub via_web: bool,
    /// Каталог файловой базы; `None` — база серверная.
    pub file_base: Option<&'a std::path::Path>,
    pub application: ApplicationCode,
    pub infobase_user: Option<&'a str>,
    /// `--raw-key` и `tools.enterprise.additional-launch-keys`.
    pub raw_keys: &'a [String],
}

pub(crate) fn plan(facts: &LaunchFacts<'_>) -> Plan {
    let unobservable = if facts.via_web {
        Some(Unobservable::ViaWeb)
    } else if facts.file_base.is_none() {
        Some(Unobservable::ServerBase)
    } else if facts.raw_keys.iter().any(|raw| names_connection_key(raw)) {
        Some(Unobservable::ConnectionKeys)
    } else if !cfg!(windows) {
        Some(Unobservable::NotWindows)
    } else {
        None
    };
    if let Some(reason) = unobservable {
        return Plan::Unobservable(reason);
    }
    let (Some(base), Some(os_user)) = (
        facts.file_base,
        crate::support::machine::account::account_name(),
    ) else {
        return Plan::Unobservable(Unobservable::UnknownOsUser);
    };
    match EventLogTail::mark(base) {
        Ok(log) => Plan::Observe {
            log: Box::new(log),
            attempt: OurAttempt {
                application: facts.application,
                infobase_user: facts.infobase_user.map(str::to_owned),
                os_user,
                this_machine: crate::domain::sessions::from_this_machine,
            },
        },
        Err(error) => Plan::Unobservable(error.into()),
    }
}

/// Ключ, который выбирает базу или пользователя мимо реквизитов базы: с ним попытку входа не
/// сопоставить с `infobase.user`.
fn names_connection_key(raw: &str) -> bool {
    const KEYS: [&str; 7] = ["f", "s", "ws", "ibconnectionstring", "n", "p", "wa"];
    let normalized = raw
        .trim_start()
        .trim_start_matches(['/', '-'])
        .to_ascii_lowercase();
    KEYS.iter()
        .any(|key| crate::domain::runner::launch_key_alias_matches(raw, key))
        || (raw.trim_start().starts_with(['/', '-'])
            && matches!(normalized.trim_end(), "wa+" | "wa-"))
}

/// Состояние клиента для решения.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Client {
    Running {
        /// Сколько прошло с тех пор, как раннер увидел `Session.Start` этой попытки.
        start_seen_for: Option<Duration>,
    },
    Exited {
        code: i32,
        /// Журнал после выхода дочитан.
        settled: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Решения ещё нет; `start_seen` — `Session.Start` этой попытки уже в журнале.
    Pending { start_seen: bool },
    Decided {
        outcome: StartupOutcome,
        event_at: Option<NaiveDateTime>,
        /// Почему `unconfirmed`.
        reason: Option<&'static str>,
    },
}

const ANOTHER_LOGIN: &str =
    "another sign-in to the base was recorded in the window, so this attempt cannot be told apart";
const MANY_ATTEMPTS: &str =
    "more than one sign-in of this client's kind was recorded, so this attempt cannot be told apart";
const UNKNOWN_LOGIN_EVENT: &str =
    "the sign-in of this attempt was recorded by an event the runner does not know";
const CLEAN_EXIT_WITHOUT_RECORDS: &str =
    "the client exited with code 0 and the event log recorded no sign-in";
const WINDOW_ENDED: &str = "the window ended before the event log showed the outcome";

/// Решение по событиям журнала после метки и состоянию клиента. Чистая функция: её таблицу
/// проверяют тесты на любой ОС.
pub(crate) fn decide(events: &[LogEvent], ours: &OurAttempt, client: Client) -> Verdict {
    let unconfirmed = |reason| Verdict::Decided {
        outcome: StartupOutcome::Unconfirmed,
        event_at: None,
        reason: Some(reason),
    };
    let decided = |outcome, event_at| Verdict::Decided {
        outcome,
        event_at,
        reason: None,
    };
    let logins: Vec<&LogEvent> = events
        .iter()
        .filter(|event| event.event.starts_with(LOGIN_EVENT_PREFIX))
        .collect();
    if logins.iter().any(|event| !ours.owns(event)) {
        return unconfirmed(ANOTHER_LOGIN);
    }
    let login = match logins.as_slice() {
        [] => {
            return match client {
                Client::Running { .. } | Client::Exited { settled: false, .. } => {
                    Verdict::Pending { start_seen: false }
                }
                Client::Exited { code: 0, .. } => unconfirmed(CLEAN_EXIT_WITHOUT_RECORDS),
                Client::Exited { .. } => decided(StartupOutcome::StartFailed, None),
            }
        }
        [login] => *login,
        _ => return unconfirmed(MANY_ATTEMPTS),
    };
    if login.event == AUTHENTICATION_ERROR {
        return match client {
            Client::Running { .. } => Verdict::Pending { start_seen: false },
            Client::Exited { .. } => decided(StartupOutcome::AuthFailed, Some(login.at)),
        };
    }
    if login.event != AUTHENTICATION {
        return unconfirmed(UNKNOWN_LOGIN_EVENT);
    }
    let of_this_session = |name: &str| {
        events.iter().find(|event| {
            event.event == name
                && event.connection == login.connection
                && event.session == login.session
        })
    };
    let start = of_this_session(SESSION_START);
    let ended =
        of_this_session(SESSION_FINISH).is_some() || of_this_session(ACCESS_DENIED).is_some();
    match (client, start) {
        (Client::Running { .. }, None) => Verdict::Pending { start_seen: false },
        (Client::Running { .. }, Some(_)) if ended => Verdict::Pending { start_seen: false },
        (Client::Running { start_seen_for }, Some(start)) => {
            if start_seen_for.is_some_and(|lived| lived >= LIFE_AFTER_START) {
                decided(StartupOutcome::Started, Some(start.at))
            } else {
                Verdict::Pending { start_seen: true }
            }
        }
        (Client::Exited { code: 0, .. }, Some(start)) => {
            decided(StartupOutcome::Closed, Some(start.at))
        }
        (Client::Exited { .. }, Some(start)) => {
            decided(StartupOutcome::StartFailed, Some(start.at))
        }
        (Client::Exited { settled: false, .. }, None) => Verdict::Pending { start_seen: false },
        (Client::Exited { code: 0, .. }, None) => unconfirmed(CLEAN_EXIT_WITHOUT_RECORDS),
        (Client::Exited { .. }, None) => decided(StartupOutcome::StartFailed, None),
    }
}

/// Чем кончилось наблюдение.
#[derive(Debug)]
pub(crate) enum Observation {
    Decided {
        startup: LaunchStartup,
        /// Почему исход не подтверждён.
        reason: Option<String>,
    },
    /// Команду отменили во время окна.
    Cancelled,
}

/// Наблюдение за запущенным клиентом: опрос его выхода и журнала, пока не будет решения,
/// не кончится окно или не придёт отмена. Клиента наблюдение не трогает.
pub(crate) fn observe(
    client: &mut ObservedSpawn,
    log: &mut EventLogTail,
    ours: &OurAttempt,
    window: Duration,
    cancellation: &CancellationToken,
) -> Observation {
    let deadline = Instant::now() + window;
    let mut events = Vec::new();
    let mut start_seen_at: Option<Instant> = None;
    let mut exited: Option<(i32, Instant)> = None;
    let unconfirmed = |exit_code, reason: String| Observation::Decided {
        startup: LaunchStartup {
            exit_code,
            ..LaunchStartup::unconfirmed()
        },
        reason: Some(reason),
    };
    loop {
        if cancellation.is_cancelled() {
            return Observation::Cancelled;
        }
        if exited.is_none() {
            match client.exit_code() {
                Ok(Some(code)) => exited = Some((code, Instant::now())),
                Ok(None) => {}
                Err(error) => {
                    return unconfirmed(None, format!("the client state could not be read: {error}"))
                }
            }
        }
        let exit_code = exited.map(|(code, _)| code);
        match log.read_new() {
            Ok(new) => events.extend(new),
            Err(error) => return unconfirmed(exit_code, error.to_string()),
        }
        let state = match exited {
            Some((code, at)) => Client::Exited {
                code,
                settled: at.elapsed() >= LOG_AFTER_EXIT,
            },
            None => Client::Running {
                start_seen_for: start_seen_at.map(|at| at.elapsed()),
            },
        };
        match decide(&events, ours, state) {
            Verdict::Decided {
                outcome,
                event_at,
                reason,
            } => {
                return Observation::Decided {
                    startup: LaunchStartup {
                        outcome,
                        event_at: event_at.and_then(local_time),
                        exit_code,
                    },
                    reason: reason.map(str::to_owned),
                }
            }
            Verdict::Pending { start_seen } => {
                if start_seen && start_seen_at.is_none() {
                    start_seen_at = Some(Instant::now());
                }
            }
        }
        if exited.is_none() && Instant::now() >= deadline {
            return unconfirmed(None, WINDOW_ENDED.to_owned());
        }
        std::thread::sleep(POLL);
    }
}

/// Местное время записи журнала со смещением зоны этой машины. Час, который при переводе
/// часов был дважды, — первый из двух; часа, которого не было, — `None`.
fn local_time(at: NaiveDateTime) -> Option<DateTime<FixedOffset>> {
    Local
        .from_local_datetime(&at)
        .earliest()
        .map(|time| time.fixed_offset())
}

/// Хвост сообщения `launch` об исходе входа.
pub(crate) fn summary(startup: &LaunchStartup, reason: Option<&str>) -> String {
    let mut text = format!("startup: {}", startup.outcome.as_str());
    if let Some(at) = startup.event_at {
        text.push_str(&format!(" at {}", at.to_rfc3339()));
    }
    if let Some(code) = startup.exit_code {
        text.push_str(&format!(", exit code {code}"));
    }
    if let Some(reason) = reason {
        text.push_str(&format!(" ({reason})"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::event_log::fixture::{self, *};

    fn stamp(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y%m%d%H%M%S").expect("stamp")
    }

    fn admin_thick() -> OurAttempt {
        OurAttempt {
            application: ApplicationCode::ThickClient,
            infobase_user: Some("админ".to_owned()),
            os_user: "IGOR".to_owned(),
            this_machine: |computer| computer.eq_ignore_ascii_case("w11p-igor"),
        }
    }

    fn event(name: &str, at: &str, application: &str, connection: u64) -> LogEvent {
        LogEvent {
            at: stamp(at),
            event: name.to_owned(),
            user: Some("Админ".to_owned()),
            computer: "w11p-igor".to_owned(),
            application: application.to_owned(),
            connection,
            session: 1,
            login: None,
        }
    }

    fn by_name(mut event: LogEvent, user: &str, os_user: &str) -> LogEvent {
        event.login = Some(LoginAttempt::ByName {
            user: user.to_owned(),
            os_user: os_user.to_owned(),
        });
        event
    }

    fn auth(at: &str, connection: u64) -> LogEvent {
        by_name(
            event(AUTHENTICATION, at, "1CV8", connection),
            "Админ",
            "w11p-igor\\Igor",
        )
    }

    fn auth_error(at: &str) -> LogEvent {
        let mut event = by_name(
            event(AUTHENTICATION_ERROR, at, "1CV8", 1),
            "Админ",
            "w11p-igor\\Igor",
        );
        event.user = None;
        event
    }

    fn running(seen: u64) -> Client {
        Client::Running {
            start_seen_for: Some(Duration::from_millis(seen)),
        }
    }

    const RUNNING: Client = Client::Running {
        start_seen_for: None,
    };

    fn exited(code: i32) -> Client {
        Client::Exited {
            code,
            settled: true,
        }
    }

    fn outcome(verdict: &Verdict) -> Option<(StartupOutcome, Option<NaiveDateTime>)> {
        match verdict {
            Verdict::Decided {
                outcome, event_at, ..
            } => Some((*outcome, *event_at)),
            Verdict::Pending { .. } => None,
        }
    }

    /// Таблица решения: каждый исход и каждая причина `unconfirmed`.
    #[test]
    fn the_decision_table_names_every_outcome() {
        let ours = admin_thick();
        let error = auth_error("20261119040916");
        let login = auth("20261119040014", 1);
        let start = event(SESSION_START, "20261119040015", "1CV8", 1);
        let pending = Verdict::Pending { start_seen: false };

        // Отказ во входе решает выход клиента.
        assert_eq!(decide(&[error.clone()], &ours, RUNNING), pending);
        assert_eq!(
            outcome(&decide(&[error.clone()], &ours, exited(1))),
            Some((StartupOutcome::AuthFailed, Some(error.at)))
        );
        // Вход по ОС: без `/N` наша попытка — вида 513.
        let mut os_login = error.clone();
        os_login.login = Some(LoginAttempt::ByOsUser {
            os_user: "w11p-igor\\Igor".to_owned(),
        });
        let by_os = OurAttempt {
            infobase_user: None,
            ..admin_thick()
        };
        assert_eq!(
            outcome(&decide(&[os_login.clone()], &by_os, exited(1))),
            Some((StartupOutcome::AuthFailed, Some(error.at)))
        );
        assert_eq!(
            outcome(&decide(&[os_login], &ours, exited(1))),
            Some((StartupOutcome::Unconfirmed, None))
        );

        // Сеанс начат: `started` — только после двух секунд жизни после `Start`.
        let both = [login.clone(), start.clone()];
        assert_eq!(
            decide(&both, &ours, RUNNING),
            Verdict::Pending { start_seen: true }
        );
        assert_eq!(
            decide(&both, &ours, running(1_999)),
            Verdict::Pending { start_seen: true }
        );
        assert_eq!(
            outcome(&decide(&both, &ours, running(2_000))),
            Some((StartupOutcome::Started, Some(start.at)))
        );
        assert_eq!(
            outcome(&decide(&both, &ours, exited(0))),
            Some((StartupOutcome::Closed, Some(start.at)))
        );
        assert_eq!(
            outcome(&decide(&both, &ours, exited(1))),
            Some((StartupOutcome::StartFailed, Some(start.at)))
        );
        // `AccessDenied` или `Finish` этого сеанса отменяют `started`.
        for end in [ACCESS_DENIED, SESSION_FINISH] {
            let denied = [
                login.clone(),
                start.clone(),
                event(end, "20261119040016", "1CV8", 1),
            ];
            assert_eq!(decide(&denied, &ours, running(5_000)), pending, "{end}");
        }
        // `Finish` чужого соединения не мешает.
        let other_finish = [
            login.clone(),
            start.clone(),
            event(SESSION_FINISH, "20261119040016", "1CV8", 2),
        ];
        assert_eq!(
            outcome(&decide(&other_finish, &ours, running(2_000))),
            Some((StartupOutcome::Started, Some(start.at)))
        );
        // Вход без `Start`, затем выход.
        assert_eq!(
            outcome(&decide(&[login.clone()], &ours, exited(1))),
            Some((StartupOutcome::StartFailed, None))
        );
        assert_eq!(
            outcome(&decide(&[login.clone()], &ours, exited(0))),
            Some((StartupOutcome::Unconfirmed, None))
        );

        // Записей нет: ненулевой выход — `start_failed`, но только после дочитывания журнала.
        assert_eq!(decide(&[], &ours, RUNNING), pending);
        assert_eq!(
            decide(
                &[],
                &ours,
                Client::Exited {
                    code: 1,
                    settled: false
                }
            ),
            pending
        );
        assert_eq!(
            outcome(&decide(&[], &ours, exited(1))),
            Some((StartupOutcome::StartFailed, None))
        );
        assert_eq!(
            decide(&[], &ours, exited(0)),
            Verdict::Decided {
                outcome: StartupOutcome::Unconfirmed,
                event_at: None,
                reason: Some(CLEAN_EXIT_WITHOUT_RECORDS)
            }
        );
    }

    /// Любая попытка входа, которую не связать с этим клиентом, делает исход неподтверждённым.
    #[test]
    fn a_login_that_is_not_told_apart_leaves_the_outcome_unconfirmed() {
        let ours = admin_thick();
        let error = auth_error("20261119040916");
        let reason_of = |events: &[LogEvent]| match decide(events, &ours, exited(1)) {
            Verdict::Decided {
                outcome: StartupOutcome::Unconfirmed,
                reason,
                ..
            } => reason,
            other => panic!("not unconfirmed: {other:?}"),
        };
        // Две попытки этого вида — например, второй толстый клиент с тем же именем.
        let mut second = error.clone();
        second.connection = 2;
        assert_eq!(reason_of(&[error.clone(), second]), Some(MANY_ATTEMPTS));

        let mut foreign_user = error.clone();
        foreign_user.login = Some(LoginAttempt::ByName {
            user: "БезПрав".to_owned(),
            os_user: "w11p-igor\\Igor".to_owned(),
        });
        let mut foreign_os_user = error.clone();
        foreign_os_user.login = Some(LoginAttempt::ByName {
            user: "Админ".to_owned(),
            os_user: "w11p-igor\\Other".to_owned(),
        });
        let mut thin = error.clone();
        thin.application = "1CV8C".to_owned();
        let mut designer = auth("20261119040916", 3);
        designer.application = "Designer".to_owned();
        let mut elsewhere = error.clone();
        elsewhere.computer = "other-host".to_owned();
        let mut unknown_data = error.clone();
        unknown_data.login = None;
        for foreign in [
            foreign_user,
            foreign_os_user,
            thin,
            designer,
            elsewhere,
            unknown_data,
        ] {
            assert_eq!(
                reason_of(&[error.clone(), foreign.clone()]),
                Some(ANOTHER_LOGIN),
                "{foreign:?}"
            );
            assert_eq!(reason_of(&[foreign.clone()]), Some(ANOTHER_LOGIN));
        }

        let mut unknown_event = error.clone();
        unknown_event.event = "_$Session$_.AuthenticationSecondFactor".to_owned();
        assert_eq!(reason_of(&[unknown_event]), Some(UNKNOWN_LOGIN_EVENT));
    }

    #[test]
    fn connection_and_user_keys_are_recognised_in_raw_keys() {
        for raw in [
            "/F",
            "/F C:\\base",
            "-s",
            "/S srv\\ib",
            "/WS",
            "/IBConnectionString",
            "/ibconnectionstring=File=x",
            "/N",
            "/N=user",
            "/P",
            "/WA-",
            "/WA+",
            "/wa",
        ] {
            assert!(names_connection_key(raw), "{raw}");
        }
        for raw in [
            "/TESTMANAGER",
            "/UsePrivilegedMode",
            "/DisplayAllFunctions",
            "/WSN",
            "/WSP",
            "/Prefix",
            "F",
        ] {
            assert!(!names_connection_key(raw), "{raw}");
        }
    }

    #[test]
    fn an_unobservable_launch_is_planned_before_the_log_is_read() {
        fn facts<'a>(
            via_web: bool,
            file_base: Option<&'a std::path::Path>,
            raw_keys: &'a [String],
        ) -> LaunchFacts<'a> {
            LaunchFacts {
                via_web,
                file_base,
                application: ApplicationCode::ThickClient,
                infobase_user: Some("Админ"),
                raw_keys,
            }
        }
        let base = tempfile::tempdir().expect("base");
        let keys = ["/N".to_owned(), "other".to_owned()];
        let reason = |plan: Plan| match plan {
            Plan::Unobservable(reason) => reason.to_string(),
            Plan::Observe { .. } => "observe".to_owned(),
        };
        assert!(reason(plan(&facts(true, Some(base.path()), &[]))).contains("web address"));
        assert!(reason(plan(&facts(false, None, &[]))).contains("server base"));
        assert!(reason(plan(&facts(false, Some(base.path()), &keys))).contains("--raw-key"));
        let local = reason(plan(&facts(false, Some(base.path()), &[])));
        if cfg!(windows) {
            assert_eq!(local, "observe");
            std::fs::create_dir_all(base.path().join("1Cv8Log")).expect("log dir");
            std::fs::write(base.path().join("1Cv8Log").join("1Cv8.lgd"), b"").expect("lgd");
            assert!(reason(plan(&facts(false, Some(base.path()), &[]))).contains("SQLite"));
        } else {
            assert!(local.contains("only on Windows"), "{local}");
        }
    }

    #[test]
    fn the_summary_names_the_outcome_time_and_code() {
        let startup = LaunchStartup {
            outcome: StartupOutcome::AuthFailed,
            event_at: DateTime::parse_from_rfc3339("2026-11-19T04:09:16+03:00").ok(),
            exit_code: Some(1),
        };
        assert_eq!(
            summary(&startup, None),
            "startup: auth_failed at 2026-11-19T04:09:16+03:00, exit code 1"
        );
        assert_eq!(
            summary(&LaunchStartup::unconfirmed(), Some(WINDOW_ENDED)),
            format!("startup: unconfirmed ({WINDOW_ENDED})")
        );
        let json = serde_json::to_value(&startup).expect("json");
        assert_eq!(
            json,
            serde_json::json!({
                "outcome": "auth_failed",
                "event_at": "2026-11-19T04:09:16+03:00",
                "exit_code": 1
            })
        );
    }

    /// Клиент, живущий `seconds` секунд и выходящий с кодом `code`.
    fn client_request(seconds: u32, code: i32) -> crate::platform::process::ProcessRequest {
        #[cfg(windows)]
        let (program, args) = (
            "cmd.exe",
            vec![
                "/S".to_owned(),
                "/C".to_owned(),
                format!("\"ping -n {} 127.0.0.1 >nul & exit {code}\"", seconds + 1),
            ],
        );
        #[cfg(not(windows))]
        let (program, args) = (
            "/bin/sh",
            vec!["-c".to_owned(), format!("sleep {seconds}; exit {code}")],
        );
        crate::platform::process::ProcessRequest {
            program: program.into(),
            args,
            workdir: None,
            stdout_log_path: None,
            stderr_log_path: None,
            startup_probe: None,
            windows_verbatim_args: if cfg!(windows) { vec![2] } else { Vec::new() },
        }
    }

    fn observed(
        seconds: u32,
        code: i32,
        records: &[&str],
        window: Duration,
        cancellation: &CancellationToken,
    ) -> (Observation, ObservedSpawn, Duration) {
        use crate::platform::process::{ProcessExecutor, ProcessRunner, WorkGiven};
        let base = tempfile::tempdir().expect("base");
        fixture::write_log(base.path(), &[]);
        let mut log = EventLogTail::mark(base.path()).expect("mark");
        if !records.is_empty() {
            fixture::append(
                base.path(),
                "20261116000000.lgp",
                format!(",\r\n{}", fixture::joined(records)),
            );
        }
        let work = WorkGiven::for_command();
        let mut client = ProcessExecutor
            .spawn_observed(&client_request(seconds, code), &work)
            .expect("spawn");
        assert!(work.given(), "a started client is the command's work");
        let started = Instant::now();
        let ours = OurAttempt {
            os_user: "igor".to_owned(),
            this_machine: |_| true,
            ..admin_thick()
        };
        let observation = observe(&mut client, &mut log, &ours, window, cancellation);
        (observation, client, started.elapsed())
    }

    fn decided(observation: Observation) -> LaunchStartup {
        match observation {
            Observation::Decided { startup, .. } => startup,
            Observation::Cancelled => panic!("cancelled"),
        }
    }

    /// Цикл наблюдения на настоящем процессе: отказ во входе — после выхода клиента; ранний
    /// выход без записей — `start_failed` после дочитывания журнала.
    #[test]
    fn an_exited_client_is_answered_by_its_records_and_its_code() {
        let none = CancellationToken::new();
        let (observation, _, _) = observed(
            1,
            1,
            &[THICK_AUTHENTICATION_ERROR],
            Duration::from_secs(60),
            &none,
        );
        let startup = decided(observation);
        assert_eq!(startup.outcome, StartupOutcome::AuthFailed);
        assert_eq!(startup.exit_code, Some(1));
        assert_eq!(
            startup.event_at.map(|at| at.naive_local()),
            Some(stamp("20261119040916"))
        );

        let (observation, _, elapsed) = observed(0, 3, &[], Duration::from_secs(60), &none);
        let startup = decided(observation);
        assert_eq!(
            (startup.outcome, startup.event_at, startup.exit_code),
            (StartupOutcome::StartFailed, None, Some(3))
        );
        assert!(elapsed >= LOG_AFTER_EXIT, "{elapsed:?}");
    }

    /// Живой клиент: `started` досрочно, через две секунды после `Start`; без записей —
    /// `unconfirmed` в конце окна; отмена — отмена. Клиент наблюдение не снимает.
    #[test]
    fn a_running_client_is_started_unconfirmed_or_cancelled() {
        let none = CancellationToken::new();
        let (observation, client, elapsed) = observed(
            30,
            0,
            &[THICK_AUTHENTICATION, THICK_START],
            Duration::from_secs(60),
            &none,
        );
        let startup = decided(observation);
        assert_eq!(
            (startup.outcome, startup.exit_code),
            (StartupOutcome::Started, None)
        );
        assert!(elapsed >= LIFE_AFTER_START && elapsed < Duration::from_secs(20));
        let _ = client.kill();

        let (observation, mut client, elapsed) =
            observed(30, 0, &[], Duration::from_millis(500), &none);
        let startup = decided(observation);
        assert_eq!(
            (startup.outcome, startup.exit_code),
            (StartupOutcome::Unconfirmed, None)
        );
        assert!(elapsed < Duration::from_secs(20));
        assert_eq!(
            client.exit_code().expect("state"),
            None,
            "observation leaves the client running"
        );
        let _ = client.kill();

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let (observation, client, _) =
            observed(30, 0, &[], Duration::from_secs(60), &cancelled);
        assert!(matches!(observation, Observation::Cancelled));
        let error = client.kill();
        assert!(matches!(
            error,
            crate::platform::process::ProcessError::Cancelled {
                delivered: true,
                ..
            }
        ));
    }
}
