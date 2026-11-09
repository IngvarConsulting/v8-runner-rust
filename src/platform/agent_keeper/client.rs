//! Клиент сторожа: находит живого агента ключа по записи или поднимает сторожа, открывает
//! сессию команды (`begin` … `end`) и говорит с ним по каналу. Замок реестра учётной записи
//! берётся только на поиск и на запись «запускается», а не на все 6–9 с подъёма агента.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::BufReader;

use super::channel::{connect, Stream};
use super::protocol::{read_line, write_line, Frame, Holder, KeeperStatus, Request};
use super::{credentials_digest, KeeperHome};
use crate::platform::agent::{AgentError, AgentMessage, AgentReply, WaitPolicy};
use crate::platform::live_registry::lock_agent_registry;
use crate::platform::process::{ProcessExecutor, WorkGiven};
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

/// Сторож не поднят.
#[derive(Debug, thiserror::Error)]
pub enum KeeperStartError {
    /// Раннер в Job с kill-on-close без права выхода: сторож не переживёт команду, и команда
    /// работает с разовым агентом (`INV.PLATFORM.A-CLOSED-JOB-KEEPS-THE-AGENT-ONE-SHOT`).
    #[error("the runner runs in a Job that kills its processes on close and forbids leaving it")]
    Job,
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
    runtime: tokio::runtime::Runtime,
}

/// Что делать со сторожем ключа по его записи.
enum Found {
    Live(u32),
    /// Его поднимает другой клиент.
    Starting,
    /// Его нет: запись «запускается» уже стоит за этим клиентом.
    Absent,
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

impl KeeperClient {
    /// Живой агент ключа `key` с открытой сессией команды: берётся, если его сторож жив, иначе
    /// поднимается новый сторож. `work` — отметка работы команды: поднятый сторож переживает
    /// команду, поэтому его запуск — уже её работа.
    pub fn connect_or_start(
        key: &str,
        launch: &KeeperLaunch,
        credentials: &Credentials,
        work: &WorkGiven,
    ) -> Result<Self, KeeperStartError> {
        let state = crate::support::machine::account::account_state_dir()?;
        let home = KeeperHome::open(&state, key)?;
        let digest = credentials_digest(&state, &credentials.user, &credentials.password)?;
        let mut client = Self {
            channel: home.channel(),
            home,
            digest,
            keeper: 0,
            reused: true,
            session: None,
            runtime: runtime()?,
        };
        let deadline =
            Instant::now() + Duration::from_millis(launch.startup_timeout_ms) + START_MARGIN;
        let mut respawns = 0;
        loop {
            match client.find()? {
                Found::Live(keeper) => {
                    client.keeper = keeper;
                    match client.take() {
                        Ok(()) => return Ok(client),
                        // Сторож закрывается или его канал уже закрыт: ждём, пока он уйдёт,
                        // и поднимаем нового.
                        Err(
                            AgentError::LiveAgentGone { .. } | AgentError::KeeperUnreachable { .. },
                        ) => {}
                        Err(error) => return Err(error.into()),
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
                        Started::Answered => {
                            client.take()?;
                            return Ok(client);
                        }
                        Started::Exited if respawns < RESPAWNS => respawns += 1,
                        Started::Exited => {
                            return Err(AgentError::Keeper {
                                detail: format!(
                                    "the keeper of the live agent exited before answering; see {}",
                                    client.home.paths().log().display()
                                ),
                            }
                            .into())
                        }
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(AgentError::Keeper {
                    detail: format!(
                        "the keeper of the live agent did not answer in time; see {}",
                        client.home.paths().log().display()
                    ),
                }
                .into());
            }
            std::thread::sleep(WAIT_STEP);
        }
    }

    /// Живой сторож ключа `key`, если он есть, — без сессии команды и без подъёма нового: для
    /// `status`, `stop` и снятия.
    pub fn attach(state: &Path, key: &str) -> Result<Option<Self>, KeeperStartError> {
        let home = KeeperHome::open(state, key)?;
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
        Ok(Some(Self {
            channel: home.channel(),
            home,
            digest: String::new(),
            keeper,
            reused: true,
            session: None,
            runtime: runtime()?,
        }))
    }

    /// Взят ли уже живой агент.
    pub fn reused(&self) -> bool {
        self.reused
    }

    /// `AgentBaseDir` агента.
    pub fn base_dir(&self) -> PathBuf {
        self.home.base_dir()
    }

    /// Найденный живой сторож годится: отвечает, называет ключ хоста из записи, и сессия
    /// команды открыта.
    fn take(&mut self) -> Result<(), AgentError> {
        self.check_fingerprint()?;
        self.begin()
    }

    /// Ищет сторожа под замком реестра; не нашёл — ставит запись «запускается» за собой.
    fn find(&self) -> Result<Found, KeeperStartError> {
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
        let program = keeper_exe(self.home.state())?;
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
                other => KeeperStartError::Agent(AgentError::Keeper {
                    detail: format!("the keeper did not start: {other}"),
                }),
            })
    }

    /// Ждёт, пока поднятый сторож поднимет агента и ответит.
    fn await_start(&self, pid: u32, deadline: Instant) -> Result<Started, KeeperStartError> {
        loop {
            match self.status() {
                Ok(_) => return Ok(Started::Answered),
                // Канала ещё нет: сторож запускается.
                Err(AgentError::KeeperUnreachable { .. }) => {}
                Err(error) => return Err(error.into()),
            }
            if process_facts(pid).is_none() {
                self.forget_start();
                return Ok(Started::Exited);
            }
            if Instant::now() >= deadline {
                self.forget_start();
                return Err(AgentError::Keeper {
                    detail: format!(
                        "the keeper of the live agent did not answer in time; see {}",
                        self.home.paths().log().display()
                    ),
                }
                .into());
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
    fn begin(&mut self) -> Result<(), AgentError> {
        let request = Request::Begin {
            credentials: self.digest.clone(),
        };
        let (channel, keeper) = (self.channel.clone(), self.keeper);
        let opened = self.runtime.block_on(async {
            let stream = connect(&channel, keeper)
                .await
                .map_err(|source| unreachable_at(&channel, source))?;
            let mut stream = BufReader::new(stream);
            write_line(stream.get_mut(), &request)
                .await
                .map_err(|source| unreachable_at(&channel, source))?;
            match read_line::<_, Frame>(&mut stream).await {
                Ok(Some(Frame::Begun { session })) => Ok(OpenSession {
                    id: session,
                    stream,
                }),
                Ok(Some(Frame::Error { error })) => Err(error.into_agent()),
                Ok(Some(other)) => Err(unexpected(&other)),
                Ok(None) | Err(_) => Err(AgentError::LiveAgentGone {
                    detail: "the keeper closed the channel before the session began".to_owned(),
                }),
            }
        })?;
        self.session = Some(opened);
        Ok(())
    }

    /// Закрывает сессию команды: сторож отключает агента от базы, а сам агент остаётся жить.
    /// Ответ не важен: ушедшую сессию сторож закрывает и без него.
    pub fn end(&mut self) {
        let Some(mut session) = self.session.take() else {
            return;
        };
        self.runtime.block_on(async {
            let _ = tokio::time::timeout(END_PATIENCE, async {
                write_line(session.stream.get_mut(), &Request::End).await?;
                read_line::<_, Frame>(&mut session.stream).await
            })
            .await;
        });
    }

    /// Команда агенту через сторожа в открытой сессии. Кадры ответа отдаются `on_batch` по мере
    /// прихода. Отмена некритической команды пишет сторожу `cancel` и уходит — сторож бросает
    /// команду и снимает агента; критическую отмена не прерывает: её доводят до исхода.
    pub fn run(
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
        self.runtime.block_on(async {
            let stream = connect(&self.channel, self.keeper)
                .await
                .map_err(|source| unreachable_at(&self.channel, source))?;
            let (read, mut write) = tokio::io::split(stream);
            let mut reader = BufReader::new(read);
            write_line(&mut write, &request)
                .await
                .map_err(|source| unreachable_at(&self.channel, source))?;
            policy.mark_work_given();
            let mut messages = Vec::new();
            loop {
                let frame = tokio::select! {
                    frame = read_line::<_, Frame>(&mut reader) => frame,
                    () = policy.cancellation.cancelled(), if !policy.is_critical() => {
                        // Строка `cancel` доходит до сторожа и тогда, когда разрыв канала
                        // он увидел бы не сразу.
                        let _ = write_line(&mut write, &Request::Cancel).await;
                        return Err(AgentError::Cancelled {
                            command: command.to_owned(),
                            delivered: true,
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
        match self.exchange(&Request::Status)? {
            Frame::Status { status } => Ok(status),
            other => Err(unexpected(&other)),
        }
    }

    pub fn hold(&self, holder: Holder) -> Result<Vec<Holder>, AgentError> {
        match self.exchange(&Request::Hold { holder })? {
            Frame::Holders { holders } => Ok(holders),
            other => Err(unexpected(&other)),
        }
    }

    pub fn release(&self, holder: Holder) -> Result<Vec<Holder>, AgentError> {
        match self.exchange(&Request::Release { holder })? {
            Frame::Holders { holders } => Ok(holders),
            other => Err(unexpected(&other)),
        }
    }

    /// Сторож отпускает и снимает агента и выходит; ответ приходит, когда агент снят.
    pub fn stop(&self) -> Result<(), AgentError> {
        match self.exchange(&Request::Stop)? {
            Frame::Stopped => Ok(()),
            other => Err(unexpected(&other)),
        }
    }

    /// Запрос с одним итоговым кадром; отказ сторожа — отказом агентского пути.
    fn exchange(&self, request: &Request) -> Result<Frame, AgentError> {
        self.runtime.block_on(async {
            let stream = connect(&self.channel, self.keeper)
                .await
                .map_err(|source| unreachable_at(&self.channel, source))?;
            let (read, mut write) = tokio::io::split(stream);
            let mut reader = BufReader::new(read);
            write_line(&mut write, request)
                .await
                .map_err(|source| unreachable_at(&self.channel, source))?;
            match read_line::<_, Frame>(&mut reader).await {
                Ok(Some(Frame::Error { error })) => Err(error.into_agent()),
                Ok(Some(frame)) => Ok(frame),
                Ok(None) | Err(_) => Err(AgentError::LiveAgentGone {
                    detail: "the keeper closed the channel before the answer".to_owned(),
                }),
            }
        })
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
    let Ok(keys) = std::fs::read_dir(state.join("agents")) else {
        return Vec::new();
    };
    keys.flatten()
        .filter_map(|key| {
            read_json::<ServiceRecord>(&key.path().join(super::SERVICE).join("record.json"))
        })
        .filter(|record| adopt_recorded_pid(record, &process_facts, machine.as_deref()))
        .map(|record| record.bin)
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

    /// Клиент отдаёт кадры по мере прихода, а отменённая некритическая команда пишет сторожу
    /// `cancel`, и сторож видит уход клиента, пока тот ещё жив
    /// (`INV.PLATFORM.AN-INTERRUPTED-COMMAND-REMOVES-THE-LIVE-AGENT`).
    #[test]
    fn a_cancelled_run_leaves_the_channel_and_the_keeper_sees_it() {
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
                let mut probe = [0u8; 64];
                let left = tokio::time::timeout(Duration::from_secs(5), reader.read(&mut probe))
                    .await
                    .is_ok();
                seen_tx.send((request, left)).unwrap();
                // Клиент ещё жив: сторож видит строку `cancel`, а не выход процесса.
                let _ = done_rx.recv_timeout(Duration::from_secs(10));
                listener.remove();
            });
        });
        bound_rx.recv().unwrap();
        let client = KeeperClient {
            home,
            channel,
            digest: "digest".to_owned(),
            keeper: std::process::id(),
            reused: true,
            session: Some(OpenSession {
                id: "session".to_owned(),
                stream: BufReader::new(Box::new(tokio::io::duplex(64).0)),
            }),
            runtime: runtime().unwrap(),
        };
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
        let (request, left) = seen_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let _ = done_tx.send(());
        assert!(
            matches!(request, Request::Run { critical: false, ref credentials, ref session, .. }
                if credentials == "digest" && session == "session"),
            "{request:?}"
        );
        assert!(
            left,
            "the keeper sees the client leave while the client still lives"
        );
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
