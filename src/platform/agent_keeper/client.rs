//! Клиент сторожа: находит живого агента ключа по записи или поднимает сторожа, открывает
//! сессию команды (`begin` … `end`) и говорит с ним по каналу. Замок реестра учётной записи
//! берётся только на поиск и на запись «запускается», а не на все 6–9 с подъёма агента.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::BufReader;
use tokio_util::sync::CancellationToken;

use super::channel::{connect, Stream};
use super::protocol::{read_line, write_line, Frame, Holder, KeeperError, KeeperStatus, Request};
use super::{credentials_digest, KeeperHome};
use crate::platform::agent::{AgentError, AgentMessage, AgentReply, WaitPolicy};
use crate::platform::live_registry::lock_agent_registry;
use crate::platform::process::ProcessInterruptionReason;
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
    /// `workPath` рабочей копии: он идёт в запись агента, и его называет ожидание на пределе.
    #[serde(default)]
    pub work_path: Option<PathBuf>,
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
    /// `held` — удерживаемые агенты, когда удерживаются все; у команды, которая не ждёт, может
    /// быть пуст.
    #[error("the OS account already keeps {limit} live agents, and none of them is idle")]
    Limit { limit: u8, held: Vec<HeldAgent> },
    /// Живой агент этого ключа ещё служит сессии другой команды.
    #[error("the live agent still serves another command{}", .command.as_deref().map(|command| format!(": {command}")).unwrap_or_default())]
    Busy { command: Option<String> },
    /// Сторож не поднялся или не ответил: путь канала, отказ запуска, срок. `source` — отказ
    /// ОС или запуска, если он был.
    #[error("{detail}")]
    Unavailable {
        detail: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
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
/// Сколько клиент ждёт ухода процесса сторожа, оборвавшего канал, прежде чем считать сторожа
/// живым.
const GONE_PATIENCE: Duration = Duration::from_secs(3);

/// Открытая сессия команды: её подключение держится до `end`.
struct OpenSession {
    id: String,
    stream: BufReader<Stream>,
    /// Ответ агента на подключение к базе, ещё не записанный в журнал команды.
    connected: Vec<AgentMessage>,
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

/// Чем кончился безусловный `stop`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    /// Агент снят: сторож ответил или подтверждённо ушёл.
    Done,
    /// Сторож принял `stop`, но не ответил за срок или ожидание отменили: агент снимется после
    /// команды, которую выполняет.
    Pending,
}

/// Чем кончился `stop` с условием.
#[derive(Debug)]
pub enum Stopped {
    /// Агент снят: сторож ответил или подтверждённо ушёл.
    Removed,
    /// Сторож принял `stop`, но не ответил за срок или команду отменили, пока ждали ответа:
    /// агент снимется, как только кончится команда, которую он выполняет.
    Pending,
    /// Агент занят или его держат: сторож его не снял.
    InUse {
        command: Option<String>,
        holders: Vec<Holder>,
    },
}

/// Живой агент на пределе, которого держат, — для отказа `agent_limit`.
#[derive(Debug, Clone)]
pub struct HeldAgent {
    pub key: String,
    pub infobase: Option<String>,
    pub work_path: Option<String>,
    pub holders: Vec<Holder>,
    /// Сторож не отвечает дольше [`SILENCE_LIMIT`]: держит ли кто агента, неизвестно, и ждать
    /// его не стоит.
    pub answering: bool,
}

/// Что сделал или увидел `make_room` на пределе.
enum Room {
    /// Простаивающий агент без держателей снят: место есть.
    Freed,
    /// Снять некого, но агент освободится сам: он выполняет команду без держателей, свежий
    /// или закрывается. Строка называет, чего ждут.
    Wait(String),
    /// Все агенты на пределе удерживаются.
    Held(Vec<HeldAgent>),
}

/// Шаг ожидания места на пределе: каждый шаг спрашивает сторожей учётной записи.
const ROOM_STEP: Duration = Duration::from_millis(500);
/// Сколько сторож может молчать подряд, пока его ещё ждут на пределе: дольше — его агент
/// считается неизвестным и ожидания не продлевает.
const SILENCE_LIMIT: Duration = Duration::from_secs(3 * 5);

/// Что сделать со взятым живым агентом.
enum Take<'a> {
    /// Открыть сессию команды.
    Session,
    /// Поставить держателя.
    Hold(&'a Holder),
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
    KeeperStartError::Unavailable {
        detail,
        source: None,
    }
}

fn unavailable_by(
    detail: &str,
    source: impl std::error::Error + Send + Sync + 'static,
) -> KeeperStartError {
    KeeperStartError::Unavailable {
        detail: detail.to_owned(),
        source: Some(Box::new(source)),
    }
}

/// Отмена команды, пока она ждала сторожа: агенту ничего не отправлено.
fn cancelled_waiting() -> KeeperStartError {
    KeeperStartError::Agent(AgentError::NotSent {
        command: "start of the live agent".to_owned(),
    })
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
    /// поднимается новый сторож. Подъём сторожа и агента работой команды не считается. Отмена
    /// `cancel` прекращает ожидание сторожа.
    pub fn connect_or_start(
        key: &str,
        launch: &KeeperLaunch,
        credentials: &Credentials,
        limit: u8,
        cancel: &CancellationToken,
    ) -> Result<Self, KeeperStartError> {
        let mut client = Self::prepare(key, credentials)?;
        client.obtain(
            launch,
            credentials,
            limit,
            cancel,
            &Take::Session,
            &mut |_| {},
        )?;
        Ok(client)
    }

    /// Живой агент ключа `key` с держателем `holder`: берётся или поднимается, как у
    /// [`connect_or_start`](Self::connect_or_start), но к базе не подключается. Отметка ставится,
    /// как только агент взят, а агент, которого снимают, ожидается до ухода и поднимается заново:
    /// свежего агента ради предела не снимут, пока отметки нет. Ответ — клиент и держатели после
    /// отметки. Для `agent hold`.
    pub fn start_and_hold(
        key: &str,
        launch: &KeeperLaunch,
        credentials: &Credentials,
        limit: u8,
        cancel: &CancellationToken,
        holder: &Holder,
        on_wait: &mut dyn FnMut(&str),
    ) -> Result<(Self, Vec<Holder>), KeeperStartError> {
        let mut client = Self::prepare(key, credentials)?;
        let holders = client.obtain(
            launch,
            credentials,
            limit,
            cancel,
            &Take::Hold(holder),
            on_wait,
        )?;
        Ok((client, holders))
    }

    fn prepare(key: &str, credentials: &Credentials) -> Result<Self, KeeperStartError> {
        let state = crate::support::machine::account::account_state_dir()?;
        let home = KeeperHome::open(&state, key)?;
        let digest = credentials_digest(&state, &credentials.user, &credentials.password)?;
        let mut client = Self::detached(home, 0)?;
        client.digest = digest;
        Ok(client)
    }

    /// Берёт или поднимает агента и делает с ним `take`. Сторож, который закрывается, отказал
    /// учётным данным или ещё не принимает подключений, ожидается до ответа или ухода; ушёл —
    /// поднимается новый. Ответ — держатели после отметки ([`Take::Hold`]).
    fn obtain(
        &mut self,
        launch: &KeeperLaunch,
        credentials: &Credentials,
        limit: u8,
        cancel: &CancellationToken,
        take: &Take<'_>,
        on_wait: &mut dyn FnMut(&str),
    ) -> Result<Vec<Holder>, KeeperStartError> {
        let key = self.home.key().to_owned();
        // `agent hold` на пределе ждёт агентов, которые освободятся сами; команда не ждёт, а
        // работает с разовым агентом.
        let waits = matches!(take, Take::Hold(_));
        let mut waiting_for = String::new();
        let mut silent: std::collections::HashMap<String, Instant> =
            std::collections::HashMap::new();
        let mut deadline =
            Instant::now() + Duration::from_millis(launch.startup_timeout_ms) + START_MARGIN;
        let mut respawns = 0;
        let mut evictions = 0;
        loop {
            if cancel.is_cancelled() {
                return Err(cancelled_waiting());
            }
            match self.find(limit)? {
                Found::OverLimit => {
                    // Сверх предела снимается свой простаивающий агент без держателей;
                    // занятого раннер не снимает.
                    match make_room(self.home.state(), &key, cancel, &mut silent) {
                        Room::Freed if waits || evictions <= usize::from(limit) => {
                            evictions += 1;
                            continue;
                        }
                        Room::Wait(what) if waits => {
                            if what != waiting_for {
                                on_wait(&what);
                                waiting_for = what;
                            }
                            std::thread::sleep(ROOM_STEP);
                            // Ожидание места срок подъёма сторожа не съедает.
                            deadline = Instant::now()
                                + Duration::from_millis(launch.startup_timeout_ms)
                                + START_MARGIN;
                            continue;
                        }
                        Room::Held(held) if !held.is_empty() => {
                            return Err(KeeperStartError::Limit { limit, held })
                        }
                        // Удерживаемых не видно, а предел занят: запись сменилась между подсчётом
                        // и обходом — ключ ищется заново.
                        Room::Held(_) if waits => {
                            std::thread::sleep(ROOM_STEP);
                            continue;
                        }
                        Room::Freed | Room::Wait(_) | Room::Held(_) => {
                            return Err(KeeperStartError::Limit {
                                limit,
                                held: Vec::new(),
                            })
                        }
                    }
                }
                Found::Live(keeper) => {
                    self.keeper = keeper;
                    match self.take(take) {
                        Ok(holders) => return Ok(holders),
                        // Сторож закрывается, отказал учётным данным, его канал уже закрыт или
                        // он ещё поднимает агента: ждём, пока он ответит или уйдёт.
                        Err(KeeperStartError::Agent(
                            AgentError::LiveAgentGone { .. } | AgentError::KeeperUnreachable { .. },
                        )) => {}
                        Err(error) => return Err(error),
                    }
                }
                Found::Starting => {}
                Found::Absent => {
                    self.reused = false;
                    let pid = match self.spawn(launch, credentials) {
                        Ok(pid) => pid,
                        Err(error) => {
                            self.forget_start();
                            return Err(error);
                        }
                    };
                    self.keeper = pid;
                    match self.await_start(pid, deadline, cancel)? {
                        Started::Answered => match self.take(take) {
                            Ok(holders) => return Ok(holders),
                            // Поднятого агента сняли, не дав взять (простой, предел): ключ
                            // ищется заново, и при нужде поднимается новый сторож.
                            Err(KeeperStartError::Agent(
                                AgentError::LiveAgentGone { .. }
                                | AgentError::KeeperUnreachable { .. },
                            )) => self.reused = true,
                            Err(error) => return Err(error),
                        },
                        Started::Exited if respawns < RESPAWNS => respawns += 1,
                        Started::Exited => {
                            return Err(unavailable(format!(
                                "the keeper of the live agent exited before answering; see {}",
                                self.home.paths().log().display()
                            )))
                        }
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(unavailable(format!(
                    "the keeper of the live agent did not answer in time; see {}",
                    self.home.paths().log().display()
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

    /// Ответ агента на подключение к базе в начале сессии команды — один раз: для журнала
    /// разговора команды.
    pub fn take_connect_reply(&mut self) -> Vec<AgentMessage> {
        self.session
            .as_mut()
            .map(|session| std::mem::take(&mut session.connected))
            .unwrap_or_default()
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

    /// Найденный живой сторож годится: отвечает, называет ключ хоста из записи, и сессия
    /// команды открыта или держатель поставлен. Ответ — держатели после отметки.
    fn take(&mut self, take: &Take<'_>) -> Result<Vec<Holder>, KeeperStartError> {
        self.check_fingerprint()?;
        match take {
            Take::Session => {
                self.begin()?;
                Ok(Vec::new())
            }
            Take::Hold(holder) => {
                let request = Request::Hold {
                    holder: (*holder).clone(),
                    credentials: Some(self.digest.clone()),
                };
                match self.exchange(&request, QUERY_PATIENCE)? {
                    Frame::Holders { holders } => Ok(holders),
                    other => Err(unexpected(&other).into()),
                }
            }
        }
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
    ) -> Result<u32, KeeperStartError> {
        let program = keeper_exe(self.home.state())
            .map_err(|error| unavailable_by("no copy of the runner for the keeper", error))?;
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
        // Подъём сторожа — подъём сессии, а не работа команды: признак работы он не ставит
        // (`INV.WIRE.PROVIDER-DISPATCHED-SAYS-WHETHER-AN-EXECUTOR-GOT-WORK`).
        crate::platform::service::spawn_hidden(&request).map_err(|error| match error {
            ServiceSpawnError::WouldNotSurvive => KeeperStartError::Job,
            other => unavailable_by("the keeper did not start", other),
        })
    }

    /// Ждёт, пока поднятый сторож поднимет агента и ответит. Отмена прекращает ожидание, а
    /// поднятый сторож остаётся: он переживает команду и уйдёт по простою.
    fn await_start(
        &self,
        pid: u32,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<Started, KeeperStartError> {
        loop {
            if cancel.is_cancelled() {
                self.abandon_start(pid, deadline);
                return Err(cancelled_waiting());
            }
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

    /// Команду отменили, пока поднятый ею сторож поднимал агента: сторожа снимают безусловно,
    /// чтобы не остался свежий агент, которого никто не возьмёт. Канал появляется, когда агент
    /// поднят, поэтому сторожа ждут до ответа, ухода или срока подъёма.
    fn abandon_start(&self, pid: u32, deadline: Instant) {
        while Instant::now() < deadline && process_facts(pid).is_some() {
            if self.status().is_ok() {
                if let Err(error) = self.stop(&CancellationToken::new()) {
                    tracing::warn!(%error, "the keeper started by a cancelled command was not stopped");
                }
                return;
            }
            std::thread::sleep(WAIT_STEP);
        }
        self.forget_start();
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
        // Журнал команды называет сессию, как у разового агента: учётные данные уходят
        // входом в сессию через сторожа, а не ключами запуска.
        tracing::debug!(keeper, "opening agent session");
        let opened = self.runtime.block_on(async {
            let stream = connect_within(&channel, keeper, QUERY_PATIENCE).await?;
            let mut stream = BufReader::new(stream);
            write_line(stream.get_mut(), &request)
                .await
                .map_err(|source| unreachable_at(&channel, source))?;
            match read_line::<_, Frame>(&mut stream).await {
                Ok(Some(Frame::Begun { session, messages })) => Ok(OpenSession {
                    id: session,
                    stream,
                    connected: messages,
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
    /// если сторож не решил его снять. Ответ — `status` сторожа после `end`: `closing` в нём
    /// значит, что агент не останется; отказ — сторож не ответил, и ушёл ли он, говорит
    /// [`Self::gone`].
    pub fn end(&mut self) -> Result<KeeperStatus, AgentError> {
        if let Some(mut session) = self.session.take() {
            self.runtime.block_on(async {
                let _ = tokio::time::timeout(END_PATIENCE, async {
                    write_line(session.stream.get_mut(), &Request::End).await?;
                    read_line::<_, Frame>(&mut session.stream).await
                })
                .await;
            });
        }
        self.status()
    }

    /// Ушёл ли сторож после отказа `error`: канала нет или процесса сторожа подтверждённо нет.
    /// Отказ иного рода (занятый канал, чужой процесс на нём, срок) — сторож может быть жив.
    pub fn gone(&self, error: &AgentError) -> bool {
        let absent = matches!(
            error,
            AgentError::KeeperUnreachable { source, .. }
                if matches!(
                    source.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                )
        );
        absent || self.keeper_exited_within(GONE_PATIENCE)
    }

    fn keeper_exited_within(&self, patience: Duration) -> bool {
        let deadline = Instant::now() + patience;
        loop {
            if self.keeper == 0 || process_facts(self.keeper).is_none() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(WAIT_STEP);
        }
    }

    /// Процесс сторожа.
    pub fn keeper_pid(&self) -> u32 {
        self.keeper
    }

    /// Процесс агента из записи сторожа.
    pub fn agent_pid(&self) -> Option<u32> {
        read_json::<ServiceRecord>(&self.home.paths().record())
            .and_then(|record| record.agent)
            .and_then(|agent| agent.pid)
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
        // Срок некритической команды уже истёк: команда агенту не уходит, и агента ей не
        // прерывают.
        if let Some(deadline) = policy.deadline.filter(|_| !policy.is_critical()) {
            if Instant::now() >= deadline {
                return Err(AgentError::TimedOut {
                    command: command.to_owned(),
                    timeout_ms: 0,
                });
            }
        }
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
        // В отказе — срок, который команда получила, а не прошедшее время.
        let budget_ms = policy.deadline.map_or(0, |deadline| {
            u64::try_from(
                deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis(),
            )
            .unwrap_or(u64::MAX)
        });
        let deadline = policy.deadline.map(tokio::time::Instant::from_std);
        self.runtime.block_on(async {
            let stream = connect_within(&self.channel, self.keeper, QUERY_PATIENCE).await?;
            let (read, mut write) = tokio::io::split(stream);
            let mut reader = BufReader::new(read);
            write_line(&mut write, &request)
                .await
                .map_err(|source| unreachable_at(&self.channel, source))?;
            policy.mark_work_given();
            match await_reply(
                &mut reader,
                &policy.cancellation,
                critical,
                deadline,
                on_batch,
            )
            .await
            {
                Ok(reply) => Ok(reply),
                Err(Abandoned::Failed(error)) => Err(error),
                Err(Abandoned::Cancelled) => {
                    // Строка `cancel` доходит до сторожа и тогда, когда разрыв канала он
                    // увидел бы не сразу.
                    let _ = write_line(&mut write, &Request::Cancel).await;
                    Err(AgentError::Cancelled {
                        command: command.to_owned(),
                        delivered: true,
                    })
                }
                Err(Abandoned::TimedOut) => {
                    let _ = write_line(&mut write, &Request::Cancel).await;
                    Err(AgentError::TimedOut {
                        command: command.to_owned(),
                        timeout_ms: budget_ms,
                    })
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

    /// Снимает держателя; ответ — держатели после запроса.
    pub fn release(&self, holder: Holder) -> Result<Vec<Holder>, AgentError> {
        match self.exchange(&Request::Release { holder }, QUERY_PATIENCE)? {
            Frame::Holders { holders } => Ok(holders),
            other => Err(unexpected(&other)),
        }
    }

    /// Сторож отпускает и снимает агента и выходит; ответ приходит, когда агент снят. Ждёт
    /// не дольше [`STOP_PATIENCE`] и до отмены `cancel`: не дождался — [`Stopped::Pending`].
    pub fn stop(&self, cancel: &CancellationToken) -> Result<Removal, AgentError> {
        match self.stop_if(false, false, cancel)? {
            Stopped::Removed => Ok(Removal::Done),
            Stopped::Pending => Ok(Removal::Pending),
            Stopped::InUse { .. } => Err(AgentError::Keeper {
                detail: "the keeper refused an unconditional stop".to_owned(),
            }),
        }
    }

    /// `stop` с условием, которое сторож проверяет под своим замком: `only_idle` — агент не
    /// выполняет команду и сессии нет, `only_unheld` — держателей нет. Снятым агент считается,
    /// только если сторож ответил `stopped` или подтверждённо ушёл ([`Self::gone`]); иной отказ
    /// канала — отказ, а не снятие.
    pub fn stop_if(
        &self,
        only_idle: bool,
        only_unheld: bool,
        cancel: &CancellationToken,
    ) -> Result<Stopped, AgentError> {
        let request = Request::Stop {
            only_idle,
            only_unheld,
        };
        let answer = self.runtime.block_on(async {
            let stream = connect_within(&self.channel, self.keeper, QUERY_PATIENCE).await?;
            let (read, mut write) = tokio::io::split(stream);
            let mut reader = BufReader::new(read);
            write_line(&mut write, &request)
                .await
                .map_err(|source| unreachable_at(&self.channel, source))?;
            // Запрос уже у сторожа: срок и отмена его не отменяют, а только кончают ожидание.
            tokio::select! {
                biased;
                frame = tokio::time::timeout(STOP_PATIENCE, read_line::<_, Frame>(&mut reader)) => {
                    Ok(frame.ok())
                }
                () = cancel.cancelled() => Ok(None),
            }
        });
        match answer {
            Ok(None) => Ok(Stopped::Pending),
            Ok(Some(Ok(Some(Frame::Stopped)))) => Ok(Stopped::Removed),
            Ok(Some(Ok(Some(Frame::Error {
                error: KeeperError::NotIdle { command, holders },
            })))) => Ok(Stopped::InUse { command, holders }),
            Ok(Some(Ok(Some(Frame::Error { error })))) => Err(error.into_agent()),
            Ok(Some(Ok(Some(other)))) => Err(unexpected(&other)),
            Ok(Some(Ok(None) | Err(_))) => {
                let error = AgentError::LiveAgentGone {
                    detail: "the keeper closed the channel before the answer to stop".to_owned(),
                };
                if self.gone(&error) {
                    Ok(Stopped::Removed)
                } else {
                    Err(error)
                }
            }
            Err(error) if self.gone(&error) => Ok(Stopped::Removed),
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

/// Почему клиент перестал ждать ответа на `run`.
enum Abandoned {
    /// Некритическую команду отменили: клиент пишет сторожу `cancel`.
    Cancelled,
    /// Срок некритической команды истёк: клиент пишет сторожу `cancel`.
    TimedOut,
    /// Сторож отказал или ушёл.
    Failed(AgentError),
}

/// Кадры ответа на `run` до итогового. Чтение кадра создаётся один раз на кадр и переживает
/// отмену и срок: `read_line` к отмене не устойчив, и брошенное посреди строки чтение теряло бы
/// половину кадра. Пришедший кадр важнее отмены и срока. Некритическую команду отмена и срок
/// бросают; критическую — нет: клиент один раз пишет строку отложенного прерывания, как
/// разовый агент, и ждёт исхода.
async fn await_reply<R>(
    reader: &mut R,
    cancellation: &CancellationToken,
    critical: bool,
    deadline: Option<tokio::time::Instant>,
    on_batch: &mut dyn FnMut(&[AgentMessage]),
) -> Result<AgentReply, Abandoned>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let expired = async {
        match deadline {
            Some(deadline) => tokio::time::sleep_until(deadline).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(expired);
    let mut cancel_noted = false;
    let mut deadline_noted = false;
    let mut messages = Vec::new();
    loop {
        let read = read_line::<_, Frame>(&mut *reader);
        tokio::pin!(read);
        let frame = loop {
            tokio::select! {
                biased;
                frame = &mut read => break frame,
                () = cancellation.cancelled(), if !cancel_noted => {
                    if !critical {
                        // Та же строка, что у разового агента: работа команды брошена.
                        tracing::info!("{}", crate::platform::agent::AGENT_COMMAND_ABANDONED);
                        return Err(Abandoned::Cancelled);
                    }
                    cancel_noted = true;
                    note_deferral(ProcessInterruptionReason::Cancelled);
                }
                () = &mut expired, if !deadline_noted => {
                    if !critical {
                        return Err(Abandoned::TimedOut);
                    }
                    deadline_noted = true;
                    note_deferral(ProcessInterruptionReason::TimedOut);
                }
            }
        };
        match frame {
            Ok(Some(Frame::Batch { messages: batch })) => {
                on_batch(&batch);
                messages.extend(batch);
            }
            Ok(Some(Frame::Done)) => return Ok(AgentReply { messages }),
            Ok(Some(Frame::Error { error })) => return Err(Abandoned::Failed(error.into_agent())),
            Ok(Some(other)) => return Err(Abandoned::Failed(unexpected(&other))),
            Ok(None) | Err(_) => {
                return Err(Abandoned::Failed(AgentError::LiveAgentGone {
                    detail: "the keeper closed the channel before the reply".to_owned(),
                }))
            }
        }
    }
}

/// Строка отложенного прерывания критической команды, как у разового агента. Команду в
/// журнал не пишет: маски учётных данных у клиента нет.
fn note_deferral(reason: ProcessInterruptionReason) {
    tracing::warn!(
        reason = ?reason,
        "{}",
        crate::platform::process::CRITICAL_INTERRUPTION_DEFERRED
    );
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
    let Ok(dirs) = std::fs::read_dir(crate::platform::live_registry::agents_path(state)) else {
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

/// Снимает живого агента ключа `key`, если он есть; ответ — был ли он. Агента, который
/// выполняет критическую команду, сторож снимает после неё.
pub fn stop_keeper(state: &Path, key: &str) -> Result<bool, AgentError> {
    match KeeperClient::attach(state, key) {
        Ok(Some(keeper)) => keeper.stop(&CancellationToken::new()).map(|_| true),
        Ok(None) => Ok(false),
        Err(error) => Err(AgentError::Keeper {
            detail: error.to_string(),
        }),
    }
}

/// Освобождает место на пределе: снимает простаивающего дольше всех живого агента без
/// держателей, кроме своего ключа `own`. Условие «простаивает и без держателей» сторож проверяет
/// сам под своим замком (`stop` с `only_idle` и `only_unheld`): команда, открывшая сессию между
/// вопросом и снятием, агента сохраняет. Освобождением считается только подтверждённое снятие.
/// Снять некого — ответ говорит, освободится ли кто-то сам (выполняет команду без держателей,
/// свежий, закрывается, не ответил) или удерживаются все. Сторожа другой версии раннера условным
/// `stop` не трогают — прежний сторож понял бы его как безусловный, — а ждут его простоя.
fn make_room(
    state: &Path,
    own: &str,
    cancel: &CancellationToken,
    silent: &mut std::collections::HashMap<String, Instant>,
) -> Room {
    let ours = Some(env!("CARGO_PKG_VERSION"));
    let mut idle: Vec<(u64, LiveKeeper, String)> = Vec::new();
    let mut wait: Option<String> = None;
    let mut held = Vec::new();
    let found = match lock_agent_registry(state) {
        Ok(_registry) => live_keys(state),
        Err(error) => {
            tracing::warn!(%error, "the live agent registry cannot be read");
            return Room::Wait("the live agent registry to be readable".to_owned());
        }
    };
    for (key, record) in found {
        if key == own {
            continue;
        }
        let place = place_of(&record);
        let infobase = record.agent.as_ref().map(|agent| agent.infobase.clone());
        let work_path = record
            .agent
            .as_ref()
            .and_then(|agent| agent.work_path.clone());
        // Запись «запускается» предел уже занимает, а канала у неё ещё нет.
        let Some(client) = record.pid.and_then(|pid| {
            let home = KeeperHome::existing(state, &key).ok()??;
            KeeperClient::detached(home, pid).ok()
        }) else {
            wait.get_or_insert_with(|| "a live agent to start".to_owned());
            continue;
        };
        let keeper = LiveKeeper { key, client };
        let status = keeper.client.status();
        if status.is_ok() {
            silent.remove(&keeper.key);
        }
        match status {
            Err(error) => {
                tracing::warn!(key = %keeper.key, %error, "a live agent did not answer its status");
                let since = *silent
                    .entry(keeper.key.clone())
                    .or_insert_with(Instant::now);
                if since.elapsed() < SILENCE_LIMIT {
                    wait.get_or_insert_with(|| format!("{place} to answer"));
                } else {
                    held.push(HeldAgent {
                        key: keeper.key.clone(),
                        infobase,
                        work_path,
                        holders: Vec::new(),
                        answering: false,
                    });
                }
            }
            Ok(status) if status.closing => {
                wait.get_or_insert_with(|| format!("{place} to close"));
            }
            Ok(status) if !status.holders.is_empty() => held.push(HeldAgent {
                key: keeper.key.clone(),
                infobase,
                work_path,
                holders: status.holders,
                answering: true,
            }),
            Ok(status) if status.holds_the_infobase() => {
                wait.get_or_insert_with(|| {
                    format!("{place} to finish '{}'", verb(status.command()))
                });
            }
            Ok(status) if status.fresh => {
                wait.get_or_insert_with(|| {
                    format!("{place} to be taken by the command that started it")
                });
            }
            Ok(_) if record.version.as_deref() != ours => {
                wait.get_or_insert_with(|| {
                    format!("{place} of another runner version to idle out")
                });
            }
            Ok(status) => idle.push((status.idle_ms, keeper, place)),
        }
    }
    idle.sort_by_key(|(idle_ms, _, _)| std::cmp::Reverse(*idle_ms));
    for (_, keeper, place) in idle {
        match keeper.client.stop_if(true, true, cancel) {
            Ok(Stopped::Removed) => {
                tracing::info!(key = %keeper.key, "an idle live agent is removed to stay within the limit");
                return Room::Freed;
            }
            Ok(Stopped::Pending) => {
                wait.get_or_insert_with(|| format!("{place} to close"));
            }
            Ok(Stopped::InUse { command, holders }) => {
                tracing::debug!(key = %keeper.key, "the live agent got busy before it was removed");
                if holders.is_empty() {
                    wait.get_or_insert_with(|| {
                        format!("{place} to finish '{}'", verb(command.as_deref()))
                    });
                }
            }
            Err(error) => {
                tracing::warn!(key = %keeper.key, %error, "an idle live agent was not removed");
                wait.get_or_insert_with(|| format!("{place} to answer"));
            }
        }
    }
    match wait {
        Some(what) => Room::Wait(what),
        None => Room::Held(held),
    }
}

/// Глагол команды чужого агента — первые два слова (`config dump-cfg`): её ключи и пути в
/// строку ожидания не идут.
fn verb(command: Option<&str>) -> String {
    match command {
        Some(command) => command
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" "),
        None => "its command".to_owned(),
    }
}

/// Живой агент для строки ожидания: база и рабочая копия по его записи.
fn place_of(record: &ServiceRecord) -> String {
    let Some(agent) = record.agent.as_ref() else {
        return "a live agent".to_owned();
    };
    match &agent.work_path {
        Some(work_path) => format!(
            "the live agent of {} (working copy {work_path})",
            agent.infobase
        ),
        None => format!("the live agent of {}", agent.infobase),
    }
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
    let Ok(keys) = std::fs::read_dir(crate::platform::live_registry::agents_path(state)) else {
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

    /// Клиент с открытой сессией, приёмник «запрос и что клиент написал после», отбой
    /// поддельному сторожу и каталог состояния.
    type FakeKeeper = (
        KeeperClient,
        std::sync::mpsc::Receiver<(Request, Option<String>)>,
        std::sync::mpsc::Sender<()>,
        tempfile::TempDir,
    );

    /// Поддельный сторож на одно подключение `run`: отвечает кадром `batch`, затем `after`.
    /// Ответ — клиент с открытой сессией и приёмник: запрос и то, что клиент написал после.
    fn fake_keeper(after: After) -> FakeKeeper {
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
            connected: Vec::new(),
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
            written
                .as_deref()
                .is_some_and(|line| line.contains("cancel")),
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
        assert!(
            !client.interrupted(),
            "the finished command keeps the agent"
        );
    }

    /// Запись «запускается» без процесса сторожа предел уже занимает: на пределе её ждут, а не
    /// отказывают с пустым перечнем удерживаемых.
    #[test]
    fn a_starting_record_at_the_limit_is_waited_for() {
        let state = if cfg!(unix) {
            tempfile::tempdir_in("/tmp").unwrap()
        } else {
            tempfile::tempdir().unwrap()
        };
        let home = KeeperHome::open(state.path(), &"0a".repeat(16)).unwrap();
        let own = std::process::id();
        let record = ServiceRecord {
            state: RecordState::Starting,
            pid: None,
            started: None,
            image: None,
            machine: machine_fingerprint(),
            starter: process_facts(own).map(|facts| (own, facts.started)),
            port: 0,
            explicit: false,
            bin: PathBuf::new(),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            agent: None,
        };
        write_json(&home.paths().record(), &record).unwrap();
        let mut silent = std::collections::HashMap::new();
        let room = make_room(
            state.path(),
            &"0b".repeat(16),
            &CancellationToken::new(),
            &mut silent,
        );
        assert!(
            matches!(&room, Room::Wait(what) if what == "a live agent to start"),
            "the starting agent is waited for"
        );
    }

    /// Отмена критической команды посреди кадра его не рвёт: чтение кадра переживает отмену,
    /// и ответ, дописанный после неё, приходит целым.
    #[test]
    fn a_cancellation_in_the_middle_of_a_frame_keeps_the_frame_whole() {
        use tokio::io::AsyncWriteExt;
        let runtime = runtime().unwrap();
        runtime.block_on(async {
            let (mut keeper, client) = tokio::io::duplex(1024);
            let mut reader = BufReader::new(client);
            let cancellation = CancellationToken::new();
            let line = format!(
                "{}\n{}\n",
                serde_json::to_string(&Frame::Batch {
                    messages: serde_json::from_str(
                        r#"[{"type":"progress","body":{"message":"load","percent":50}}]"#
                    )
                    .unwrap(),
                })
                .unwrap(),
                serde_json::to_string(&Frame::Done).unwrap()
            );
            let (head, tail) = line.split_at(line.len() / 3);
            let (head, tail) = (head.to_owned(), tail.to_owned());
            let writer = {
                let cancellation = cancellation.clone();
                async move {
                    keeper.write_all(head.as_bytes()).await.unwrap();
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    cancellation.cancel();
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    keeper.write_all(tail.as_bytes()).await.unwrap();
                    keeper
                }
            };
            let mut batches = 0;
            let mut count = |_: &[AgentMessage]| batches += 1;
            let (reply, _keeper) = tokio::join!(
                await_reply(&mut reader, &cancellation, true, None, &mut count),
                writer
            );
            assert!(
                matches!(&reply, Ok(reply) if reply.messages.len() == 1),
                "the frame cut by the cancellation came whole"
            );
            assert_eq!(batches, 1);
        });
    }

    /// Срок некритической команды истёк до отправки: команда сторожу не уходит, и агента ей не
    /// прерывают.
    #[test]
    fn a_run_whose_deadline_has_passed_is_not_sent() {
        let state = tempfile::tempdir().unwrap();
        let home = KeeperHome::open(state.path(), &"cd".repeat(16)).unwrap();
        let mut client = KeeperClient::detached(home, std::process::id()).unwrap();
        // Сторожа на канале нет: отправка была бы отказом подключения, а не сроком.
        client.channel = format!("{}-absent", client.channel);
        client.session = Some(OpenSession {
            id: "session".to_owned(),
            stream: BufReader::new(Box::new(tokio::io::duplex(64).0)),
            connected: Vec::new(),
        });
        let mut policy = WaitPolicy::for_keeper(tokio_util::sync::CancellationToken::new(), false);
        policy.deadline = Some(Instant::now() - Duration::from_millis(1));
        let outcome = client.run("config dump-cfg", &policy, &mut |_| {});
        assert!(
            matches!(outcome, Err(AgentError::TimedOut { timeout_ms: 0, .. })),
            "{outcome:?}"
        );
        assert!(!client.interrupted(), "nothing was sent, nothing to remove");
    }

    /// Живой процесс, который не сторож: сторож может быть жив, и его отказ — не снятие.
    #[cfg(any(windows, target_os = "linux"))]
    fn a_live_process() -> std::process::Child {
        let mut command = if cfg!(windows) {
            let mut command = std::process::Command::new("ping");
            command.args(["-n", "60", "127.0.0.1"]);
            command
        } else {
            let mut command = std::process::Command::new("sleep");
            command.arg("60");
            command
        };
        command.stdout(std::process::Stdio::null()).spawn().unwrap()
    }

    /// `stop` считает агента снятым только по ответу сторожа, по отсутствию канала или по
    /// ушедшему процессу сторожа. Канал, который обслуживает не сторож из записи, — отказ, а не
    /// снятие: так предел не засчитывает освобождение, которого не было.
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn a_stop_counts_as_a_removal_only_when_the_keeper_is_gone() {
        let state = if cfg!(unix) {
            tempfile::tempdir_in("/tmp").unwrap()
        } else {
            tempfile::tempdir().unwrap()
        };
        let home = KeeperHome::open(state.path(), &"ef".repeat(16)).unwrap();
        let channel = if cfg!(windows) {
            format!(
                r"\\.\pipe\v8-runner-agent-test-{}",
                uuid::Uuid::new_v4().simple()
            )
        } else {
            home.channel()
        };
        let mut keeper = a_live_process();
        let mut client = KeeperClient::detached(home, keeper.id()).unwrap();
        client.channel = channel.clone();
        let never = CancellationToken::new();

        // Канала нет — сторожа нет.
        assert!(matches!(
            client.stop_if(true, true, &never),
            Ok(Stopped::Removed)
        ));

        // Канал есть, но обслуживает его этот процесс, а не сторож из записи.
        let runtime = runtime().unwrap();
        let listener =
            runtime.block_on(async { super::super::channel::Listener::bind(&channel).unwrap() });
        let outcome = client.stop_if(true, true, &never);
        assert!(
            matches!(&outcome, Err(AgentError::KeeperUnreachable { source, .. })
                if source.kind() == std::io::ErrorKind::PermissionDenied),
            "{outcome:?}"
        );
        listener.remove();
        drop(listener);
        let _ = keeper.kill();
        let _ = keeper.wait();
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
                work_path: None,
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
