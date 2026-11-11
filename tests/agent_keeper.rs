//! Сторож живого агента (#577): настоящий процесс `v8-runner agent keeper` поднимает двойника
//! агента (поддельный `1cv8` и SSH-сервер двойника в процессе теста) и держит его сессию. Тест
//! говорит со сторожем его протоколом — построчным JSON по каналу сторожа, — как команда.
//!
//! У каждого теста свой каталог состояния (`V8_RUNNER_STATE_DIR`) и свой ключ; в конце сторожу
//! отдаётся `stop`, а если он не ответил, его процесс снимается.
#![cfg(any(unix, windows))]

mod support;

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use support::fake_agent::{
    collide_with_first_managed_launch, serve_managed_launches, FakeAgent, Hold, HoldReply,
    AGENT_PASSWORD,
};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Срок, за который сторож с коротким простоем обязан уйти.
const GONE_WITHIN: Duration = Duration::from_secs(20);

/// Двойник агента, поддельный `1cv8` и каталог состояния одного теста.
struct Bench {
    dir: TempDir,
    /// Каталог состояния: на Unix — короткий, в `/tmp`, потому что путь сокета сторожа
    /// ограничен, а временный каталог macOS длинен.
    state: TempDir,
    designer: PathBuf,
    args_log: PathBuf,
    pid_file: PathBuf,
    commands_log: PathBuf,
}

impl Bench {
    /// `held` — двойник держит `config dump-config-to-files`, пока тест его не отпустит.
    fn new(held: bool) -> Self {
        Self::with_server(held, |agent| serve_managed_launches(agent, None)).0
    }

    fn with_server<T>(held: bool, serve: impl FnOnce(FakeAgent) -> T) -> (Self, T) {
        let dir = tempfile::tempdir().expect("bench dir");
        let root = dir.path().to_path_buf();
        let hold = held.then(|| held_dump(&root));
        let mut agent = FakeAgent::new(
            true,
            root.join("commands.log"),
            None,
            root.join("base-dir.txt"),
            root.join("designer.pid"),
        );
        agent.any_credentials = true;
        agent.hold = hold;
        let server = serve(agent);
        let designer = write_designer(&root);
        let bench = Self {
            designer,
            args_log: root.join("designer.args"),
            pid_file: root.join("designer.pid"),
            commands_log: root.join("commands.log"),
            dir,
            state: if cfg!(unix) {
                tempfile::tempdir_in("/tmp").expect("state dir")
            } else {
                tempfile::tempdir().expect("state dir")
            },
        };
        (bench, server)
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn state(&self) -> PathBuf {
        self.state.path().to_path_buf()
    }

    fn hold(&self) -> Hold {
        held_dump(self.root())
    }

    /// Запуски поддельного `1cv8`: по строке на запуск.
    fn launches(&self) -> usize {
        fs::read_to_string(&self.args_log)
            .unwrap_or_default()
            .lines()
            .count()
    }

    /// Процесс поддельного агента (на Windows — его PowerShell).
    fn designer_pid(&self) -> u32 {
        assert!(
            wait_until(Duration::from_secs(30), || fs::read_to_string(
                &self.pid_file
            )
            .is_ok_and(|text| text.trim().parse::<u32>().is_ok())),
            "the fake 1cv8 never wrote its pid"
        );
        fs::read_to_string(&self.pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }
}

fn held_dump(root: &Path) -> Hold {
    Hold {
        command: "config dump-config-to-files".to_owned(),
        started: root.join("held.started"),
        release: root.join("held.release"),
        reply: HoldReply::Normal,
    }
}

/// Поддельный `1cv8` в агентском режиме: пишет ключи запуска, раскладку `AgentBaseDir` как
/// платформа и файл запроса для двойника, затем живёт, пока его не снимут.
#[cfg(unix)]
fn write_designer(root: &Path) -> PathBuf {
    let path = root.join("1cv8");
    support::fake_agent::write_fake_designer(
        &path,
        &root.join("designer.args"),
        &root.join("designer.pid"),
        &root.join("base-dir.txt"),
    );
    path
}

#[cfg(windows)]
fn write_designer(root: &Path) -> PathBuf {
    let launch = support::fake_agent::launch_request_file(&root.join("base-dir.txt"));
    let script = root.join("fake1cv8.ps1");
    fs::write(
        &script,
        format!(
            r#"$argsLog = '{args_log}'
$pidFile = '{pid_file}'
$baseFile = '{base_file}'
$launch = '{launch}'
Add-Content -LiteralPath $argsLog -Value ($args -join ' ')
[IO.File]::WriteAllText($pidFile, "$PID")
$base = $null; $port = $null; $key = $null
for ($i = 0; $i -lt $args.Count - 1; $i++) {{
  switch ($args[$i]) {{
    '/AgentBaseDir' {{ $base = $args[$i + 1] }}
    '/AgentPort' {{ $port = $args[$i + 1] }}
    '/AgentSSHHostKey' {{ $key = $args[$i + 1] }}
  }}
}}
if (-not $base) {{ exit 3 }}
New-Item -ItemType Directory -Force -Path (Join-Path $base '0') | Out-Null
[IO.File]::WriteAllText((Join-Path $base 'agentbasedir.json'), '{{"usersInfo":[{{"name":"","dir":"0"}}]}}')
[IO.File]::WriteAllText($launch + '.tmp', "$port`n$key`n")
Move-Item -Force -LiteralPath ($launch + '.tmp') -Destination $launch
[IO.File]::WriteAllText($baseFile, $base)
while ($true) {{ Start-Sleep -Seconds 1 }}
"#,
            args_log = root.join("designer.args").display(),
            pid_file = root.join("designer.pid").display(),
            base_file = root.join("base-dir.txt").display(),
            launch = launch.display(),
        ),
    )
    .expect("fake 1cv8 script");
    let path = root.join("1cv8.cmd");
    fs::write(
        &path,
        format!(
            "@echo off\r\npowershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{}\" %*\r\n",
            script.display()
        ),
    )
    .expect("fake 1cv8");
    path
}

/// Процесс сторожа, которого тест поднял сам.
struct Keeper {
    child: Child,
    channel: String,
    state: PathBuf,
    key: String,
    /// Вывод сторожа: улика, когда он ведёт себя не так.
    log: PathBuf,
    runtime: tokio::runtime::Runtime,
}

impl Keeper {
    fn start(bench: &Bench, idle_ms: u64) -> Self {
        Self::start_as(bench, idle_ms, "", AGENT_PASSWORD)
    }

    fn start_as(bench: &Bench, idle_ms: u64, user: &str, password: &str) -> Self {
        Self::start_with(bench, idle_ms, user, password, None)
    }

    /// Сторож, который поднимает агента без подключения к базе (`at_open: Idle`, #574).
    fn start_idle(bench: &Bench, idle_ms: u64) -> Self {
        Self::start_with(bench, idle_ms, "", AGENT_PASSWORD, Some("Idle"))
    }

    fn start_with(
        bench: &Bench,
        idle_ms: u64,
        user: &str,
        password: &str,
        at_open: Option<&str>,
    ) -> Self {
        let key = uuid::Uuid::new_v4().simple().to_string();
        let state = bench.state();
        let mut start = json!({
            "launch": {
                "v8": bench.designer,
                "infobase_args": ["/F", bench.root().join("ib")],
                "port": null,
                "host_key": null,
                "startup_timeout_ms": 30000,
                "infobase": bench.root().join("ib"),
            },
            "credentials": { "user": user, "password": password },
        });
        if let Some(at_open) = at_open {
            start["at_open"] = json!(at_open);
        }
        let log_path = bench.root().join(format!("keeper-{key}.log"));
        let log = fs::File::create(&log_path).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_v8-runner"))
            .args(["--log-level", "debug", "agent", "keeper", "--key", &key])
            .env("V8_RUNNER_STATE_DIR", &state)
            .env("V8_RUNNER_AGENT_IDLE_MS", idle_ms.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("start the keeper");
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, "{start}").unwrap();
        drop(stdin);
        let channel = if cfg!(windows) {
            format!(r"\\.\pipe\v8-runner-agent-{key}")
        } else {
            state
                .join("agents")
                .join(format!("{}.sock", &key[..32]))
                .display()
                .to_string()
        };
        Self {
            child,
            channel,
            state,
            key,
            log: log_path,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        }
    }

    fn record(&self) -> PathBuf {
        self.state
            .join("agents")
            .join(&self.key)
            .join("agent")
            .join("record.json")
    }

    /// Подключение к каналу сторожа: канал появляется не сразу после запуска.
    async fn connect(&self) -> Box<dyn Duplex> {
        let started = Instant::now();
        loop {
            match open(&self.channel).await {
                Ok(stream) => return stream,
                Err(error) if started.elapsed() < Duration::from_secs(30) => {
                    let _ = error;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(error) => panic!("no keeper channel {}: {error}", self.channel),
            }
        }
    }

    /// Запрос и все кадры ответа до итогового.
    fn ask(&self, request: Value) -> Vec<Value> {
        self.runtime.block_on(async {
            let stream = self.connect().await;
            let (read, mut write) = tokio::io::split(stream);
            write
                .write_all(format!("{request}\n").as_bytes())
                .await
                .unwrap();
            let mut reader = BufReader::new(read);
            let mut frames = Vec::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    return frames;
                }
                let frame: Value = serde_json::from_str(line.trim_end()).unwrap();
                let end = frame["frame"] != "batch";
                frames.push(frame);
                if end {
                    return frames;
                }
            }
        })
    }

    fn last(&self, request: Value) -> Value {
        self.ask(request).pop().expect("an answer")
    }

    fn status(&self) -> Value {
        self.last(json!({"op": "status"}))
    }

    fn credentials(&self) -> String {
        digest(&self.state, "", AGENT_PASSWORD)
    }

    /// Открывает сессию команды; её подключение держится, пока сессия не закрыта.
    fn begin(&self) -> Session {
        self.try_begin()
            .unwrap_or_else(|frame| panic!("the session did not begin: {frame}"))
    }

    fn try_begin(&self) -> Result<Session, Value> {
        let credentials = self.credentials();
        self.runtime.block_on(async {
            let mut stream = BufReader::new(self.connect().await);
            let request = json!({"op": "begin", "credentials": credentials});
            stream
                .get_mut()
                .write_all(format!("{request}\n").as_bytes())
                .await
                .unwrap();
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            let frame: Value = serde_json::from_str(line.trim_end()).unwrap();
            if frame["frame"] == "begun" {
                Ok(Session {
                    id: frame["session"].as_str().unwrap().to_owned(),
                    stream,
                })
            } else {
                Err(frame)
            }
        })
    }

    /// Закрывает сессию команды: ответ сторожа.
    fn end(&self, mut session: Session) -> Value {
        self.runtime.block_on(async {
            session
                .stream
                .get_mut()
                .write_all(b"{\"op\":\"end\"}\n")
                .await
                .unwrap();
            let mut line = String::new();
            session.stream.read_line(&mut line).await.unwrap();
            serde_json::from_str(line.trim_end()).unwrap()
        })
    }

    fn run(&self, session: &str, command: &str, critical: bool) -> Vec<Value> {
        self.run_with(session, command, critical, &self.credentials())
    }

    fn run_with(
        &self,
        session: &str,
        command: &str,
        critical: bool,
        credentials: &str,
    ) -> Vec<Value> {
        self.ask(json!({
            "op": "run", "session": session, "command": command, "critical": critical,
            "credentials": credentials,
        }))
    }

    /// Одна команда в своей сессии: `begin`, `run`, `end`.
    fn once(&self, command: &str) -> Vec<Value> {
        let session = self.begin();
        let frames = self.run(&session.id, command, false);
        let ended = self.end(session);
        assert_eq!(ended["frame"], "ended", "{ended}");
        frames
    }

    /// Ждёт выхода сторожа.
    fn exited_within(&mut self, limit: Duration) -> bool {
        let exited = wait_until(limit, || matches!(self.child.try_wait(), Ok(Some(_))));
        if !exited {
            eprintln!(
                "keeper log:
{}",
                fs::read_to_string(&self.log).unwrap_or_default()
            );
        }
        exited
    }
}

impl Drop for Keeper {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.ask(json!({"op": "stop"}));
            if !self.exited_within(Duration::from_secs(15)) {
                let _ = self.child.kill();
            }
        }
        let _ = self.child.wait();
    }
}

/// Открытая сессия команды: подключение `begin` и номер сессии.
struct Session {
    id: String,
    stream: BufReader<Box<dyn Duplex>>,
}

trait Duplex: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Duplex for T {}

#[cfg(windows)]
async fn open(channel: &str) -> std::io::Result<Box<dyn Duplex>> {
    Ok(Box::new(
        tokio::net::windows::named_pipe::ClientOptions::new().open(channel)?,
    ))
}

#[cfg(unix)]
async fn open(channel: &str) -> std::io::Result<Box<dyn Duplex>> {
    Ok(Box::new(tokio::net::UnixStream::connect(channel).await?))
}

/// Отпечаток учётных данных, как его считает раннер: HMAC-SHA256 с солью учётной записи над
/// её идентификатором и частями, каждая с длиной.
fn digest(state: &Path, user: &str, password: &str) -> String {
    use sha2::{Digest, Sha256};
    let salt_path = state.join("account-salt");
    assert!(
        wait_until(Duration::from_secs(10), || salt_path.is_file()),
        "the keeper makes the account salt"
    );
    let salt = fs::read(salt_path).unwrap();
    let mut message = Vec::new();
    for part in [account_id().as_str(), "agent-credentials", user, password] {
        message.extend_from_slice(&(part.len() as u64).to_be_bytes());
        message.extend_from_slice(part.as_bytes());
    }
    let mut block = [0u8; 64];
    block[..salt.len()].copy_from_slice(&salt);
    let pad = |byte: u8| block.map(|at| at ^ byte);
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(&message)
        .finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(unix)]
fn account_id() -> String {
    // SAFETY: `getuid` takes nothing and cannot fail.
    unsafe { libc::getuid() }.to_string()
}

#[cfg(windows)]
fn account_id() -> String {
    let output = Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .expect("whoami");
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim()
        .rsplit(',')
        .next()
        .expect("sid column")
        .trim_matches('"')
        .to_owned()
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    done()
}

/// Время старта процесса в единицах раннера: тики FILETIME на Windows, тики часов после
/// загрузки на Linux.
#[cfg(windows)]
fn started_of(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: plain values; the handle is checked and closed once.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0;
        CloseHandle(handle);
        ok.then(|| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }
}

#[cfg(target_os = "linux")]
fn started_of(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 2..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    support::fake_agent::process_is_alive(pid)
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    support::fake_agent::process_is_alive(pid)
        && !fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            stat.rsplit(')')
                .next()
                .is_some_and(|rest| rest.trim_start().starts_with('Z'))
        })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn ready(keeper: &Keeper) -> Value {
    let status = keeper.status();
    assert_eq!(status["frame"], "status", "{status}");
    status["status"].clone()
}

#[test]
fn the_keeper_relaunches_after_a_foreign_agent_takes_its_automatic_port() {
    let (bench, ports) = Bench::with_server(false, collide_with_first_managed_launch);
    let keeper = Keeper::start(&bench, 60_000);
    ready(&keeper);
    let frames = keeper.once("config generation-id");
    assert_eq!(frames.last().unwrap()["frame"], "done", "{frames:?}");

    let ports: Vec<_> = ports.try_iter().collect();
    let [first, second] = ports.as_slice() else {
        panic!("expected two launches, got {ports:?}");
    };
    assert_ne!(first, second);
    let record: Value = serde_json::from_slice(&fs::read(keeper.record()).unwrap()).unwrap();
    assert_eq!(record["port"], *second, "{record}");
}

/// Вторая команда берёт живого агента: процесс `1cv8` поднят один раз, сессия та же
/// (`INV.PLATFORM.A-MANAGED-AGENT-OUTLIVES-THE-COMMAND`).
#[test]
fn a_second_client_reuses_the_live_agent_without_a_new_launch() {
    let bench = Bench::new(false);
    let keeper = Keeper::start(&bench, 60_000);
    let status = ready(&keeper);
    assert!(status["agent_pid"].is_u64(), "{status}");
    assert!(
        status["fingerprint"]
            .as_str()
            .is_some_and(|fingerprint| fingerprint.starts_with("SHA256:")),
        "{status}"
    );
    for _ in 0..2 {
        let frames = keeper.once("config generation-id");
        assert_eq!(frames.last().unwrap()["frame"], "done", "{frames:?}");
        assert!(
            frames.iter().any(|frame| frame["frame"] == "batch"
                && frame["messages"][0]["type"] == "success"),
            "{frames:?}"
        );
    }
    assert_eq!(bench.launches(), 1, "one 1cv8 for both commands");
    let log = fs::read_to_string(&bench.commands_log).unwrap();
    assert_eq!(log.matches("config generation-id").count(), 2, "{log}");
    // Агент подключается к базе при подъёме и остаётся подключённым между сессиями команд:
    // вторая сессия `common connect-ib` не шлёт
    // (`INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS`).
    let session_lines = log
        .lines()
        .filter(|line| line.starts_with("common ") || line.starts_with("config "))
        .collect::<Vec<_>>();
    assert_eq!(
        session_lines,
        [
            "common connect-ib",
            "config generation-id",
            "config generation-id",
        ],
        "{log}"
    );
    let status = ready(&keeper);
    assert_eq!(status["connected"], false, "{status}");
    assert_eq!(status["attached"], true, "{status}");
    let record: Value = serde_json::from_slice(&fs::read(keeper.record()).unwrap()).unwrap();
    assert_eq!(record["pid"], keeper.child.id(), "{record}");
    assert_eq!(
        record["agent"]["fingerprint"], status["fingerprint"],
        "{record}"
    );
}

/// Пока идёт долгая команда, `status` называет её, а `hold` отвечает; долгая команда простоем
/// не считается (`INV.PLATFORM.ANOTHER-LAUNCH-MODE-WAITS-ONLY-FOR-A-BUSY-AGENT`,
/// `INV.PLATFORM.A-LIVE-AGENT-CLOSES-WHEN-NOBODY-HOLDS-IT`).
#[test]
fn status_and_hold_answer_during_a_long_run() {
    let bench = Bench::new(true);
    let hold = bench.hold();
    // Простой короче удержанной команды: сторож обязан её пережить.
    let keeper = Keeper::start(&bench, 1_000);
    ready(&keeper);
    let session = keeper.begin();
    let channel_keeper = &keeper;
    let id = session.id.clone();
    std::thread::scope(|scope| {
        let run = scope.spawn(move || {
            channel_keeper.run(&id, "config dump-config-to-files --dir dump", false)
        });
        assert!(
            wait_until(Duration::from_secs(30), || hold.started.exists()),
            "the command reached the agent"
        );
        let status = ready(&keeper);
        assert!(
            status["busy"]
                .as_str()
                .is_some_and(|busy| busy.contains("dump-config-to-files")),
            "{status}"
        );
        let held = keeper.last(json!({
            "op": "hold",
            "holder": {"kind": "ttl", "id": "probe", "until_ms": now_ms() + 600_000},
        }));
        assert_eq!(held["frame"], "holders", "{held}");
        let released = keeper.last(json!({
            "op": "release",
            "holder": {"kind": "ttl", "id": "probe", "until_ms": 0},
        }));
        assert_eq!(released["holders"], json!([]), "{released}");
        // Простой истёк бы трижды, если бы долгая команда считалась простоем.
        std::thread::sleep(Duration::from_secs(3));
        fs::write(&hold.release, "go").unwrap();
        let frames = run.join().unwrap();
        assert_eq!(frames.last().unwrap()["frame"], "done", "{frames:?}");
        assert!(
            frames
                .iter()
                .filter(|frame| frame["frame"] == "batch")
                .count()
                >= 2,
            "progress arrives as separate batches: {frames:?}"
        );
    });
    // Между запросами своей сессии агент тоже не простаивает: простой короче этой паузы.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(ready(&keeper)["connected"], true);
    assert_eq!(keeper.end(session)["frame"], "ended");
}

/// Агент без держателя закрывается после срока простоя: процесс `1cv8` снят, запись убрана
/// (`INV.PLATFORM.A-LIVE-AGENT-CLOSES-WHEN-NOBODY-HOLDS-IT`).
#[test]
fn an_idle_agent_closes_after_its_idle_limit() {
    let bench = Bench::new(false);
    let mut keeper = Keeper::start(&bench, 1_500);
    ready(&keeper);
    let designer = bench.designer_pid();
    let frames = keeper.once("config generation-id");
    assert_eq!(frames.last().unwrap()["frame"], "done", "{frames:?}");
    assert!(keeper.exited_within(GONE_WITHIN), "the idle keeper left");
    assert!(
        wait_until(Duration::from_secs(10), || !alive(designer)),
        "the agent is removed"
    );
    assert!(!keeper.record().exists(), "the record is removed");
    let log = fs::read_to_string(&bench.commands_log).unwrap();
    assert!(
        log.contains("common disconnect-ib"),
        "released before removal: {log}"
    );
}

/// Держатель-процесс держит агента, пока жив; его выход снимает агента сразу, без срока
/// простоя (`INV.PLATFORM.A-LIVE-AGENT-CLOSES-WITH-ITS-HOLDER`,
/// `INV.PLATFORM.AN-AGENT-HOLDER-IS-A-LIVE-PROCESS`).
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn the_agent_closes_when_its_holder_process_exits() {
    let bench = Bench::new(false);
    let mut keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let mut holder = if cfg!(windows) {
        Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 120"])
            .spawn()
    } else {
        Command::new("sleep").arg("120").spawn()
    }
    .expect("holder process");
    let started = started_of(holder.id()).expect("holder start time");
    let held = keeper.last(json!({
        "op": "hold",
        "holder": {"kind": "process", "pid": holder.id(), "started": started},
    }));
    assert_eq!(held["holders"].as_array().map(Vec::len), Some(1), "{held}");
    // Повторный `hold` того же держателя второй отметки не ставит.
    let again = keeper.last(json!({
        "op": "hold",
        "holder": {"kind": "process", "pid": holder.id(), "started": started},
    }));
    assert_eq!(
        again["holders"].as_array().map(Vec::len),
        Some(1),
        "{again}"
    );
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        matches!(keeper.child.try_wait(), Ok(None)),
        "the held agent stays"
    );
    let _ = holder.kill();
    let _ = holder.wait();
    assert!(
        keeper.exited_within(GONE_WITHIN),
        "the agent left with its holder"
    );
}

/// Держатель со сроком пропадает, когда срок истёк, и агент уходит сразу
/// (`INV.PLATFORM.A-LIVE-AGENT-CLOSES-WITH-ITS-HOLDER`).
#[test]
fn a_ttl_holder_expires_and_takes_the_agent_with_it() {
    let bench = Bench::new(false);
    let mut keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let designer = bench.designer_pid();
    let held = keeper.last(json!({
        "op": "hold",
        "holder": {"kind": "ttl", "id": "ci", "until_ms": now_ms() + 2_000},
    }));
    assert_eq!(held["frame"], "holders", "{held}");
    assert!(
        keeper.exited_within(GONE_WITHIN),
        "the agent left when the term ran out"
    );
    assert!(
        wait_until(Duration::from_secs(10), || !alive(designer)),
        "the agent is removed"
    );
}

/// Команда с другими учётными данными агента не получает: сторож снимает агента и отвечает
/// отказом, по которому команда поднимает нового (`INV.PLATFORM.A-LIVE-AGENT-IS-TAKEN-ONLY-BY-ITS-KEY`).
#[test]
fn other_credentials_remove_the_agent() {
    let bench = Bench::new(false);
    let mut keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let designer = bench.designer_pid();
    let frames = keeper.run_with(
        "none",
        "config generation-id",
        false,
        &digest(&keeper.state, "", "another password"),
    );
    assert_eq!(
        frames.last().unwrap()["error"]["error"],
        "credentials_mismatch",
        "{frames:?}"
    );
    assert!(
        keeper.exited_within(GONE_WITHIN),
        "the keeper removed the agent"
    );
    assert!(
        wait_until(Duration::from_secs(10), || !alive(designer)),
        "the agent is removed"
    );
    let log = fs::read_to_string(&bench.commands_log).unwrap_or_default();
    assert!(!log.contains("config generation-id"), "{log}");
}

/// Команда, ушедшая посреди некритического запроса, снимает агента: следующая не возьмёт
/// агента, который ещё занят чужой работой (`INV.PLATFORM.AN-INTERRUPTED-COMMAND-REMOVES-THE-LIVE-AGENT`).
#[test]
fn a_client_leaving_in_the_middle_of_a_command_removes_the_agent() {
    let bench = Bench::new(true);
    let hold = bench.hold();
    let mut keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let designer = bench.designer_pid();
    let credentials = keeper.credentials();
    let session = keeper.begin();
    keeper.runtime.block_on(async {
        let stream = keeper.connect().await;
        let (_read, mut write) = tokio::io::split(stream);
        let request = json!({
            "op": "run", "session": session.id,
            "command": "config dump-config-to-files --dir dump",
            "critical": false, "credentials": credentials,
        });
        write
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        let started = Instant::now();
        while !hold.started.exists() && started.elapsed() < Duration::from_secs(30) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(hold.started.exists(), "the command reached the agent");
        // Клиент бросает команду строкой `cancel`, а подключение держит: сторож видит уход по
        // строке, а не по выходу процесса.
        write.write_all(b"{\"op\":\"cancel\"}\n").await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        drop((_read, write));
        // Сессия уходит без `end`. Канал Windows закрывается, когда рантайм обработает отмену
        // своего чтения, поэтому рантайм ещё крутится.
        drop(session);
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
    assert!(
        keeper.exited_within(GONE_WITHIN),
        "the keeper removed the agent"
    );
    assert!(
        wait_until(Duration::from_secs(10), || !alive(designer)),
        "the agent is removed"
    );
    fs::write(&hold.release, "go").unwrap();
}

/// `stop` отвечает, когда агент отпущен и снят, и сторож убирает свою запись
/// (`INV.PLATFORM.A-MANAGED-AGENT-IS-RELEASED-AND-REMOVED`).
#[test]
fn stop_releases_and_removes_the_agent_and_its_record() {
    let bench = Bench::new(false);
    let mut keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let designer = bench.designer_pid();
    assert!(keeper.record().exists());
    let stopped = keeper.last(json!({"op": "stop"}));
    assert_eq!(stopped["frame"], "stopped", "{stopped}");
    assert!(!alive(designer) || wait_until(Duration::from_secs(5), || !alive(designer)));
    assert!(keeper.exited_within(GONE_WITHIN));
    assert!(!keeper.record().exists(), "the record is removed");
    let log = fs::read_to_string(&bench.commands_log).unwrap();
    assert!(log.contains("common disconnect-ib"), "{log}");
}

/// Сторож снят — агент уходит с ним, даже без следующей команды
/// (`INV.PLATFORM.A-LIVE-AGENT-DIES-WITH-ITS-KEEPER`). На macOS механизма нет.
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn the_agent_dies_with_its_keeper() {
    let bench = Bench::new(false);
    let mut keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let designer = bench.designer_pid();
    assert!(alive(designer));
    keeper.child.kill().expect("kill the keeper");
    let _ = keeper.child.wait();
    assert!(
        wait_until(Duration::from_secs(15), || !alive(designer)),
        "the agent outlived its keeper"
    );
}

/// Второй сторож того же ключа канала не получает и выходит, не трогая первого.
#[test]
fn a_second_keeper_of_the_same_key_refuses_to_start() {
    let bench = Bench::new(false);
    let keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let mut second = Command::new(env!("CARGO_BIN_EXE_v8-runner"))
        .args(["agent", "keeper", "--key", &keeper.key])
        .env("V8_RUNNER_STATE_DIR", &keeper.state)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        second.stdin.take().unwrap(),
        "{}",
        json!({"launch": {"v8": bench.designer, "infobase_args": [], "port": null, "host_key": null,
                          "startup_timeout_ms": 1000, "infobase": ""},
               "credentials": {"user": "", "password": ""}})
    )
    .unwrap();
    let output = second.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("another keeper"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(bench.launches(), 1);
    assert_eq!(ready(&keeper)["keeper_pid"], keeper.child.id());
}

/// Пока сессия команды открыта, простой не истекает, даже без запросов; он считается от её
/// конца (`INV.PLATFORM.A-LIVE-AGENT-CLOSES-WHEN-NOBODY-HOLDS-IT`).
#[test]
fn an_open_session_keeps_the_agent_from_idling_out() {
    let bench = Bench::new(false);
    let mut keeper = Keeper::start(&bench, 1_000);
    ready(&keeper);
    let session = keeper.begin();
    std::thread::sleep(Duration::from_secs(4));
    assert!(
        matches!(keeper.child.try_wait(), Ok(None)),
        "the keeper idled out under an open session"
    );
    assert_eq!(keeper.end(session)["frame"], "ended");
    assert!(keeper.exited_within(GONE_WITHIN), "the idle keeper left");
}

/// Агент, поднятый без подключения (#574), к базе не подключается, пока его не подключат сессией
/// команды или `attach`; подключённый, он остаётся подключённым и после конца сессии
/// (`INV.PLATFORM.A-LIVE-AGENT-STARTS-WHILE-THE-COMMAND-PREPARES`,
/// `INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS`).
#[test]
fn an_agent_started_idle_connects_only_when_a_session_begins() {
    let bench = Bench::new(false);
    let keeper = Keeper::start_idle(&bench, 600_000);
    ready(&keeper);
    assert_eq!(ready(&keeper)["attached"], false);
    let log = fs::read_to_string(&bench.commands_log).unwrap_or_default();
    assert_eq!(log.matches("common connect-ib").count(), 0, "{log}");
    assert_eq!(log.matches("common disconnect-ib").count(), 0, "{log}");
    let session = keeper.begin();
    assert_eq!(ready(&keeper)["connected"], true);
    assert_eq!(keeper.end(session)["frame"], "ended");
    assert_eq!(ready(&keeper)["attached"], true);
    let log = fs::read_to_string(&bench.commands_log).unwrap();
    assert_eq!(log.matches("common connect-ib").count(), 1, "{log}");
    assert_eq!(log.matches("common disconnect-ib").count(), 0, "{log}");
    assert_eq!(bench.launches(), 1);
}

/// `attach` подключает агента к базе без сессии команды — так `push` подключает его, пока идёт
/// разбор, — и следующая сессия берёт это подключение без `common connect-ib` (#644,
/// `INV.PLATFORM.A-LIVE-AGENT-STARTS-WHILE-THE-COMMAND-PREPARES`).
#[test]
fn an_attach_connects_without_a_session_and_the_session_takes_it() {
    let bench = Bench::new(false);
    let keeper = Keeper::start_idle(&bench, 600_000);
    ready(&keeper);
    let attached = keeper.last(json!({"op": "attach", "credentials": keeper.credentials()}));
    assert_eq!(attached["frame"], "attached", "{attached}");
    let status = ready(&keeper);
    assert_eq!(status["attached"], true, "{status}");
    assert_eq!(status["connected"], false, "{status}");
    let frames = keeper.once("config generation-id");
    assert_eq!(frames.last().unwrap()["frame"], "done", "{frames:?}");
    let log = fs::read_to_string(&bench.commands_log).unwrap();
    assert_eq!(log.matches("common connect-ib").count(), 1, "{log}");
}

/// `disconnect` отключает простаивающего агента от базы, а сам агент остаётся; занятый сессией
/// команды — отказ `not_idle`, и подключение остаётся. Следующая сессия подключается заново
/// (`INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS`).
#[test]
fn a_disconnect_spares_a_session_and_releases_an_idle_agent() {
    let bench = Bench::new(false);
    let keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let session = keeper.begin();
    let refused = keeper.last(json!({"op": "disconnect"}));
    assert_eq!(refused["error"]["error"], "not_idle", "{refused}");
    assert_eq!(keeper.end(session)["frame"], "ended");
    assert_eq!(ready(&keeper)["attached"], true);

    let released = keeper.last(json!({"op": "disconnect"}));
    assert_eq!(released, json!({"frame": "disconnected", "was": true}));
    assert_eq!(ready(&keeper)["attached"], false);
    let again = keeper.last(json!({"op": "disconnect"}));
    assert_eq!(again, json!({"frame": "disconnected", "was": false}));

    let frames = keeper.once("config generation-id");
    assert_eq!(frames.last().unwrap()["frame"], "done", "{frames:?}");
    let log = fs::read_to_string(&bench.commands_log).unwrap();
    assert_eq!(log.matches("common connect-ib").count(), 2, "{log}");
    assert_eq!(log.matches("common disconnect-ib").count(), 1, "{log}");
    assert_eq!(bench.launches(), 1, "the agent itself stays");
}

/// Подключение, которое устарело между командами (сеанс завершили, сервер перезапустили),
/// чинится первой некритической командой сессии: сторож переподключает агента и повторяет её
/// один раз, и команда получает ответ, а не отказ (#644).
#[test]
fn the_first_read_on_a_stale_connection_reconnects_and_repeats() {
    let (bench, stale) = Bench::with_server(false, |agent| {
        let connected = Arc::clone(&agent.connected);
        agent.require_connection.store(true, Ordering::SeqCst);
        serve_managed_launches(agent, None);
        connected
    });
    let keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let frames = keeper.once("config generation-id");
    assert_eq!(frames.last().unwrap()["frame"], "done", "{frames:?}");
    // Сеанс агента завершили снаружи.
    stale.store(false, Ordering::SeqCst);

    let frames = keeper.once("config generation-id");

    assert_eq!(frames.last().unwrap()["frame"], "done", "{frames:?}");
    assert!(
        frames
            .iter()
            .all(|frame| frame["frame"] != "batch" || frame["messages"][0]["type"] == "success"),
        "the failed first try is not shown: {frames:?}"
    );
    let log = fs::read_to_string(&bench.commands_log).unwrap();
    let lines = log
        .lines()
        .filter(|line| line.starts_with("common ") || line.starts_with("config "))
        .collect::<Vec<_>>();
    assert_eq!(
        lines,
        [
            "common connect-ib",
            "config generation-id",
            "config generation-id",
            "common disconnect-ib",
            "common connect-ib",
            "config generation-id",
        ],
        "{log}"
    );
}

/// Агент, поднятый без подключения, чьё первое подключение не удалось, не остаётся держать сеанс
/// лицензии: сторож снимает его, как агента, который не поднялся (#574).
#[test]
fn an_agent_started_idle_that_cannot_connect_is_removed() {
    let (bench, ()) = Bench::with_server(false, |agent| {
        agent
            .fail_connect
            .store(true, std::sync::atomic::Ordering::SeqCst);
        serve_managed_launches(agent, None);
    });
    let mut keeper = Keeper::start_idle(&bench, 600_000);
    ready(&keeper);
    assert!(keeper.try_begin().is_err(), "the connection is refused");
    assert!(
        keeper.exited_within(GONE_WITHIN),
        "the agent that never connected is removed"
    );
}

/// Критическую команду на устаревшем подключении сторож не повторяет: отказ доходит до команды,
/// а следующая сессия подключается заново (#644).
#[test]
fn a_critical_command_on_a_stale_connection_is_not_repeated_and_the_next_session_reconnects() {
    let (bench, stale) = Bench::with_server(false, |agent| {
        let connected = Arc::clone(&agent.connected);
        agent.require_connection.store(true, Ordering::SeqCst);
        serve_managed_launches(agent, None);
        connected
    });
    let keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let frames = keeper.once("config generation-id");
    assert_eq!(frames.last().unwrap()["frame"], "done", "{frames:?}");
    stale.store(false, Ordering::SeqCst);

    let session = keeper.begin();
    let frames = keeper.run(&session.id, "config update-db-cfg --dynamic-disable", true);
    assert_eq!(keeper.end(session)["frame"], "ended");
    let frames_text = format!("{frames:?}");
    assert!(
        frames_text.contains("DesignerNotConnectedToInfoBase"),
        "the refusal reaches the command: {frames_text}"
    );
    let frames = keeper.once("config generation-id");

    assert_eq!(frames.last().unwrap()["frame"], "done", "{frames:?}");
    let log = fs::read_to_string(&bench.commands_log).unwrap();
    assert_eq!(
        log.matches("config update-db-cfg").count(),
        1,
        "a critical command is never repeated: {log}"
    );
    assert_eq!(log.matches("common connect-ib").count(), 2, "{log}");
}

/// Команда, ушедшая без `end`, не оставляет агента подключённым к базе: сторож не знает, в каком
/// состоянии она его оставила, отключает его сам, и агент остаётся следующей команде
/// (`INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS`).
#[test]
fn a_session_left_without_end_disconnects_the_agent() {
    let bench = Bench::new(false);
    let keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let session = keeper.begin();
    assert_eq!(ready(&keeper)["connected"], true);
    drop(session);
    assert!(
        wait_until(Duration::from_secs(10), || ready(&keeper)["attached"]
            == false),
        "the keeper ended the session its client left"
    );
    let log = fs::read_to_string(&bench.commands_log).unwrap();
    assert_eq!(log.matches("common disconnect-ib").count(), 1, "{log}");
    let frames = keeper.once("config generation-id");
    assert_eq!(frames.last().unwrap()["frame"], "done", "{frames:?}");
    assert_eq!(bench.launches(), 1);
}

/// Процесс, которого уже нет, держателем не становится
/// (`INV.PLATFORM.AN-AGENT-HOLDER-IS-A-LIVE-PROCESS`).
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn a_dead_process_cannot_hold_the_agent() {
    let bench = Bench::new(false);
    let keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let mut holder = if cfg!(windows) {
        Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
            .spawn()
    } else {
        Command::new("sleep").arg("30").spawn()
    }
    .expect("holder process");
    let pid = holder.id();
    let started = started_of(pid).expect("holder start time");
    let _ = holder.kill();
    let _ = holder.wait();
    let held = keeper.last(json!({
        "op": "hold",
        "holder": {"kind": "process", "pid": pid, "started": started},
    }));
    assert_eq!(held["error"]["error"], "holder_gone", "{held}");
}

/// `stop` с условием сторож проверяет под своим замком: агента с открытой сессией команды или с
/// держателем он не снимает, а отказывает `not_idle`; простаивающего без держателей — снимает.
/// Так снятие ради предела или пересоздаваемой базы не обрывает команду, которая успела взять
/// агента между вопросом о нём и снятием (`INV.CLI.AN-AGENT-HOLD-OVER-THE-LIMIT-IS-REFUSED`).
#[test]
fn a_conditional_stop_spares_an_agent_in_use() {
    let bench = Bench::new(false);
    let mut keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let session = keeper.begin();
    let refused = keeper.last(json!({"op": "stop", "only_idle": true, "only_unheld": true}));
    assert_eq!(refused["frame"], "error", "{refused}");
    assert_eq!(refused["error"]["error"], "not_idle", "{refused}");
    let status = ready(&keeper);
    assert_eq!(status["connected"], true, "{status}");
    assert_eq!(status["closing"], false, "{status}");
    assert_eq!(keeper.end(session)["frame"], "ended");

    let until = now_ms() + 600_000;
    let held =
        keeper.last(json!({"op": "hold", "holder": {"kind": "ttl", "id": "t", "until_ms": until}}));
    assert_eq!(held["frame"], "holders", "{held}");
    let refused = keeper.last(json!({"op": "stop", "only_idle": true, "only_unheld": true}));
    assert_eq!(refused["error"]["error"], "not_idle", "{refused}");
    // Пересоздаваемая база снимает и держимого агента, если он не подключён к ней.
    let stopped = keeper.last(json!({"op": "stop", "only_idle": true}));
    assert_eq!(stopped["frame"], "stopped", "{stopped}");
    assert!(keeper.exited_within(GONE_WITHIN));
}

/// `hold` с отпечатком других учётных данных агента не держит: сторож отказывает и снимает
/// агента, как при `begin`, и `agent hold` поднимет нового (`INV.PLATFORM.A-LIVE-AGENT-IS-TAKEN-ONLY-BY-ITS-KEY`).
#[test]
fn a_hold_with_other_credentials_removes_the_agent() {
    let bench = Bench::new(false);
    let mut keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let refused = keeper.last(json!({
        "op": "hold",
        "holder": {"kind": "ttl", "id": "t", "until_ms": now_ms() + 600_000},
        "credentials": digest(&keeper.state, "", "another password"),
    }));
    assert_eq!(
        refused["error"]["error"], "credentials_mismatch",
        "{refused}"
    );
    assert!(
        keeper.exited_within(GONE_WITHIN),
        "the keeper removed the agent"
    );
}

/// Агента, которого уже решили снять, отметка не удерживает: `hold` отказывает `closing`, и
/// клиент ждёт нового сторожа.
#[test]
fn a_hold_on_a_closing_agent_is_refused() {
    let bench = Bench::new(false);
    let mut keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let held = keeper.last(json!({
        "op": "hold",
        "holder": {"kind": "ttl", "id": "t", "until_ms": now_ms() + 2_000},
        "credentials": keeper.credentials(),
    }));
    assert_eq!(held["frame"], "holders", "{held}");
    // Открытая сессия держит сторожа, пока ушедший держатель уже решил снять агента.
    let session = keeper.begin();
    assert!(
        wait_until(Duration::from_secs(10), || ready(&keeper)["closing"]
            == true),
        "the expired holder closes the agent"
    );
    let refused = keeper.last(json!({
        "op": "hold",
        "holder": {"kind": "ttl", "id": "t", "until_ms": now_ms() + 600_000},
        "credentials": keeper.credentials(),
    }));
    assert_eq!(refused["error"]["error"], "closing", "{refused}");
    keeper.end(session);
    assert!(keeper.exited_within(GONE_WITHIN));
}

/// Повторный `hold` того же держателя срок не укорачивает: остаётся поздний
/// (`INV.CLI.AN-AGENT-HOLD-IS-IDEMPOTENT-PER-HOLDER`).
#[test]
fn a_repeated_ttl_hold_keeps_the_later_term() {
    let bench = Bench::new(false);
    let keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let later = now_ms() + 600_000;
    for until in [later, now_ms() + 5_000] {
        let held = keeper.last(json!({
            "op": "hold",
            "holder": {"kind": "ttl", "id": "t", "until_ms": until},
        }));
        assert_eq!(held["frame"], "holders", "{held}");
        assert_eq!(
            held["holders"],
            json!([{"kind": "ttl", "id": "t", "until_ms": later}]),
            "{held}"
        );
    }
}

/// Сторож понимает `stop` клиента прежней версии — без условий.
#[test]
fn the_former_stop_without_conditions_still_stops() {
    let bench = Bench::new(false);
    let mut keeper = Keeper::start(&bench, 600_000);
    ready(&keeper);
    let stopped = keeper.last(json!({"op": "stop"}));
    assert_eq!(stopped["frame"], "stopped", "{stopped}");
    assert!(keeper.exited_within(GONE_WITHIN));
}

/// Свежего агента, которого ещё не взяли ни `begin`, ни `hold`, условный `stop` не снимает:
/// поднявший его клиент успевает взять агента, и два клиента на пределе не вытесняют свежих
/// агентов друг друга (`INV.CLI.AN-AGENT-HOLD-OVER-THE-LIMIT-IS-REFUSED`). Взятый и отпущенный
/// агент снимается, как прежде.
#[test]
fn a_fresh_agent_is_not_evicted_before_it_is_taken() {
    let bench = Bench::new(false);
    let mut keeper = Keeper::start(&bench, 600_000);
    assert_eq!(ready(&keeper)["fresh"], true);
    let refused = keeper.last(json!({"op": "stop", "only_idle": true, "only_unheld": true}));
    assert_eq!(refused["error"]["error"], "not_idle", "{refused}");
    let session = keeper.begin();
    assert_eq!(keeper.end(session)["frame"], "ended");
    assert_eq!(ready(&keeper)["fresh"], false, "a taken agent is not fresh");
    let stopped = keeper.last(json!({"op": "stop", "only_idle": true, "only_unheld": true}));
    assert_eq!(stopped["frame"], "stopped", "{stopped}");
    assert!(keeper.exited_within(GONE_WITHIN));
}
