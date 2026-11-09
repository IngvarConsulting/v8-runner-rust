//! Сторож живого агента: скрытая подкоманда `v8-runner agent keeper --key <hex>`.
//!
//! Запуск агента и данные для него приходят одной строкой JSON в stdin ([`KeeperStart`]):
//! учётные данные базы не попадают в командную строку. Сторож сначала занимает канал (второй
//! сторож того же ключа на нём отказывает), пишет свою запись, затем поднимает агента
//! ([`ManagedAgent::launch_for_keeper`]) и держит его сессию SSH.
//!
//! Агентом владеет один долгоживущий поток: он поднимает процесс агента (на Linux сигнал смерти
//! родителя привязан к потоку, `INV.PLATFORM.A-LIVE-AGENT-DIES-WITH-ITS-KEEPER`) и выполняет
//! команды по одной. Подключения обслуживают задачи `tokio`, каждое своё: `status`, `hold` и
//! `release` отвечают и во время долгого `run`. Срок простоя считает тикер
//! (`INV.PLATFORM.A-LIVE-AGENT-CLOSES-WHEN-NOBODY-HOLDS-IT`, `…-CLOSES-WITH-ITS-HOLDER`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, BufReader};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::channel::{Listener, Stream};
use super::client::KeeperStart;
use super::protocol::{read_line, write_line, Frame, Holder, KeeperError, KeeperStatus, Request};
use super::{credentials_digest, idle_limit, KeeperHome};
use crate::platform::agent::{
    free_managed_port, host_key_fallback_roots, AgentError, AgentLaunch, AgentSessionRequest,
    EphemeralHostKey, HostKeyExpectation, LaunchHostKey, ManagedAgent, WaitPolicy,
};
use crate::platform::live_registry::lock_agent_registry;
use crate::platform::process::ProcessExecutor;
use crate::platform::service::process_facts;
use crate::platform::service_record::{
    machine_fingerprint, read_json, write_json, LiveAgentFacts, RecordState, ServiceRecord,
};

/// Как часто сторож проверяет держателей-процессы и сам процесс агента.
const HOLDER_POLL: Duration = Duration::from_secs(2);
/// Шаг тикера: сроки держателей и простоя проверяются с этой точностью.
const TICK: Duration = Duration::from_millis(200);
/// Сколько сторож, не поднявший агента, ещё отвечает тем, кто его ждёт, прежде чем выйти.
const FAILED_GRACE: Duration = Duration::from_secs(3);
/// Сколько сторож даёт ответам `stop` уйти в канал, прежде чем выйти.
const STOP_FLUSH: Duration = Duration::from_millis(200);

/// Точка входа подкоманды: код выхода процесса.
pub fn run_keeper(key: &str) -> i32 {
    match serve(key) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("agent keeper: {error}");
            1
        }
    }
}

fn serve(key: &str) -> Result<(), String> {
    let state = crate::support::machine::account::account_state_dir()
        .map_err(|error| format!("no state directory: {error}"))?;
    let home = KeeperHome::open(&state, key).map_err(|error| error.to_string())?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|error| format!("cannot read the start request from stdin: {error}"))?;
    let start: KeeperStart = serde_json::from_str(line.trim_end())
        .map_err(|error| format!("the start request is not readable: {error}"))?;
    let digest = credentials_digest(
        home.state(),
        &start.credentials.user,
        &start.credentials.password,
    )
    .map_err(|error| format!("cannot make the credentials digest: {error}"))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(keep(home, start, digest))
}

/// Как прошёл подъём агента.
#[derive(Debug, Clone)]
enum Readiness {
    Launching,
    Ready {
        agent_pid: Option<u32>,
        fingerprint: Option<String>,
    },
    Failed(KeeperError),
}

/// Что знает только сторож.
struct State {
    /// Команда, которую агент выполняет сейчас.
    busy: Option<String>,
    /// Отмена выполняемой команды и её класс: `stop` бросает только некритическую.
    current: Option<(CancellationToken, bool)>,
    holders: Vec<Holder>,
    /// С какого мгновения агент простаивает.
    idle_since: Instant,
    /// Агента снимут, как только он освободится.
    close_requested: bool,
    /// Когда подъём агента не удался.
    failed_at: Option<Instant>,
}

/// Работа потоку агента.
enum Job {
    Run {
        command: String,
        critical: bool,
        cancel: CancellationToken,
        started: Arc<AtomicBool>,
        frames: mpsc::UnboundedSender<Frame>,
    },
    Close,
}

struct Shared {
    home: KeeperHome,
    digest: String,
    state: Mutex<State>,
    readiness: watch::Sender<Readiness>,
    jobs: Mutex<std::sync::mpsc::Sender<Job>>,
    shutdown: CancellationToken,
    stopped: watch::Sender<bool>,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn request_close(&self, why: &str) {
        let mut state = self.lock();
        if !state.close_requested {
            info!(reason = why, "the live agent will be removed");
        }
        state.close_requested = true;
    }

    fn closing(&self) -> bool {
        self.lock().close_requested || self.shutdown.is_cancelled()
    }

    fn send(&self, job: Job) -> bool {
        self.jobs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .send(job)
            .is_ok()
    }

    /// Ждёт исхода подъёма агента: `Err` — отказ запуска или закрытие.
    async fn ready(&self) -> Result<(Option<u32>, Option<String>), KeeperError> {
        let mut readiness = self.readiness.subscribe();
        let seen = readiness
            .wait_for(|readiness| !matches!(readiness, Readiness::Launching))
            .await
            .map(|seen| seen.clone())
            .map_err(|_| KeeperError::Closing)?;
        match seen {
            Readiness::Ready {
                agent_pid,
                fingerprint,
            } => {
                if self.closing() {
                    Err(KeeperError::Closing)
                } else {
                    Ok((agent_pid, fingerprint))
                }
            }
            Readiness::Failed(error) => Err(error),
            Readiness::Launching => unreachable!("waited past launching"),
        }
    }
}

async fn keep(home: KeeperHome, start: KeeperStart, digest: String) -> Result<(), String> {
    let mut listener = Listener::bind(&home.channel()).map_err(|error| error.to_string())?;
    write_record(&home, 0, None).map_err(|error| format!("cannot write the record: {error}"))?;
    let (jobs, queue) = std::sync::mpsc::channel();
    let (readiness, _) = watch::channel(Readiness::Launching);
    let (stopped, _) = watch::channel(false);
    let shared = Arc::new(Shared {
        home,
        digest,
        state: Mutex::new(State {
            busy: None,
            current: None,
            holders: Vec::new(),
            idle_since: Instant::now(),
            close_requested: false,
            failed_at: None,
        }),
        readiness,
        jobs: Mutex::new(jobs),
        shutdown: CancellationToken::new(),
        stopped,
    });

    let (worker_done, worker_finished) = tokio::sync::oneshot::channel::<()>();
    let worker = {
        let shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("live-agent".to_owned())
            .spawn(move || {
                work(&shared, start, &queue);
                let _ = worker_done.send(());
            })
            .map_err(|error| format!("cannot start the agent thread: {error}"))?
    };
    tokio::spawn(tick(Arc::clone(&shared)));

    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(stream) => {
                    tokio::spawn(handle(Arc::clone(&shared), stream));
                }
                Err(error) => warn!(%error, "the keeper channel refused a connection"),
            },
            () = shared.shutdown.cancelled() => break,
        }
    }

    // Новых подключений нет; поток агента снимает агента после команды, которую уже ведёт.
    shared.send(Job::Close);
    let _ = worker_finished.await;
    let _ = worker.join();
    remove_record(&shared.home);
    listener.remove();
    let _ = shared.stopped.send(true);
    tokio::time::sleep(STOP_FLUSH).await;
    info!("the keeper is done");
    Ok(())
}

/// Поток агента: поднимает агента и выполняет команды по одной, затем отпускает и снимает его.
fn work(shared: &Shared, start: KeeperStart, queue: &std::sync::mpsc::Receiver<Job>) {
    let infobase = start.launch.infobase.clone();
    let (mut agent, port) = match launch(&shared.home, start) {
        Ok(launched) => launched,
        Err(error) => {
            warn!(%error, "the live agent did not start");
            shared.lock().failed_at = Some(Instant::now());
            shared.request_close("the agent did not start");
            let failure = KeeperError::Launch {
                message: error.to_string(),
            };
            let _ = shared.readiness.send(Readiness::Failed(failure.clone()));
            for job in queue {
                match job {
                    Job::Run { frames, .. } => {
                        let _ = frames.send(Frame::Error {
                            error: failure.clone(),
                        });
                    }
                    Job::Close => break,
                }
            }
            return;
        }
    };
    let fingerprint = match agent.host_key_expectation() {
        HostKeyExpectation::Pinned(fingerprint) => Some(fingerprint.to_string()),
        HostKeyExpectation::Unpinned => None,
    };
    let agent_pid = agent.pid();
    let facts = LiveAgentFacts {
        pid: agent_pid.unwrap_or_default(),
        started: agent_pid.and_then(process_facts).map(|facts| facts.started),
        fingerprint: fingerprint.clone(),
        channel: shared.home.channel(),
        infobase,
    };
    if let Err(error) = write_record(&shared.home, port, Some(facts)) {
        warn!(%error, "cannot write the live agent record");
    }
    shared.lock().idle_since = Instant::now();
    let _ = shared.readiness.send(Readiness::Ready {
        agent_pid,
        fingerprint,
    });
    info!(pid = ?agent_pid, "the live agent is up");

    for job in queue {
        let Job::Run {
            command,
            critical,
            cancel,
            started,
            frames,
        } = job
        else {
            break;
        };
        {
            let mut state = shared.lock();
            if cancel.is_cancelled() {
                continue;
            }
            if state.close_requested {
                let _ = frames.send(Frame::Error {
                    error: KeeperError::Closing,
                });
                continue;
            }
            started.store(true, Ordering::Release);
            state.busy = Some(command.clone());
            state.current = Some((cancel.clone(), critical));
        }
        debug!(%command, critical, "running a command for a client");
        let policy = WaitPolicy::for_keeper(cancel, critical);
        let result = agent
            .session()
            .run_streaming(&command, &policy, &mut |batch| {
                let _ = frames.send(Frame::Batch {
                    messages: batch.to_vec(),
                });
            });
        let frame = match result {
            Ok(_) => Frame::Done,
            Err(error) => {
                // Сессия, оборванная посреди ответа, уже не годится следующей команде.
                if matches!(
                    error,
                    AgentError::SessionClosed { .. }
                        | AgentError::Transport { .. }
                        | AgentError::InvalidReply { .. }
                        | AgentError::Cancelled { .. }
                ) {
                    shared.request_close("the agent session broke off");
                }
                Frame::Error {
                    error: KeeperError::from_agent(error),
                }
            }
        };
        {
            let mut state = shared.lock();
            state.busy = None;
            state.current = None;
            state.idle_since = Instant::now();
        }
        let _ = frames.send(frame);
    }
    agent.release_and_remove();
    info!("the live agent is released and removed");
}

/// Поднимает агента ключа и открывает его сессию.
fn launch(home: &KeeperHome, start: KeeperStart) -> Result<(ManagedAgent, u16), AgentError> {
    let KeeperStart {
        launch,
        credentials,
    } = start;
    let port = match launch.port {
        Some(port) => port,
        None => free_managed_port()?,
    };
    let host_key = match launch.host_key {
        Some(declared) => LaunchHostKey::Declared(declared),
        None => LaunchHostKey::OneTime(EphemeralHostKey::create_for(
            &home.key_dir(),
            &host_key_fallback_roots(),
        )?),
    };
    let root = home.paths().root();
    let agent = AgentLaunch {
        v8: launch.v8,
        infobase_args: launch.infobase_args,
        port,
        host_key,
        base_dir: home.base_dir(),
        workdir: root.join("workdir"),
        process_log: root.join("agent.process"),
    };
    let request = AgentSessionRequest {
        endpoint: agent.endpoint(),
        user: credentials.user,
        password: credentials.password,
        // Журнал разговора пишет команда у себя: сторож журнала запросов не ведёт.
        transcript_log: None,
        host_key: agent.host_key.expectation(),
    };
    ManagedAgent::launch_for_keeper(
        &ProcessExecutor,
        agent,
        request,
        Duration::from_millis(launch.startup_timeout_ms.max(1)),
        &WaitPolicy::for_keeper(CancellationToken::new(), false),
    )
    .map(|agent| (agent, port))
}

/// Сроки: держатели, простой, просьба закрыться.
async fn tick(shared: Arc<Shared>) {
    let idle = idle_limit();
    let mut polled = Instant::now();
    loop {
        tokio::time::sleep(TICK).await;
        if shared.shutdown.is_cancelled() {
            return;
        }
        let poll = polled.elapsed() >= HOLDER_POLL;
        if poll {
            polled = Instant::now();
        }
        let agent_pid = match &*shared.readiness.borrow() {
            Readiness::Ready { agent_pid, .. } => *agent_pid,
            _ => None,
        };
        // Процесс агента, которого уже нет, никому не служит.
        if poll && agent_pid.is_some_and(|pid| process_facts(pid).is_none()) {
            shared.request_close("the agent process is gone");
        }
        let now_ms = now_ms();
        let mut state = shared.lock();
        if let Some(failed_at) = state.failed_at {
            if failed_at.elapsed() >= FAILED_GRACE {
                shared.shutdown.cancel();
                return;
            }
            continue;
        }
        let before = state.holders.len();
        state.holders.retain(|holder| match holder {
            Holder::Ttl { until_ms, .. } => *until_ms > now_ms,
            Holder::Process { pid, started } => {
                !poll || process_facts(*pid).is_some_and(|facts| facts.started == *started)
            }
        });
        if before > 0 && state.holders.is_empty() {
            info!("the last holder is gone");
            state.close_requested = true;
        }
        let idle_over = agent_pid.is_some()
            && state.holders.is_empty()
            && state.busy.is_none()
            && state.idle_since.elapsed() >= idle;
        if idle_over {
            info!(
                idle_ms = idle.as_millis() as u64,
                "the live agent stayed idle"
            );
        }
        if (state.close_requested || idle_over) && state.busy.is_none() {
            drop(state);
            shared.shutdown.cancel();
            return;
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// Одно подключение — один запрос.
async fn handle(shared: Arc<Shared>, stream: Stream) {
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read);
    let request = match read_line::<_, Request>(&mut reader).await {
        Ok(Some(request)) => request,
        Ok(None) => return,
        Err(error) => {
            let _ = write_line(
                &mut write,
                &Frame::Error {
                    error: KeeperError::Protocol {
                        message: error.to_string(),
                    },
                },
            )
            .await;
            return;
        }
    };
    let answer = match request {
        Request::Status => match shared.ready().await {
            Ok((agent_pid, fingerprint)) => {
                let state = shared.lock();
                Frame::Status {
                    status: KeeperStatus {
                        keeper_pid: std::process::id(),
                        agent_pid,
                        fingerprint,
                        busy: state.busy.clone(),
                        holders: state.holders.clone(),
                    },
                }
            }
            Err(error) => Frame::Error { error },
        },
        Request::Hold { holder } => match shared.ready().await {
            Ok(_) => {
                let mut state = shared.lock();
                match state.holders.iter_mut().find(|held| held.same(&holder)) {
                    // Повторный `hold` второй отметки не ставит: срок продлевается, процесс
                    // остаётся тем же (`INV.CLI.AN-AGENT-HOLD-IS-IDEMPOTENT-PER-HOLDER`).
                    Some(held) => *held = holder,
                    None => state.holders.push(holder),
                }
                Frame::Holders {
                    holders: state.holders.clone(),
                }
            }
            Err(error) => Frame::Error { error },
        },
        Request::Release { holder } => {
            let mut state = shared.lock();
            let before = state.holders.len();
            state.holders.retain(|held| !held.same(&holder));
            if before > 0 && state.holders.is_empty() {
                // После последнего `release` простой считается с этого мгновения.
                state.idle_since = Instant::now();
            }
            Frame::Holders {
                holders: state.holders.clone(),
            }
        }
        Request::Stop => {
            {
                let mut state = shared.lock();
                state.close_requested = true;
                if let Some((cancel, false)) = &state.current {
                    cancel.cancel();
                }
            }
            let mut stopped = shared.stopped.subscribe();
            let _ = stopped.wait_for(|stopped| *stopped).await;
            Frame::Stopped
        }
        Request::Run {
            command,
            critical,
            credentials,
        } => {
            run(
                &shared,
                &mut reader,
                &mut write,
                command,
                critical,
                credentials,
            )
            .await;
            return;
        }
    };
    let _ = write_line(&mut write, &answer).await;
}

/// `run`: команда идёт в очередь потока агента, кадры ответа — клиенту по мере прихода.
/// Клиент ушёл посреди некритической команды — она брошена, и агента снимают; посреди
/// критической — её доводят до конца и снимают агента после
/// (`INV.PLATFORM.AN-INTERRUPTED-COMMAND-REMOVES-THE-LIVE-AGENT`).
async fn run<R, W>(
    shared: &Shared,
    reader: &mut R,
    write: &mut W,
    command: String,
    critical: bool,
    credentials: String,
) where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let refusal = match shared.ready().await {
        Err(error) => Some(error),
        Ok(_) if credentials != shared.digest => {
            shared.request_close("a command came with other credentials");
            Some(KeeperError::CredentialsMismatch)
        }
        Ok(_) => None,
    };
    if let Some(error) = refusal {
        let _ = write_line(write, &Frame::Error { error }).await;
        return;
    }
    let (frames, mut replies) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let started = Arc::new(AtomicBool::new(false));
    let queued = shared.send(Job::Run {
        command,
        critical,
        cancel: cancel.clone(),
        started: Arc::clone(&started),
        frames,
    });
    if !queued {
        let _ = write_line(
            write,
            &Frame::Error {
                error: KeeperError::Closing,
            },
        )
        .await;
        return;
    }
    let mut gone = false;
    let mut probe = [0u8; 64];
    loop {
        if gone {
            match replies.recv().await {
                Some(frame) if !frame.is_terminal() => continue,
                _ => return,
            }
        }
        tokio::select! {
            frame = replies.recv() => match frame {
                None => return,
                Some(frame) => {
                    let end = frame.is_terminal();
                    if write_line(write, &frame).await.is_err() {
                        gone = true;
                        client_gone(shared, &cancel, &started, critical);
                    } else if end {
                        return;
                    }
                }
            },
            read = reader.read(&mut probe) => {
                if matches!(read, Ok(0) | Err(_)) {
                    gone = true;
                    client_gone(shared, &cancel, &started, critical);
                }
            }
        }
    }
}

/// Клиент ушёл, не дождавшись ответа. Команда, которой агент ещё не получал, просто
/// снимается с очереди; начатая — прерывает агента.
fn client_gone(shared: &Shared, cancel: &CancellationToken, started: &AtomicBool, critical: bool) {
    let mut state = shared.lock();
    if started.load(Ordering::Acquire) {
        if !critical {
            cancel.cancel();
        }
        if !state.close_requested {
            info!(critical, "the client left in the middle of a command");
        }
        state.close_requested = true;
    } else {
        cancel.cancel();
    }
}

/// Запись сторожа: он сам и, когда поднят, его агент. Пишется под замком реестра.
fn write_record(
    home: &KeeperHome,
    port: u16,
    agent: Option<LiveAgentFacts>,
) -> std::io::Result<()> {
    let _registry = lock_agent_registry(home.state())?;
    let own = std::process::id();
    let facts = process_facts(own);
    let record = ServiceRecord {
        state: RecordState::Running,
        pid: Some(own),
        started: facts.as_ref().map(|facts| facts.started),
        image: facts.map(|facts| facts.image),
        machine: machine_fingerprint(),
        starter: None,
        port,
        explicit: false,
        bin: std::env::current_exe().unwrap_or_default(),
        version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        agent,
    };
    write_json(&home.paths().record(), &record)
}

/// Убирает запись, если она всё ещё своя: следующий сторож мог уже записать свою.
fn remove_record(home: &KeeperHome) {
    let Ok(_registry) = lock_agent_registry(home.state()) else {
        return;
    };
    let path = home.paths().record();
    if read_json::<ServiceRecord>(&path)
        .is_some_and(|record| record.pid == Some(std::process::id()))
    {
        let _ = std::fs::remove_file(path);
    }
}
