//! Клиент сторожа: находит живого агента ключа по записи или поднимает сторожа, открывает
//! сессию команды (`begin` … `end`) и говорит с ним по каналу. Замок реестра учётной записи
//! берётся только на поиск и на запись «запускается», а не на все 6–9 с подъёма агента.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::BufReader;

use super::channel::{connect, Stream};
use super::protocol::{
    read_line, write_line, Frame, Holder, KeeperError, KeeperStatus, Request,
};
use super::{credentials_digest, KeeperHome};
use crate::platform::agent::{AgentError, AgentMessage, AgentReply, WaitPolicy};
use crate::platform::live_registry::lock_agent_registry;
use crate::platform::process::{ProcessExecutor, ProcessInterruptionReason, WorkGiven};
use crate::platform::service::{process_facts, ServiceRequest, ServiceSpawnError};
use crate::platform::service_record::{
    adopt_recorded_pid, machine_fingerprint, observe_service, read_json, write_json, Observed,
    RecordState, ServiceRecord,
};

/// Что поднять агентом: всё, кроме учётных данных.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeeperLaunch {
    /// `1cv8` выбранной платформы.
    pub v8: PathBuf,
    /// Адрес базы для командной строки агента (`/F`, `/S`).
    pub infobase_args: Vec<String>,
    /// Объявленный порт агента; без него — свободный.
    pub port: Option<u16>,
    /// Объявленный ключ хоста (`tools.designer_agent.host-key`); без него — одноразовый.
    pub host_key: Option<PathBuf>,
    pub startup_timeout_ms: u64,
    /// Путь базы в каноническом виде: он идёт в запись агента.
    pub infobase: String,
}

/// Учётные данные базы: сторож открывает ими сессию агента. В командную строку и журнал они не
/// попадают.
#[derive(Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub user: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials { .. }")
    }
}

/// Строка stdin сторожа.
#[derive(Debug, Serialize, Deserialize)]
pub struct KeeperStart {
    pub launch: KeeperLaunch,
    pub credentials: Credentials,
}

/// Сторож не поднят или агент не взят.
#[derive(Debug, thiserror::Error)]
pub enum KeeperStartError {
    /// Раннер в Job с kill-on-close без права выхода: сторож не переживёт команду, и команда
    /// работает с разовым агентом (`INV.PLATFORM.A-CLOSED-JOB-KEEPS-THE-AGENT-ONE-SHOT`).
    #[error("the runner runs in a Job that kills its processes on close and forbids leaving it")]
    Job,
    /// Живых агентов учётной записи уже столько, сколько допускает предел, и снять
    /// простаивающего некого (`INV.PLATFORM.A-COMMAND-OVER-THE-LIMIT-USES-A-ONE-SHOT-AGENT`).
    #[error("the OS account already keeps {limit} live agents, and none of them is idle")]
    Limit { limit: u8 },
    /// Живой агент этого ключа ещё служит сессии другой команды.
    #[error("the live agent still serves another command{}", .command.as_deref().map(|command| format!(": {command}")).unwrap_or_default())]
    Busy { command: Option<String> },
    /// Сторож не поднялся или не ответил: путь канала, отказ запуска, срок.
    #[error("{detail}")]
    Unavailable { detail: String },
    #[error(transparent)]
    Agent(#[from] AgentError),
}

impl From<std::io::Error> for KeeperStartError {
    fn from(error: std::io::Error) -> Self {
        Self::Agent(AgentError::Keeper {
            detail: error.to_string(),
        })
    }
}

/// Сколько ждут сторожа сверх срока подъёма агента.
const START_MARGIN: Duration = Duration::from_secs(30);
/// Шаг ожидания сторожа, который поднимает другой клиент или который закрывается.
const WAIT_STEP: Duration = Duration::from_millis(100);
/// Сколько раз клиент поднимает сторожа заново, если поднятый вышел, не ответив: так бывает,
/// когда канал ещё держит уходящий сторож того же ключа.
const RESPAWNS: u32 = 3;
/// Сколько клиент ждёт ответа на `end`: отключение от базы ограничено своим сроком у сторожа.
const END_PATIENCE: Duration = Duration::from_secs(30);
/// Сколько клиент ждёт подключения к каналу и ответа на `status`, `hold`, `release`: зависший
/// сторож не держит ни команду, ни обход сторожей учётной записи.
const QUERY_PATIENCE: Duration = Duration::from_secs(5);
/// Сколько клиент ждёт ответа на `stop`: сторож отвечает, когда агент снят, а критическую
/// команду он доводит до исхода.
const STOP_PATIENCE: Duration = Duration::from_secs(120);

/// Открытая сессия команды: её подключение держится до `end`.
struct OpenSession {
    id: String,
    stream: BufReader<Stream>,
}

/// Подключение команды к сторожу ключа.
pub struct KeeperClient {
    home: KeeperHome,
    channel: String,
    digest: String,
    /// Процесс сторожа: канал должен обслуживать он.
    keeper: u32,
    /// Взят ли уже живой агент, а не поднят этим клиентом.
    reused: bool,
    session: Option<OpenSession>,
    /// Прерывание, отложенное в критической фазе за всю сессию, и последней командой.
    deferred: Option<ProcessInterruptionReason>,
    last_deferral: Option<ProcessInterruptionReason>,
    /// Команду прервали, пока агент выполнял её запрос, или его сессия оборвалась: сторож
    /// снимает агента (`INV.PLATFORM.AN-INTERRUPTED-COMMAND-REMOVES-THE-LIVE-AGENT`).
    interrupted: bool,
    runtime: tokio::runtime::Runtime,
}

/// Что делать со сторожем ключа по его записи.
enum Found {
    Live(u32),
    /// Его поднимает другой клиент.
    Starting,
    /// Его нет: запись «запускается» уже стоит за этим клиентом.
    Absent,
    /// Его нет, а живых агентов других ключей уже столько, сколько допускает предел.
    OverLimit,
}

/// Живой сторож учётной записи, найденный по записи.
pub struct LiveKeeper {
    pub key: String,
    pub client: KeeperClient,
}

/// Чем кончился `stop` с условием.
#[derive(Debug)]
pub enum Stopped {
    /// Агент снят, или сторож ушёл сам, пока его снимали.
    Removed,
    /// Агент занят или его держат: сторож его не снял.
    InUse {
        command: Option<String>,
        holders: Vec<Holder>,
    },
}

/// Чем кончилось ожидание поднятого сторожа.
enum Started {
    Answered,
    /// Сторож вышел, не ответив: ключ ищется заново.
    Exited,
}

fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

fn unavailable(detail: String) -> KeeperStartError {
    KeeperStartError::Unavailable { detail }
}

impl KeeperClient {
    /// Клиент сторожа `keeper` в каталоге `home` без сессии команды и без учётных данных: для
    /// `status`, `hold`, `release` и `stop`.
    pub fn detached(home: KeeperHome, keeper: u32) -> std::io::Result<Self> {
        Ok(Self {
            channel: home.channel(),
            home,
            digest: String::new(),
            keeper,
            reused: true,
            session: None,
            deferred: None,
            last_deferral: None,
            interrupted: false,
            runtime: runtime()?,
        })
    }

    /// Живой агент ключа `key` с открытой сессией команды: берётся, если его сторож жив, иначе
    /// поднимается новый сторож. `work` — отметка работы команды: поднятый сторож переживает
    /// команду, поэтому его запуск — уже её работа.
    pub fn connect_or_start(
        key: &str,
        launch: &KeeperLaunch,
        credentials: &Credentials,
        limit: u8,
        work: &WorkGiven,
    ) -> Result<Self, KeeperStartError> {
        Self::obtain(key, launch, credentials, limit, work, true)
    }

    /// Живой агент ключа `key` без сессии команды: берётся или поднимается, как у
    /// [`connect_or_start`](Self::connect_or_start), но к базе не подключается. Для `agent hold`.
    pub fn start_or_attach(
        key: &str,
        launch: &KeeperLaunch,
        credentials: &Credentials,
        limit: u8,
        work: &WorkGiven,
    ) -> Result<Self, KeeperStartError> {
        Self::obtain(key, launch, credentials, limit, work, false)
    }

    fn obtain(
        key: &str,
        launch: &KeeperLaunch,
        credentials: &Credentials,
        limit: u8,
        work: &WorkGiven,
        session: bool,
    ) -> Result<Self, KeeperStartError> {
        let state = crate::support::machine::account::account_state_dir()?;
        let home = KeeperHome::open(&state, key)?;
        let digest = credentials_digest(&state, &credentials.user, &credentials.password)?;
        let mut client = Self::detached(home, 0)?;
        client.digest = digest;
        let deadline =
            Instant::now() + Duration::from_millis(launch.startup_timeout_ms) + START_MARGIN;
        let mut respawns = 0;
        let mut evictions = 0;
        loop {
            match client.find(limit)? {
                Found::OverLimit => {
                    // Сверх предела снимается свой простаивающий агент без держателей;
                    // занятого раннер не снимает.
                    if evictions > usize::from(limit) || !evict_idle(client.home.state(), key) {
                        return Err(KeeperStartError::Limit { limit });
                    }
                    evictions += 1;
                    continue;
                }
                Found::Live(keeper) => {
                    client.keeper = keeper;
                    match client.take(session) {
                        Ok(()) => return Ok(client),
                        // Сторож закрывается, его канал уже закрыт или он ещё поднимает агента:
                        // ждём, пока он ответит или уйдёт.
                        Err(KeeperStartError::Agent(
                            AgentError::LiveAgentGone { .. } | AgentError::KeeperUnreachable { .. },
                        )) => {}
                        Err(error) => return Err(error),
                    }
                }
                Found::Starting => {}
                Found::Absent => {
                    client.reused = false;
                    let pid = match client.spawn(launch, credentials, work) {
                        Ok(pid) => pid,
                        Err(error) => {
                            client.forget_start();
                            return Err(error);
                        }
                    };
                    client.keeper = pid;
                    match client.await_start(pid, deadline)? {
                        Started::Answered => match client.take(session) {
                            Ok(()) => return Ok(client),
                            // Поднятого агента сняли, не дав взять (простой, предел): ключ
                            // ищется заново, и при нужде поднимается новый сторож.
                            Err(KeeperStartError::Agent(
                                AgentError::LiveAgentGone { .. }
                                | AgentError::KeeperUnreachable { .. },
                            )) => client.reused = true,
                            Err(error) => return Err(error),
                        },
                        Started::Exited if respawns < RESPAWNS => respawns += 1,
                        Started::Exited => {
                            return Err(unavailable(format!(
                                "the keeper of the live agent exited before answering; see {}",
                                client.home.paths().log().display()
                            )))
                        }
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(unavailable(format!(
                    "the keeper of the live agent did not answer in time; see {}",
                    client.home.paths().log().display()
                )));
            }
            std::thread::sleep(WAIT_STEP);
        }
    }

    /// Живой сторож ключа `key`, если он есть, — без сессии команды и без подъёма нового: для
    /// `status`, `stop` и снятия. Каталогов не создаёт: сторожа, которого не поднимали, нет.
    pub fn attach(state: &Path, key: &str) -> std::io::Result<Option<Self>> {
        let Some(home) = KeeperHome::existing(state, key)? else {
            return Ok(None);
        };
        let machine = machine_fingerprint();
        let observed = {
            let _registry = lock_agent_registry(state)?;
            observe_service(
                home.paths(),
                machine.as_deref(),
                &process_facts,
                &|record| adopt_recorded_pid(record, &process_facts, machine.as_deref()),
            )
        };
        let Observed::Own(record) = observed else {
            return Ok(None);
        };
        let Some(keeper) = record.pid else {
            return Ok(None);
        };
        Self::detached(home, keeper).map(Some)
    }

    /// Взят ли уже живой агент.
    pub fn reused(&self) -> bool {
        self.reused
    }

    /// Ключ живого агента.
    pub fn key(&self) -> &str {
        self.home.key()
    }

    /// Порт агента из записи сторожа.
    pub fn port(&self) -> Option<u16> {
        read_json::<ServiceRecord>(&self.home.paths().record())
            .map(|record| record.port)
            .filter(|port| *port != 0)
    }

    /// `AgentBaseDir` агента.
    pub fn base_dir(&self) -> PathBuf {
        self.home.base_dir()
    }

    /// Найденный живой сторож годится: отвечает, называет ключ хоста из записи, и, если
    /// `session`, сессия команды открыта.
    fn take(&mut self, session: bool) -> Result<(), KeeperStartError> {
        self.check_fingerprint()?;
        if session {
            self.begin()?;
        }
        Ok(())
    }

    /// Ищет сторожа под замком реестра; не нашёл — ставит запись «запускается» за собой, если
    /// живых агентов других ключей меньше `limit`. Подсчёт и запись идут под одним замком,
    /// поэтому одновременные команды предел вместе не превысят.
    fn find(&self, limit: u8) -> Result<Found, KeeperStartError> {
        let _registry = lock_agent_registry(self.home.state())?;
        let paths = self.home.paths();
        let machine = machine_fingerprint();
        let observed = observe_service(paths, machine.as_deref(), &process_facts, &|record| {
            adopt_recorded_pid(record, &process_facts, machine.as_deref())
        });
        match observed {
            Observed::Own(record) => Ok(record.pid.map_or(Found::Starting, Found::Live)),
            Observed::Starting(_) => Ok(Found::Starting),
            Observed::Nothing | Observed::Stale => {
                let others = live_keys(self.home.state())
                    .into_iter()
                    .filter(|(key, _)| key != self.home.key())
                    .count();
                if others >= usize::from(limit) {
                    return Ok(Found::OverLimit);
                }
                let own = std::process::id();
                let started = process_facts(own).map(|facts| facts.started);
                let record = ServiceRecord {
                    state: RecordState::Starting,
                    pid: None,
                    started: None,
                    image: None,
                    machine,
                    starter: started.map(|started| (own, started)),
                    port: 0,
                    explicit: false,
                    bin: PathBuf::new(),
                    version: Some(env!("CARGO_PKG_VERSION").to_owned()),
                    agent: None,
                };
                write_json(&paths.record(), &record)?;
                Ok(Found::Absent)
            }
        }
    }

    /// Снимает свою запись «запускается», когда сторож так и не поднят.
    fn forget_start(&self) {
        let Ok(_registry) = lock_agent_registry(self.home.state()) else {
            return;
        };
        let path = self.home.paths().record();
        let own = std::process::id();
        if read_json::<ServiceRecord>(&path).is_some_and(|record| {
            record.state == RecordState::Starting
                && record.starter.is_some_and(|(pid, _)| pid == own)
        }) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Поднимает сторожа отделённым процессом из копии исполняемого файла раннера.
    fn spawn(
        &self,
        launch: &KeeperLaunch,
        credentials: &Credentials,
        work: &WorkGiven,
    ) -> Result<u32, KeeperStartError> {
        let program = keeper_exe(self.home.state())
            .map_err(|error| unavailable(format!("no copy of the runner for the keeper: {error}")))?;
        let start = KeeperStart {
            launch: launch.clone(),
            credentials: credentials.clone(),
        };
        let request = ServiceRequest {
            program,
            args: vec![
                "agent".to_owned(),
                "keeper".to_owned(),
                "--key".to_owned(),
                self.home.key().to_owned(),
            ],
            workdir: Some(self.home.paths().root().to_path_buf()),
            log_path: self.home.paths().log(),
            stdin: Some(serde_json::to_string(&start).map_err(std::io::Error::other)?),
        };
        ProcessExecutor
            .spawn_service(&request, work)
            .map_err(|error| match error {
                ServiceSpawnError::WouldNotSurvive => KeeperStartError::Job,
                other => unavailable(format!("the keeper did not start: {other}")),
            })
    }

    /// Ждёт, пока поднятый сторож поднимет агента и ответит.
    fn await_start(&self, pid: u32, deadline: Instant) -> Result<Started, KeeperStartError> {
        loop {
            match self.status() {
                Ok(_) => return Ok(Started::Answered),
                // Канала ещё нет или агент ещё поднимается: сторож запускается.
                Err(AgentError::KeeperUnreachable { .. }) => {}
                Err(error) => return Err(error.into()),
            }
            if process_facts(pid).is_none() {
                self.forget_start();
                return Ok(Started::Exited);
            }
            if Instant::now() >= deadline {
                self.forget_start();
                return Err(unavailable(format!(
                    "the keeper of the live agent did not answer in time; see {}",
                    self.home.paths().log().display()
                )));
            }
            std::thread::sleep(WAIT_STEP);
        }
    }

    /// Отпечаток ключа хоста, который назвал сторож, — тот же, что в его записи
    /// (`INV.PLATFORM.A-LIVE-AGENT-IS-TOLD-APART-BY-ITS-RECORD`).
    fn check_fingerprint(&self) -> Result<(), AgentError> {
        let status = self.status()?;
        let recorded = read_json::<ServiceRecord>(&self.home.paths().record())
            .and_then(|record| record.agent)
            .and_then(|agent| agent.fingerprint);
        if recorded.is_some() && recorded != status.fingerprint {
            return Err(AgentError::Keeper {
                detail: format!(
                    "the live agent answers with host key {:?}, its record names {:?}",
                    status.fingerprint, recorded
                ),
            });
        }
        Ok(())
    }

    /// Открывает сессию команды: сторож подключает агента к базе.
    fn begin(&mut self) -> Result<(), KeeperStartError> {
        let request = Request::Begin {
            credentials: self.digest.clone(),
        };
        let (channel, keeper) = (self.channel.clone(), self.keeper);
        let opened = self.runtime.block_on(async {
            let stream = connect_within(&channel, keeper, QUERY_PATIENCE).await?;
            let mut stream = BufReader::new(stream);
            write_line(stream.get_mut(), &request)
                .await
                .map_err(|source| unreachable_at(&channel, source))?;
            match read_line::<_, Frame>(&mut stream).await {
                Ok(Some(Frame::Begun { session })) => Ok(OpenSession {
                    id: session,
                    stream,
                }),
                Ok(Some(Frame::Error {
                    error: KeeperError::Busy { command },
                })) => Err(KeeperStartError::Busy { command }),
                Ok(Some(Frame::Error { error })) => Err(error.into_agent().into()),
                Ok(Some(other)) => Err(unexpected(&other).into()),
                Ok(None) | Err(_) => Err(AgentError::LiveAgentGone {
                    detail: "the keeper closed the channel before the session began".to_owned(),
                }
                .into()),
            }
        })?;
        self.session = Some(opened);
        Ok(())
    }

    /// Закрывает сессию команды: сторож отключает агента от базы, а сам агент остаётся жить,
    /// если сторож не решил его снять. Ответ: останется ли агент жить, по `status` после
    /// `end`; сторож не ответил — `false`.
    pub fn end(&mut self) -> Option<KeeperStatus> {
        if let Some(mut session) = self.session.take() {
            self.runtime.block_on(async {
                let _ = tokio::time::timeout(END_PATIENCE, async {
                    write_line(session.stream.get_mut(), &Request::End).await?;
                    read_line::<_, Frame>(&mut session.stream).await
                })
                .await;
            });
        }
        self.status().ok()
    }

    /// Команда агенту через сторожа в открытой сессии. Кадры ответа отдаются `on_batch` по мере
    /// прихода. Отмена или истёкший срок некритической команды пишут сторожу `cancel` и уходят —
    /// сторож бросает команду и снимает агента; критическую они не прерывают: её доводят до
    /// исхода, а прерывание записывается отложенным.
    pub fn run(
        &mut self,
        command: &str,
        policy: &WaitPolicy,
        on_batch: &mut dyn FnMut(&[AgentMessage]),
    ) -> Result<AgentReply, AgentError> {
        self.last_deferral = None;
        let outcome = self.exchange_run(command, policy, on_batch);
        // Критическую команду ни отмена, ни срок не прерывают: сторож доводит её до исхода, а
        // команда называет отложенное прерывание, как с разовым агентом.
        if policy.is_critical() {
            if policy.cancellation.is_cancelled() {
                self.defer(ProcessInterruptionReason::Cancelled);
            }
            if policy
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                self.defer(ProcessInterruptionReason::TimedOut);
            }
        }
        if outcome.as_ref().is_err_and(|error| {
            matches!(
                error,
                AgentError::Cancelled {
                    delivered: true,
                    ..
                } | AgentError::SessionClosed { .. }
                    | AgentError::InvalidReply { .. }
                    | AgentError::TimedOut { .. }
                    | AgentError::LiveAgentGone { .. }
                    | AgentError::KeeperUnreachable { .. }
            )
        }) {
            self.interrupted = true;
        }
        outcome
    }

    fn defer(&mut self, reason: ProcessInterruptionReason) {
        self.last_deferral.get_or_insert(reason);
        self.deferred.get_or_insert(reason);
    }

    /// Прерывание, отложенное в критической фазе за всю сессию.
    pub fn deferred_interruption(&self) -> Option<ProcessInterruptionReason> {
        self.deferred
    }

    /// Прерывание, которое отложила последняя команда.
    pub fn last_command_deferral(&self) -> Option<ProcessInterruptionReason> {
        self.last_deferral
    }

    /// Прервана ли команда посреди запроса агенту: тогда сторож снимает агента.
    pub fn interrupted(&self) -> bool {
        self.interrupted
    }

    fn exchange_run(
        &self,
        command: &str,
        policy: &WaitPolicy,
        on_batch: &mut dyn FnMut(&[AgentMessage]),
    ) -> Result<AgentReply, AgentError> {
        if policy.cancellation.is_cancelled() {
            return Err(AgentError::NotSent {
                command: command.to_owned(),
            });
        }
        let Some(session) = &self.session else {
            return Err(AgentError::LiveAgentGone {
                detail: "the command session with the live agent is closed".to_owned(),
            });
        };
        let request = Request::Run {
            session: session.id.clone(),
            command: command.to_owned(),
            critical: policy.is_critical(),
            credentials: self.digest.clone(),
        };
        let critical = policy.is_critical();
        // Срок прерывает только некритическую команду; критической он записывается отложенным
        // после ответа (`run`).
        let deadline = policy
            .deadline
            .filter(|_| !critical)
            .map(tokio::time::Instant::from_std);
        let started = Instant::now();
        self.runtime.block_on(async {
            let stream = connect_within(&self.channel, self.keeper, QUERY_PATIENCE).await?;
            let (read, mut write) = tokio::io::split(stream);
            let mut reader = BufReader::new(read);
            write_line(&mut write, &request)
                .await
                .map_err(|source| unreachable_at(&self.channel, source))?;
            policy.mark_work_given();
            let mut messages = Vec::new();
            let expired = async {
                match deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            };
            tokio::pin!(expired);
            loop {
                let frame = tokio::select! {
                    frame = read_line::<_, Frame>(&mut reader) => frame,
                    () = policy.cancellation.cancelled(), if !critical => {
                        // Строка `cancel` доходит до сторожа и тогда, когда разрыв канала
                        // он увидел бы не сразу.
                        let _ = write_line(&mut write, &Request::Cancel).await;
                        return Err(AgentError::Cancelled {
                            command: command.to_owned(),
                            delivered: true,
                        });
                    }
                    () = &mut expired => {
                        let _ = write_line(&mut write, &Request::Cancel).await;
                        return Err(AgentError::TimedOut {
                            command: command.to_owned(),
                            timeout_ms: u64::try_from(started.elapsed().as_millis())
                                .unwrap_or(u64::MAX),
                        });
                    }
                };
                match frame {
                    Ok(Some(Frame::Batch { messages: batch })) => {
                        on_batch(&batch);
                        messages.extend(batch);
                    }
                    Ok(Some(Frame::Done)) => return Ok(AgentReply { messages }),
                    Ok(Some(Frame::Error { error })) => return Err(error.into_agent()),
                    Ok(Some(other)) => return Err(unexpected(&other)),
                    Ok(None) | Err(_) => {
                        return Err(AgentError::LiveAgentGone {
                            detail: "the keeper closed the channel before the reply".to_owned(),
                        })
                    }
                }
            }
        })
    }

    pub fn status(&self) -> Result<KeeperStatus, AgentError> {
        match self.exchange(&Request::Status, QUERY_PATIENCE)? {
            Frame::Status { status } => Ok(status),
            other => Err(unexpected(&other)),
        }
    }

    /// Ставит держателя; ответ — держатели после запроса.
    pub fn hold(&self, holder: Holder) -> Result<Vec<Holder>, AgentError> {
        match self.exchange(&Request::Hold { holder }, QUERY_PATIENCE)? {
            Frame::Holders { holders } => Ok(holders),
            other => Err(unexpected(&other)),
        }
    }

    /// Снимает держателя; ответ — держатели после запроса.
    pub fn release(&self, holder: Holder) -> Result<Vec<Holder>, AgentError> {
        match self.exchange(&Request::Release { holder }, QUERY_PATIENCE)? {
            Frame::Holders { holders } => Ok(holders),
            other => Err(unexpected(&other)),
        }
    }

    /// Сторож отпускает и снимает агента и выходит; ответ приходит, когда агент снят.
    pub fn stop(&self) -> Result<(), AgentError> {
        match self.stop_if(false, false)? {
            Stopped::Removed => Ok(()),
            Stopped::InUse { .. } => Err(AgentError::Keeper {
                detail: "the keeper refused an unconditional stop".to_owned(),
            }),
        }
    }

    /// `stop` с условием, которое сторож проверяет под своим замком: `only_idle` — агент не
    /// выполняет команду и сессии нет, `only_unheld` — держателей нет. Сторож, ушедший сам, —
    /// тоже снятый агент.
    pub fn stop_if(&self, only_idle: bool, only_unheld: bool) -> Result<Stopped, AgentError> {
        let request = Request::Stop {
            only_idle,
            only_unheld,
        };
        match self.exchange_frame(&request, STOP_PATIENCE) {
            Ok(Frame::Stopped) => Ok(Stopped::Removed),
            Ok(Frame::Error {
                error: KeeperError::NotIdle { command, holders },
            }) => Ok(Stopped::InUse { command, holders }),
            Ok(Frame::Error { error }) => Err(error.into_agent()),
            Ok(other) => Err(unexpected(&other)),
            Err(AgentError::LiveAgentGone { .. }) => Ok(Stopped::Removed),
            Err(AgentError::KeeperUnreachable { source, .. })
                if source.kind() != std::io::ErrorKind::TimedOut =>
            {
                Ok(Stopped::Removed)
            }
            Err(error) => Err(error),
        }
    }

    /// Запрос с одним итоговым кадром; отказ сторожа — отказом агентского пути.
    fn exchange(&self, request: &Request, patience: Duration) -> Result<Frame, AgentError> {
        match self.exchange_frame(request, patience)? {
            Frame::Error { error } => Err(error.into_agent()),
            frame => Ok(frame),
        }
    }

    /// Запрос с одним итоговым кадром, как он пришёл. Подключение ждёт не дольше
    /// [`QUERY_PATIENCE`], ответ — не дольше `patience`: зависший сторож — отказ
    /// [`AgentError::KeeperUnreachable`] с родом `TimedOut`.
    fn exchange_frame(&self, request: &Request, patience: Duration) -> Result<Frame, AgentError> {
        self.runtime.block_on(async {
            let stream = connect_within(&self.channel, self.keeper, QUERY_PATIENCE).await?;
            let (read, mut write) = tokio::io::split(stream);
            let mut reader = BufReader::new(read);
            write_line(&mut write, request)
                .await
                .map_err(|source| unreachable_at(&self.channel, source))?;
            match tokio::time::timeout(patience, read_line::<_, Frame>(&mut reader)).await {
                Err(_) => Err(timed_out(&self.channel, patience)),
                Ok(Ok(Some(frame))) => Ok(frame),
                Ok(Ok(None) | Err(_)) => Err(AgentError::LiveAgentGone {
                    detail: "the keeper closed the channel before the answer".to_owned(),
                }),
            }
        })
    }
}

/// Подключение к каналу сторожа не дольше `patience`.
async fn connect_within(
    channel: &str,
    keeper: u32,
    patience: Duration,
) -> Result<Stream, AgentError> {
    match tokio::time::timeout(patience, connect(channel, keeper)).await {
        Ok(Ok(stream)) => Ok(stream),
        Ok(Err(source)) => Err(unreachable_at(channel, source)),
        Err(_) => Err(timed_out(channel, patience)),
    }
}

fn timed_out(channel: &str, patience: Duration) -> AgentError {
    unreachable_at(
        channel,
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("the keeper did not answer within {} s", patience.as_secs()),
        ),
    )
}

/// Ключи живых сторожей учётной записи и их записи: свой процесс по записи или запуск, который
/// ведёт живой раннер. Читается под замком реестра.
fn live_keys(state: &Path) -> Vec<(String, ServiceRecord)> {
    let machine = machine_fingerprint();
    let Ok(dirs) = std::fs::read_dir(state.join("agents")) else {
        return Vec::new();
    };
    dirs.flatten()
        .filter(|dir| dir.path().is_dir())
        .filter_map(|dir| {
            let key = dir.file_name().to_str()?.to_owned();
            let paths =
                crate::platform::service_record::ServicePaths::under(&dir.path(), super::SERVICE);
            match observe_service(&paths, machine.as_deref(), &process_facts, &|record| {
                adopt_recorded_pid(record, &process_facts, machine.as_deref())
            }) {
                Observed::Own(record) | Observed::Starting(record) => Some((key, record)),
                Observed::Nothing | Observed::Stale => None,
            }
        })
        .collect()
}

/// Живые сторожа учётной записи с каналом, которому можно задать вопрос, и их записи.
pub fn live_keepers(state: &Path) -> Vec<(LiveKeeper, ServiceRecord)> {
    let found = match lock_agent_registry(state) {
        Ok(_registry) => live_keys(state),
        Err(error) => {
            tracing::warn!(%error, "the live agent registry cannot be read");
            return Vec::new();
        }
    };
    found
        .into_iter()
        .filter_map(|(key, record)| {
            let keeper = record.pid?;
            let home = KeeperHome::existing(state, &key).ok()??;
            let client = KeeperClient::detached(home, keeper).ok()?;
            Some((LiveKeeper { key, client }, record))
        })
        .collect()
}

/// Живые агенты базы `infobase` (путь в каноническом виде) с их состоянием. Состояние
/// спрашивается под замком реестра: пока его держат, новый агент не поднимается. Сторож, который
/// не ответил за [`QUERY_PATIENCE`], идёт с отказом.
pub fn keepers_of(
    state: &Path,
    infobase: &str,
) -> Vec<(LiveKeeper, Result<KeeperStatus, AgentError>)> {
    let _registry = match lock_agent_registry(state) {
        Ok(registry) => registry,
        Err(error) => {
            tracing::warn!(%error, "the live agent registry cannot be read");
            return Vec::new();
        }
    };
    live_keys(state)
        .into_iter()
        .filter_map(|(key, record)| {
            record.agent.filter(|agent| agent.infobase == infobase)?;
            let keeper = record.pid?;
            let home = KeeperHome::existing(state, &key).ok()??;
            let client = KeeperClient::detached(home, keeper).ok()?;
            let status = client.status();
            Some((LiveKeeper { key, client }, status))
        })
        .collect()
}

/// Снимает живого агента ключа `key`, если он есть; ответ — был ли он.
pub fn stop_keeper(state: &Path, key: &str) -> Result<bool, AgentError> {
    match KeeperClient::attach(state, key) {
        Ok(Some(keeper)) => keeper.stop().map(|()| true),
        Ok(None) => Ok(false),
        Err(error) => Err(AgentError::Keeper {
            detail: error.to_string(),
        }),
    }
}

/// Снимает простаивающего дольше всех живого агента без держателей, кроме своего ключа `own`;
/// ответ — снят ли кто-то. Условие «простаивает и без держателей» сторож проверяет сам под своим
/// замком (`stop` с `only_idle` и `only_unheld`): команда, открывшая сессию между вопросом и
/// снятием, агента сохраняет.
fn evict_idle(state: &Path, own: &str) -> bool {
    let mut idle: Vec<(u64, LiveKeeper)> = live_keepers(state)
        .into_iter()
        .map(|(keeper, _)| keeper)
        .filter(|keeper| keeper.key != own)
        .filter_map(|keeper| match keeper.client.status() {
            Ok(status) => (!status.holds_the_infobase() && status.holders.is_empty())
                .then_some((status.idle_ms, keeper)),
            Err(error) => {
                tracing::warn!(key = %keeper.key, %error, "a live agent did not answer its status");
                None
            }
        })
        .collect();
    idle.sort_by_key(|(idle_ms, _)| std::cmp::Reverse(*idle_ms));
    for (_, keeper) in idle {
        match keeper.client.stop_if(true, true) {
            Ok(Stopped::Removed) => {
                tracing::info!(key = %keeper.key, "an idle live agent is removed to stay within the limit");
                return true;
            }
            Ok(Stopped::InUse { .. }) => {
                tracing::debug!(key = %keeper.key, "the live agent got busy before it was removed");
            }
            Err(error) => {
                tracing::warn!(key = %keeper.key, %error, "an idle live agent was not removed");
            }
        }
    }
    false
}

fn unreachable_at(channel: &str, source: std::io::Error) -> AgentError {
    AgentError::KeeperUnreachable {
        channel: channel.to_owned(),
        source,
    }
}

fn unexpected(frame: &Frame) -> AgentError {
    AgentError::Keeper {
        detail: format!("the keeper gave an unexpected answer: {frame:?}"),
    }
}

/// Сколько живёт копия раннера, которую никто не держит, и недописанная копия: моложе их не
/// трогают — их может прямо сейчас класть другой клиент.
const STALE_COPY: Duration = Duration::from_secs(10 * 60);

/// Копия исполняемого файла раннера, из которой запускается сторож:
/// `<state>/bin/<версия>/<метка>/<имя>`. Метка — от размера и времени изменения файла, поэтому
/// пересобранный раннер той же версии получает свою копию, а работающий сторож держит свою.
/// Прежние копии, из которых не работает ни один сторож, и брошенные недописанные копии
/// убираются.
fn keeper_exe(state: &Path) -> std::io::Result<PathBuf> {
    let source = std::env::current_exe()?;
    let name = source
        .file_name()
        .ok_or_else(|| std::io::Error::other("the runner executable has no file name"))?
        .to_owned();
    let metadata = std::fs::metadata(&source)?;
    let modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let stamp = {
        use sha2::{Digest, Sha256};
        let digest = Sha256::new()
            .chain_update(metadata.len().to_be_bytes())
            .chain_update(modified.to_be_bytes())
            .finalize();
        digest[..6]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let dir = state
        .join("bin")
        .join(env!("CARGO_PKG_VERSION"))
        .join(stamp);
    let target = dir.join(&name);
    sweep_copies(state, &dir);
    if target.is_file() {
        return Ok(target);
    }
    std::fs::create_dir_all(&dir)?;
    let staged = dir.join(format!(
        ".{}-{}.tmp",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::copy(&source, &staged)?;
    match std::fs::rename(&staged, &target) {
        Ok(()) => Ok(target),
        // Другой клиент успел положить ту же копию.
        Err(_) if target.is_file() => {
            let _ = std::fs::remove_file(&staged);
            Ok(target)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&staged);
            Err(error)
        }
    }
}

/// Убирает копии раннера под `<state>/bin`, кроме `current` и тех, из которых работает живой
/// сторож, и недописанные копии старше [`STALE_COPY`]. Отказы не мешают: копию, которую держит
/// процесс, Windows удалить не даст, и её уберёт следующий раз.
fn sweep_copies(state: &Path, current: &Path) {
    let in_use = live_keeper_bins(state);
    let old = |path: &Path| {
        std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age >= STALE_COPY)
    };
    let Ok(versions) = std::fs::read_dir(state.join("bin")) else {
        return;
    };
    for version in versions.flatten() {
        let Ok(stamps) = std::fs::read_dir(version.path()) else {
            continue;
        };
        for stamp in stamps.flatten() {
            let path = stamp.path();
            if path == current {
                if let Ok(files) = std::fs::read_dir(&path) {
                    for file in files.flatten() {
                        let file = file.path();
                        if file.extension().is_some_and(|extension| extension == "tmp")
                            && old(&file)
                        {
                            let _ = std::fs::remove_file(file);
                        }
                    }
                }
                continue;
            }
            let files_old = std::fs::read_dir(&path)
                .is_ok_and(|files| files.flatten().all(|file| old(&file.path())));
            if in_use.iter().any(|bin| bin.starts_with(&path)) || !files_old {
                continue;
            }
            let _ = std::fs::remove_dir_all(&path);
        }
        let _ = std::fs::remove_dir(version.path());
    }
}

/// Исполняемые файлы, из которых работают живые сторожа учётной записи.
fn live_keeper_bins(state: &Path) -> Vec<PathBuf> {
    let machine = machine_fingerprint();
    let Ok(keys) = std::fs::read_dir(state.join("agents")) else {
        return Vec::new();
    };
    keys.flatten()
        .filter_map(|key| {
            let paths =
                crate::platform::service_record::ServicePaths::under(&key.path(), super::SERVICE);
            match observe_service(&paths, machine.as_deref(), &process_facts, &|record| {
                adopt_recorded_pid(record, &process_facts, machine.as_deref())
            }) {
                Observed::Own(record) => Some(record.bin),
                Observed::Starting(_) | Observed::Nothing | Observed::Stale => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Сторож запускается из своей копии раннера, а не из файла, который обновляют
    /// (`INV.PLATFORM.A-LIVE-AGENT-DIES-WITH-ITS-KEEPER`); вторая копия той же сборки не
    /// делается, а брошенные копии убираются.
    #[test]
    fn the_keeper_runs_from_its_own_copy_of_the_runner() {
        let state = tempfile::tempdir().unwrap();
        let stale = state.path().join("bin").join("0.0.1").join("abcdef");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("v8-runner"), "old").unwrap();
        let old = SystemTime::now() - STALE_COPY - Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(stale.join("v8-runner"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let first = keeper_exe(state.path()).unwrap();
        let second = keeper_exe(state.path()).unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with(state.path().join("bin").join(env!("CARGO_PKG_VERSION"))));
        assert_eq!(
            std::fs::metadata(&first).unwrap().len(),
            std::fs::metadata(std::env::current_exe().unwrap())
                .unwrap()
                .len()
        );
        assert_eq!(
            std::fs::read_dir(first.parent().unwrap()).unwrap().count(),
            1
        );
        assert!(!stale.exists(), "a copy no keeper runs from is swept");
    }

    /// Что делает поддельный сторож после первого кадра ответа.
    #[derive(Clone, Copy)]
    enum After {
        /// Ждёт, что клиент напишет или уйдёт, и отдаёт прочитанное.
        Probe,
        /// Через `ms` отвечает `done`.
        DoneAfter(u64),
    }

    /// Поддельный сторож на одно подключение `run`: отвечает кадром `batch`, затем `after`.
    /// Ответ — клиент с открытой сессией и приёмник: запрос и то, что клиент написал после.
    fn fake_keeper(
        after: After,
    ) -> (
        KeeperClient,
        std::sync::mpsc::Receiver<(Request, Option<String>)>,
        std::sync::mpsc::Sender<()>,
        tempfile::TempDir,
    ) {
        use tokio::io::AsyncReadExt;

        // Путь сокета короток: временный каталог macOS для него слишком длинен.
        let state = if cfg!(unix) {
            tempfile::tempdir_in("/tmp").unwrap()
        } else {
            tempfile::tempdir().unwrap()
        };
        let home = KeeperHome::open(state.path(), &"ab".repeat(16)).unwrap();
        let channel = if cfg!(windows) {
            format!(
                r"\\.\pipe\v8-runner-agent-test-{}",
                uuid::Uuid::new_v4().simple()
            )
        } else {
            home.channel()
        };
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        let server_channel = channel.clone();
        let (bound_tx, bound_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let mut listener = super::super::channel::Listener::bind(&server_channel).unwrap();
                bound_tx.send(()).unwrap();
                let stream = listener.accept().await.unwrap();
                let (read, mut write) = tokio::io::split(stream);
                let mut reader = BufReader::new(read);
                let request: Request = read_line(&mut reader).await.unwrap().unwrap();
                let batch: Vec<AgentMessage> = serde_json::from_str(
                    r#"[{"type":"progress","body":{"message":"dump","percent":10}}]"#,
                )
                .unwrap();
                write_line(&mut write, &Frame::Batch { messages: batch })
                    .await
                    .unwrap();
                let written = match after {
                    After::Probe => {
                        let mut probe = [0u8; 64];
                        tokio::time::timeout(Duration::from_secs(5), reader.read(&mut probe))
                            .await
                            .ok()
                            .map(|read| {
                                String::from_utf8_lossy(&probe[..read.unwrap_or(0)]).into_owned()
                            })
                    }
                    After::DoneAfter(ms) => {
                        tokio::time::sleep(Duration::from_millis(ms)).await;
                        write_line(&mut write, &Frame::Done).await.unwrap();
                        None
                    }
                };
                seen_tx.send((request, written)).unwrap();
                // Клиент ещё жив: сторож видит строку `cancel`, а не выход процесса.
                let _ = done_rx.recv_timeout(Duration::from_secs(10));
                listener.remove();
            });
        });
        bound_rx.recv().unwrap();
        let mut client = KeeperClient::detached(home, std::process::id()).unwrap();
        client.channel = channel;
        client.digest = "digest".to_owned();
        client.session = Some(OpenSession {
            id: "session".to_owned(),
            stream: BufReader::new(Box::new(tokio::io::duplex(64).0)),
        });
        (client, seen_rx, done_tx, state)
    }

    /// Клиент отдаёт кадры по мере прихода, а отменённая некритическая команда пишет сторожу
    /// `cancel`, и сторож видит уход клиента, пока тот ещё жив
    /// (`INV.PLATFORM.AN-INTERRUPTED-COMMAND-REMOVES-THE-LIVE-AGENT`).
    #[test]
    fn a_cancelled_run_leaves_the_channel_and_the_keeper_sees_it() {
        let (mut client, seen, done, _state) = fake_keeper(After::Probe);
        let cancellation = tokio_util::sync::CancellationToken::new();
        let policy = WaitPolicy::for_keeper(cancellation.clone(), false);
        let mut batches = 0;
        let outcome = client.run("config dump-config-to-files --dir d", &policy, &mut |_| {
            batches += 1;
            cancellation.cancel();
        });
        assert!(
            matches!(
                outcome,
                Err(AgentError::Cancelled {
                    delivered: true,
                    ..
                })
            ),
            "{outcome:?}"
        );
        assert_eq!(batches, 1);
        let (request, left) = seen.recv_timeout(Duration::from_secs(10)).unwrap();
        let _ = done.send(());
        assert!(
            matches!(request, Request::Run { critical: false, ref credentials, ref session, .. }
                if credentials == "digest" && session == "session"),
            "{request:?}"
        );
        assert!(
            left.is_some(),
            "the keeper sees the client leave while the client still lives"
        );
        assert!(client.interrupted());
    }

    /// Срок шага у живого агента — тот же, что у разового: некритическую команду он прерывает
    /// строкой `cancel` и отказом `TimedOut`, и сторож снимает агента
    /// (`INV.PLATFORM.AN-INTERRUPTED-COMMAND-REMOVES-THE-LIVE-AGENT`).
    #[test]
    fn a_run_past_its_deadline_is_cancelled() {
        let (mut client, seen, done, _state) = fake_keeper(After::Probe);
        let mut policy = WaitPolicy::for_keeper(tokio_util::sync::CancellationToken::new(), false);
        policy.deadline = Some(Instant::now() + Duration::from_millis(300));
        let outcome = client.run("config dump-config-to-files --dir d", &policy, &mut |_| {});
        assert!(
            matches!(outcome, Err(AgentError::TimedOut { .. })),
            "{outcome:?}"
        );
        let (_, written) = seen.recv_timeout(Duration::from_secs(10)).unwrap();
        let _ = done.send(());
        assert!(
            written.as_deref().is_some_and(|line| line.contains("cancel")),
            "the keeper gets `cancel`: {written:?}"
        );
        assert!(client.interrupted(), "the timed-out agent is removed");
        assert_eq!(client.last_command_deferral(), None);
    }

    /// Критическую команду срок не прерывает: её доводят до исхода, а истёкший срок команда
    /// называет отложенным прерыванием, как с разовым агентом.
    #[test]
    fn a_critical_run_past_its_deadline_is_finished_and_the_deadline_is_deferred() {
        let (mut client, seen, done, _state) = fake_keeper(After::DoneAfter(800));
        let mut policy = WaitPolicy::for_keeper(tokio_util::sync::CancellationToken::new(), true);
        policy.deadline = Some(Instant::now() + Duration::from_millis(100));
        let outcome = client.run("config update-db-cfg", &policy, &mut |_| {});
        let _ = seen.recv_timeout(Duration::from_secs(10)).unwrap();
        let _ = done.send(());
        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(
            client.last_command_deferral(),
            Some(ProcessInterruptionReason::TimedOut)
        );
        assert_eq!(
            client.deferred_interruption(),
            Some(ProcessInterruptionReason::TimedOut)
        );
        assert!(!client.interrupted(), "the finished command keeps the agent");
    }

    #[test]
    fn credentials_never_show_in_debug_output() {
        let start = KeeperStart {
            launch: KeeperLaunch {
                v8: PathBuf::from("1cv8"),
                infobase_args: Vec::new(),
                port: None,
                host_key: None,
                startup_timeout_ms: 1,
                infobase: String::new(),
            },
            credentials: Credentials {
                user: "Администратор".to_owned(),
                password: "secret".to_owned(),
            },
        };
        let shown = format!("{start:?}");
        assert!(
            !shown.contains("secret") && !shown.contains("Администратор"),
            "{shown}"
        );
    }
}
