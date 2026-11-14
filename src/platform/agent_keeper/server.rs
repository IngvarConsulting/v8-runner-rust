//! Сторож живого агента: скрытая подкоманда `v8-runner agent keeper --key <hex>`.
//!
//! Запуск агента и данные для него приходят одной строкой JSON в stdin ([`KeeperStart`]):
//! учётные данные базы не попадают в командную строку. Дом агента сторож находит по рабочей копии
//! из этой строки и своему ключу ([`KeeperHome::of`]). Сторож сначала занимает канал (второй
//! сторож того же ключа на нём отказывает), пишет свою запись, затем поднимает агента
//! ([`ManagedAgent::launch_for_keeper`]), отключает его от базы и держит его сессию SSH. Уходя,
//! он держит канал, пока агент не ушёл, затем убирает запись и дом агента (#735).
//!
//! Агентом владеет один долгоживущий поток: он поднимает процесс агента (на Linux сигнал смерти
//! родителя привязан к потоку, `INV.PLATFORM.A-LIVE-AGENT-DIES-WITH-ITS-KEEPER`) и выполняет
//! работу по одной: подключение к базе в начале сессии команды, команды, отключение в конце.
//! Подключения обслуживают задачи `tokio`, каждое своё: `status`, `hold` и `release` отвечают и
//! во время долгого `run`. Срок простоя считает тикер
//! (`INV.PLATFORM.A-LIVE-AGENT-CLOSES-WHEN-NOBODY-HOLDS-IT`, `…-CLOSES-WITH-ITS-HOLDER`).
//!
//! Подключение агента к базе живёт дольше сессии команды
//! (`INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS`, #644): следующая сессия берёт
//! его без `common connect-ib`. Отключают агента `disconnect` (команда, которой база нужна
//! монопольно), уход команды без `end` и снятие агента.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::channel::{Listener, Stream};
use super::client::KeeperStart;
use super::protocol::{
    read_line, write_line, AgentFailure, Frame, Holder, KeeperError, KeeperStatus, Request,
};
use super::{idle_limit, KeeperHome};
use crate::platform::agent::{
    AgentError, AgentErrorType, AgentLaunch, AgentMessage, AtOpen, HostKeyExpectation,
    ManagedAgent, WaitPolicy,
};
use crate::platform::live_registry::Registry;
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
/// Сколько сторож ждёт первой строки подключения: молчащее подключение задачу не держит.
const FIRST_LINE: Duration = Duration::from_secs(10);
/// Сколько свежий агент, которого ещё никто не взял, не отдаётся условному `stop`: поднявший
/// его клиент успевает открыть сессию или поставить держателя, и два клиента на пределе не
/// снимают свежих агентов друг друга.
const FRESH_GRACE: Duration = Duration::from_secs(30);

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
    let registry = Registry::of_account()
        .map_err(|error| format!("the live agent registry is unknown: {error}"))?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|error| format!("cannot read the start request from stdin: {error}"))?;
    let start: KeeperStart = serde_json::from_str(line.trim_end())
        .map_err(|error| format!("the start request is not readable: {error}"))?;
    let home = KeeperHome::of(&registry, &start.launch.work_path, key)
        .map_err(|error| error.to_string())?;
    // Отпечаток учётных данных считает команда своей солью рабочей копии: сторож соли не читает.
    let digest = start.credentials.digest.clone();
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
        /// Время старта процесса агента: по pid без него жив ли тот же процесс, не сказать.
        agent_started: Option<u64>,
        fingerprint: Option<String>,
    },
    Failed(KeeperError),
}

/// Открытая сессия команды: агент подключён к базе.
struct Session {
    id: String,
    /// Последняя команда сессии.
    last: Option<String>,
    /// Сессия взяла подключение, оставшееся от прежней, и её команд агент ещё не выполнял:
    /// первая из них проверяет, живо ли оно ([`run_command`]).
    reused: bool,
}

/// Что знает только сторож.
struct State {
    /// Команда, которую агент выполняет сейчас.
    busy: Option<String>,
    /// Отмена выполняемой команды и её класс: `stop` бросает только некритическую.
    current: Option<(CancellationToken, bool)>,
    session: Option<Session>,
    holders: Vec<Holder>,
    /// С какого мгновения агент простаивает: с подъёма, с конца сессии или с последнего
    /// `release`.
    idle_since: Instant,
    /// Агента снимут, как только кончится сессия команды.
    close_requested: bool,
    /// `stop`: агента снимут, как только он не выполняет команду, и с открытой сессией тоже.
    stop_requested: bool,
    /// Когда подъём агента не удался.
    failed_at: Option<Instant>,
    /// До какого мгновения свежий агент, которого ещё не взяли ни `begin`, ни `hold`, не
    /// снимается условным `stop`.
    fresh_until: Option<Instant>,
    /// Агент подключён к базе: в сессии команды или между командами.
    attached: bool,
    /// Подключение под подозрением: первая критическая команда на нём упала, и следующая
    /// сессия подключается заново.
    reconnect: bool,
    /// Подключался ли агент к базе хоть раз: при подъёме или сессией команды. Агент, поднятый
    /// без подключения (#574), чьё первое подключение не удалось, не годится и снимается, как
    /// агент, который не поднялся: сеанс лицензии он зря не держит.
    ever_connected: bool,
}

/// Работа потоку агента.
enum Job {
    Begin {
        session: String,
        reply: oneshot::Sender<Result<Vec<AgentMessage>, KeeperError>>,
    },
    Run {
        session: String,
        command: String,
        critical: bool,
        cancel: CancellationToken,
        started: Arc<AtomicBool>,
        frames: mpsc::UnboundedSender<Frame>,
    },
    End {
        session: String,
        reply: Option<oneshot::Sender<()>>,
        how: SessionEnd,
    },
    Attach {
        reply: oneshot::Sender<Result<Vec<AgentMessage>, KeeperError>>,
    },
    Disconnect {
        reply: oneshot::Sender<Result<bool, KeeperError>>,
    },
    Close,
}

/// Как кончилась сессия команды.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionEnd {
    /// Команда закрыла её строкой `end`: агент остаётся подключённым к базе.
    Closed,
    /// Команда ушла без `end`: в каком состоянии она оставила агента, неизвестно, и его
    /// отключают от базы.
    Abandoned,
}

/// Что сессия команды делает с подключением агента в начале.
enum Connection {
    /// Берёт подключение, оставшееся от прежней сессии.
    Kept,
    /// Подключение под подозрением: агента отключают и подключают заново.
    Suspect,
    /// Агент не подключён: его подключают.
    Absent,
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
                ..
            } => Ok((agent_pid, fingerprint)),
            Readiness::Failed(error) => Err(error),
            Readiness::Launching => unreachable!("waited past launching"),
        }
    }

    /// Готов ли сторож взять работу: агент поднят, учётные данные те же, и агента не снимают.
    /// Команде уже открытой сессии (`within_session`) агент служит и тогда, когда его решили
    /// снять после сессии; отказывают ей только `stop` и чужие учётные данные.
    async fn open_for_work(
        &self,
        credentials: &str,
        within_session: bool,
    ) -> Result<(), KeeperError> {
        self.ready().await?;
        if credentials != self.digest {
            self.request_close("a command came with other credentials");
            return Err(KeeperError::CredentialsMismatch);
        }
        let state = self.lock();
        if state.stop_requested
            || self.shutdown.is_cancelled()
            || (state.close_requested && !within_session)
        {
            return Err(KeeperError::Closing);
        }
        Ok(())
    }
}

async fn keep(home: KeeperHome, start: KeeperStart, digest: String) -> Result<(), String> {
    home.registry()
        .prepare()
        .map_err(|error| format!("cannot prepare the keeper channel: {error}"))?;
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
            session: None,
            holders: Vec::new(),
            idle_since: Instant::now(),
            close_requested: false,
            stop_requested: false,
            failed_at: None,
            fresh_until: None,
            attached: false,
            reconnect: false,
            ever_connected: false,
        }),
        readiness,
        jobs: Mutex::new(jobs),
        shutdown: CancellationToken::new(),
        stopped,
    });

    let (worker_done, mut worker_finished) = tokio::sync::oneshot::channel::<()>();
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

    // Поток агента снимает агента после команды, которую уже ведёт. Канал остаётся открытым,
    // пока агент не ушёл: сторож всё ещё занимает место на пределе учётной записи (его агент
    // держит сеанс лицензии и базу) и называет себя уходящим; команда, нашедшая его, ждёт его
    // ухода и поднимает нового.
    shared.send(Job::Close);
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(stream) => {
                    tokio::spawn(handle(Arc::clone(&shared), stream));
                }
                Err(error) => warn!(%error, "the keeper channel refused a connection"),
            },
            _ = &mut worker_finished => break,
        }
    }
    let _ = worker.join();
    let home = if shared.lock().failed_at.is_none() {
        HomeAfter::Removed
    } else {
        HomeAfter::KeptAsEvidence
    };
    // Уборка дома ждёт файлов, которые уходящий агент ещё держит: не в потоке, который
    // принимает подключения.
    let leaving = Arc::clone(&shared);
    let _ = tokio::task::spawn_blocking(move || remove_record(&leaving.home, home)).await;
    let _ = shared.stopped.send(true);
    // Ответы `stop` уходят в канал, затем канал закрывается.
    let flush = tokio::time::sleep(STOP_FLUSH);
    tokio::pin!(flush);
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                if let Ok(stream) = accepted {
                    tokio::spawn(handle(Arc::clone(&shared), stream));
                }
            }
            () = &mut flush => break,
        }
    }
    listener.remove();
    drop(listener);
    info!("the keeper is done");
    Ok(())
}

/// Отказ, после которого сессия SSH агента следующей работе не годится.
fn breaks_the_session(error: &AgentError) -> bool {
    matches!(
        error,
        AgentError::SessionClosed { .. }
            | AgentError::Transport { .. }
            | AgentError::InvalidReply { .. }
            | AgentError::Cancelled { .. }
            | AgentError::TimedOut { .. }
    )
}

/// Поток агента: поднимает агента и выполняет работу по одной, затем отпускает и снимает его.
fn work(shared: &Shared, start: KeeperStart, queue: &std::sync::mpsc::Receiver<Job>) {
    let infobase = start.launch.infobase.clone();
    let work_path = Some(start.launch.work_path.display().to_string());
    let at_open = start.at_open;
    let mut agent = match launch(&shared.home, start) {
        Ok(launched) => launched,
        Err(error) => {
            warn!(%error, "the live agent did not start");
            shared.lock().failed_at = Some(Instant::now());
            shared.request_close("the agent did not start");
            let failure = KeeperError::Launch {
                failure: AgentFailure::from_agent(error),
            };
            let _ = shared.readiness.send(Readiness::Failed(failure.clone()));
            for job in queue {
                match job {
                    Job::Begin { reply, .. } => {
                        let _ = reply.send(Err(failure.clone()));
                    }
                    Job::Run { frames, .. } => {
                        let _ = frames.send(Frame::Error {
                            error: failure.clone(),
                        });
                    }
                    Job::End { reply, .. } => {
                        if let Some(reply) = reply {
                            let _ = reply.send(());
                        }
                    }
                    Job::Attach { reply } => {
                        let _ = reply.send(Err(failure.clone()));
                    }
                    Job::Disconnect { reply } => {
                        let _ = reply.send(Ok(false));
                    }
                    Job::Close => break,
                }
            }
            return;
        }
    };
    // Агент, поднятый с подключением, остаётся подключённым: первая сессия команды берёт его
    // без `common connect-ib` (#644).
    let fingerprint = match agent.host_key_expectation() {
        HostKeyExpectation::Pinned(fingerprint) => Some(fingerprint.to_string()),
        HostKeyExpectation::Unpinned => None,
    };
    let agent_pid = agent.pid();
    let agent_started = agent_pid.and_then(process_facts).map(|facts| facts.started);
    let facts = LiveAgentFacts {
        pid: agent_pid,
        started: agent_started,
        fingerprint: fingerprint.clone(),
        channel: shared.home.channel(),
        infobase,
        work_path,
    };
    if let Err(error) = write_record(&shared.home, agent.endpoint().port, Some(facts)) {
        warn!(%error, "cannot write the live agent record");
    }
    {
        let mut state = shared.lock();
        state.idle_since = Instant::now();
        state.fresh_until = Some(Instant::now() + FRESH_GRACE);
        state.ever_connected = at_open == AtOpen::Connect;
        state.attached = at_open == AtOpen::Connect;
    }
    let _ = shared.readiness.send(Readiness::Ready {
        agent_pid,
        agent_started,
        fingerprint,
    });
    info!(pid = ?agent_pid, "the live agent is up");

    for job in queue {
        match job {
            Job::Close => break,
            Job::Begin { session, reply } => {
                let _ = reply.send(begin_session(shared, &mut agent, session));
            }
            Job::Run {
                session,
                command,
                critical,
                cancel,
                started,
                frames,
            } => {
                let job = RunJob {
                    session,
                    command,
                    critical,
                    cancel,
                    started,
                };
                if let Some(frame) = run_command(shared, &mut agent, job, &frames) {
                    let _ = frames.send(frame);
                }
            }
            Job::End {
                session,
                reply,
                how,
            } => {
                end_session(shared, &mut agent, &session, how);
                if let Some(reply) = reply {
                    let _ = reply.send(());
                }
            }
            Job::Attach { reply } => {
                let _ = reply.send(attach(shared, &mut agent));
            }
            Job::Disconnect { reply } => {
                let _ = reply.send(disconnect(shared, &mut agent));
            }
        }
    }
    agent.release_and_remove();
    info!("the live agent is released and removed");
}

/// Начало сессии команды: агент подключается к базе, если ещё не подключён. Ответ — то, что
/// агент сказал при подключении (пусто, если подключение осталось от прежней сессии): команда
/// пишет это в свой журнал.
fn begin_session(
    shared: &Shared,
    agent: &mut ManagedAgent,
    session: String,
) -> Result<Vec<AgentMessage>, KeeperError> {
    let connection = {
        let mut state = shared.lock();
        if state.close_requested || state.stop_requested {
            return Err(KeeperError::Closing);
        }
        if let Some(open) = &state.session {
            return Err(KeeperError::Busy {
                command: open.last.clone(),
            });
        }
        // Сессия открыта с этой минуты: простой её не снимет и пока агент подключается.
        state.fresh_until = None;
        let connection = match (state.attached, std::mem::take(&mut state.reconnect)) {
            (true, false) => Connection::Kept,
            (true, true) => Connection::Suspect,
            (false, _) => Connection::Absent,
        };
        state.session = Some(Session {
            id: session,
            last: None,
            reused: matches!(connection, Connection::Kept),
        });
        connection
    };
    match connection {
        Connection::Kept => {
            debug!("the command session takes the connection left by the previous one");
            return Ok(Vec::new());
        }
        Connection::Suspect => drop_connection(shared, agent),
        Connection::Absent => {}
    }
    connect(shared, agent).inspect_err(|_| {
        let mut state = shared.lock();
        state.session = None;
        state.idle_since = Instant::now();
    })
}

/// Подключает агента к базе (`common connect-ib`). Отказ у агента, который ни разу не
/// подключался (#574), снимает его, как не поднявшегося: сеанса лицензии он зря не держит.
fn connect(shared: &Shared, agent: &mut ManagedAgent) -> Result<Vec<AgentMessage>, KeeperError> {
    let policy = WaitPolicy::for_keeper(CancellationToken::new(), false);
    let mut said = Vec::new();
    match agent
        .session()
        .connect_infobase_reporting(&policy, &mut |batch| said.extend_from_slice(batch))
    {
        Ok(()) => {
            let mut state = shared.lock();
            state.ever_connected = true;
            state.attached = true;
            state.reconnect = false;
            Ok(said)
        }
        Err(error) => {
            let never_connected = !shared.lock().ever_connected;
            if breaks_the_session(&error) {
                shared.request_close("the agent session broke off");
            } else if never_connected {
                shared.request_close("the agent raised without a connection could not connect");
            }
            Err(KeeperError::from_agent(error))
        }
    }
}

/// `attach`: агент подключается к базе без сессии команды (#644). Пока он подключается, он
/// занят: `disconnect` и условный `stop` его не трогают.
fn attach(shared: &Shared, agent: &mut ManagedAgent) -> Result<Vec<AgentMessage>, KeeperError> {
    {
        let mut state = shared.lock();
        if state.close_requested || state.stop_requested {
            return Err(KeeperError::Closing);
        }
        // Свежий агент остаётся свежим: сессию команда откроет, когда разбор найдёт работу, и
        // до тех пор условный `stop` другой команды на пределе его не снимает.
        if state.attached || state.session.is_some() {
            return Ok(Vec::new());
        }
        state.busy = Some(CONNECT.to_owned());
    }
    let outcome = connect(shared, agent);
    let mut state = shared.lock();
    state.busy = None;
    state.idle_since = Instant::now();
    outcome
}

/// `disconnect`: агент отключается от базы, сам остаётся. Условие проверяется здесь, в потоке
/// агента: сессия, вставшая в очередь раньше, уже открыта, и её подключение не снимается.
fn disconnect(shared: &Shared, agent: &mut ManagedAgent) -> Result<bool, KeeperError> {
    {
        let state = shared.lock();
        if state.busy.is_some() || state.session.is_some() {
            return Err(KeeperError::NotIdle {
                command: state
                    .busy
                    .clone()
                    .or_else(|| state.session.as_ref().and_then(|open| open.last.clone())),
                holders: state.holders.clone(),
            });
        }
        if !state.attached {
            return Ok(false);
        }
    }
    drop_connection(shared, agent);
    info!("the live agent is disconnected from the infobase on request");
    Ok(true)
}

/// Отключает агента от базы. Отказ, после которого сессия SSH не годится, снимает агента.
/// Агент, который отключения не подтвердил, а сессию сохранил, считается подключённым: следующее
/// подключение он примет ответом «уже подключён».
fn drop_connection(shared: &Shared, agent: &mut ManagedAgent) {
    let disconnected = match agent
        .session()
        .disconnect_infobase(&WaitPolicy::for_keeper_release())
    {
        Ok(()) => true,
        Err(error) => {
            warn!(%error, "the live agent did not confirm common disconnect-ib");
            let broken = breaks_the_session(&error);
            if broken {
                shared.request_close("the agent session broke off");
            }
            broken
        }
    };
    let mut state = shared.lock();
    if disconnected {
        state.attached = false;
    }
    state.reconnect = false;
}

/// Команда подключения: ею занят агент, пока `attach` его подключает.
const CONNECT: &str = "common connect-ib";

/// Конец сессии команды. Агент остаётся подключённым к базе (#644); команда, ушедшая без `end`
/// ([`SessionEnd::Abandoned`]), могла оставить агента в любом состоянии, и сторож отключает его. Агента,
/// которого снимают, не отключают: его сессия могла оборваться посреди ответа, а снятие само
/// его отпустит.
fn end_session(shared: &Shared, agent: &mut ManagedAgent, session: &str, how: SessionEnd) {
    let removing = {
        let state = shared.lock();
        if state.session.as_ref().is_none_or(|open| open.id != session) {
            return;
        }
        state.close_requested || state.stop_requested
    };
    if how == SessionEnd::Abandoned && !removing {
        drop_connection(shared, agent);
    }
    let mut state = shared.lock();
    state.session = None;
    state.idle_since = Instant::now();
    debug!("the command session is over");
}

struct RunJob {
    session: String,
    command: String,
    critical: bool,
    cancel: CancellationToken,
    started: Arc<AtomicBool>,
}

/// Команда в сессии: кадры ответа уходят по мере прихода, итоговый возвращается.
fn run_command(
    shared: &Shared,
    agent: &mut ManagedAgent,
    job: RunJob,
    frames: &mpsc::UnboundedSender<Frame>,
) -> Option<Frame> {
    let RunJob {
        session,
        command,
        critical,
        cancel,
        started,
    } = job;
    let first_on_kept = {
        let mut state = shared.lock();
        if cancel.is_cancelled() {
            return None;
        }
        if state.stop_requested {
            return Some(Frame::Error {
                error: KeeperError::Closing,
            });
        }
        let first_on_kept = match state.session.as_mut() {
            Some(open) if open.id == session => {
                open.last = Some(command.clone());
                std::mem::take(&mut open.reused)
            }
            _ => {
                return Some(Frame::Error {
                    error: KeeperError::NoSession,
                })
            }
        };
        started.store(true, Ordering::Release);
        state.busy = Some(command.clone());
        state.current = Some((cancel.clone(), critical));
        first_on_kept
    };
    debug!(%command, critical, "running a command for a client");
    let policy = WaitPolicy::for_keeper(cancel, critical);
    let forward = |batch: &[AgentMessage]| {
        let _ = frames.send(Frame::Batch {
            messages: batch.to_vec(),
        });
    };
    // Первая некритическая команда на подключении, оставшемся от прежней сессии, проверяет, живо
    // ли оно: подключение могло устареть (сеанс завершили, сервер перезапустили). Её ответ из
    // одного массива придерживается, пока не ясен исход (#644).
    let heal = first_on_kept && !critical;
    let mut held: Option<Vec<AgentMessage>> = None;
    let mut forwarded = false;
    let mut result = agent
        .session()
        .run_streaming(&command, &policy, &mut |batch| {
            if heal && !forwarded && held.is_none() {
                held = Some(batch.to_vec());
                return;
            }
            if let Some(held) = held.take() {
                forward(&held);
            }
            forwarded = true;
            forward(batch);
        });
    let stale = matches!(
        &result,
        Ok(reply) if matches!(
            reply.outcome(),
            Err(AgentError::Command {
                error_type: AgentErrorType::DesignerNotConnectedToInfoBase,
                ..
            })
        )
    );
    if heal && !forwarded && stale {
        warn!(%command, "the first command on the kept connection failed: the agent reconnects to the infobase and repeats it once");
        drop_connection(shared, agent);
        let closing = {
            let state = shared.lock();
            state.close_requested || state.stop_requested
        };
        // Сторож уже решил снять агента: его сессию не переподключают.
        let reconnected = if closing {
            Err(KeeperError::Closing)
        } else {
            connect(shared, agent).map(drop)
        };
        result = match reconnected {
            Ok(()) => agent
                .session()
                .run_streaming(&command, &policy, &mut |batch| forward(batch)),
            Err(error) => {
                let mut state = shared.lock();
                state.busy = None;
                state.current = None;
                return Some(Frame::Error { error });
            }
        };
    } else {
        if let Some(held) = held.take() {
            forward(&held);
        }
        // Критическую команду не повторяют, некритическую с ответом из нескольких массивов —
        // тоже (часть ответа уже у команды); если такая первой упала на старом подключении,
        // следующая сессия подключается заново.
        if first_on_kept && stale {
            warn!(%command, "the first command on the kept connection failed: the next session reconnects to the infobase");
            shared.lock().reconnect = true;
        }
    }
    let frame = match result {
        Ok(_) => Frame::Done,
        Err(error) => {
            // Сессия, оборванная посреди ответа, уже не годится следующей команде.
            if breaks_the_session(&error) {
                shared.request_close("the agent session broke off");
            }
            Frame::Error {
                error: KeeperError::from_agent(error),
            }
        }
    };
    let mut state = shared.lock();
    state.busy = None;
    state.current = None;
    Some(frame)
}

/// Поднимает агента ключа и открывает его сессию.
fn launch(home: &KeeperHome, start: KeeperStart) -> Result<ManagedAgent, AgentError> {
    let KeeperStart {
        launch,
        credentials,
        at_open,
    } = start;
    let root = home.paths().root();
    let agent = AgentLaunch {
        v8: launch.v8,
        infobase_args: launch.infobase_args,
        port: launch.port,
        host_key: launch.host_key,
        host_key_dir: home.host_keys(),
        base_dir: home.base_dir(),
        workdir: home.workdir(),
        process_log: root.join("agent.process"),
        user: credentials.user,
        password: credentials.password,
        // Журнал разговора пишет команда у себя: сторож журнала запросов не ведёт.
        transcript_log: None,
    };
    ManagedAgent::launch_for_keeper(
        &ProcessExecutor,
        agent,
        Duration::from_millis(launch.startup_timeout_ms.max(1)),
        &WaitPolicy::for_keeper(CancellationToken::new(), false),
        at_open,
    )
}

/// Жив ли процесс `pid`, стартовавший в `started`: повторно выданный pid тем же процессом
/// не считается.
fn alive(pid: u32, started: Option<u64>) -> bool {
    process_facts(pid).is_some_and(|facts| started.is_none_or(|started| facts.started == started))
}

/// Убран ли дом агента: нет записи сторожа или `AgentBaseDir`.
fn home_is_gone(home: &KeeperHome) -> bool {
    !home.paths().record().is_file() || !home.base_dir().is_dir()
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
        let agent = match &*shared.readiness.borrow() {
            Readiness::Ready {
                agent_pid,
                agent_started,
                ..
            } => Some((*agent_pid, *agent_started)),
            _ => None,
        };
        // Процесс агента, которого уже нет, никому не служит.
        if poll {
            if let Some((Some(pid), started)) = agent {
                if !alive(pid, started) {
                    shared.request_close("the agent process is gone");
                }
            }
            // Дом агента убрали, пока сторож жив (`git clean`, удалённый `workPath`): без записи
            // команды его не найдут, а без `AgentBaseDir` агент им не годится. Сторож уходит сам,
            // доведя начатое, и следующая команда поднимает нового агента (#735).
            if agent.is_some() && home_is_gone(&shared.home) {
                shared.request_close("the agent's home in the working copy is gone");
            }
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
            Holder::Process { pid, started } => !poll || alive(*pid, Some(*started)),
        });
        if before > 0 && state.holders.is_empty() {
            info!("the last holder is gone");
            state.close_requested = true;
        }
        // Простой — только время без сессии команды: пока команда держит агента, он занят,
        // даже между её запросами.
        let idle_over = agent.is_some()
            && state.holders.is_empty()
            && state.busy.is_none()
            && state.session.is_none()
            && state.idle_since.elapsed() >= idle;
        if idle_over && !state.close_requested {
            info!(
                idle_ms = idle.as_millis() as u64,
                "the live agent stayed idle"
            );
            // Запрос, который уже стоит в очереди, получит ясный отказ, а не начнётся и не
            // оборвётся снятием.
            state.close_requested = true;
        }
        let free = state.busy.is_none() && (state.session.is_none() || state.stop_requested);
        if free && (state.close_requested || state.stop_requested) {
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
    let request = match tokio::time::timeout(FIRST_LINE, read_line::<_, Request>(&mut reader)).await
    {
        Ok(Ok(Some(request))) => request,
        Ok(Ok(None)) | Err(_) => return,
        Ok(Err(error)) => {
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
        Request::Whereabouts => {
            let state = shared.lock();
            Frame::Whereabouts {
                key: shared.home.key().to_owned(),
                home: shared.home.paths().root().to_path_buf(),
                pid: std::process::id(),
                closing: state.close_requested
                    || state.stop_requested
                    || shared.shutdown.is_cancelled(),
            }
        }
        Request::Status => match shared.ready().await {
            Ok((agent_pid, fingerprint)) => {
                let state = shared.lock();
                Frame::Status {
                    status: KeeperStatus {
                        keeper_pid: std::process::id(),
                        agent_pid,
                        fingerprint,
                        busy: state.busy.clone(),
                        connected: state.session.is_some(),
                        attached: state.attached,
                        last: state.session.as_ref().and_then(|open| open.last.clone()),
                        holders: state.holders.clone(),
                        closing: state.close_requested
                            || state.stop_requested
                            || shared.shutdown.is_cancelled(),
                        fresh: state
                            .fresh_until
                            .is_some_and(|until| Instant::now() < until),
                        idle_ms: if state.busy.is_none() && state.session.is_none() {
                            u64::try_from(state.idle_since.elapsed().as_millis())
                                .unwrap_or(u64::MAX)
                        } else {
                            0
                        },
                    },
                }
            }
            Err(error) => Frame::Error { error },
        },
        Request::Hold {
            holder,
            credentials,
        } => hold(&shared, holder, credentials.as_deref()).await,
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
        Request::Stop {
            only_idle,
            only_unheld,
        } => match stop(&shared, only_idle, only_unheld) {
            Some(refusal) => refusal,
            None => {
                let mut stopped = shared.stopped.subscribe();
                let _ = stopped.wait_for(|stopped| *stopped).await;
                Frame::Stopped
            }
        },
        Request::Begin { credentials } => {
            begin(&shared, &mut reader, &mut write, credentials).await;
            return;
        }
        Request::Attach { credentials } => attach_request(&shared, &credentials).await,
        Request::Disconnect => disconnect_request(&shared).await,
        Request::Run {
            session,
            command,
            critical,
            credentials,
        } => {
            let job = RunRequest {
                session,
                command,
                critical,
                credentials,
            };
            run(&shared, &mut reader, &mut write, job).await;
            return;
        }
        Request::End | Request::Cancel => Frame::Error {
            error: KeeperError::Protocol {
                message: "end and cancel answer only within a begin or a run".to_owned(),
            },
        },
    };
    let _ = write_line(&mut write, &answer).await;
}

/// `stop`: условие и решение снять — под одним замком состояния, поэтому команда не откроет
/// сессию между проверкой и снятием. Ответ — отказ, если условие не выполнено.
fn stop(shared: &Shared, only_idle: bool, only_unheld: bool) -> Option<Frame> {
    let mut state = shared.lock();
    let fresh = state
        .fresh_until
        .is_some_and(|until| Instant::now() < until);
    let in_use = only_idle && (state.busy.is_some() || state.session.is_some() || fresh);
    let held = only_unheld && !state.holders.is_empty();
    if in_use || held {
        return Some(Frame::Error {
            error: KeeperError::NotIdle {
                command: state
                    .busy
                    .clone()
                    .or_else(|| state.session.as_ref().and_then(|open| open.last.clone())),
                holders: state.holders.clone(),
            },
        });
    }
    state.stop_requested = true;
    state.close_requested = true;
    if let Some((cancel, false)) = &state.current {
        cancel.cancel();
    }
    None
}

/// `hold`: процесс, которого нет, держателем не становится. Учётные данные не те, с которыми
/// поднят агент, — отказ, и агента снимают, как при `begin`; агента, которого уже снимают,
/// отметка не удержит — отказ [`KeeperError::Closing`], и клиент ждёт нового сторожа.
async fn hold(shared: &Shared, holder: Holder, credentials: Option<&str>) -> Frame {
    if let Err(error) = shared.ready().await {
        return Frame::Error { error };
    }
    if credentials.is_some_and(|credentials| credentials != shared.digest) {
        shared.request_close("a hold came with other credentials");
        return Frame::Error {
            error: KeeperError::CredentialsMismatch,
        };
    }
    if let Holder::Process { pid, started } = &holder {
        if !alive(*pid, Some(*started)) {
            return Frame::Error {
                error: KeeperError::HolderGone,
            };
        }
    }
    let mut state = shared.lock();
    if state.close_requested || state.stop_requested || shared.shutdown.is_cancelled() {
        return Frame::Error {
            error: KeeperError::Closing,
        };
    }
    match state.holders.iter_mut().find(|held| held.same(&holder)) {
        // Повторный `hold` второй отметки не ставит, а срок не укорачивает: остаётся
        // поздний из двух (`INV.CLI.AN-AGENT-HOLD-IS-IDEMPOTENT-PER-HOLDER`).
        Some(Holder::Ttl { until_ms, .. }) => {
            if let Holder::Ttl {
                until_ms: requested,
                ..
            } = holder
            {
                *until_ms = (*until_ms).max(requested);
            }
        }
        Some(Holder::Process { .. }) => {}
        None => state.holders.push(holder),
    }
    state.fresh_until = None;
    Frame::Holders {
        holders: state.holders.clone(),
    }
}

/// `attach`: агент подключается к базе без сессии команды. Агента, которого снимают, не
/// подключают.
async fn attach_request(shared: &Shared, credentials: &str) -> Frame {
    if let Err(error) = shared.open_for_work(credentials, false).await {
        return Frame::Error { error };
    }
    let (reply, answer) = oneshot::channel();
    if !shared.send(Job::Attach { reply }) {
        return Frame::Error {
            error: KeeperError::Closing,
        };
    }
    match answer.await.unwrap_or(Err(KeeperError::Closing)) {
        Ok(messages) => Frame::Attached { messages },
        Err(error) => Frame::Error { error },
    }
}

/// `disconnect`: агент отключается от базы, если не занят. Учётных данных не нужно: отключение
/// у базы ничего не берёт.
async fn disconnect_request(shared: &Shared) -> Frame {
    if shared.ready().await.is_err() {
        return Frame::Disconnected { was: false };
    }
    let (reply, answer) = oneshot::channel();
    if !shared.send(Job::Disconnect { reply }) {
        return Frame::Disconnected { was: false };
    }
    match answer.await.unwrap_or(Ok(false)) {
        Ok(was) => Frame::Disconnected { was },
        Err(error) => Frame::Error { error },
    }
}

/// `begin`: сессия команды живёт, пока открыто её подключение. `end` по нему закрывает её с
/// ответом; разрыв или чужая строка закрывают её без ответа — команда ушла.
async fn begin<R, W>(shared: &Shared, reader: &mut R, write: &mut W, credentials: String)
where
    R: tokio::io::AsyncBufRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    if let Err(error) = shared.open_for_work(&credentials, false).await {
        let _ = write_line(write, &Frame::Error { error }).await;
        return;
    }
    let session = uuid::Uuid::new_v4().simple().to_string();
    let (reply, answer) = oneshot::channel();
    let outcome = if shared.send(Job::Begin {
        session: session.clone(),
        reply,
    }) {
        answer.await.unwrap_or(Err(KeeperError::Closing))
    } else {
        Err(KeeperError::Closing)
    };
    let messages = match outcome {
        Ok(messages) => messages,
        Err(error) => {
            let _ = write_line(write, &Frame::Error { error }).await;
            return;
        }
    };
    if write_line(
        write,
        &Frame::Begun {
            session: session.clone(),
            messages,
        },
    )
    .await
    .is_err()
    {
        shared.send(Job::End {
            session,
            reply: None,
            how: SessionEnd::Abandoned,
        });
        return;
    }
    let mut stopped = shared.stopped.subscribe();
    let ended = tokio::select! {
        line = read_line::<_, Request>(reader) => matches!(line, Ok(Some(Request::End))),
        _ = stopped.wait_for(|stopped| *stopped) => return,
    };
    if ended {
        let (reply, done) = oneshot::channel();
        if shared.send(Job::End {
            session,
            reply: Some(reply),
            how: SessionEnd::Closed,
        }) {
            let _ = done.await;
        }
        let _ = write_line(write, &Frame::Ended).await;
    } else {
        info!("the client left without ending its session");
        shared.send(Job::End {
            session,
            reply: None,
            how: SessionEnd::Abandoned,
        });
    }
}

struct RunRequest {
    session: String,
    command: String,
    critical: bool,
    credentials: String,
}

/// `run`: команда идёт в очередь потока агента, кадры ответа — клиенту по мере прихода.
/// Клиент ушёл посреди некритической команды — она брошена, и агента снимают; посреди
/// критической — её доводят до конца и снимают агента после
/// (`INV.PLATFORM.AN-INTERRUPTED-COMMAND-REMOVES-THE-LIVE-AGENT`).
async fn run<R, W>(shared: &Shared, reader: &mut R, write: &mut W, request: RunRequest)
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let RunRequest {
        session,
        command,
        critical,
        credentials,
    } = request;
    if let Err(error) = shared.open_for_work(&credentials, true).await {
        let _ = write_line(write, &Frame::Error { error }).await;
        return;
    }
    let (frames, mut replies) = mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    let started = Arc::new(AtomicBool::new(false));
    let queued = shared.send(Job::Run {
        session,
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
            // Пока ответа нет, клиенту писать нечего: строка `cancel`, разрыв или любая
            // другая строка значат, что он ушёл.
            _ = reader.read(&mut probe) => {
                gone = true;
                client_gone(shared, &cancel, &started, critical);
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

/// Запись сторожа: он сам и, когда поднят, его агент. Пишется под замком ключа; дом, который
/// уборка успела убрать, заводится заново.
fn write_record(
    home: &KeeperHome,
    port: u16,
    agent: Option<LiveAgentFacts>,
) -> std::io::Result<()> {
    let _held = home.lock()?;
    home.prepare()?;
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
        location: None,
    };
    write_json(&home.paths().record(), &record)
}

/// Убирает запись, если она всё ещё своя: следующий сторож или команда, которая его поднимает,
/// могли уже записать свою. С записью уходит и дом агента (`AgentBaseDir`, текущий каталог,
/// ключ хоста, журналы процесса агента), если агент поднимался: дом не поднявшегося агента
/// остаётся уликой рядом с выводом сторожа. Чужая запись — дом не трогают.
fn remove_record(home: &KeeperHome, after: HomeAfter) {
    // Дома уже нет (рабочую копию убрали): убирать нечего, а замок завёл бы каталоги заново.
    if !home.paths().root().is_dir() {
        return;
    }
    let Ok(_held) = home.lock() else {
        return;
    };
    let path = home.paths().record();
    match read_json::<ServiceRecord>(&path) {
        Some(record) if record.pid == Some(std::process::id()) => {
            let _ = std::fs::remove_file(path);
        }
        Some(_) => return,
        None => {}
    }
    if after == HomeAfter::Removed {
        home.remove();
    }
}

/// Что сторож делает с домом агента, уходя.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HomeAfter {
    /// Агент поднимался: дом убирается.
    Removed,
    /// Агент не поднялся: дом остаётся уликой рядом с выводом сторожа.
    KeptAsEvidence,
}
