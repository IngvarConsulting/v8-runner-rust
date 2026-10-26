mod support;

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use support::command_data::assert_full_pull_publication_contract;
use support::fake_agent::{start_fake_agent, FakeAgent, Hold, HoldReply, AGENT_PASSWORD};

fn assert_ready_receipt(ready: &Value, extension: bool) {
    assert_eq!(ready["command"], "pull");
    assert_eq!(ready["requested_mode"], "FULL");
    assert_eq!(ready["mode"], "FULL");
    assert!(ready["mode_reason"].is_null());
    assert_eq!(ready["provider_dispatched"], true);
    assert_eq!(ready["extension"], json!(extension.then_some("main")));
}

fn assert_reported_path(reported: &Value, expected: &Path) {
    let reported = reported.as_str().expect("reported filesystem path");
    #[cfg(windows)]
    {
        // The loader presents DOS paths, while canonicalize returns the equivalent
        // verbatim prefix. These tests compare the selected target, not that spelling.
        let ordinary = |path: &str| {
            path.strip_prefix(r"\\?\UNC\")
                .map(|tail| format!(r"\\{tail}"))
                .unwrap_or_else(|| path.strip_prefix(r"\\?\").unwrap_or(path).to_owned())
        };
        assert_eq!(ordinary(reported), ordinary(expected.to_str().unwrap()));
    }
    #[cfg(not(windows))]
    assert_eq!(Path::new(reported), expected);
}

struct Platform {
    _directory: tempfile::TempDir,
    directory: PathBuf,
}

fn platform() -> &'static Platform {
    static PLATFORM: OnceLock<Platform> = OnceLock::new();
    PLATFORM.get_or_init(|| {
        let directory = tempfile::tempdir().expect("platform directory");
        let bin = directory.path().join("8.3.27.2074/bin");
        fs::create_dir_all(&bin).expect("platform bin");
        let executable = bin.join(format!("1cv8{}", std::env::consts::EXE_SUFFIX));
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake_platform.rs");
        let compile = Command::new("rustc")
            .arg("--edition=2021")
            .arg(source)
            .arg("-o")
            .arg(&executable)
            .output()
            .expect("compile controlled platform");
        assert!(
            compile.status.success(),
            "controlled platform compilation failed: {}",
            String::from_utf8_lossy(&compile.stderr)
        );
        fs::copy(
            &executable,
            bin.join(format!("ibcmd{}", std::env::consts::EXE_SUFFIX)),
        )
        .expect("ibcmd fixture");
        Platform {
            directory: fs::canonicalize(bin).expect("canonical platform"),
            _directory: directory,
        }
    })
}

struct Project {
    _directory: tempfile::TempDir,
    config: PathBuf,
    sources: PathBuf,
    stage: PathBuf,
    work: PathBuf,
    calls: PathBuf,
    provider: &'static str,
    extension: bool,
}

impl Project {
    fn new(provider: &'static str, extension: bool) -> Self {
        let directory = tempfile::tempdir().expect("project directory");
        let root = fs::canonicalize(directory.path()).expect("canonical project");
        let config = root.join("v8project.yaml");
        let sources = root.join("sources");
        let stage = root.join("private-stage");
        let work = root.join("work");
        let calls = root.join("platform-calls.log");
        fs::create_dir(&sources).expect("source directory");
        fs::create_dir(&stage).expect("external stage");
        fs::create_dir(root.join("ib")).expect("file infobase");
        fs::write(root.join("ib/1Cv8.1CD"), "controlled file infobase").expect("database");
        fs::write(sources.join("Configuration.xml"), "<Configuration/>\n").expect("source");
        fs::write(sources.join("before.txt"), "original local sources").expect("local source");
        let mut source_sets = Vec::new();
        if extension {
            fs::create_dir(root.join("base")).expect("base configuration source");
            fs::write(root.join("base/Configuration.xml"), "<Configuration/>\n").unwrap();
            source_sets.push(json!({
                "name": "base", "type": "CONFIGURATION", "path": "base"
            }));
        }
        source_sets.push(json!({
            "name": "main",
            "type": if extension { "EXTENSION" } else { "CONFIGURATION" },
            "path": "sources"
        }));
        let executable = platform().directory.join(format!(
            "{}{}",
            if provider == "ibcmd" { "ibcmd" } else { "1cv8" },
            std::env::consts::EXE_SUFFIX
        ));
        fs::write(
            &config,
            serde_yaml::to_string(&json!({
                "workPath": work,
                "format": "DESIGNER",
                "providers": {"push": provider, "pull": provider},
                "source-set": source_sets,
                "tools": {"platform": {
                    "path": executable,
                    "strict": true,
                    "version": "8.3.27.2074"
                }}
            }))
            .expect("project YAML"),
        )
        .expect("project config");
        fs::write(
            root.join("v8project.local.yaml"),
            serde_yaml::to_string(&json!({
                "infobases": {"origin": {
                    "connection": format!("File={}", root.join("ib").display())
                }}
            }))
            .expect("local YAML"),
        )
        .expect("local config");
        Self {
            _directory: directory,
            config,
            sources,
            stage,
            work,
            calls,
            provider,
            extension,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_v8-runner"));
        command
            .arg("--config")
            .arg(&self.config)
            .env("V8_RUNNER_FAKE_PLATFORM_CALLS", &self.calls);
        command
    }

    fn native(&self, arguments: &[&str]) -> Value {
        let output = self
            .command()
            .arg("--json-message")
            .args(arguments)
            .output()
            .expect("native command");
        assert!(
            output.status.success(),
            "native command {arguments:?} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("ordinary single JSON envelope")
    }

    fn seed_baseline(&self) {
        self.native(&["pull", "main", "--force"]);
        assert!(self
            .work
            .join("infobases/origin/hashes/main.redb")
            .is_file());
        fs::write(&self.calls, "").expect("clear calls after baseline");
    }

    fn baseline(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        snapshot(&self.work.join("infobases/origin"))
    }

    fn calls(&self) -> Vec<u8> {
        fs::read(&self.calls).unwrap_or_default()
    }

    fn start(&self) -> Session {
        Session::start(
            self.command()
                .args(["pull", "main", "--force", "--publication-stdio"])
                .arg(&self.stage),
        )
    }

    fn selected(&self, session: &mut Session) -> Value {
        let selected = session.next("selected");
        assert_eq!(selected["version"], 1);
        assert_eq!(selected["provider"]["selected"], self.provider);
        assert_eq!(selected["executor"]["kind"], "local");
        let expected_executable = platform().directory.join(format!(
            "{}{}",
            if self.provider == "ibcmd" {
                "ibcmd"
            } else {
                "1cv8"
            },
            std::env::consts::EXE_SUFFIX
        ));
        assert_reported_path(&selected["executor"]["executable"], &expected_executable);
        assert!(selected["executor"]["endpoint"].is_null());
        assert!(self.calls().is_empty(), "provider ran before authorization");
        selected
    }

    fn prepared(&self, session: &mut Session, selected: &Value) -> Value {
        let ready = session.next("ready");
        assert_eq!(ready["nonce"], selected["nonce"]);
        assert_eq!(ready["source_set"], "main");
        assert_reported_path(&ready["target_path"], &self.sources);
        assert_reported_path(&ready["stage_path"], &self.stage);
        assert_eq!(ready["snapshot"], "prepared");
        assert_ready_receipt(&ready, self.extension);
        assert!(self.stage.join("Configuration.xml").is_file());
        ready
    }

    fn ready(&self, session: &mut Session) -> Value {
        let selected = self.selected(session);
        session.decide(&selected, "continue");
        self.prepared(session, &selected)
    }

    fn publish(&self) {
        fs::rename(
            &self.sources,
            self.sources.with_file_name("previous-sources"),
        )
        .expect("caller retains old sources");
        fs::rename(&self.stage, &self.sources).expect("caller publishes prepared sources");
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        if !directory.exists() {
            return;
        }
        for entry in fs::read_dir(directory).expect("snapshot directory") {
            let entry = entry.expect("snapshot entry");
            let path = entry.path();
            if entry.file_type().expect("entry kind").is_dir() {
                collect(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .expect("relative snapshot")
                        .to_path_buf(),
                    fs::read(path).expect("snapshot bytes"),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    collect(root, root, &mut files);
    files
}

struct AttachedAgent {
    calls: PathBuf,
    endpoint: String,
    disconnect_started: PathBuf,
    disconnect_release: PathBuf,
}

impl AttachedAgent {
    fn configure(project: &Project, teardown: HoldReply, accept_password: bool) -> Self {
        let root = project.config.parent().unwrap();
        let base = root.join("agent-base");
        fs::create_dir_all(base.join("0")).expect("attached agent user directory");
        fs::write(
            base.join("agentbasedir.json"),
            r#"{"usersInfo":[{"name":"","dir":"0"}]}"#,
        )
        .expect("attached agent user map");
        let calls = root.join("agent-commands.log");
        let disconnect_started = root.join("disconnect-started");
        let disconnect_release = root.join("disconnect-release");
        let mut agent = FakeAgent::new(
            accept_password,
            calls.clone(),
            Some(base.clone()),
            root.join("agent-base-dir.txt"),
            root.join("designer.pid"),
        );
        agent.hold = Some(Hold {
            command: "common disconnect-ib".into(),
            started: disconnect_started.clone(),
            release: disconnect_release.clone(),
            reply: teardown,
        });
        let port = start_fake_agent(agent);
        let endpoint = format!("127.0.0.1:{port}");
        let mut config: Value =
            serde_yaml::from_slice(&fs::read(&project.config).unwrap()).unwrap();
        config["providers"] = json!({"push": "agent", "pull": "agent"});
        config["tools"]["designer_agent"] = json!({
            "attach": endpoint,
            "base-dir": base
        });
        fs::write(&project.config, serde_yaml::to_string(&config).unwrap()).unwrap();
        let local = root.join("v8project.local.yaml");
        let mut config: Value = serde_yaml::from_slice(&fs::read(&local).unwrap()).unwrap();
        config["infobases"]["origin"]["password"] = json!(AGENT_PASSWORD);
        fs::write(local, serde_yaml::to_string(&config).unwrap()).unwrap();
        Self {
            calls,
            endpoint,
            disconnect_started,
            disconnect_release,
        }
    }

    fn selected(&self, project: &Project, session: &mut Session) -> Value {
        let selected = session.next("selected");
        assert_eq!(selected["provider"]["selected"], "agent");
        assert_eq!(selected["executor"]["kind"], "attached");
        assert!(selected["executor"]["executable"].is_null());
        assert_eq!(selected["executor"]["endpoint"], self.endpoint);
        assert!(
            project.calls().is_empty(),
            "local provider started before authorization"
        );
        assert!(
            fs::read(&self.calls).unwrap_or_default().is_empty(),
            "attached agent received a command before authorization"
        );
        selected
    }

    fn ready(&self, project: &Project, session: &mut Session) -> Value {
        let selected = self.selected(project, session);
        session.decide(&selected, "continue");
        let ready = session.next("ready");
        assert_eq!(ready["nonce"], selected["nonce"]);
        assert_eq!(ready["source_set"], "main");
        assert_reported_path(&ready["target_path"], &project.sources);
        assert_reported_path(&ready["stage_path"], &project.stage);
        assert_eq!(ready["snapshot"], "prepared");
        assert_ready_receipt(&ready, project.extension);
        assert!(project.stage.join("Configuration.xml").is_file());
        assert!(
            !self.disconnect_started.exists(),
            "agent was released before publication"
        );
        ready
    }
}

// This observer is local to the publication test: retain the gated dump process,
// rather than infer its exit from a PID lookup, a fixture marker or pipe EOF.
struct WriterExit {
    #[cfg(target_os = "macos")]
    pid: u32,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    event: std::os::fd::OwnedFd,
    #[cfg(windows)]
    event: std::os::windows::io::OwnedHandle,
}

impl WriterExit {
    #[cfg(target_os = "linux")]
    fn retain(pid: u32) -> io::Result<Self> {
        use std::os::fd::FromRawFd;
        // SAFETY: pid names the fixture held by our connected gate; flags is zero.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            // SAFETY: pidfd_open returned a fresh descriptor owned by this observer.
            event: unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) },
        })
    }

    #[cfg(target_os = "macos")]
    fn retain(pid: u32) -> io::Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd};
        // SAFETY: kqueue has no pointer arguments.
        let fd = unsafe { libc::kqueue() };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: kqueue returned a fresh descriptor owned by this observer.
        let event = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
        let change = libc::kevent {
            ident: pid as usize,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        // SAFETY: change is initialized; no output events are requested. Register
        // while the fixture is held, before its PID could be reaped or reused.
        if unsafe {
            libc::kevent(
                event.as_raw_fd(),
                &change,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { pid, event })
    }

    #[cfg(windows)]
    fn retain(pid: u32) -> io::Result<Self> {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
        // SAFETY: the gated fixture is alive; only synchronization access is requested.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            // SAFETY: OpenProcess returned a fresh handle owned by this observer.
            event: unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) },
        })
    }

    #[cfg(target_os = "linux")]
    fn exited(&self) -> io::Result<bool> {
        use std::os::fd::AsRawFd;
        let mut descriptor = libc::pollfd {
            fd: self.event.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            // SAFETY: descriptor is initialized and remains live for this poll.
            let count = unsafe { libc::poll(&mut descriptor, 1, 0) };
            if count < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if descriptor.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
                return Err(io::Error::other("invalid writer pidfd event"));
            }
            // A retained pidfd becomes readable on exit; HUP also covers reaping.
            return Ok(descriptor.revents & (libc::POLLIN | libc::POLLHUP) != 0);
        }
    }

    #[cfg(target_os = "macos")]
    fn exited(&self) -> io::Result<bool> {
        use std::os::fd::AsRawFd;
        let mut event = std::mem::MaybeUninit::<libc::kevent>::uninit();
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        loop {
            // SAFETY: one output slot and initialized zero timeout are valid;
            // assume_init is used only when kevent reports one initialized event.
            let count = unsafe {
                libc::kevent(
                    self.event.as_raw_fd(),
                    std::ptr::null(),
                    0,
                    event.as_mut_ptr(),
                    1,
                    &timeout,
                )
            };
            if count < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if count == 0 {
                return Ok(false);
            }
            // SAFETY: count is one, so the single output slot was initialized.
            let event = unsafe { event.assume_init() };
            if event.flags & libc::EV_ERROR != 0 {
                return Err(io::Error::from_raw_os_error(event.data as i32));
            }
            if event.ident != self.pid as usize || event.fflags & libc::NOTE_EXIT == 0 {
                return Err(io::Error::other("unexpected writer process event"));
            }
            return Ok(true);
        }
    }

    #[cfg(windows)]
    fn exited(&self) -> io::Result<bool> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        // SAFETY: this observer retains the exact process handle throughout the wait.
        match unsafe { WaitForSingleObject(self.event.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            _ => Err(io::Error::other("unexpected writer process wait result")),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    fn retain(_pid: u32) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "native writer-exit observer",
        ))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    fn exited(&self) -> io::Result<bool> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "native writer-exit observer",
        ))
    }
}

enum WriterReadyCheck {
    Disabled,
    Waiting,
    Watching(WriterExit),
}

struct Session {
    child: Child,
    input: Option<ChildStdin>,
    frames: mpsc::Receiver<Result<Value, String>>,
    output_reader: Option<thread::JoinHandle<()>>,
    error_reader: Option<thread::JoinHandle<Vec<u8>>>,
    ready_receipt: Option<Value>,
    writer_ready: Arc<Mutex<WriterReadyCheck>>,
}

impl Session {
    fn start(command: &mut Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn external-publication CLI");
        let input = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout pipe");
        let mut stderr = child.stderr.take().expect("stderr pipe");
        let (send, frames) = mpsc::channel();
        let writer_ready = Arc::new(Mutex::new(WriterReadyCheck::Disabled));
        let ready_check = Arc::clone(&writer_ready);
        let output_reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let frame = line.map_err(|error| error.to_string()).and_then(|line| {
                    let frame: Value = serde_json::from_str(&line)
                        .map_err(|error| format!("invalid protocol frame {line:?}: {error}"))?;
                    if frame["phase"] == "ready" {
                        let check = ready_check.lock().map_err(|_| "writer observer poisoned")?;
                        match &*check {
                            WriterReadyCheck::Disabled => {}
                            WriterReadyCheck::Waiting => {
                                return Err(
                                    "Ready received before retaining the held writer".into()
                                );
                            }
                            WriterReadyCheck::Watching(writer) => {
                                let exited = writer.exited().map_err(|error| {
                                    format!("cannot observe writer exit at Ready: {error}")
                                })?;
                                if !exited {
                                    return Err(
                                        "Ready received while the dump writer is alive".into()
                                    );
                                }
                            }
                        }
                    }
                    Ok(frame)
                });
                if send.send(frame).is_err() {
                    return;
                }
            }
        });
        let error_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).expect("read CLI stderr");
            bytes
        });
        Self {
            child,
            input,
            frames,
            output_reader: Some(output_reader),
            error_reader: Some(error_reader),
            ready_receipt: None,
            writer_ready,
        }
    }

    fn next(&mut self, phase: &str) -> Value {
        let received = self.frames.recv_timeout(Duration::from_secs(30));
        let received = received.unwrap_or_else(|error| {
            let stderr = if self.child.try_wait().expect("inspect CLI exit").is_some() {
                self.error_reader
                    .take()
                    .unwrap()
                    .join()
                    .expect("stderr reader")
            } else {
                Vec::new()
            };
            panic!(
                "no {phase} frame: {error}; stderr={}",
                String::from_utf8_lossy(&stderr)
            );
        });
        let frame = received.unwrap_or_else(|error| panic!("{error}"));
        assert_full_pull_publication_contract(&frame);
        assert_eq!(frame["phase"], phase, "{frame}");
        assert_eq!(frame["version"], 1, "{frame}");
        if phase == "ready" {
            self.ready_receipt = Some(frame.clone());
        } else if phase == "terminal" {
            if let Some(ready) = &self.ready_receipt {
                assert_eq!(ready["command"], frame["result"]["command"], "{frame}");
                for field in [
                    "requested_mode",
                    "mode",
                    "extension",
                    "mode_reason",
                    "provider_dispatched",
                ] {
                    assert_eq!(
                        ready[field], frame["result"]["data"][field],
                        "Ready and terminal disagree on {field}: Ready={ready}, terminal={frame}"
                    );
                }
            }
        }
        frame
    }

    fn decide(&mut self, frame: &Value, decision: &str) {
        let reply = json!({
            "version": 1,
            "nonce": frame["nonce"],
            "phase": frame["phase"],
            "decision": decision
        });
        assert_full_pull_publication_contract(&reply);
        self.send(&reply);
    }

    fn send(&mut self, frame: &Value) {
        let mut bytes = serde_json::to_vec(frame).expect("control frame");
        bytes.push(b'\n');
        self.raw(&bytes).expect("send control frame");
    }

    fn raw(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let input = self.input.as_mut().expect("open stdin");
        input.write_all(bytes)?;
        input.flush()
    }

    fn close_input(&mut self) {
        self.input.take();
    }

    fn finish(mut self) -> (ExitStatus, Value) {
        let terminal = self.next("terminal");
        self.close_input();
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("wait for CLI") {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "CLI retained its process after terminal frame"
            );
            thread::sleep(Duration::from_millis(10));
        };
        self.output_reader
            .take()
            .unwrap()
            .join()
            .expect("stdout reader");
        let stderr = self
            .error_reader
            .take()
            .unwrap()
            .join()
            .expect("stderr reader");
        assert!(
            self.frames.try_recv().is_err(),
            "CLI printed an extra protocol frame after terminal; stderr={}",
            String::from_utf8_lossy(&stderr)
        );
        (status, terminal)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close_input();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn rejecting_the_selected_executor_starts_no_platform_and_preserves_baseline() {
    let project = Project::new("designer", false);
    project.seed_baseline();
    let before = snapshot(&project.sources);
    let baseline = project.baseline();
    let mut session = project.start();
    let selected = session.next("selected");
    assert_eq!(selected["executor"]["kind"], "local");
    assert!(project.calls().is_empty());
    session.decide(&selected, "reject");
    let (status, terminal) = session.finish();
    assert!(!status.success(), "{terminal}");
    assert_eq!(terminal["publication"], "not_published");
    assert_eq!(terminal["memory"], "unchanged");
    assert!(
        project.calls().is_empty(),
        "rejected executable received work"
    );
    assert_eq!(snapshot(&project.sources), before);
    assert_eq!(project.baseline(), baseline);
}

#[test]
fn rejecting_the_selected_attached_agent_starts_no_session_and_preserves_baseline() {
    let project = Project::new("designer", false);
    project.seed_baseline();
    // An early session attempt would fail authentication before Selected; rejection
    // must be possible without ever starting that session.
    let agent = AttachedAgent::configure(&project, HoldReply::Normal, false);
    let sources = snapshot(&project.sources);
    let baseline = project.baseline();
    let mut session = project.start();
    let selected = agent.selected(&project, &mut session);
    session.decide(&selected, "reject");
    let (status, terminal) = session.finish();
    assert!(!status.success(), "{terminal}");
    assert_eq!(terminal["publication"], "not_published");
    assert_eq!(terminal["memory"], "unchanged");
    assert!(fs::read(&agent.calls).unwrap_or_default().is_empty());
    assert!(!agent.disconnect_started.exists());
    assert!(project.calls().is_empty());
    assert!(snapshot(&project.stage).is_empty());
    assert_eq!(snapshot(&project.sources), sources);
    assert_eq!(project.baseline(), baseline);
}

#[test]
fn an_attached_agent_teardown_failure_preserves_the_acknowledged_publication() {
    for teardown in [HoldReply::Error, HoldReply::Close] {
        let project = Project::new("designer", false);
        project.seed_baseline();
        let agent = AttachedAgent::configure(&project, teardown, true);
        fs::write(&agent.disconnect_release, "reply immediately").unwrap();
        let baseline = project.baseline();
        let mut session = project.start();
        let ready = agent.ready(&project, &mut session);
        assert_eq!(project.baseline(), baseline);
        project.publish();
        session.decide(&ready, "published");
        let (status, terminal) = session.finish();
        assert!(status.success(), "{terminal}");
        assert_eq!(terminal["publication"], "published");
        assert_eq!(terminal["memory"], "saved");
        assert_eq!(terminal["result"]["ok"], true);
        support::command_data::assert_data_matches_its_command_form(
            &terminal["result"],
            "published full pull after attached-agent teardown failure",
        );
        assert!(
            agent.disconnect_started.exists(),
            "teardown failure was not exercised"
        );
        assert!(project.sources.join("Configuration.xml").is_file());
        assert!(!project.sources.join("before.txt").exists());
        let calls = fs::read(&agent.calls).unwrap();
        let push = project.native(&["push", "main"]);
        assert!(
            push["data"]["steps"]
                .as_array()
                .unwrap()
                .iter()
                .all(|step| step["mode"] == "skipped"),
            "{push}"
        );
        assert_eq!(
            fs::read(&agent.calls).unwrap(),
            calls,
            "unchanged push started an agent session"
        );
    }
}

#[cfg(unix)]
#[test]
fn cancellation_during_postpublication_agent_release_keeps_the_saved_publication() {
    let project = Project::new("designer", false);
    project.seed_baseline();
    let agent = AttachedAgent::configure(&project, HoldReply::Normal, true);
    let mut session = project.start();
    let ready = agent.ready(&project, &mut session);
    project.publish();
    session.decide(&ready, "published");
    assert!(
        support::wait_for_file(&agent.disconnect_started, Duration::from_secs(10)),
        "runner never entered its postpublication release"
    );
    // SAFETY: the child is owned by this session. The observed disconnect command
    // runs after baseline commit, so this signal cannot race the Published decision.
    assert_eq!(
        unsafe { libc::kill(session.child.id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    fs::write(&agent.disconnect_release, "finish release").unwrap();
    let (status, terminal) = session.finish();
    assert!(status.success(), "{terminal}");
    assert_eq!(terminal["publication"], "published");
    assert_eq!(terminal["memory"], "saved");
    assert_eq!(terminal["result"]["ok"], true);
    assert!(project.sources.join("Configuration.xml").is_file());
    assert!(!project.sources.join("before.txt").exists());
    let push = project.native(&["push", "main"]);
    assert!(
        push["data"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["mode"] == "skipped"),
        "{push}"
    );
}

#[test]
fn rejecting_the_ready_tree_keeps_every_baseline_and_the_existing_sources() {
    let project = Project::new("ibcmd", false);
    project.seed_baseline();
    fs::write(project.sources.join("local.bsl"), "keep this local edit").unwrap();
    let before = snapshot(&project.sources);
    let baseline = project.baseline();
    let mut session = project.start();
    let ready = project.ready(&mut session);
    assert_eq!(
        project.baseline(),
        baseline,
        "baseline was written before publication"
    );
    session.decide(&ready, "rejected");
    let (status, terminal) = session.finish();
    assert!(!status.success(), "{terminal}");
    assert_eq!(terminal["publication"], "not_published");
    assert_eq!(terminal["memory"], "unchanged");
    assert_eq!(snapshot(&project.sources), before);
    assert_eq!(project.baseline(), baseline);
}

#[test]
fn accepted_external_full_pull_remembers_the_original_source_for_the_next_push() {
    for provider in ["designer", "ibcmd"] {
        for extension in [false, true] {
            let project = Project::new(provider, extension);
            project.seed_baseline();
            fs::write(
                project.sources.join("local.bsl"),
                "replace only after acceptance",
            )
            .unwrap();
            let config = fs::read(&project.config).unwrap();
            let baseline = project.baseline();
            let gate = TcpListener::bind("127.0.0.1:0").expect("own dump-writer gate");
            gate.set_nonblocking(true).expect("bounded gate accept");
            let mut session = Session::start(
                project
                    .command()
                    .env(
                        "V8_RUNNER_FAKE_PLATFORM_DUMP_GATE",
                        gate.local_addr().unwrap().to_string(),
                    )
                    .args(["pull", "main", "--force", "--publication-stdio"])
                    .arg(&project.stage),
            );
            // Arm before Continue: any Ready before retaining the held writer is a failure.
            *session.writer_ready.lock().unwrap() = WriterReadyCheck::Waiting;
            let selected = project.selected(&mut session);
            session.decide(&selected, "continue");
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut writer_gate = loop {
                match gate.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "dump writer never reached its gate"
                        );
                        assert!(
                            session.child.try_wait().unwrap().is_none(),
                            "CLI exited before dump gate"
                        );
                        let before_gate = session.frames.try_recv();
                        assert!(
                            matches!(&before_gate, Err(mpsc::TryRecvError::Empty)),
                            "CLI produced a frame or EOF before dump gate: {before_gate:?}"
                        );
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("accept own writer gate: {error}"),
                }
            };
            // Winsock may inherit nonblocking mode from the listener.
            writer_gate
                .set_nonblocking(false)
                .expect("blocking writer gate");
            writer_gate
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            writer_gate
                .set_write_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            let mut pid = [0; 4];
            writer_gate
                .read_exact(&mut pid)
                .expect("held writing process PID");
            let pid = u32::from_le_bytes(pid);
            assert_ne!(
                pid,
                session.child.id(),
                "writer must be the dispatched platform"
            );
            let writer = WriterExit::retain(pid).expect("retain held writer's native exit event");
            assert!(
                !writer.exited().unwrap(),
                "fixture writer must still be held alive"
            );
            *session.writer_ready.lock().unwrap() = WriterReadyCheck::Watching(writer);
            // Keep the finite writer held while the real reader can observe an
            // illegal Ready. This bounded observation is not an emission-time proof.
            let held_frame = session.frames.recv_timeout(Duration::from_secs(2));
            assert!(
                matches!(&held_frame, Err(mpsc::RecvTimeoutError::Timeout)),
                "CLI produced Ready, terminal or EOF while the dump writer was held: {held_frame:?}"
            );
            {
                let check = session.writer_ready.lock().unwrap();
                match &*check {
                    WriterReadyCheck::Watching(writer) => assert!(
                        !writer.exited().unwrap(),
                        "finite dump writer exited before its gate was released"
                    ),
                    WriterReadyCheck::Disabled | WriterReadyCheck::Waiting => {
                        panic!("held writer observer changed before gate release");
                    }
                }
            }
            writer_gate
                .write_all(&[1])
                .expect("release finite dump writer");
            drop(writer_gate);
            // The stdout reader checks native exit BEFORE enqueueing this Ready.
            let ready = project.prepared(&mut session, &selected);
            assert!(
                session.child.try_wait().unwrap().is_none(),
                "coordinator must stay alive to receive Published"
            );
            let before_publication = session.frames.try_recv();
            assert!(
                matches!(&before_publication, Err(mpsc::TryRecvError::Empty)),
                "coordinator sent a terminal frame or EOF before Published: {before_publication:?}"
            );
            assert_eq!(project.baseline(), baseline);
            assert!(project.sources.join("local.bsl").is_file());
            let calls = project.calls();
            project.publish();
            session.decide(&ready, "published");
            session.close_input();
            let (status, terminal) = session.finish();
            assert!(
                status.success(),
                "{provider} extension={extension}: {terminal}"
            );
            assert_eq!(terminal["publication"], "published");
            assert_eq!(terminal["memory"], "saved");
            assert_eq!(terminal["result"]["ok"], true);
            assert_reported_path(&terminal["result"]["data"]["target_path"], &project.sources);
            // Замену делает вызывающий, но ответ называет уничтоженное, как и своя замена.
            let losses = &terminal["result"]["data"]["losses"];
            assert!(losses["count"].as_u64() >= Some(1), "{terminal}");
            assert!(
                terminal["result"]["data"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("discarded on request")),
                "{terminal}"
            );
            assert!(
                losses["named"]
                    .as_array()
                    .is_some_and(|named| named.iter().any(|path| path
                        .as_str()
                        .is_some_and(|path| path.ends_with("local.bsl")))),
                "{provider} extension={extension}: {terminal}"
            );
            assert_eq!(
                project.calls(),
                calls,
                "platform ran after Published acknowledgement"
            );
            assert!(!project.sources.join("local.bsl").exists());
            assert_eq!(fs::read(&project.config).unwrap(), config);
            let push = project.native(&["push", "main"]);
            let steps = push["data"]["steps"].as_array().expect("push steps");
            assert!(!steps.is_empty(), "{push}");
            assert!(steps.iter().all(|step| step["mode"] == "skipped"), "{push}");
            assert_eq!(
                project.calls(),
                calls,
                "unchanged push dispatched a provider"
            );
        }
    }
}

#[test]
fn invalid_decisions_after_ready_are_unknown_and_do_not_commit_memory() {
    for defect in [
        "nonce",
        "version",
        "phase",
        "unknown_field",
        "malformed",
        "oversize",
        "eof",
    ] {
        let project = Project::new("designer", false);
        project.seed_baseline();
        let before = snapshot(&project.sources);
        let baseline = project.baseline();
        let mut session = project.start();
        let ready = project.ready(&mut session);
        let mut decision = json!({
            "version": 1, "nonce": ready["nonce"], "phase": "ready", "decision": "published"
        });
        match defect {
            "nonce" => {
                let mut foreign = ready["nonce"].as_str().unwrap().to_owned();
                let replacement = if foreign.starts_with('0') { "1" } else { "0" };
                foreign.replace_range(..1, replacement);
                decision["nonce"] = json!(foreign);
            }
            "version" => decision["version"] = json!(2),
            "phase" => decision["phase"] = json!("selected"),
            "unknown_field" => decision["unexpected"] = json!(true),
            "malformed" => {
                let _ = session.raw(b"{invalid\n");
            }
            "oversize" => {
                let _ = session.raw(&vec![b'x'; 2 * 1024 * 1024]);
            }
            "eof" => session.close_input(),
            _ => unreachable!(),
        }
        if !matches!(defect, "malformed" | "oversize" | "eof") {
            session.send(&decision);
        }
        session.close_input();
        let (status, terminal) = session.finish();
        assert!(!status.success(), "{defect}: {terminal}");
        assert_eq!(terminal["publication"], "unknown", "{defect}: {terminal}");
        assert_eq!(terminal["memory"], "unchanged", "{defect}: {terminal}");
        assert_eq!(snapshot(&project.sources), before, "{defect}");
        assert_eq!(project.baseline(), baseline, "{defect}");
    }
}

#[test]
fn eof_after_the_callers_publication_keeps_its_fact_and_does_not_fabricate_memory() {
    let project = Project::new("designer", false);
    project.seed_baseline();
    fs::write(project.sources.join("local.bsl"), "old sources").unwrap();
    let baseline = project.baseline();
    let mut session = project.start();
    project.ready(&mut session);
    project.publish();
    let published = snapshot(&project.sources);
    session.close_input();
    let (status, terminal) = session.finish();
    assert!(!status.success(), "{terminal}");
    assert_eq!(terminal["publication"], "unknown");
    assert_eq!(terminal["memory"], "unchanged");
    assert_eq!(snapshot(&project.sources), published);
    assert!(!project.sources.join("local.bsl").exists());
    assert_eq!(project.baseline(), baseline);
}

#[test]
fn a_handed_off_stage_name_replacement_is_not_deleted_after_losing_the_caller() {
    let project = Project::new("designer", false);
    project.seed_baseline();
    let before = snapshot(&project.sources);
    let baseline = project.baseline();
    let mut session = project.start();
    project.ready(&mut session);
    let retained = project.stage.with_file_name("caller-retained-stage");
    fs::rename(&project.stage, &retained).unwrap();
    fs::create_dir(&project.stage).unwrap();
    fs::write(
        project.stage.join("foreign.txt"),
        "belongs to another owner",
    )
    .unwrap();
    session.close_input();
    let (status, terminal) = session.finish();
    assert!(!status.success(), "{terminal}");
    assert_eq!(terminal["publication"], "unknown");
    assert_eq!(
        fs::read(project.stage.join("foreign.txt")).unwrap(),
        b"belongs to another owner"
    );
    assert!(retained.join("Configuration.xml").is_file());
    assert_eq!(snapshot(&project.sources), before);
    assert_eq!(project.baseline(), baseline);
}

fn nested_target_project() -> Project {
    let mut project = Project::new("designer", false);
    let root = project.config.parent().unwrap().to_path_buf();
    project.sources = root.join("not-created/nested/sources");
    let mut config: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(&project.config).unwrap()).unwrap();
    config["source-set"][0]["path"] =
        serde_yaml::Value::String("not-created/nested/sources".into());
    fs::write(&project.config, serde_yaml::to_string(&config).unwrap()).unwrap();
    project
}

fn assert_no_sync_baseline(project: &Project) {
    let memory = project.work.join("infobases/origin");
    for name in ["hashes", "generation.json", "dump-info"] {
        assert!(
            !memory.join(name).exists(),
            "unpublished export wrote {name}"
        );
    }
}

#[test]
fn an_external_apply_without_a_target_parent_refuses_before_platform_or_memory_writes() {
    let project = nested_target_project();
    let root = project.config.parent().unwrap();
    let sources = snapshot(&root.join("sources"));
    let mut session = project.start();
    let selected = project.selected(&mut session);
    let baseline = project.baseline();
    assert_no_sync_baseline(&project);
    assert!(!root.join("not-created").exists());
    session.decide(&selected, "continue");
    let (status, terminal) = session.finish();
    assert!(!status.success(), "{terminal}");
    assert_eq!(terminal["publication"], "not_published");
    assert_eq!(terminal["memory"], "unchanged");
    assert_eq!(
        terminal["result"]["error"]["code"], "invalid_argument",
        "{terminal}"
    );
    assert!(
        !root.join("not-created").exists(),
        "refused apply created public ancestors"
    );
    assert!(
        project.calls().is_empty(),
        "refused apply dispatched its platform"
    );
    assert!(snapshot(&project.stage).is_empty());
    assert_no_sync_baseline(&project);
    assert_eq!(project.baseline(), baseline);
    assert_eq!(snapshot(&root.join("sources")), sources);
}

#[test]
fn the_caller_may_create_target_parents_between_executor_selection_and_authorization() {
    let project = nested_target_project();
    let mut session = project.start();
    let selected = project.selected(&mut session);
    let baseline = project.baseline();
    assert_no_sync_baseline(&project);
    assert!(
        !project.sources.parent().unwrap().exists(),
        "selection created source ancestors"
    );
    fs::create_dir_all(project.sources.parent().unwrap()).unwrap();
    session.decide(&selected, "continue");
    let ready = project.prepared(&mut session, &selected);
    assert!(
        !project.sources.exists(),
        "export published before the caller decision"
    );
    assert_no_sync_baseline(&project);
    assert_eq!(
        project.baseline(),
        baseline,
        "memory changed before publication"
    );
    fs::rename(&project.stage, &project.sources).unwrap();
    session.decide(&ready, "published");
    let (status, terminal) = session.finish();
    assert!(status.success(), "{terminal}");
    assert_eq!(terminal["publication"], "published");
    assert_eq!(terminal["memory"], "saved");
    let push = project.native(&["push", "main"]);
    assert!(
        push["data"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["mode"] == "skipped"),
        "{push}"
    );
}

#[test]
fn external_preview_authorizes_the_executor_and_leaves_sources_stage_and_work_unchanged() {
    for nested in [false, true] {
        let project = if nested {
            nested_target_project()
        } else {
            Project::new("designer", false)
        };
        let before = snapshot(&project.sources);
        let original = snapshot(&project.config.parent().unwrap().join("sources"));
        let mut session = Session::start(
            project
                .command()
                .args([
                    "pull",
                    "main",
                    "--force",
                    "--dry-run",
                    "--publication-stdio",
                ])
                .arg(&project.stage),
        );
        let selected = session.next("selected");
        session.decide(&selected, "continue");
        let (status, terminal) = session.finish();
        assert!(status.success(), "{terminal}");
        assert_eq!(terminal["publication"], "not_published");
        assert_eq!(terminal["memory"], "unchanged");
        assert_eq!(terminal["result"]["data"]["provider_dispatched"], false);
        assert!(project.calls().is_empty());
        assert!(!project.work.exists());
        assert_eq!(snapshot(&project.sources), before);
        assert_eq!(
            snapshot(&project.config.parent().unwrap().join("sources")),
            original
        );
        assert!(snapshot(&project.stage).is_empty());
        if nested {
            assert!(
                !project.sources.parent().unwrap().exists(),
                "preview created source ancestors"
            );
        }
    }
}

#[test]
fn the_workspace_lock_is_held_while_waiting_for_both_caller_decisions() {
    for after_ready in [false, true] {
        let project = Project::new("designer", false);
        project.seed_baseline();
        let mut session = project.start();
        let frame = if after_ready {
            project.ready(&mut session)
        } else {
            session.next("selected")
        };
        let calls = project.calls();
        let contender = project
            .command()
            .args(["--json-message", "push", "main"])
            .output()
            .expect("run contending command");
        assert!(
            !contender.status.success(),
            "contender entered a held workspace"
        );
        let rejection: Value =
            serde_json::from_slice(&contender.stdout).expect("contender envelope");
        assert_eq!(rejection["error"]["code"], "workspace_busy", "{rejection}");
        assert_eq!(
            project.calls(),
            calls,
            "contender dispatched while publisher held the workspace"
        );
        session.decide(&frame, if after_ready { "rejected" } else { "reject" });
        session.finish();
    }
}

#[test]
fn an_edit_after_publication_is_not_absorbed_into_the_prepared_baseline() {
    let project = Project::new("designer", false);
    project.seed_baseline();
    let mut session = project.start();
    let ready = project.ready(&mut session);
    let prepared_version = fs::read(project.stage.join("ConfigDumpInfo.xml")).unwrap();
    project.publish();
    let edited = b"Procedure UserEditedAfterPublication()\nEndProcedure\n";
    fs::write(project.sources.join("Module.bsl"), edited).unwrap();
    let edited_version = b"<ConfigDumpInfo version=\"2.20\"><UserEdit/></ConfigDumpInfo>\n";
    fs::write(project.sources.join("ConfigDumpInfo.xml"), edited_version).unwrap();
    session.decide(&ready, "published");
    let (status, terminal) = session.finish();
    assert!(status.success(), "{terminal}");
    assert_eq!(terminal["publication"], "published");
    assert_eq!(terminal["memory"], "saved");
    assert_eq!(
        fs::read(project.sources.join("Module.bsl")).unwrap(),
        edited
    );
    assert_eq!(
        fs::read(project.sources.join("ConfigDumpInfo.xml")).unwrap(),
        edited_version
    );
    assert_eq!(
        fs::read(
            project
                .work
                .join("infobases/origin/dump-info/main/ConfigDumpInfo.xml")
        )
        .unwrap(),
        prepared_version,
        "version memory absorbed an edit made after external publication"
    );
    let push = project.native(&["push", "main", "--dry-run"]);
    let steps = push["data"]["steps"]
        .as_array()
        .expect("planned push steps");
    assert!(
        steps.iter().any(|step| step["mode"] != "skipped"),
        "edit became baseline: {push}"
    );
}

#[test]
fn a_memory_write_failure_keeps_the_published_sources_and_names_the_unsaved_baseline() {
    let project = Project::new("designer", false);
    project.seed_baseline();
    fs::write(project.sources.join("local.bsl"), "previous target").unwrap();
    let mut session = project.start();
    let ready = project.ready(&mut session);
    let memory = project.work.join("infobases/origin/hashes/main.redb");
    fs::rename(&memory, memory.with_file_name("retained-main.redb")).unwrap();
    fs::create_dir(&memory).unwrap();
    project.publish();
    session.decide(&ready, "published");
    let (status, terminal) = session.finish();
    assert!(status.success(), "{terminal}");
    assert_eq!(terminal["publication"], "published");
    assert_eq!(terminal["memory"], "not_saved");
    assert_eq!(terminal["result"]["ok"], true);
    assert!(
        terminal["result"]["data"]["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty()),
        "{terminal}"
    );
    assert!(!project.sources.join("local.bsl").exists());
    assert!(project.sources.join("Module.bsl").is_file());
}

#[cfg(unix)]
#[test]
fn cancellation_interrupts_stdin_wait_before_and_after_ready_without_committing_memory() {
    for after_ready in [false, true] {
        let project = Project::new("designer", false);
        project.seed_baseline();
        let before = snapshot(&project.sources);
        let baseline = project.baseline();
        let mut session = project.start();
        if after_ready {
            project.ready(&mut session);
        } else {
            session.next("selected");
        }
        // SAFETY: child.id() is the live process owned by this session; SIGTERM is its
        // documented operator cancellation signal. The stdin pipe deliberately stays open.
        assert_eq!(
            unsafe { libc::kill(session.child.id() as libc::pid_t, libc::SIGTERM) },
            0
        );
        let (status, terminal) = session.finish();
        assert!(!status.success(), "{terminal}");
        assert_eq!(
            terminal["publication"],
            if after_ready {
                "unknown"
            } else {
                "not_published"
            }
        );
        assert_eq!(terminal["memory"], "unchanged");
        assert_eq!(snapshot(&project.sources), before);
        assert_eq!(project.baseline(), baseline);
    }
}
