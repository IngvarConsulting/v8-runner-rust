//! `infobase create --recreate` файловой базы на любой ОС: поддельный Конфигуратор создаёт
//! базу, тест держит её или ломает создание.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::tempdir;

fn compile_fake_creator(target_dir: &Path) -> PathBuf {
    fs::create_dir_all(target_dir).expect("platform dir");
    let target = target_dir.join(format!("1cv8{}", std::env::consts::EXE_SUFFIX));
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake_create_platform.rs");
    let compile = Command::new("rustc")
        .arg("--edition=2021")
        .arg(&source)
        .arg("-o")
        .arg(&target)
        .output()
        .expect("run rustc for the fake platform");
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    target
}

/// Проект с пустым набором и файловой базой `ib`, в которой лежат прежняя база, её журнал и
/// посторонний файл.
struct Stand {
    root: tempfile::TempDir,
    config: PathBuf,
    base: PathBuf,
}

impl Stand {
    fn new() -> Self {
        let root = tempdir().expect("tempdir");
        let platform = compile_fake_creator(&root.path().join("platform"));
        let project = root.path().join("project");
        let work = root.path().join("work");
        let base = root.path().join("ib");
        fs::create_dir_all(project.join("sources")).expect("project");
        fs::write(
            project.join("sources").join("Configuration.xml"),
            "<Configuration/>\n",
        )
        .expect("source");
        fs::create_dir_all(&work).expect("work");
        fs::create_dir_all(base.join("1Cv8Log")).expect("event log");
        fs::write(base.join("1Cv8.1CD"), "old").expect("old base");
        fs::write(base.join("1Cv8Log").join("1Cv8.lgf"), "log").expect("old log");
        fs::write(base.join("notes.txt"), "mine").expect("foreign file");
        let config = root.path().join("v8project.yaml");
        fs::write(
            &config,
            format!(
                "workPath: '{}'\nformat: DESIGNER\nproviders:\n  infobase.create: designer\ninfobase:\n  connection: 'File={}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/sources\ntools:\n  platform:\n    path: '{}'\n",
                work.display(),
                base.display(),
                platform.display(),
            ),
        )
        .expect("config");
        Self { root, config, base }
    }

    fn calls(&self) -> PathBuf {
        self.root.path().join("calls.log")
    }

    fn run(&self, extra: &[&str], fail: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_v8-runner"));
        command
            .arg("--config")
            .arg(&self.config)
            .args(["--json-message", "infobase", "create"])
            .args(extra)
            .env("V8_RUNNER_FAKE_PLATFORM_CALLS", self.calls());
        if fail {
            command.env("V8_RUNNER_FAKE_CREATE_FAIL", "1");
        }
        command.output().expect("run v8-runner")
    }

    fn aside(&self) -> PathBuf {
        self.root.path().join(".ib.v8-runner.recreate")
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.base.join(name)).unwrap_or_default()
    }
}

fn envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "json: {error}; stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn removed(envelope: &Value) -> Vec<(String, String)> {
    envelope["data"]["removed"]
        .as_array()
        .expect("removed")
        .iter()
        .map(|entry| {
            let name = entry["name"].as_str().expect("name");
            let file = Path::new(name).file_name().map_or_else(
                || name.to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
            (file, entry["state"].as_str().expect("state").to_owned())
        })
        .collect()
}

/// Держит файл базы, как держит его открытый сеанс платформы.
struct Held(#[allow(dead_code, reason = "держит дескриптор")] fs::File);

fn hold(base: &Path) -> Held {
    let path = base.join("1Cv8.1CD");
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Как держит платформа: с разделением чтения и записи, без исключительного доступа.
        Held(
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0x1 | 0x2)
                .open(path)
                .expect("hold the base"),
        )
    }
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .expect("open the base");
        // SAFETY: flock holds only integers; the descriptor lives as long as `Held`.
        let mut lock: libc::flock = unsafe { std::mem::zeroed() };
        lock.l_type = libc::F_RDLCK as _;
        lock.l_whence = libc::SEEK_SET as _;
        lock.l_start = 0x7FFF_FFFF_FFFF_FFFE;
        lock.l_len = 1;
        // SAFETY: the descriptor is open and `lock` is a valid request.
        assert_ne!(
            unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &lock) },
            -1,
            "lock the base"
        );
        Held(file)
    }
}

/// Пересоздание файловой базы убирает только записи `1Cv8*` верхнего уровня: новая база на
/// месте, посторонний файл цел, отложенного каталога не осталось, ответ называет удалённое.
#[test]
fn a_recreated_file_base_is_new_and_keeps_a_foreign_file() {
    let stand = Stand::new();

    let output = stand.run(&["--recreate"], false);

    let envelope = envelope(&output);
    assert!(output.status.success(), "{envelope}");
    assert_eq!(stand.read("1Cv8.1CD"), "new");
    assert_eq!(stand.read("notes.txt"), "mine");
    assert!(!stand.base.join("1Cv8Log").exists());
    assert!(!stand.aside().exists());
    assert_eq!(envelope["data"]["steps"][0]["action"], "recreate");
    assert_eq!(envelope["data"]["steps"][0]["status"], "ok");
    let removed = removed(&envelope);
    assert!(
        removed.contains(&("1Cv8.1CD".to_owned(), "removed".to_owned()))
            && removed.contains(&("1Cv8Log".to_owned(), "removed".to_owned()))
            && !removed.iter().any(|(name, _)| name == "notes.txt"),
        "{removed:?}"
    );
}

/// Превью ничего не пишет и называет, чья база и что было бы удалено.
#[test]
fn a_recreate_preview_names_what_it_would_remove_and_touches_nothing() {
    let stand = Stand::new();

    let output = stand.run(&["--recreate", "--dry-run"], false);

    let envelope = envelope(&output);
    assert!(output.status.success(), "{envelope}");
    let step = &envelope["data"]["steps"][0];
    assert_eq!(step["status"], "planned");
    assert!(
        step["message"]
            .as_str()
            .expect("message")
            .contains("this working copy's own"),
        "{step}"
    );
    assert!(removed(&envelope)
        .iter()
        .all(|(_, state)| state == "planned"));
    assert_eq!(removed(&envelope).len(), 2, "{envelope}");
    assert_eq!(envelope["data"]["steps"][1]["status"], "planned");
    assert_eq!(stand.read("1Cv8.1CD"), "old");
    assert!(stand.base.join("1Cv8Log").exists());
    assert!(!stand.aside().exists());
    assert!(!stand.calls().exists(), "the preview starts no platform");
}

/// Базу держит сеанс: отказ `infobase_sessions` до любых изменений, платформа не запускалась.
#[test]
fn a_recreate_of_a_held_base_is_refused_with_its_sessions_and_removes_nothing() {
    let stand = Stand::new();
    let held = hold(&stand.base);

    let output = stand.run(&["--recreate"], false);

    drop(held);
    let envelope = envelope(&output);
    assert_eq!(output.status.code(), Some(3), "{envelope}");
    assert_eq!(envelope["error"]["code"], "infobase_sessions", "{envelope}");
    assert_eq!(envelope["error"]["kind"], "workspace");
    let sessions = &envelope["error"]["sessions"];
    assert!(sessions["list"].is_null(), "{sessions}");
    assert_eq!(sessions["terminable"], false);
    assert_eq!(sessions["lock"]["state"], "none");
    assert_eq!(stand.read("1Cv8.1CD"), "old");
    assert!(stand.base.join("1Cv8Log").exists());
    assert!(!stand.aside().exists());
    assert!(!stand.calls().exists());
}

/// Базу держит другая команда раннера: отказ `infobase_busy`, файлы целы.
#[test]
fn a_recreate_of_a_base_held_by_another_command_is_refused_as_busy() {
    let stand = Stand::new();
    let parent = fs::canonicalize(stand.root.path()).expect("canonical root");
    fs::write(
        parent.join(".ib.v8-runner.infobase.lock"),
        format!(
            "{{\"tool\":\"v8-runner\",\"pid\":{},\"owner_id\":\"other\",\"created_at\":\"2026-09-02T00:00:00Z\"}}",
            std::process::id()
        ),
    )
    .expect("held lock");

    let output = stand.run(&["--recreate"], false);

    let envelope = envelope(&output);
    assert_eq!(envelope["error"]["code"], "infobase_busy", "{envelope}");
    assert_eq!(stand.read("1Cv8.1CD"), "old");
    assert!(!stand.calls().exists());
}

/// Неудачное создание возвращает прежнюю базу и убирает то, что оставило само.
#[test]
fn a_failed_recreation_puts_the_old_base_back() {
    let stand = Stand::new();

    let output = stand.run(&["--recreate"], true);

    let envelope = envelope(&output);
    assert!(!output.status.success(), "{envelope}");
    assert_eq!(stand.read("1Cv8.1CD"), "old");
    assert!(stand.base.join("1Cv8Log").join("1Cv8.lgf").exists());
    assert_eq!(stand.read("notes.txt"), "mine");
    assert!(!stand.aside().exists());
    assert!(
        removed(&envelope)
            .iter()
            .all(|(_, state)| state == "restored"),
        "{envelope}"
    );
    let message = envelope["data"]["steps"][0]["message"]
        .as_str()
        .expect("message");
    assert!(message.contains("put back"), "{message}");
}

/// Остаток пересоздания, оборванного аварийным выходом, называют и `create`, и `--recreate`.
#[test]
fn a_leftover_of_an_interrupted_recreation_is_named() {
    let stand = Stand::new();
    fs::remove_file(stand.base.join("1Cv8.1CD")).expect("moved away");
    fs::create_dir_all(stand.aside()).expect("leftover");
    fs::write(stand.aside().join("1Cv8.1CD"), "old").expect("old base aside");

    for extra in [&["--recreate"][..], &[][..]] {
        let output = stand.run(extra, false);
        let envelope = envelope(&output);
        assert!(!output.status.success(), "{extra:?}: {envelope}");
        let message = envelope["error"]["message"].as_str().expect("message");
        assert!(message.contains(".ib.v8-runner.recreate"), "{message}");
    }
    assert!(!stand.calls().exists());
    assert_eq!(
        fs::read_to_string(stand.aside().join("1Cv8.1CD")).expect("aside"),
        "old"
    );
}

/// Без ключа существующая база — отказ, и он советует `--recreate`; без базы `--recreate`
/// просто создаёт её, и удалять было нечего.
#[test]
fn create_refuses_an_existing_base_with_the_recreate_advice_and_recreate_creates_a_missing_one() {
    let stand = Stand::new();
    let output = stand.run(&[], false);
    let envelope = envelope(&output);
    assert!(
        envelope["error"]["message"]
            .as_str()
            .expect("message")
            .contains("--recreate"),
        "{envelope}"
    );

    fs::remove_dir_all(&stand.base).expect("no base");
    let output = stand.run(&["--recreate"], false);
    let envelope = self::envelope(&output);
    assert!(output.status.success(), "{envelope}");
    assert_eq!(envelope["data"]["removed"], serde_json::json!([]));
    assert_eq!(stand.read("1Cv8.1CD"), "new");
}

/// База другой рабочей копии пересоздаётся с предупреждением, которое называет владельца.
#[test]
fn a_base_of_another_copy_is_recreated_with_a_warning() {
    let stand = Stand::new();
    let other = stand.root.path().join("other");
    fs::create_dir_all(&other).expect("other copy");
    let marker = serde_json::json!({
        "version": 2,
        "owners": [{
            "machine": "0".repeat(64),
            "host": "elsewhere",
            "project": fs::canonicalize(&other).expect("canonical other"),
            "since": "2026-01-01T00:00:00Z",
        }],
    });
    fs::write(
        stand.root.path().join(".ib.v8-runner.owners.json"),
        serde_json::to_string_pretty(&marker).expect("marker"),
    )
    .expect("marker");

    for extra in [&["--recreate", "--dry-run"][..], &["--recreate"][..]] {
        let output = stand.run(extra, false);
        let envelope = envelope(&output);
        assert!(output.status.success(), "{extra:?}: {envelope}");
        let warned = envelope["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .filter_map(Value::as_str)
            .any(|warning| warning.contains("elsewhere"));
        assert!(warned, "{extra:?}: {envelope}");
    }
    assert_eq!(stand.read("1Cv8.1CD"), "new");
}
