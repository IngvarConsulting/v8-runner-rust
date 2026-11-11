//! Политика чужих сеансов у применения (`push`/`apply --sessions`, #211): что делать, когда
//! обновлению конфигурации базы данных нужен монопольный доступ, а базу держат сеансы
//! (`INV.CLI.APPLY-SESSIONS-GOVERNS-FOREIGN-SESSIONS`).
//!
//! Признак «базу держат» берётся из состояния, а не из текста платформы
//! (`INV.PLATFORM.PROSE-DEBT-ONLY-SHRINKS`): у агента — `error-type`
//! `InfoBaseExclusiveLockRequired`, у файловой базы — блокировка файлов `1Cv8.1CD` (сеансы) и
//! `1Cv8.cgr` (Конфигуратор), у сервера без улики исполнителя — чужие живые сеансы в его
//! собственном списке: `rac session list` кластера раннера, `ibcmd session list` автономного
//! сервера (замер #613). Текст отказа даёт только список сеансов, и тот разобран целиком или не
//! дан вовсе.
//!
//! Завершать сеансы раннер умеет только у сервера ключом самого исполнителя, и только когда
//! признак говорит, что базу держат сеансы. У файловой базы их не завершает ни один ключ (замер
//! #211, #613), а процессы людей раннер не снимает: отказ называет сеансы, и выход из него —
//! тот же вызов, когда их закроют.

use std::sync::{Arc, Mutex, PoisonError};

use crate::config::model::AppConfig;
use crate::domain::capability::TargetKind;
use crate::domain::next_step::NextStep;
use crate::domain::sessions::{
    InfobaseSessions, SessionEntry, SessionLock, SessionsEvidence, SessionsOutcome,
};
use crate::platform::agent::AgentErrorType;
use crate::platform::file_base_hold;
use crate::platform::secrets::mask_text;
use crate::platform::session_list::parse_session_list;
use crate::support::error::AppError;
use crate::use_cases::result::{UseCaseError, UseCaseErrorKind};

/// Ключ `--sessions`.
pub const SESSIONS_KEY: &str = "--sessions";

/// Сообщение завершаемым сеансам, если `--sessions-message` не задан.
pub const DEFAULT_MESSAGE: &str = "Сеанс завершён: v8-runner обновляет конфигурацию базы данных";

/// Что делать с чужими сеансами, когда применению нужен монопольный доступ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SessionsPolicy {
    /// Не трогать: отказ называет сеансы (`--sessions disable`, умолчание).
    #[default]
    Disable,
    /// Завершить их ключом исполнителя, где он это может (`--sessions force`).
    Force {
        /// Сообщение тем, чьи сеансы завершаются.
        message: String,
    },
}

/// Политика команды и то, что она сделала. Клоны контекста делят исход, как отметку работы.
#[derive(Debug, Clone, Default)]
pub struct SessionsGate {
    policy: SessionsPolicy,
    /// Этот же вызов без ключей сеансов: из него отказ строит `next`. Нет его — у MCP, где
    /// ключа нет, — отказ шага не называет.
    call: Option<NextStep>,
    outcome: Arc<Mutex<Option<SessionsOutcome>>>,
}

impl SessionsGate {
    /// Политика вызова `call` — команды и набора, как их назвал вызывающий.
    pub fn new(policy: SessionsPolicy, call: NextStep) -> Self {
        Self {
            policy,
            call: Some(call),
            outcome: Arc::default(),
        }
    }

    pub(crate) const fn policy(&self) -> &SessionsPolicy {
        &self.policy
    }

    /// Исход `--sessions force`; у `disable` его нет.
    pub fn outcome(&self) -> Option<SessionsOutcome> {
        self.slot().clone()
    }

    /// Дописывает исход одного применения: команда применяет наборы по очереди.
    pub(crate) fn note(&self, outcome: SessionsOutcome) {
        let mut slot = self.slot();
        match slot.as_mut() {
            None => *slot = Some(outcome),
            Some(known) => {
                known.terminated = match (known.terminated.take(), outcome.terminated) {
                    (Some(mut before), Some(now)) => {
                        before.extend(now);
                        Some(before)
                    }
                    _ => None,
                };
                known.lock = outcome.lock;
                known.message_delivered |= outcome.message_delivered;
            }
        }
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<SessionsOutcome>> {
        self.outcome.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Этот же вызов; с `force` — с ключом `--sessions force`.
    fn next(&self, force: bool) -> Option<NextStep> {
        let call = self.call.clone()?;
        Some(if force {
            call.with_key(SESSIONS_KEY, "force")
        } else {
            call
        })
    }
}

/// Форма, несущая исход `--sessions force`: `push` и `apply`.
pub(crate) trait CarriesSessions {
    fn sessions_mut(&mut self) -> &mut Option<SessionsOutcome>;
}

impl CarriesSessions for crate::domain::build::BuildResult {
    fn sessions_mut(&mut self) -> &mut Option<SessionsOutcome> {
        &mut self.sessions
    }
}

impl CarriesSessions for crate::domain::apply::ApplyResult {
    fn sessions_mut(&mut self) -> &mut Option<SessionsOutcome> {
        &mut self.sessions
    }
}

/// Ставит в ответ исход `--sessions force` — и удачи, и отказа, если применение до него дошло.
pub(crate) fn stamp_sessions<T: CarriesSessions>(
    mut outcome: crate::use_cases::result::UseCaseResult<T>,
    context: &crate::use_cases::context::ExecutionContext,
) -> crate::use_cases::result::UseCaseResult<T> {
    let Some(sessions) = context.sessions().outcome() else {
        return outcome;
    };
    if let Some(payload) = crate::use_cases::result::payload_mut(&mut outcome) {
        *payload.sessions_mut() = Some(sessions);
    }
    outcome
}

/// Кто держит базу после отказа исполнителя.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Holder {
    /// Сеансы по улике исполнителя; список — из текста отказа, если он разобран целиком.
    Sessions(Option<Vec<SessionEntry>>),
    /// Чужие живые сеансы из собственного списка сервера: улики исполнителя нет.
    Listed(Vec<SessionEntry>),
    /// Конфигуратор файловой базы.
    Designer,
}

impl Holder {
    /// Сеансы, которые держат базу, и откуда это известно; у Конфигуратора — `None`.
    pub(crate) fn sessions(self) -> Option<(Option<Vec<SessionEntry>>, SessionsEvidence)> {
        match self {
            Self::Sessions(list) => Some((list, SessionsEvidence::Executor)),
            Self::Listed(list) => Some((Some(list), SessionsEvidence::ServerList)),
            Self::Designer => None,
        }
    }
}

/// Чем было применение, которое отказало.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Executor {
    /// Процесс Конфигуратора или `ibcmd`: к моменту пробы он уже вышел и базу не держит.
    Process,
    /// Агент после `connect-ib`: базу для конфигурирования держит он сам.
    Agent,
}

/// Отказ одного запуска исполнителя — то, из чего решается, кто держит базу.
#[derive(Debug, Default)]
pub(crate) struct Evidence<'a> {
    /// Текст отказа: `/Out` Конфигуратора, `message` агента. Источник списка, а не признак.
    pub(crate) text: Option<&'a str>,
    /// `error-type` агента.
    pub(crate) agent_error: Option<&'a AgentErrorType>,
    /// Агент на ключ завершения ответил вопросом «Отмена»/«Повторить» (замер #211): базе нужен
    /// монопольный доступ, а сеансы его ключ не завершил.
    pub(crate) agent_asked: bool,
}

/// Кто держит базу после отказа. Сначала улика исполнителя: у агента — `error-type`
/// `InfoBaseExclusiveLockRequired` или его вопрос (список — из текста, а если тот не разобран, —
/// из списка сервера); Конфигуратор при агенте по файлу не узнаётся — файл держит он сам. У
/// процесса файловой базы — блокировка файлов; при любом её сеансе обновление с запретом
/// динамического не проходит (замер #211), и отказ — из-за сеансов. Без улики исполнителя у
/// сервера — чужие живые сеансы из его собственного списка (`server_sessions`: `rac` кластера
/// раннера, `ibcmd session list` автономного сервера; замер #613), и список тогда оттуда. Проба
/// или источник, которые не ответили, признаком не служат: отказ остаётся отказом исполнителя.
pub(crate) fn holder(
    config: &AppConfig,
    executor: Executor,
    evidence: &Evidence<'_>,
    server_sessions: &dyn Fn() -> Option<Vec<SessionEntry>>,
) -> Option<Holder> {
    let server = config.target_kind() != TargetKind::File;
    if executor == Executor::Agent {
        let held = evidence.agent_asked
            || evidence.agent_error == Some(&AgentErrorType::InfoBaseExclusiveLockRequired);
        if held {
            let list = evidence
                .text
                .and_then(parse_session_list)
                .or_else(|| server.then(server_sessions).flatten());
            return Some(Holder::Sessions(list));
        }
    }
    if server {
        return server_sessions().map(Holder::Listed);
    }
    if executor == Executor::Agent {
        return None;
    }
    let base = file_base_dir(config)?;
    if file_base_hold::designer_hold(&base).ok()?.is_held() {
        return Some(Holder::Designer);
    }
    file_base_hold::sessions_hold(&base)
        .ok()?
        .is_held()
        .then(|| Holder::Sessions(evidence.text.and_then(parse_session_list)))
}

/// Каталог файловой базы, как его понимает платформа: без кавычек и от каталога проекта.
fn file_base_dir(config: &AppConfig) -> Option<std::path::PathBuf> {
    config.v8_connection().file_infobase_dir(&config.base_path)
}

/// Отказ загрузки процессом раннера, который уже вышел: если файловую базу держит чужой
/// Конфигуратор, это `infobase_designer_busy` — загрузка отказывает раньше применения. Иначе
/// отказ остаётся как был.
pub(crate) fn designer_busy_at_load(
    context: &crate::use_cases::context::ExecutionContext,
    config: &AppConfig,
    error: AppError,
) -> AppError {
    if error.cancellation().is_some() {
        return error;
    }
    let held = file_base_dir(config).is_some_and(|base| {
        file_base_hold::designer_hold(&base).is_ok_and(file_base_hold::Hold::is_held)
    });
    if held {
        refusal(context.sessions(), config, Holder::Designer, &error)
    } else {
        error
    }
}

/// Блокировка начала сеансов у базы: у файловой — файл `1Cv8.cdn`, у сервера узнать нельзя.
pub(crate) fn session_lock(config: &AppConfig) -> SessionLock {
    match config.target_kind() {
        TargetKind::File => file_base_dir(config).map_or_else(SessionLock::unknown, |base| {
            file_base_hold::session_lock(&base)
        }),
        TargetKind::Cluster | TargetKind::Standalone => SessionLock::unknown(),
    }
}

/// Сообщение, с которым применение повторяется ключом завершения сеансов: только при
/// `--sessions force` и только у сервера. Файловой базе ключ не помогает ни у одного
/// исполнителя, а агент на нём ждёт ответа (замер #211), поэтому туда он не идёт никогда.
pub(crate) fn force_message(policy: &SessionsPolicy, config: &AppConfig) -> Option<String> {
    match policy {
        SessionsPolicy::Force { message } if terminable(config) => Some(message.clone()),
        SessionsPolicy::Force { .. } | SessionsPolicy::Disable => None,
    }
}

/// Может ли раннер завершить сеансы базы: у сервера — ключом исполнителя, у файловой — нет.
pub(crate) fn terminable(config: &AppConfig) -> bool {
    config.target_kind() != TargetKind::File
}

/// Тот же компьютер: имена сравниваются без домена и без учёта регистра.
fn same_machine(computer: &str, host: &str) -> bool {
    let short = |name: &str| name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    short(computer) == short(host)
}

/// Отказ «базу держат»: `infobase_sessions` со списком или `infobase_designer_busy`. Улика —
/// отказ исполнителя — идёт в конце текста.
pub(crate) fn refusal(
    gate: &SessionsGate,
    config: &AppConfig,
    holder: Holder,
    cause: &AppError,
) -> AppError {
    let (kind, list, terminable, evidence, headline) = match holder.sessions() {
        None => (
            UseCaseErrorKind::InfobaseDesignerBusy,
            None,
            false,
            SessionsEvidence::Executor,
            "the infobase is open in the Designer, and the runner does not close it: the changes there may be unsaved; close the Designer and repeat the command".to_owned(),
        ),
        Some((list, evidence)) => {
            let terminable = terminable(config);
            let named = list.as_deref().map_or_else(
                || "the platform did not list them".to_owned(),
                name_sessions,
            );
            let way_out = if terminable {
                "end them with --sessions force, or close them and repeat the command"
            } else {
                "the runner does not end sessions of a file infobase: close them and repeat the command"
            };
            let held = match evidence {
                SessionsEvidence::Executor => "other sessions hold the infobase",
                SessionsEvidence::ServerList => {
                    "the server lists other sessions of the infobase, and the executor did not say why it refused"
                }
            };
            (
                UseCaseErrorKind::InfobaseSessions,
                list,
                terminable,
                evidence,
                format!(
                    "the database configuration was not updated: {held} ({named}); {way_out}"
                ),
            )
        }
    };
    let executor_error = (evidence == SessionsEvidence::ServerList).then(|| cause.to_string());
    // Живой агент раннера, оставшийся подключённым к базе, — тоже её сеанс Конфигуратора (#644):
    // отказ называет его и выход `agent release`, не утверждая, что держит именно он.
    let headline = match crate::use_cases::infobase_lock::attached_live_agents_hint(config) {
        Some(agents) => format!("{headline}; {agents}"),
        None => headline,
    };
    let mut refusal = UseCaseError::new(kind, format!("{headline}; {cause}")).with_sessions(
        infobase_sessions(config, list, terminable, evidence, executor_error),
    );
    if let Some(next) = gate.next(terminable) {
        refusal = refusal.with_next(next);
    }
    AppError::Refused(Box::new(refusal))
}

/// Поле `error.sessions` — единственный построитель у всех отказов `infobase_sessions`:
/// база без учётных данных, сеансы, с этой ли машины, можно ли их завершить, блокировка
/// начала сеансов, источник признака и — при признаке из списка сервера — отказ исполнителя.
pub(crate) fn infobase_sessions(
    config: &AppConfig,
    list: Option<Vec<SessionEntry>>,
    terminable: bool,
    evidence: SessionsEvidence,
    executor_error: Option<String>,
) -> InfobaseSessions {
    let password = config.infobase.password.as_deref().unwrap_or_default();
    let from_this_machine = match (&list, crate::support::machine::host_name()) {
        (Some(list), Some(host)) => list
            .iter()
            .any(|entry| same_machine(&entry.computer, &host)),
        _ => false,
    };
    InfobaseSessions {
        base: mask_text(&config.infobase.connection, &[password]),
        list,
        from_this_machine,
        terminable,
        lock: session_lock(config),
        evidence,
        executor_error,
    }
}

/// Свой `ibsrv` над файловой базой (`standalone.managed.db-path`): на 8.5.4.2028 после
/// завершения сеанса он отказывает в обновлении, пока его не перезапустят (замер #613), — и
/// неудачный повтор после `--sessions force` лечит `standalone restart`.
pub(crate) fn restart_heals(config: &AppConfig) -> bool {
    config
        .infobase
        .standalone
        .as_ref()
        .and_then(|standalone| standalone.managed.as_ref())
        .is_some_and(|managed| managed.db_path.is_some())
}

/// Отказ повтора после завершения сеансов у своего `ibsrv` над файловой базой: тот же отказ
/// исполнителя с выходом `standalone restart`.
pub(crate) fn restart_refusal(cause: AppError) -> AppError {
    let cause = UseCaseError::from(cause);
    let refusal = UseCaseError::new(
        cause.kind(),
        format!(
            "the sessions were ended, but the update failed again: a standalone server the runner started over a file infobase may refuse updates after it ended a session until it is restarted (measured on 8.5.4.2028), so standalone restart is the likely way out; then repeat the command; {}",
            cause.message()
        ),
    )
    .with_next(NextStep::command("standalone restart"));
    AppError::Refused(Box::new(refusal))
}

fn name_sessions(list: &[SessionEntry]) -> String {
    list.iter()
        .map(|entry| {
            format!(
                "{} on {} since {}",
                entry.application_title, entry.computer, entry.started
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::sessions::SessionLockState;

    fn outcome(terminated: Option<Vec<SessionEntry>>, delivered: bool) -> SessionsOutcome {
        SessionsOutcome {
            terminated,
            lock: SessionLock::unknown(),
            message_delivered: delivered,
        }
    }

    fn entry(computer: &str) -> SessionEntry {
        SessionEntry {
            computer: computer.to_owned(),
            application: None,
            application_title: "Тонкий клиент".to_owned(),
            started: "2026-11-11T02:49:42".to_owned(),
            user: None,
            id: None,
        }
    }

    #[test]
    fn outcomes_of_several_applies_add_up() {
        let gate = SessionsGate::new(
            SessionsPolicy::Force {
                message: "m".to_owned(),
            },
            NextStep::command("push"),
        );
        gate.note(outcome(Some(vec![entry("a")]), false));
        gate.note(outcome(Some(vec![entry("b")]), true));
        let known = gate.outcome().expect("outcome");
        assert_eq!(known.terminated.map(|list| list.len()), Some(2));
        assert!(known.message_delivered);
        assert_eq!(known.lock.state, SessionLockState::Unknown);

        gate.note(outcome(None, false));
        assert_eq!(gate.outcome().expect("outcome").terminated, None);
    }

    #[test]
    fn the_next_step_is_the_same_call_with_or_without_force() {
        let gate = SessionsGate::new(
            SessionsPolicy::Disable,
            NextStep::command("apply").for_source_set("main"),
        );
        let force = gate.next(true).expect("next");
        assert_eq!(force.command, "apply");
        assert_eq!(force.source_set.as_deref(), Some("main"));
        assert_eq!(
            force.keys.get(SESSIONS_KEY).map(String::as_str),
            Some("force")
        );
        assert!(gate.next(false).expect("next").keys.is_empty());
        assert!(SessionsGate::default().next(true).is_none());
    }

    #[test]
    fn the_agent_is_held_by_sessions_only_by_its_error_type_or_its_question() {
        let dir = tempfile::tempdir().expect("dir");
        let config = file_config(dir.path());
        let message = crate::platform::session_list::tests::MEASURED_AGENT_MESSAGE;
        let exclusive = AgentErrorType::InfoBaseExclusiveLockRequired;
        let held = holder(
            &config,
            Executor::Agent,
            &Evidence {
                text: Some(message),
                agent_error: Some(&exclusive),
                agent_asked: false,
            },
            &|| None,
        );
        let Some(Holder::Sessions(Some(list))) = held else {
            panic!("held by sessions: {held:?}");
        };
        assert_eq!(list.len(), 1);
        assert!(matches!(
            holder(
                &config,
                Executor::Agent,
                &Evidence {
                    agent_asked: true,
                    ..Evidence::default()
                },
                &|| None,
            ),
            Some(Holder::Sessions(None))
        ));
        let other = AgentErrorType::ConfigFilesError;
        assert_eq!(
            holder(
                &config,
                Executor::Agent,
                &Evidence {
                    text: Some(message),
                    agent_error: Some(&other),
                    agent_asked: false,
                },
                &|| None,
            ),
            None,
            "the words of the message decide nothing"
        );
    }

    /// Сервер без улики исполнителя: признак — живые сеансы из его собственного списка, и
    /// список берётся оттуда, а не из текста. Так у кластера и у автономного сервера, после
    /// пакетного Конфигуратора и после агента, чья ошибка не о сеансах (агент SSH-шлюза
    /// отказывает из-за сеансов итогом `canceled`, замер #613). Улика агента списка сервера не
    /// требует; его спрашивают, только если текст агента не разобран.
    #[test]
    fn a_server_is_held_by_the_awake_sessions_of_its_own_list() {
        let dir = tempfile::tempdir().expect("dir");
        let cluster = AppConfig {
            infobase: crate::config::model::InfobaseConfig::file("Srvr=host;Ref=ib"),
            ..file_config(dir.path())
        };
        let mut standalone = cluster.clone();
        standalone.infobase.standalone = Some(crate::config::model::StandaloneConfig {
            managed: None,
            gate: None,
            host_fingerprint: None,
            exchange: None,
        });
        let text = crate::platform::session_list::tests::MEASURED_LIST_RU;
        let process = Evidence {
            text: Some(text),
            ..Evidence::default()
        };
        let other = AgentErrorType::Other("Canceled".to_owned());
        let agent_without_evidence = Evidence {
            text: Some(text),
            agent_error: Some(&other),
            agent_asked: false,
        };
        let listed = || Some(vec![entry("server-host")]);
        for config in [&cluster, &standalone] {
            for (executor, evidence) in [
                (Executor::Process, &process),
                (Executor::Agent, &agent_without_evidence),
            ] {
                assert_eq!(
                    holder(config, executor, evidence, &listed),
                    Some(Holder::Listed(vec![entry("server-host")])),
                    "{executor:?}"
                );
                assert_eq!(
                    holder(config, executor, evidence, &|| None),
                    None,
                    "the text alone decides nothing: {executor:?}"
                );
            }
        }
        let exclusive = AgentErrorType::InfoBaseExclusiveLockRequired;
        let asked = std::cell::Cell::new(0);
        let counted = || {
            asked.set(asked.get() + 1);
            listed()
        };
        let held = holder(
            &cluster,
            Executor::Agent,
            &Evidence {
                text: Some(text),
                agent_error: Some(&exclusive),
                agent_asked: false,
            },
            &counted,
        );
        assert!(matches!(held, Some(Holder::Sessions(Some(ref list))) if list.len() == 2));
        assert_eq!(asked.get(), 0, "the agent's own text names the sessions");
        assert_eq!(
            holder(
                &cluster,
                Executor::Agent,
                &Evidence {
                    agent_asked: true,
                    ..Evidence::default()
                },
                &counted,
            ),
            Some(Holder::Sessions(Some(vec![entry("server-host")])))
        );
    }

    /// Ключ завершения сеансов не идёт к файловой базе ни при каком `--sessions`: агент на нём
    /// ждёт ответа, а сеанс он не завершает ни у одного исполнителя (замер #211).
    #[test]
    fn a_file_base_never_gets_the_key_that_ends_sessions() {
        let dir = tempfile::tempdir().expect("dir");
        let force = SessionsPolicy::Force {
            message: "m".to_owned(),
        };
        assert_eq!(force_message(&force, &file_config(dir.path())), None);
        let server = AppConfig {
            infobase: crate::config::model::InfobaseConfig::file("Srvr=host;Ref=ib"),
            ..file_config(dir.path())
        };
        assert_eq!(force_message(&force, &server).as_deref(), Some("m"));
        assert_eq!(force_message(&SessionsPolicy::Disable, &server), None);
    }

    /// Каталог базы — от каталога проекта и без кавычек, как его понимает платформа.
    #[test]
    fn a_relative_quoted_file_base_is_probed_where_the_platform_opens_it() {
        let dir = tempfile::tempdir().expect("dir");
        let base = dir.path().join("build").join("ib");
        std::fs::create_dir_all(&base).expect("base");
        std::fs::write(
            base.join(file_base_hold::SESSION_LOCK_FILE),
            "{1,20261111000000,00010101000000,\"admin\",\"K\",\"\",0}",
        )
        .expect("cdn");
        let config = AppConfig {
            infobase: crate::config::model::InfobaseConfig::file("File=\"build/ib\""),
            ..file_config(dir.path())
        };
        let lock = session_lock(&config);
        assert_eq!(lock.state, SessionLockState::Foreign);
        assert_eq!(lock.message.as_deref(), Some("admin"));
    }

    #[test]
    fn a_machine_is_the_same_without_its_domain_and_case() {
        assert!(same_machine("W11P-IGOR", "w11p-igor.corp.local"));
        assert!(!same_machine("w11p-igor", "w11p-ivan"));
    }

    fn file_config(dir: &std::path::Path) -> AppConfig {
        AppConfig {
            base_path: dir.to_path_buf(),
            work_path: dir.join("work"),
            local_path: dir.join(".local"),
            format: crate::config::model::SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file(format!(
                "File={}",
                dir.join("ib").display()
            )),
            infobases: Default::default(),
            infobase_name: None,
            working_copy: None,
            source_sets: Vec::new(),
            tools: crate::config::model::ToolsConfig::default(),
            mcp: Default::default(),
            tests: crate::config::model::TestsConfig::default(),
        }
    }
}
