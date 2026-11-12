//! Живой агент Конфигуратора через командную строку на любой ОС (#577): поддельный `1cv8` —
//! исполняемый файл, собранный `rustc` из `support/fake_agent_designer.rs`, а не сценарий
//! оболочки, поэтому стенд работает и на Windows. Команда берёт агента у настоящего сторожа
//! `v8-runner agent keeper`, а тот держит двойника агента (`support::fake_agent`).
//!
//! У каждого стенда свой каталог состояния учётной записи (`V8_RUNNER_STATE_DIR`); в конце
//! стенд снимает своих сторожей и их агентов.
#![cfg(any(unix, windows))]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use support::fake_agent::{
    read_or_empty, serve_managed_launches_on_a_reserved_port, FakeAgent, Hold, HoldReply,
    AGENT_PASSWORD,
};
use support::{temp_workspace, v8_runner_command};

/// Поддельный `1cv8`, собранный один раз на прогон.
fn fake_designer() -> &'static Path {
    static BUILT: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
    compiled(&BUILT, "fake_agent_designer.rs", "1cv8")
}

/// Поддельный `ibcmd` со списком расширений, собранный один раз на прогон.
fn fake_ibcmd() -> &'static Path {
    static BUILT: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
    compiled(&BUILT, "fake_extension_ibcmd.rs", "ibcmd")
}

/// Исполняемый файл `stem` из `tests/support/<source>`, собранный `rustc` один раз на прогон.
fn compiled(
    built: &'static OnceLock<(tempfile::TempDir, PathBuf)>,
    source: &str,
    stem: &str,
) -> &'static Path {
    &built
        .get_or_init(|| {
            let dir = tempfile::tempdir().expect("fake tool dir");
            let target = dir
                .path()
                .join(format!("{stem}{}", std::env::consts::EXE_SUFFIX));
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/support")
                .join(source);
            let compiled = Command::new("rustc")
                .args(["--edition=2021", "-O"])
                .arg(&source)
                .arg("-o")
                .arg(&target)
                .output()
                .expect("run rustc for a fake tool");
            assert!(
                compiled.status.success(),
                "{}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            (dir, target)
        })
        .1
}

/// Стенд: файловая база, двойник агента, поддельный `1cv8` и свой каталог состояния.
struct Bench {
    dir: tempfile::TempDir,
    state: std::rc::Rc<tempfile::TempDir>,
    config_path: PathBuf,
    /// Каталог файлов поддельного `1cv8`.
    marks: PathBuf,
}

impl Bench {
    fn new(agent_yaml: &str) -> Self {
        let state = if cfg!(unix) {
            tempfile::Builder::new()
                .prefix("v8rp")
                .tempdir_in("/tmp")
                .expect("state dir")
        } else {
            tempfile::tempdir().expect("state dir")
        };
        Self::sharing(agent_yaml, std::rc::Rc::new(state), false)
    }

    /// Стенд, двойник которого держит выгрузку `config dump-cfg`, пока тест не положит
    /// [`Self::release_held`].
    fn held(agent_yaml: &str) -> Self {
        let state = if cfg!(unix) {
            tempfile::Builder::new()
                .prefix("v8rp")
                .tempdir_in("/tmp")
                .expect("state dir")
        } else {
            tempfile::tempdir().expect("state dir")
        };
        Self::sharing(agent_yaml, std::rc::Rc::new(state), true)
    }

    /// Стенд другой рабочей копии с тем же каталогом состояния, двойник которого держит
    /// выгрузку.
    fn held_beside(&self, agent_yaml: &str) -> Self {
        Self::sharing(agent_yaml, std::rc::Rc::clone(&self.state), true)
    }

    fn held_started(&self) -> PathBuf {
        self.marks.join("held.started")
    }

    fn release_held(&self) {
        fs::write(self.marks.join("held.release"), "go").expect("release the held dump");
    }

    /// Стенд другой рабочей копии с тем же каталогом состояния учётной записи: предел живых
    /// агентов у них общий.
    fn beside(&self, agent_yaml: &str) -> Self {
        Self::sharing(agent_yaml, std::rc::Rc::clone(&self.state), false)
    }

    fn sharing(agent_yaml: &str, state: std::rc::Rc<tempfile::TempDir>, held: bool) -> Self {
        let dir = temp_workspace();
        let root = dir.path().to_path_buf();
        let project = root.join("project").join("configuration");
        fs::create_dir_all(&project).expect("configuration");
        fs::write(project.join("Configuration.xml"), "<Configuration/>").expect("root");
        fs::create_dir_all(root.join("work")).expect("work dir");
        let ib = root.join("ib");
        fs::create_dir_all(&ib).expect("ib");
        fs::write(ib.join("1Cv8.1CD"), "db").expect("ib file");
        let bin = root.join("platform").join("8.3.27.2074").join("bin");
        fs::create_dir_all(&bin).expect("platform dir");
        fs::copy(
            fake_designer(),
            bin.join(format!("1cv8{}", std::env::consts::EXE_SUFFIX)),
        )
        .expect("fake 1cv8");
        let marks = root.join("marks");
        fs::create_dir_all(&marks).expect("marks");
        let mut agent = FakeAgent::new(
            true,
            marks.join("agent-commands.log"),
            None,
            marks.join("base-dir.txt"),
            marks.join("designer.pid"),
        );
        agent.hold = held.then(|| Hold {
            command: "config dump-cfg".to_owned(),
            started: marks.join("held.started"),
            release: marks.join("held.release"),
            reply: HoldReply::Normal,
        });
        let port = serve_managed_launches_on_a_reserved_port(agent, None);
        let config_path = root.join("v8project.yaml");
        fs::write(
            &config_path,
            format!(
                "workPath: '{work}'\nformat: DESIGNER\nproviders:\n  infobase.configuration.export: agent\ninfobase:\n  connection: 'File={ib}'\n  password: '{password}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: '{platform}'\n    strict: true\n    version: '8.3.27.2074'\n  designer_agent:\n    port: {port}\n{agent_yaml}",
                work = root.join("work").display(),
                ib = ib.display(),
                password = AGENT_PASSWORD,
                platform = root.join("platform").display(),
            ),
        )
        .expect("write config");
        Self {
            dir,
            state,
            config_path,
            marks,
        }
    }

    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = v8_runner_command();
        command
            .env("V8_RUNNER_STATE_DIR", self.state.path())
            .env_remove("V8_RUNNER_AGENT_IDLE_MS")
            .env("V8R_FAKE_DESIGNER_DIR", &self.marks)
            .args([
                "--config",
                &self.config_path.display().to_string(),
                "--json-message",
            ])
            .args(arguments);
        command
    }

    fn run(&self, arguments: &[&str]) -> (i32, Value) {
        let output = self.command(arguments).output().expect("run command");
        let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "no json envelope: {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output.status.code().unwrap_or(-1), payload)
    }

    /// Команда без проекта: в пустом каталоге, без `--config`, с тем же каталогом состояния.
    fn run_without_project(&self, arguments: &[&str]) -> (i32, Value) {
        let empty = tempfile::tempdir().expect("empty dir");
        let output = v8_runner_command()
            .current_dir(empty.path())
            .env("V8_RUNNER_STATE_DIR", self.state.path())
            .arg("--json-message")
            .args(arguments)
            .output()
            .expect("run command");
        let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "no json envelope: {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output.status.code().unwrap_or(-1), payload)
    }

    /// `download` рабочей конфигурации через агента.
    fn download(&self) -> (i32, Value) {
        let output = self.download_command().output().expect("run download");
        let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "no json envelope: {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output.status.code().unwrap_or(-1), payload)
    }

    fn download_command(&self) -> Command {
        let output = self
            .dir
            .path()
            .join("dist")
            .join(format!("{}.cf", uuid::Uuid::new_v4().simple()));
        self.command(&[
            "download",
            "main",
            "--state",
            "working",
            "--output",
            &output.display().to_string(),
        ])
    }

    /// Команды, которые получил двойник агента.
    fn agent_commands(&self) -> Vec<String> {
        read_or_empty(&self.marks.join("agent-commands.log"))
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Сколько раз двойник получил команду `command`.
    fn sent(&self, command: &str) -> usize {
        self.agent_commands()
            .iter()
            .filter(|line| line.as_str() == command)
            .count()
    }

    /// Кладёт поддельный `ibcmd` рядом с поддельным `1cv8`: цепочка `extensions` файловой
    /// базы начинается с него.
    fn with_ibcmd(&self) {
        let bin = self
            .dir
            .path()
            .join("platform")
            .join("8.3.27.2074")
            .join("bin");
        fs::copy(
            fake_ibcmd(),
            bin.join(format!("ibcmd{}", std::env::consts::EXE_SUFFIX)),
        )
        .expect("fake ibcmd");
    }

    /// Запуски поддельного `ibcmd`: его ключи, по строке на запуск.
    fn ibcmd_calls(&self) -> Vec<String> {
        read_or_empty(&self.marks.join("ibcmd-args.log"))
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Назначает исполнителя операции ключом `providers.*`.
    fn assign(&self, operation: &str, provider: &str) {
        let config = fs::read_to_string(&self.config_path).expect("config");
        fs::write(
            &self.config_path,
            config.replacen(
                "providers:
",
                &format!(
                    "providers:
  {operation}: {provider}
"
                ),
                1,
            ),
        )
        .expect("assign the executor");
    }

    /// Запуски поддельного `1cv8`.
    fn launches(&self) -> usize {
        read_or_empty(&self.marks.join("designer-args.log"))
            .lines()
            .count()
    }

    fn designer_pid(&self) -> u32 {
        read_or_empty(&self.marks.join("designer.pid"))
            .trim()
            .parse()
            .expect("designer pid")
    }

    /// Записи сторожей этого каталога состояния.
    fn records(&self) -> Vec<Value> {
        let Ok(keys) = fs::read_dir(self.state.path().join("agents")) else {
            return Vec::new();
        };
        keys.flatten()
            .filter_map(|key| fs::read(key.path().join("agent").join("record.json")).ok())
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect()
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        for record in self.records() {
            for pid in [&record["pid"], &record["agent"]["pid"]] {
                if let Some(pid) = pid.as_u64() {
                    kill(pid as u32);
                }
            }
        }
        if let Ok(pid) = read_or_empty(&self.marks.join("designer.pid"))
            .trim()
            .parse::<u32>()
        {
            kill(pid);
        }
    }
}

#[cfg(unix)]
fn kill(pid: u32) {
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
}

#[cfg(windows)]
fn kill(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .output();
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

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };
    // SAFETY: plain values; the handle is checked and closed once.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
        CloseHandle(handle);
        running
    }
}

fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    done()
}

fn live(payload: &Value) -> &Value {
    &payload["data"]["provider"]["endpoint"]["live"]
}

/// Раннер здесь в Job, из которого сторож выйти не может (так запускает тесты, например,
/// оболочка агента разработки на Windows): живого агента не будет, команда работает с разовым,
/// а `agent hold` отказывает `live_agent_job` (`INV.PLATFORM.A-CLOSED-JOB-KEEPS-THE-AGENT-ONE-SHOT`).
/// Живой путь тогда проверить нечем.
fn in_a_closed_job(payload: &Value) -> bool {
    let closed = live(payload)["reason"] == "job" || payload["error"]["code"] == "live_agent_job";
    if closed {
        eprintln!("skipped: the runner runs in a Job its keeper cannot leave: {payload}");
    }
    closed
}

/// Закрытый Job оставляет обычной команде разового агента и отказывает удержанию.
#[cfg(windows)]
#[test]
fn a_closed_job_keeps_the_agent_one_shot() {
    use std::io::{self, Read, Write};
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::process::Stdio;
    use std::ptr::null;
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    const COMPLETION_FILE: &str = "V8_RUNNER_CLOSED_JOB_COMPLETION_FILE";

    if let Some(completion) = std::env::var_os(COMPLETION_FILE) {
        // Родитель сначала помещает этот процесс в Job; команды наследуют его.
        let mut start = [0; 2];
        io::stdin().read_exact(&mut start).expect("Job admission");
        assert_eq!(&start, b"go");

        let download = Bench::new("");
        let hold = Bench::new("");
        let (download_code, receipt) = download.download();
        let (hold_code, refusal) = hold.run(&["agent", "hold", "--ttl", "60"]);
        assert_eq!(download_code, 0, "{receipt}");
        assert_eq!(live(&receipt)["kept"], false, "{receipt}");
        assert_eq!(live(&receipt)["reason"], "job", "{receipt}");
        assert_eq!(download.launches(), 1, "download starts one fake 1cv8");
        assert_eq!(hold_code, 2, "{refusal}");
        assert_eq!(refusal["error"]["code"], "live_agent_job", "{refusal}");
        assert_eq!(hold.launches(), 0, "the refused hold starts no fake 1cv8");
        fs::write(completion, b"passed").expect("record fixture completion");
        return;
    }

    let temp = tempfile::tempdir().expect("closed Job fixture dir");
    let completion = temp.path().join("completed");
    // SAFETY: null arguments create an unnamed Job with a non-inheritable handle.
    let job = unsafe { CreateJobObjectW(null(), null()) };
    assert!(!job.is_null(), "create Job: {}", io::Error::last_os_error());
    // SAFETY: the successful creation returned a handle owned only by this test.
    let job = unsafe { OwnedHandle::from_raw_handle(job) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: the Job is live and the pointer and length describe the expected structure.
    let configured = unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    };
    assert_ne!(
        configured,
        0,
        "set Job limits: {}",
        io::Error::last_os_error()
    );

    let mut fixture = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "a_closed_job_keeps_the_agent_one_shot",
            "--nocapture",
        ])
        .env(COMPLETION_FILE, &completion)
        .env("TEMP", temp.path())
        .env("TMP", temp.path())
        .stdin(Stdio::piped())
        .spawn()
        .expect("start gated fixture");
    // SAFETY: both handles are live and belong to this test.
    if unsafe { AssignProcessToJobObject(job.as_raw_handle(), fixture.as_raw_handle()) } == 0 {
        let error = io::Error::last_os_error();
        let _ = fixture.kill();
        let _ = fixture.wait();
        panic!("assign fixture to closed Job: {error}");
    }
    fixture
        .stdin
        .take()
        .expect("admission pipe")
        .write_all(b"go")
        .expect("admit fixture");
    // SAFETY: the child process handle is valid throughout the bounded wait.
    let wait = unsafe { WaitForSingleObject(fixture.as_raw_handle(), 60_000) };
    // Closing the Job also ends any descendants left by a failure or timeout.
    drop(job);
    let status = fixture.wait().expect("reap closed Job fixture");
    assert_eq!(wait, WAIT_OBJECT_0, "closed Job fixture wait: {wait:#x}");
    assert!(status.success(), "closed Job fixture: {status}");
    assert!(completion.is_file(), "closed Job fixture did not run");
}

/// Вторая команда берёт живого агента и на Windows: `1cv8` поднят один раз, агент живёт между
/// командами (`INV.PLATFORM.A-MANAGED-AGENT-OUTLIVES-THE-COMMAND`), квитанция говорит, был ли
/// он живым (`INV.WIRE.A-RECEIPT-SAYS-WHETHER-THE-AGENT-WAS-LIVE`).
#[test]
fn a_second_command_reuses_the_live_agent_on_any_os() {
    let bench = Bench::new("");
    let (code, first) = bench.download();
    assert_eq!(code, 0, "{first}");
    if in_a_closed_job(&first) {
        return;
    }
    assert_eq!(
        live(&first),
        &json!({"reused": false, "holders": [], "kept": true}),
        "{first}"
    );
    let (code, second) = bench.download();
    assert_eq!(code, 0, "{second}");
    assert_eq!(
        live(&second),
        &json!({"reused": true, "holders": [], "kept": true}),
        "{second}"
    );
    assert_eq!(bench.launches(), 1, "one 1cv8 for both commands");
    assert!(alive(bench.designer_pid()), "the agent lives on");
}

#[test]
fn status_deep_reports_reusing_the_generation_agent() {
    let bench = Bench::new("    live: true\n");
    let config = fs::read_to_string(&bench.config_path).expect("config");
    fs::write(
        &bench.config_path,
        config.replacen("providers:\n", "providers:\n  extensions: agent\n", 1),
    )
    .expect("read extensions with the same agent");

    let (code, shallow) = bench.run(&["status"]);
    assert_eq!(code, 0, "{shallow}");
    assert!(shallow["data"].get("provider").is_none(), "{shallow}");
    assert_eq!(bench.launches(), 0);

    for reused in [false, true] {
        let (code, payload) = bench.run(&["status", "--deep"]);
        assert_eq!(code, 0, "{payload}");
        let base = &payload["data"]["infobases"][0]["source_sets"][0]["base"];
        assert_eq!(base["tool"], "agent", "{payload}");
        assert!(base["token"].is_string(), "{payload}");
        assert!(base["unapplied"].is_null(), "{payload}");
        assert_eq!(
            payload["data"]["provider"]["selected"], "agent",
            "{payload}"
        );
        if in_a_closed_job(&payload) {
            return;
        }
        let extensions = &payload["data"]["infobases"][0]["extensions"];
        assert_eq!(extensions["provider"]["selected"], "agent", "{payload}");
        assert!(extensions["installed"].is_array(), "{payload}");
        assert_eq!(
            live(&payload),
            &json!({"reused": reused, "holders": [], "kept": true}),
            "{payload}"
        );
    }
    assert_eq!(bench.launches(), 1, "one 1cv8 for both status commands");
}

/// `status --deep`, поколение которого спросил агент, читает состав расширений в той же сессии
/// (#631): квитанция `extensions.provider` называет агента и его точку входа, `ibcmd`, первый в
/// цепочке `extensions` файловой базы, пропущен с причиной и не запускается. Ответ совпадает с
/// ответом `ibcmd` той же базы (`INV.USE-CASES.THE-GENERATION-AGENT-LISTS-THE-EXTENSIONS`). Сессия
/// у живого и у разового агента (в закрытом Job) одна и та же, поэтому проверка от Job не
/// зависит.
#[test]
fn status_deep_lists_the_extensions_in_the_generation_agent_session() {
    let bench = Bench::new(
        "    live: true
",
    );
    bench.with_ibcmd();

    let (code, payload) = bench.run(&["status", "--deep"]);

    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["provider"]["selected"], "agent",
        "{payload}"
    );
    let extensions = &payload["data"]["infobases"][0]["extensions"];
    assert_eq!(extensions["provider"]["selected"], "agent", "{payload}");
    assert!(extensions["provider"]["endpoint"].is_object(), "{payload}");
    assert_eq!(
        extensions["provider"]["endpoint"], payload["data"]["provider"]["endpoint"],
        "one session for the generation and the extensions: {payload}"
    );
    assert_eq!(
        extensions["provider"]["skipped"][0]["provider"], "ibcmd",
        "{payload}"
    );
    assert_eq!(
        bench.sent("config extensions properties get --all-extensions"),
        1,
        "{:?}",
        bench.agent_commands()
    );
    assert_eq!(
        bench.ibcmd_calls(),
        Vec::<String>::new(),
        "no ibcmd process"
    );
    assert_eq!(
        bench.sent("common connect-ib"),
        1,
        "the agent connects once, for the generation and the extensions: {:?}",
        bench.agent_commands()
    );
    // Живой агент остаётся подключённым: другому исполнителю база не нужна. Разовый агент в
    // закрытом Job отключается при закрытии, как всегда.
    if !in_a_closed_job(&payload) {
        assert_eq!(
            bench.sent("common disconnect-ib"),
            0,
            "no other executor needs the base, so the agent is not let go: {:?}",
            bench.agent_commands()
        );
    }
    let through_agent = extensions["installed"].clone();
    assert_eq!(
        through_agent,
        json!([{"name": "Зонд", "active": true, "source_set": null, "tool": false}]),
        "{payload}"
    );

    // Тот же вопрос `ibcmd` той же базы: ключ `providers.extensions` назначает его, и агент
    // поколения состав больше не читает.
    bench.assign("extensions", "ibcmd");
    let (code, payload) = bench.run(&["status", "--deep"]);

    assert_eq!(code, 0, "{payload}");
    let extensions = &payload["data"]["infobases"][0]["extensions"];
    assert_eq!(extensions["provider"]["selected"], "ibcmd", "{payload}");
    let calls = bench.ibcmd_calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].contains("extension list"), "{calls:?}");
    assert_eq!(
        bench.sent("config extensions properties get --all-extensions"),
        1,
        "{:?}",
        bench.agent_commands()
    );
    assert_eq!(extensions["installed"], through_agent, "{payload}");
}

/// `extensions list` при живом агенте рабочей копии берёт список у него (#631): квитанция
/// называет агента, `ibcmd`, первый в цепочке файловой базы, пропущен с причиной и не
/// запускается, `name_prefix` — `null`. Остальные поля те же, что у ответа `ibcmd` той же базы,
/// который префикс подтверждает (`INV.USE-CASES.EXTENSIONS-LIST-TAKES-THE-LIVE-AGENT`). Без
/// живого агента список читает `ibcmd`, как прежде.
#[test]
fn extensions_list_takes_the_live_agent_and_reports_no_prefix() {
    let bench = Bench::new(
        "    live: true
",
    );
    bench.with_ibcmd();

    let (code, through_ibcmd) = bench.run(&["extensions", "list"]);
    assert_eq!(code, 0, "{through_ibcmd}");
    assert_eq!(
        through_ibcmd["data"]["provider"]["selected"], "ibcmd",
        "no live agent yet: {through_ibcmd}"
    );
    assert_eq!(
        through_ibcmd["data"]["extensions"][0]["name_prefix"], "Зо_",
        "{through_ibcmd}"
    );
    let ibcmd_calls = bench.ibcmd_calls().len();

    // Живой агент рабочей копии поднимает `status --deep`.
    let (code, payload) = bench.run(&["status", "--deep"]);
    assert_eq!(code, 0, "{payload}");
    if in_a_closed_job(&payload) {
        return;
    }

    let (code, through_agent) = bench.run(&["extensions", "list"]);

    assert_eq!(code, 0, "{through_agent}");
    let provider = &through_agent["data"]["provider"];
    assert_eq!(provider["selected"], "agent", "{through_agent}");
    assert_eq!(
        provider["skipped"][0]["provider"], "ibcmd",
        "{through_agent}"
    );
    assert_eq!(
        provider["endpoint"]["live"]["reused"], true,
        "{through_agent}"
    );
    assert_eq!(bench.ibcmd_calls().len(), ibcmd_calls, "no ibcmd process");
    let mut by_agent = through_agent["data"]["extensions"][0].clone();
    assert!(by_agent["name_prefix"].is_null(), "{through_agent}");
    let mut by_ibcmd = through_ibcmd["data"]["extensions"][0].clone();
    by_agent["name_prefix"] = Value::Null;
    by_ibcmd["name_prefix"] = Value::Null;
    assert_eq!(by_agent, by_ibcmd, "the other fields agree");
    assert_eq!(
        through_agent["data"]["extensions"].as_array().map(Vec::len),
        Some(1),
        "{through_agent}"
    );
}

/// `pull --all` по изменившемуся читает состав расширений своим исполнителем — агентом; `ibcmd`,
/// первый в цепочке `extensions` файловой базы, состав не читает (#631). Набор `main` без памяти
/// выгружается целиком, и это уже дело его собственного исполнителя.
#[test]
fn pull_all_lists_the_extensions_with_the_agent_and_not_with_ibcmd() {
    let bench = Bench::new(
        "    live: true
",
    );
    bench.with_ibcmd();

    let (code, payload) = bench.run(&["pull", "--all"]);

    // Набор `main` без памяти выгружает целиком `ibcmd`, и в каталоге без git сторож замены
    // отказывает ему (`validation`). Состав к этому времени уже прочитан.
    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["kind"], "validation", "{payload}");

    assert_eq!(
        payload["data"]["provider"]["selected"], "agent",
        "{payload}"
    );
    assert!(
        payload["data"]["declared"].is_array(),
        "the inventory was read: {payload}"
    );
    assert_eq!(
        bench.sent("config extensions properties get --all-extensions"),
        1,
        "{:?}",
        bench.agent_commands()
    );
    let calls = bench.ibcmd_calls();
    assert!(
        calls.iter().all(|call| !call.contains("extension list")),
        "{calls:?}"
    );
}

/// С `live: false` команда работает с разовым агентом и снимает живого агента своей рабочей
/// копии (`INV.CONFIG.A-LIVE-AGENT-IS-ON-UNLESS-TURNED-OFF`).
#[test]
fn live_off_removes_the_live_agent_on_any_os() {
    let bench = Bench::new("");
    let (code, payload) = bench.download();
    assert_eq!(code, 0, "{payload}");
    let live_agent = bench.designer_pid();
    let config = fs::read_to_string(&bench.config_path).expect("config");
    fs::write(&bench.config_path, format!("{config}    live: false\n")).expect("live off");

    let (code, payload) = bench.download();

    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        live(&payload),
        &json!({"reused": false, "holders": [], "kept": false, "reason": "live_agent_off"}),
        "{payload}"
    );
    assert!(
        wait_until(Duration::from_secs(20), || !alive(live_agent)),
        "the live agent of the working copy is removed"
    );
}

fn holders(payload: &Value) -> &Vec<Value> {
    payload["data"]["holders"]
        .as_array()
        .unwrap_or_else(|| panic!("no holders: {payload}"))
}

/// `agent hold` поднимает живого агента, если его нет, и ставит держателя; повторный `hold`
/// того же держателя второй отметки не ставит, держатели разных видов складываются, команда
/// берёт удержанного агента; `release` снимает отметку своего держателя, а без отметки ничего
/// не меняет и не отказывает (`INV.CLI.AN-AGENT-HOLD-IS-IDEMPOTENT-PER-HOLDER`,
/// `INV.PLATFORM.AN-AGENT-HOLDER-IS-A-LIVE-PROCESS`).
#[test]
fn agent_hold_is_idempotent_and_release_removes_the_mark() {
    let bench = Bench::new("");
    let own = std::process::id().to_string();

    let (code, payload) = bench.run(&["agent", "hold", "--ttl", "600"]);
    if in_a_closed_job(&payload) {
        return;
    }
    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["command"], "agent hold", "{payload}");
    assert_eq!(payload["data"]["reused"], false, "{payload}");
    assert!(holders(&payload)[0]["until"].is_string(), "{payload}");
    let (code, payload) = bench.run(&["agent", "hold", "--ttl", "600"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["reused"], true, "{payload}");
    assert_eq!(
        holders(&payload).len(),
        1,
        "a repeated hold adds no mark: {payload}"
    );
    for _ in 0..2 {
        let (code, payload) = bench.run(&["agent", "hold", "--holder-pid", &own]);
        assert_eq!(code, 0, "{payload}");
        assert_eq!(holders(&payload).len(), 2, "{payload}");
    }
    assert_eq!(bench.launches(), 1, "hold starts the agent once");

    let (code, payload) = bench.download();
    assert_eq!(code, 0, "{payload}");
    assert_eq!(live(&payload)["reused"], true, "{payload}");
    assert_eq!(
        live(&payload)["holders"].as_array().map(Vec::len),
        Some(2),
        "{payload}"
    );

    let (code, payload) = bench.run(&["agent", "release", "--ttl"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["released"], true, "{payload}");
    assert_eq!(holders(&payload).len(), 1, "{payload}");
    let (code, payload) = bench.run(&["agent", "release", "--ttl"]);
    assert_eq!(code, 0, "a release without a mark is no refusal: {payload}");
    assert_eq!(payload["data"]["released"], false, "{payload}");
    let (code, payload) = bench.run(&["agent", "release", "--holder-pid", &own]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["released"], true, "{payload}");
    assert!(holders(&payload).is_empty(), "{payload}");
    assert!(
        alive(bench.designer_pid()),
        "release does not stop the agent"
    );
}

/// Держатель — живой процесс: процесса, которого нет, и pid с другим временем старта `hold`
/// не принимает и платформу не запускает (`INV.PLATFORM.AN-AGENT-HOLDER-IS-A-LIVE-PROCESS`).
#[test]
fn agent_hold_refuses_a_holder_that_is_not_that_live_process() {
    let bench = Bench::new("");
    let mut gone = if cfg!(windows) {
        Command::new("cmd").args(["/C", "exit 0"]).spawn()
    } else {
        Command::new("true").spawn()
    }
    .expect("short process");
    let gone_pid = gone.id().to_string();
    gone.wait().expect("short process ends");

    let (code, payload) = bench.run(&["agent", "hold", "--holder-pid", &gone_pid]);
    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["code"], "invalid_argument", "{payload}");
    let own = std::process::id().to_string();
    let (code, payload) = bench.run(&[
        "agent",
        "hold",
        "--holder-pid",
        &own,
        "--holder-started-at",
        "2000-01-01T00:00:00Z",
    ]);
    assert_ne!(code, 0, "{payload}");
    assert_eq!(payload["error"]["code"], "invalid_argument", "{payload}");
    assert_eq!(bench.launches(), 0, "a refused hold launches nothing");
}

/// `agent hold` с `live: false` отказывает `live_agent_off`, а на пределе, когда все живые
/// агенты удерживаются, — `agent_limit` рода `environment` с выходом 2, называя агентов и
/// держателей, у срочного — его срок; платформу ни один отказ не запускает
/// (`INV.CONFIG.A-LIVE-AGENT-IS-ON-UNLESS-TURNED-OFF`, `INV.CLI.AN-AGENT-HOLD-OVER-THE-LIMIT-IS-REFUSED`).
#[test]
fn agent_hold_refuses_with_live_off_and_over_the_limit() {
    let off = Bench::new("    live: false\n");
    let (code, payload) = off.run(&["agent", "hold", "--ttl", "60"]);
    assert_eq!(code, 2, "{payload}");
    assert_eq!(payload["error"]["code"], "live_agent_off", "{payload}");
    assert_eq!(payload["error"]["kind"], "capability", "{payload}");
    assert_eq!(off.launches(), 0);

    let first = Bench::new("    live-max: 1\n");
    let (code, payload) = first.run(&["agent", "hold", "--ttl", "600"]);
    if in_a_closed_job(&payload) {
        return;
    }
    assert_eq!(code, 0, "{payload}");
    let key = payload["data"]["key"].as_str().expect("key").to_owned();
    let second = first.beside("    live-max: 1\n");
    let (code, payload) = second.run(&["agent", "hold", "--ttl", "600"]);
    assert_eq!(code, 2, "{payload}");
    assert_eq!(payload["error"]["code"], "agent_limit", "{payload}");
    assert_eq!(payload["error"]["kind"], "environment", "{payload}");
    let message = payload["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains(&key),
        "the refusal names the live agents: {message}"
    );
    assert!(
        message.contains("held by until "),
        "the refusal names the holder and when its term runs out: {message}"
    );
    assert_eq!(second.launches(), 0, "the refused hold launches nothing");

    // Простаивающий агент без держателей место уступает.
    let (code, payload) = first.run(&["agent", "release", "--ttl"]);
    assert_eq!(code, 0, "{payload}");
    let (code, payload) = second.run(&["agent", "hold", "--ttl", "600"]);
    assert_eq!(code, 0, "{payload}");
    assert!(
        wait_until(Duration::from_secs(20), || !alive(first.designer_pid())),
        "the idle agent gave way"
    );
}

/// На пределе агент, который выполняет команду без держателей, освободится сам: `agent hold`
/// его дожидается, снимает простаивающим и поднимает своего, а не отказывает
/// (`INV.CLI.AN-AGENT-HOLD-OVER-THE-LIMIT-IS-REFUSED`).
#[test]
fn agent_hold_over_the_limit_waits_for_an_agent_busy_with_a_command() {
    let busy = Bench::held("");
    let mut command = busy
        .download_command()
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("start the busy command");
    let reached = wait_until(Duration::from_secs(30), || busy.held_started().exists());
    let waiting = busy.held_beside(
        "    live-max: 1
",
    );
    let mut hold = waiting
        .command(&["agent", "hold", "--ttl", "600"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("start agent hold");
    std::thread::sleep(Duration::from_secs(3));
    let still_waiting = hold.try_wait().expect("hold state").is_none();
    busy.release_held();
    assert!(command.wait().expect("busy command").success());
    let output = hold.wait_with_output().expect("hold output");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("hold envelope");
    if in_a_closed_job(&payload) {
        return;
    }
    assert!(reached, "the busy command reached its agent");
    assert!(
        still_waiting,
        "hold waits while the agent runs a command: {payload}"
    );
    assert_eq!(output.status.code(), Some(0), "{payload}");
    assert_eq!(payload["data"]["reused"], false, "{payload}");
    assert!(
        wait_until(Duration::from_secs(20), || !alive(busy.designer_pid())),
        "the agent that finished its command gave way"
    );
}

/// `agent status` называет живого агента цели проекта без замка рабочего каталога, а
/// `agent stop` снимает его; с `--all` обе команды работают со всеми живыми агентами учётной
/// записи ОС и проекта не требуют (`INV.CLI.AGENT-STOP-AND-STATUS-ARE-ACCOUNT-WIDE`).
#[test]
fn agent_stop_removes_the_live_agent_and_status_lists_it() {
    let bench = Bench::new("");
    let (code, payload) = bench.run(&["agent", "status"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["agents"], json!([]), "{payload}");
    let (code, payload) = bench.run_without_project(&["agent", "status", "--all"]);
    assert_eq!(code, 0, "status --all needs no project: {payload}");
    assert_eq!(payload["data"]["agents"], json!([]), "{payload}");
    let (code, payload) = bench.download();
    assert_eq!(code, 0, "{payload}");
    if in_a_closed_job(&payload) {
        return;
    }
    let agent = bench.designer_pid();

    let (code, payload) = bench.run(&["agent", "status"]);
    assert_eq!(code, 0, "{payload}");
    let agents = payload["data"]["agents"].as_array().expect("agents");
    assert_eq!(agents.len(), 1, "{payload}");
    assert_eq!(agents[0]["this_project"], true, "{payload}");
    assert_eq!(agents[0]["connected"], false, "{payload}");
    assert_eq!(
        agents[0]["attached"], true,
        "the idle agent stays connected to the infobase: {payload}"
    );
    assert!(
        agents[0]["infobase"]
            .as_str()
            .is_some_and(|infobase| infobase.starts_with("file:")),
        "{payload}"
    );
    assert!(
        !payload.to_string().contains(AGENT_PASSWORD),
        "status carries no credentials: {payload}"
    );

    let other = bench.beside("");
    let (code, payload) = other.run(&["agent", "stop"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["stopped"],
        json!([]),
        "another project stops only its own agent: {payload}"
    );
    let (code, payload) = bench.run(&["agent", "stop"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["stopped"].as_array().map(Vec::len),
        Some(1),
        "{payload}"
    );
    assert!(
        wait_until(Duration::from_secs(20), || !alive(agent)),
        "the stopped agent is gone"
    );
    let (code, payload) = bench.run(&["agent", "status"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["agents"], json!([]), "{payload}");

    let (code, payload) = other.download();
    assert_eq!(code, 0, "{payload}");
    let (code, payload) = bench.run(&["agent", "status"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["agents"],
        json!([]),
        "status names only the agent of its project: {payload}"
    );
    let (code, payload) = bench.run_without_project(&["agent", "status", "--all"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(
        payload["data"]["agents"].as_array().map(Vec::len),
        Some(1),
        "status --all names the agents of every project: {payload}"
    );
    let (code, payload) = bench.run_without_project(&["agent", "stop", "--all"]);
    assert_eq!(code, 0, "stop --all needs no project: {payload}");
    assert_eq!(
        payload["data"]["stopped"].as_array().map(Vec::len),
        Some(1),
        "--all stops the agents of every project: {payload}"
    );
    assert_eq!(payload["data"]["refused"], json!([]), "{payload}");
}

/// Вторая команда берёт подключение к базе, оставшееся от первой: `common connect-ib` идёт один
/// раз на агента (`INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS`).
#[test]
fn a_second_command_takes_the_kept_connection_without_connect_ib() {
    let bench = Bench::new("");
    let (code, first) = bench.download();
    assert_eq!(code, 0, "{first}");
    if in_a_closed_job(&first) {
        return;
    }
    let (code, second) = bench.download();
    assert_eq!(code, 0, "{second}");
    assert_eq!(
        bench.sent("common connect-ib"),
        1,
        "{:?}",
        bench.agent_commands()
    );
    assert_eq!(
        bench.sent("common disconnect-ib"),
        0,
        "{:?}",
        bench.agent_commands()
    );
}

/// `agent release` отключает живого агента от базы — так её отпускают чужому Конфигуратору или
/// чужой монопольной операции, — а сам агент остаётся; следующая команда подключает его заново
/// (`INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS`, `CTR.WIRE.AGENT-RELEASE-DATA`).
#[test]
fn agent_release_disconnects_the_live_agent_from_the_infobase() {
    let bench = Bench::new("");
    let (code, payload) = bench.download();
    assert_eq!(code, 0, "{payload}");
    if in_a_closed_job(&payload) {
        return;
    }
    let (code, payload) = bench.run(&["agent", "release"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["released"], false, "{payload}");
    assert_eq!(payload["data"]["disconnected"], true, "{payload}");
    assert_eq!(
        bench.sent("common disconnect-ib"),
        1,
        "{:?}",
        bench.agent_commands()
    );
    let (code, payload) = bench.run(&["agent", "status"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["agents"][0]["attached"], false, "{payload}");
    assert!(
        alive(bench.designer_pid()),
        "release does not stop the agent"
    );

    let (code, payload) = bench.download();
    assert_eq!(code, 0, "{payload}");
    assert_eq!(live(&payload)["reused"], true, "{payload}");
    assert_eq!(
        bench.sent("common connect-ib"),
        2,
        "{:?}",
        bench.agent_commands()
    );
}

/// Команда, выбравшая другого исполнителя той же базы, сначала отключает от неё живого агента:
/// пакетный Конфигуратор или `ibcmd` не застают базу подключённой к нему (#644,
/// `INV.PLATFORM.ANOTHER-LAUNCH-MODE-WAITS-ONLY-FOR-A-BUSY-AGENT`).
#[test]
fn another_executor_finds_the_infobase_released_by_the_live_agent() {
    let bench = Bench::new("");
    let (code, payload) = bench.download();
    assert_eq!(code, 0, "{payload}");
    if in_a_closed_job(&payload) {
        return;
    }
    let config = fs::read_to_string(&bench.config_path).expect("config");
    fs::write(
        &bench.config_path,
        config.replace(
            "infobase.configuration.export: agent",
            "infobase.configuration.export: designer",
        ),
    )
    .expect("designer for download");

    // Поддельный `1cv8` пакетного запуска не умеет выгрузку: исход команды здесь не важен.
    let _ = bench.download();

    assert_eq!(
        bench.sent("common disconnect-ib"),
        1,
        "{:?}",
        bench.agent_commands()
    );
    let (code, payload) = bench.run(&["agent", "status"]);
    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["agents"][0]["attached"], false, "{payload}");
}

/// Держатель ушёл, пока команда работала: сторож снимает агента после её сессии, и квитанция
/// говорит `kept: false` с причиной `removed` (`INV.WIRE.A-RECEIPT-SAYS-WHETHER-THE-AGENT-WAS-LIVE`,
/// `INV.PLATFORM.A-LIVE-AGENT-CLOSES-WITH-ITS-HOLDER`).
#[test]
fn a_receipt_says_removed_when_the_last_holder_left_during_the_command() {
    let bench = Bench::held("");
    let held_at = Instant::now();
    let (code, payload) = bench.run(&["agent", "hold", "--ttl", "8"]);
    if in_a_closed_job(&payload) {
        return;
    }
    assert_eq!(code, 0, "{payload}");
    let download = bench
        .download_command()
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("start download");
    let started = wait_until(Duration::from_secs(30), || bench.held_started().exists());
    // Срок держателя кончается, пока выгрузка удержана.
    let expired = held_at + Duration::from_millis(10_500);
    std::thread::sleep(expired.saturating_duration_since(Instant::now()));
    bench.release_held();
    let output = download.wait_with_output().expect("download output");
    assert!(started, "the dump reached the agent");
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no json envelope: {error}\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(output.status.code(), Some(0), "{payload}");
    assert_eq!(live(&payload)["kept"], false, "{payload}");
    assert_eq!(live(&payload)["reason"], "removed", "{payload}");
}
