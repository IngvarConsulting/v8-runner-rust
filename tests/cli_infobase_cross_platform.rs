use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::tempdir;

fn compile_fake_designer(target_dir: &Path) -> PathBuf {
    fs::create_dir_all(target_dir).expect("platform dir");
    let target = target_dir.join(format!("1cv8{}", std::env::consts::EXE_SUFFIX));
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake_platform.rs");
    let compile = Command::new("rustc")
        .arg("--edition=2021")
        .arg(&source)
        .arg("-o")
        .arg(&target)
        .output()
        .expect("run rustc for fake platform");
    assert!(
        compile.status.success(),
        "failed to compile {}: {}",
        source.display(),
        String::from_utf8_lossy(&compile.stderr)
    );

    target
}

fn write_config(root: &Path, platform: &Path) -> PathBuf {
    let project = root.join("project");
    let work = root.join("work");
    let infobase = root.join("ib");
    fs::create_dir_all(&project).expect("project");
    fs::create_dir_all(&work).expect("work");
    fs::create_dir_all(&infobase).expect("infobase");
    fs::write(infobase.join("1Cv8.1CD"), "database").expect("infobase file");
    let config = root.join("v8project.yaml");
    fs::write(
        &config,
        format!(
            "workPath: '{}'\nformat: DESIGNER\nproviders:\n  push: designer\n  pull: designer\n  download: designer\n  infobase.dump: designer\n  infobase.restore: designer\ninfobase:\n  connection: 'File={}'\nsource-set: []\ntools:\n  platform:\n    path: '{}'\n",
            work.display(),
            infobase.display(),
            platform.display(),
),
    )
    .expect("config");
    config
}

fn run_json(config: &Path, arguments: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_v8-runner"))
        .arg("--config")
        .arg(config)
        .arg("--json-message")
        .args(arguments)
        .output()
        .expect("run v8-runner");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON envelope")
}

/// Проект с набором `main`, который выгружается фальшивым Конфигуратором.
#[cfg(windows)]
fn with_main_set(config: &Path) {
    let text = fs::read_to_string(config).expect("config");
    let sources = config.parent().expect("config dir").join("project/sources");
    fs::create_dir_all(&sources).expect("sources");
    fs::write(sources.join("Configuration.xml"), "<Configuration/>\n").expect("source");
    fs::write(
        config,
        text.replace(
            "source-set: []",
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/sources",
        ),
    )
    .expect("config with a set");
}

#[cfg(windows)]
fn ordinary_canonical(path: &Path) -> PathBuf {
    let canonical = fs::canonicalize(path).expect("canonical path");
    let text = canonical.to_str().expect("utf-8 path");
    assert!(text.starts_with(r"\\?\"), "canonical spelling: {text}");
    PathBuf::from(&text[r"\\?\".len()..])
}

/// `download` называет цель обычным путём во всех видах ответа, а файл публикует на месте.
#[cfg(windows)]
#[test]
fn a_download_answers_with_an_ordinary_path() {
    let root = tempdir().expect("tempdir");
    let platform = compile_fake_designer(&root.path().join("platform"));
    let config = write_config(root.path(), &platform);
    with_main_set(&config);
    let expected = ordinary_canonical(root.path()).join(r"out\main.cf");

    let json = Command::new(env!("CARGO_BIN_EXE_v8-runner"))
        .arg("--config")
        .arg(&config)
        .args([
            "--json-message",
            "download",
            "main",
            "--output",
            "out/main.cf",
        ])
        .output()
        .expect("run v8-runner");
    let stdout = String::from_utf8_lossy(&json.stdout);
    assert!(json.status.success(), "{stdout}");
    assert!(!stdout.contains(r"\\\\?\\"), "{stdout}");
    let envelope: Value = serde_json::from_str(&stdout).expect("JSON envelope");
    assert_eq!(
        envelope["data"]["output"],
        expected.to_str().expect("utf-8")
    );
    assert_eq!(envelope["data"]["published"], true);
    assert_eq!(fs::read(&expected).expect("published CF"), b"payload");
    let key = |name: &str| stdout.find(&format!("\"{name}\":")).expect(name);
    assert!(
        key("ok") < key("command") && key("command") < key("data"),
        "the envelope keeps its key order: {stdout}"
    );

    let text = Command::new(env!("CARGO_BIN_EXE_v8-runner"))
        .arg("--config")
        .arg(&config)
        .args(["--no-color", "download", "main", "--output", "out/main.cf"])
        .output()
        .expect("run v8-runner");
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(text.status.success(), "{stdout}");
    assert!(!stdout.contains(r"\\?\"), "{stdout}");
    assert!(
        stdout.contains(expected.to_str().expect("utf-8")),
        "{stdout}"
    );
}

/// Предупреждение о владельце базы называет обе копии обычными путями, хотя метка на диске
/// хранит канонический: она сверяется с другими копиями и не переписывается ради показа.
#[cfg(windows)]
#[test]
fn an_owner_warning_names_the_copies_with_ordinary_paths() {
    let root = tempdir().expect("tempdir");
    let platform = compile_fake_designer(&root.path().join("platform"));
    let config = write_config(root.path(), &platform);
    with_main_set(&config);
    let base = root.path().join("declared");
    fs::create_dir_all(&base).expect("declared base");
    fs::write(base.join("1Cv8.1CD"), "database").expect("infobase file");
    fs::write(
        root.path().join("v8project.local.yaml"),
        format!(
            "infobases:\n  origin:\n    connection: 'File={}'\n",
            base.display()
        ),
    )
    .expect("local layer");
    let other = root.path().join("other");
    fs::create_dir_all(&other).expect("other copy");
    let owner = fs::canonicalize(&other).expect("canonical other copy");
    let marker = serde_json::json!({
        "version": 2,
        "owners": [{
            "machine": "0".repeat(64),
            "host": "elsewhere",
            "project": owner,
            "since": "2026-01-01T00:00:00Z",
        }],
    });
    let marker_path = root.path().join(".declared.v8-runner.owners.json");
    let written = serde_json::to_string_pretty(&marker).expect("marker");
    fs::write(&marker_path, &written).expect("marker");

    let output = Command::new(env!("CARGO_BIN_EXE_v8-runner"))
        .arg("--config")
        .arg(&config)
        .args(["--json-message", "push", "--dry-run"])
        .output()
        .expect("run v8-runner");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let envelope: Value = serde_json::from_str(&stdout).expect("JSON envelope");
    let warnings = envelope["warnings"].as_array().expect("warnings");
    let warning = warnings
        .iter()
        .filter_map(Value::as_str)
        .find(|warning| warning.contains("elsewhere"))
        .unwrap_or_else(|| panic!("names the owner: {stdout}"));
    assert!(
        warning.contains(ordinary_canonical(&other).to_str().expect("utf-8")),
        "{warning}"
    );
    assert!(!stdout.contains(r"\\\\?\\"), "{stdout}");
    assert_eq!(
        fs::read_to_string(&marker_path).expect("marker"),
        written,
        "the marker is not rewritten"
    );
}

/// `CTRL_BREAK`, посланный группе раннера (`GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid
/// раннера)`), достаётся только раннеру. Его консольный потомок — здесь подставной
/// Конфигуратор, который переживает любое событие консоли и отмечает его файлом, — события
/// не получает: он в своей группе процессов. Снимает его раннер по своей отмене, и ответ —
/// `cancelled` (`INV.PLATFORM.A-CONSOLE-CHILD-IS-OUTSIDE-THE-RUNNER-PROCESS-GROUP`).
#[cfg(windows)]
#[test]
fn a_ctrl_break_to_the_runner_group_leaves_its_console_child_to_the_runner() {
    use std::io::Read;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Console::{
        AllocConsole, GenerateConsoleCtrlEvent, GetConsoleWindow, CTRL_BREAK_EVENT,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, CREATE_NEW_PROCESS_GROUP, PROCESS_SYNCHRONIZE,
    };

    // Событие доходит только до процессов той же консоли; у теста без консоли её заводят.
    // SAFETY: calls without pointer arguments on this test process's own console.
    unsafe {
        if GetConsoleWindow().is_null() {
            AllocConsole();
        }
    }
    let root = tempdir().expect("tempdir");
    let platform = compile_fake_designer(&root.path().join("platform"));
    let config = write_config(root.path(), &platform);
    with_main_set(&config);
    let ready = root.path().join("designer.ready");
    let release = root.path().join("designer.release");
    let pid_file = root.path().join("designer.pid");
    let console_event = root.path().join("designer.console-event");

    let runner = Command::new(env!("CARGO_BIN_EXE_v8-runner"))
        .arg("--config")
        .arg(&config)
        .args([
            "--json-message",
            "download",
            "main",
            "--output",
            "out/main.cf",
        ])
        .env("V8_RUNNER_FAKE_PLATFORM_READY", &ready)
        .env("V8_RUNNER_FAKE_PLATFORM_RELEASE", &release)
        .env("V8_RUNNER_FAKE_PLATFORM_PID", &pid_file)
        .env("V8_RUNNER_FAKE_PLATFORM_CONSOLE_EVENT", &console_event)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .expect("spawn v8-runner");
    let mut runner = BlockedProcess::new(runner, release.clone());
    let mut stdout = runner
        .child
        .as_mut()
        .and_then(|child| child.stdout.take())
        .expect("piped stdout");
    let reader = thread::spawn(move || {
        let mut text = String::new();
        stdout.read_to_string(&mut text).map(|_| text)
    });
    wait_for_file(&ready);
    let designer: u32 = fs::read_to_string(&pid_file)
        .expect("designer pid")
        .trim()
        .parse()
        .expect("numeric designer pid");
    // SAFETY: a plain open by PID while the designer is alive and held by the runner; the
    // handle keeps the number from being reused and is owned right below.
    let designer = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, designer) };
    assert!(!designer.is_null(), "open the designer process");
    // SAFETY: the successful open transfers this handle to the guard exactly once.
    let designer = unsafe { OwnedHandle::from_raw_handle(designer) };

    let runner_pid = runner.child.as_ref().expect("runner").id();
    // SAFETY: no pointer arguments; the runner's group is its pid (`CREATE_NEW_PROCESS_GROUP`).
    let sent = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, runner_pid) };
    assert_ne!(sent, 0, "CTRL_BREAK was not delivered");

    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        let child = runner.child.as_mut().expect("runner");
        if let Some(status) = child.try_wait().expect("wait runner") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the runner did not answer the CTRL_BREAK"
        );
        thread::sleep(Duration::from_millis(20));
    };
    let stdout = reader.join().expect("reader").expect("stdout");
    let payload: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("no json envelope: {error}: {stdout}"));
    assert_eq!(payload["error"]["code"], "cancelled", "{payload}");
    assert_eq!(payload["error"]["kind"], "interruption", "{payload}");
    assert_eq!(status.code(), Some(4), "{payload}");
    // SAFETY: the handle is open and owned by `designer`.
    let ended = unsafe { WaitForSingleObject(designer.as_raw_handle(), 10_000) };
    assert_eq!(ended, WAIT_OBJECT_0, "the runner left its designer running");
    assert!(
        !console_event.exists(),
        "the designer received the CTRL_BREAK sent to the runner's group: {}",
        fs::read_to_string(&console_event).unwrap_or_default()
    );
    assert!(
        !root.path().join("out/main.cf").exists(),
        "an interrupted download published its file"
    );
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.is_file() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

struct BlockedProcess {
    child: Option<std::process::Child>,
    release: PathBuf,
}

impl BlockedProcess {
    fn new(child: std::process::Child, release: PathBuf) -> Self {
        Self {
            child: Some(child),
            release,
        }
    }

    fn release_and_wait(mut self) -> std::process::Output {
        fs::write(&self.release, b"release").expect("release fake platform");
        self.child
            .take()
            .expect("blocked child")
            .wait_with_output()
            .expect("wait for blocked v8-runner")
    }
}

impl Drop for BlockedProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = fs::write(&self.release, b"release");
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn designer_configuration_and_dt_exports_publish_through_native_cli() {
    let root = tempdir().expect("tempdir");
    let platform = compile_fake_designer(&root.path().join("platform"));
    let config = write_config(root.path(), &platform);
    let cfe = root.path().join("project/dist/sales.cfe");
    let dt = root.path().join("project/dist/base.dt");

    let cfe_json = run_json(
        &config,
        &[
            "infobase",
            "configuration",
            "export",
            "--state",
            "database",
            "--extension",
            "Sales",
            "--output",
            cfe.to_str().expect("cfe path"),
        ],
    );
    assert_eq!(cfe_json["data"]["provider"]["selected"], "designer");
    assert_eq!(cfe_json["data"]["artifact_kind"], "cfe");
    assert_eq!(cfe_json["data"]["published"], true);
    assert_eq!(fs::read(&cfe).expect("published CFE"), b"payload");

    let dt_json = run_json(
        &config,
        &[
            "infobase",
            "dump",
            "--output",
            dt.to_str().expect("dt path"),
        ],
    );
    assert_eq!(dt_json["data"]["provider"]["selected"], "designer");
    assert_eq!(dt_json["data"]["artifact_kind"], "dt");
    assert_eq!(dt_json["data"]["published"], true);
    assert_eq!(fs::read(&dt).expect("published DT"), b"payload");
}

#[test]
fn infobase_dry_run_is_non_executing_on_the_host_platform() {
    let root = tempdir().expect("tempdir");
    let platform = compile_fake_designer(&root.path().join("platform"));
    let config = write_config(root.path(), &platform);
    let work = root.path().join("work");
    fs::remove_dir_all(&work).expect("remove setup workPath");
    let output = root.path().join("project/dist/main.cf");

    let preview = run_json(
        &config,
        &[
            "infobase",
            "configuration",
            "export",
            "--state",
            "working",
            "--output",
            output.to_str().expect("CF path"),
            "--dry-run",
        ],
    );

    assert_eq!(preview["data"]["mode"], "preview");
    assert_eq!(preview["data"]["provider_dispatched"], false);
    assert_eq!(preview["data"]["provider"]["selected"], "designer");
    assert!(!work.exists());
    assert!(!output.exists());
    assert!(!output.parent().expect("output parent").exists());
}

#[test]
fn concurrent_native_cli_processes_observe_the_workspace_lock() {
    let root = tempdir().expect("tempdir");
    let platform = compile_fake_designer(&root.path().join("platform"));
    let config = write_config(root.path(), &platform);
    let ready = root.path().join("first-platform.ready");
    let release = root.path().join("first-platform.release");
    let first_dt = root.path().join("project/dist/first.dt");
    let second_dt = root.path().join("project/dist/second.dt");

    let first = Command::new(env!("CARGO_BIN_EXE_v8-runner"))
        .arg("--config")
        .arg(&config)
        .arg("--json-message")
        .args(["infobase", "dump", "--output"])
        .arg(&first_dt)
        .env("V8_RUNNER_FAKE_PLATFORM_READY", &ready)
        .env("V8_RUNNER_FAKE_PLATFORM_RELEASE", &release)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn first v8-runner");
    let first = BlockedProcess::new(first, release.clone());
    wait_for_file(&ready);

    let second = Command::new(env!("CARGO_BIN_EXE_v8-runner"))
        .arg("--config")
        .arg(&config)
        .arg("--json-message")
        .args(["infobase", "dump", "--output"])
        .arg(&second_dt)
        .output()
        .expect("run contending v8-runner");
    assert!(!second.status.success(), "contender unexpectedly succeeded");
    let contender: Value = serde_json::from_slice(&second.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid contender JSON ({error}): stdout={} stderr={}",
            String::from_utf8_lossy(&second.stdout),
            String::from_utf8_lossy(&second.stderr)
        )
    });
    assert_eq!(contender["error"]["code"], "workspace_busy");
    assert_eq!(contender["steps"][0]["name"], "workspace lock");
    assert_eq!(contender["steps"][0]["kind"], "prepare_workspace");
    assert!(!second_dt.exists());

    let first_output = first.release_and_wait();
    assert!(
        first_output.status.success(),
        "first v8-runner failed: stdout={} stderr={}",
        String::from_utf8_lossy(&first_output.stdout),
        String::from_utf8_lossy(&first_output.stderr)
    );
    assert_eq!(fs::read(first_dt).expect("first DT"), b"payload");
}

#[cfg(windows)]
#[test]
fn ibcmd_dt_transfers_refuse_windows_during_provider_selection() {
    let root = tempdir().expect("tempdir");
    let platform = compile_fake_designer(&root.path().join("platform"));
    fs::copy(&platform, platform.with_file_name("ibcmd.exe")).expect("native IBCMD fixture");
    let config = write_config(root.path(), &platform);
    let ibcmd_config = fs::read_to_string(&config)
        .expect("config")
        .replace("infobase.dump: designer", "infobase.dump: ibcmd")
        .replace("infobase.restore: designer", "infobase.restore: ibcmd");
    let existing = root.path().join("ib");
    let absent = root.path().join("empty-ib");
    let input = root.path().join("backup.dt");
    let output = root.path().join("result.dt");
    let calls = root.path().join("platform-calls.log");
    let work = root.path().join("work");
    fs::remove_dir(&work).expect("remove empty setup workPath");
    fs::write(&input, b"backup").expect("input DT");
    fs::write(&output, b"previous snapshot").expect("existing output DT");
    let input_arg = input.to_str().expect("input DT path");
    let output_arg = output.to_str().expect("output DT path");

    for (target, arguments) in [
        (&existing, &["dump", "--output", output_arg][..]),
        (
            &existing,
            &["restore", "--input", input_arg, "--replace"][..],
        ),
        (&absent, &["restore", "--input", input_arg, "--create"][..]),
    ] {
        fs::write(
            &config,
            ibcmd_config.replace(
                &format!("File={}", existing.display()),
                &format!("File={}", target.display()),
            ),
        )
        .expect("IBCMD config");
        let mut preview_error = None;
        for dry_run in [true, false] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_v8-runner"));
            command
                .arg("--config")
                .arg(&config)
                .arg("--json-message")
                .env("V8_RUNNER_FAKE_PLATFORM_CALLS", &calls)
                .arg("infobase")
                .args(arguments);
            if dry_run {
                command.arg("--dry-run");
            }
            let result = command.output().expect("run DT transfer");
            let stdout = String::from_utf8_lossy(&result.stdout);
            assert_eq!(
                result.status.code(),
                Some(2),
                "{arguments:?}, dry_run={dry_run}: {stdout}"
            );
            let envelope: Value = serde_json::from_str(&stdout).expect("envelope");
            let error = &envelope["error"];
            assert_eq!(envelope["ok"], false);
            assert_eq!(error["kind"], "capability", "{stdout}");
            assert_eq!(error["code"], "target", "{stdout}");
            assert_eq!(
                envelope["steps"][0]["name"], "provider selection",
                "{stdout}"
            );
            assert_eq!(envelope["data"]["target_state"], "unchanged", "{stdout}");
            let provider = &envelope["data"]["provider"];
            assert!(provider["selected"].is_null(), "{stdout}");
            assert_eq!(provider["skipped"][0]["provider"], "ibcmd", "{stdout}");
            assert!(
                provider["skipped"][0]["reason"].as_str().is_some_and(
                    |reason| reason.contains("Windows session detection has not been measured")
                ),
                "{stdout}"
            );
            if dry_run {
                assert_eq!(envelope["data"]["provider_dispatched"], false, "{stdout}");
                preview_error = Some(error.clone());
                assert!(!work.exists(), "preview created workPath");
            } else {
                assert_eq!(preview_error.as_ref(), Some(error), "{stdout}");
            }
            assert!(!calls.exists(), "DT refusal started the platform");
            assert!(!absent.exists(), "DT refusal created the target");
            assert_eq!(
                fs::read(existing.join("1Cv8.1CD")).expect("database"),
                b"database"
            );
            assert_eq!(fs::read(&input).expect("input DT"), b"backup");
            assert_eq!(fs::read(&output).expect("output DT"), b"previous snapshot");
        }
    }
}
