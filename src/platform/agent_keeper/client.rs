//! Клиент сторожа: находит живого агента ключа по записи или поднимает сторожа, затем говорит с
//! ним по каналу. Замок реестра учётной записи берётся только на поиск и на запись «запускается»,
//! а не на все 6–9 с подъёма агента.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::BufReader;

use super::channel::connect;
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

/// Подключение команды к сторожу ключа.
pub struct KeeperClient {
    home: KeeperHome,
    channel: String,
    digest: String,
    /// Взят ли уже живой агент, а не поднят этим клиентом.
    reused: bool,
    runtime: tokio::runtime::Runtime,
}

/// Что делать со сторожем ключа по его записи.
enum Found {
    Live,
    /// Его поднимает другой клиент.
    Starting,
    /// Его нет: запись «запускается» уже стоит за этим клиентом.
    Absent,
}

impl KeeperClient {
    /// Живой агент ключа `key`: берётся, если его сторож жив, иначе поднимается новый сторож.
    /// `work` — отметка работы команды: поднятый сторож переживает команду, поэтому его запуск
    /// — уже её работа.
    pub fn connect_or_start(
        key: &str,
        launch: &KeeperLaunch,
        credentials: &Credentials,
        work: &WorkGiven,
    ) -> Result<Self, KeeperStartError> {
        let state = crate::support::machine::account::account_state_dir()?;
        let home = KeeperHome::open(&state, key)?;
        let digest = credentials_digest(&state, &credentials.user, &credentials.password)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let mut client = Self {
            channel: home.channel(),
            home,
            digest,
            reused: true,
            runtime,
        };
        let deadline =
            Instant::now() + Duration::from_millis(launch.startup_timeout_ms) + START_MARGIN;
        loop {
            match client.find()? {
                Found::Live => match client.status() {
                    Ok(_) => {
                        client.check_fingerprint()?;
                        return Ok(client);
                    }
                    // Сторож закрывается: ждём, пока он уйдёт, и поднимаем нового.
                    Err(AgentError::LiveAgentGone { .. }) => {}
                    Err(error) => return Err(error.into()),
                },
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
                    client.await_start(pid, deadline)?;
                    client.check_fingerprint()?;
                    return Ok(client);
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

    /// Взят ли уже живой агент.
    pub fn reused(&self) -> bool {
        self.reused
    }

    /// `AgentBaseDir` агента.
    pub fn base_dir(&self) -> PathBuf {
        self.home.base_dir()
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
            Observed::Own(_) => Ok(Found::Live),
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
    fn await_start(&self, pid: u32, deadline: Instant) -> Result<(), KeeperStartError> {
        loop {
            match self.status() {
                Ok(_) => return Ok(()),
                // Канала ещё нет: сторож запускается.
                Err(AgentError::Keeper { detail }) if detail.starts_with(NO_CHANNEL) => {}
                Err(error) => return Err(error.into()),
            }
            if process_facts(pid).is_none() || Instant::now() >= deadline {
                self.forget_start();
                return Err(AgentError::Keeper {
                    detail: format!(
                        "the keeper of the live agent exited or did not answer in time; see {}",
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
    fn check_fingerprint(&self) -> Result<(), KeeperStartError> {
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
            }
            .into());
        }
        Ok(())
    }

    /// Команда агенту через сторожа. Кадры ответа отдаются `on_batch` по мере прихода. Отмена
    /// некритической команды обрывает подключение — сторож бросает команду и снимает агента;
    /// критическую отмена не прерывает: её доводят до исхода.
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
        let request = Request::Run {
            command: command.to_owned(),
            critical: policy.is_critical(),
            credentials: self.digest.clone(),
        };
        self.runtime.block_on(async {
            let stream = connect(&self.channel).await.map_err(lost_channel)?;
            let (read, mut write) = tokio::io::split(stream);
            let mut reader = BufReader::new(read);
            write_line(&mut write, &request)
                .await
                .map_err(lost_channel)?;
            policy.mark_work_given();
            let mut messages = Vec::new();
            loop {
                let frame = tokio::select! {
                    frame = read_line::<_, Frame>(&mut reader) => frame,
                    () = policy.cancellation.cancelled(), if !policy.is_critical() => {
                        // Уход подключения и есть отмена для сторожа. На Windows канал
                        // закрывается, когда рантайм обработает отмену своего чтения, поэтому
                        // после сброса он ещё крутится.
                        drop((reader, write));
                        tokio::time::sleep(CHANNEL_CLOSE).await;
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
            let stream = connect(&self.channel).await.map_err(lost_channel)?;
            let (read, mut write) = tokio::io::split(stream);
            let mut reader = BufReader::new(read);
            write_line(&mut write, request)
                .await
                .map_err(lost_channel)?;
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

/// Сколько рантайм клиента крутится после сброса подключения: за это время канал закрывается.
const CHANNEL_CLOSE: Duration = Duration::from_millis(200);

/// Начало отказа «канала сторожа нет».
const NO_CHANNEL: &str = "no keeper channel";

fn lost_channel(error: std::io::Error) -> AgentError {
    AgentError::Keeper {
        detail: format!("{NO_CHANNEL}: {error}"),
    }
}

fn unexpected(frame: &Frame) -> AgentError {
    AgentError::Keeper {
        detail: format!("the keeper gave an unexpected answer: {frame:?}"),
    }
}

/// Копия исполняемого файла раннера, из которой запускается сторож:
/// `<state>/bin/<версия>/<метка>/<имя>`. Метка — от размера и времени изменения файла, поэтому
/// пересобранный раннер той же версии получает свою копию, а работающий сторож держит свою.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Сторож запускается из своей копии раннера, а не из файла, который обновляют
    /// (`INV.PLATFORM.A-LIVE-AGENT-DIES-WITH-ITS-KEEPER`); вторая копия той же сборки не
    /// делается.
    #[test]
    fn the_keeper_runs_from_its_own_copy_of_the_runner() {
        let state = tempfile::tempdir().unwrap();
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
    }

    /// Клиент отдаёт кадры по мере прихода, а отменённая некритическая команда обрывает
    /// подключение так, что сторож это видит (`INV.PLATFORM.AN-INTERRUPTED-COMMAND-REMOVES-THE-LIVE-AGENT`).
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
                let mut probe = [0u8; 8];
                let left = tokio::time::timeout(Duration::from_secs(5), reader.read(&mut probe))
                    .await
                    .is_ok_and(|read| matches!(read, Ok(0) | Err(_)));
                seen_tx.send((request, left)).unwrap();
                listener.remove();
            });
        });
        bound_rx.recv().unwrap();
        let client = KeeperClient {
            home,
            channel,
            digest: "digest".to_owned(),
            reused: true,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
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
        assert!(
            matches!(request, Request::Run { critical: false, ref credentials, .. } if credentials == "digest"),
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
