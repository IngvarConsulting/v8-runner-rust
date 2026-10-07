//! Замок файловой базы: две рабочие копии одной базы на этой машине.
//!
//! Каждая копия — свой проект со своим `workPath`, а база у них одна. Первая копия держит
//! базу командой, которую заглушка платформы не отпускает, пока тест не создаст файл
//! `release`; вторая в это время приходит к той же базе.
//!
//! Командная строка называет базу строкой соединения в `--infobase`: такая команда
//! подчиняется владельцу базы, но им не становится, поэтому обе копии работают с одной базой
//! и после того, как первая команда кончилась. Владельца базы проверяет
//! `tests/cli_infobase_owner.rs`; здесь — только замок.
#![cfg(unix)]

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use support::{
    hold_workspace_lock, interruptible_stub, temp_workspace, v8_runner_command, wait_for_file,
    write_shell_script, RunnerGuard,
};

/// Отказ занятой базы приходит сразу: заглушка первой копии держит базу полминуты.
const AT_ONCE: Duration = Duration::from_secs(10);

/// Поддельная платформа, которая сразу отвечает успехом и пишет снимок для `/DumpIB`.
const QUICK_PLATFORM: &str = r#"previous=''
for argument in "$@"; do
  case "$previous" in
    /DumpIB) printf 'payload' > "$argument" ;;
  esac
  previous="$argument"
done
exit 0"#;

struct Stand {
    dir: tempfile::TempDir,
    /// Каталог файловой базы, общей для обеих копий.
    base: PathBuf,
}

struct Copy {
    root: PathBuf,
    config: PathBuf,
    work: PathBuf,
    platform: PathBuf,
    /// Строка соединения общей базы для `--infobase`.
    connection: String,
}

impl Stand {
    fn new() -> Self {
        let dir = temp_workspace();
        let base = dir.path().join("shared").join("ib");
        fs::create_dir_all(&base).expect("infobase dir");
        fs::write(base.join("1Cv8.1CD"), "database").expect("infobase file");
        Self { dir, base }
    }

    /// Рабочая копия `name` с быстрой поддельной платформой.
    fn copy(&self, name: &str) -> Copy {
        let root = self.dir.path().join(name);
        let sources = root.join("sources");
        fs::create_dir_all(&sources).expect("sources");
        fs::write(sources.join("Configuration.xml"), "<Configuration/>\n").expect("source");
        let platform = root.join("1cv8");
        write_shell_script(&platform, QUICK_PLATFORM);
        let work = root.join("work");
        let config = root.join("v8project.yaml");
        fs::write(
            &config,
            format!(
                "workPath: '{}'\nformat: DESIGNER\nproviders:\n  push: designer\n  pull: designer\n  download: designer\n  infobase.dump: designer\n  infobase.restore: designer\ninfobase:\n  connection: 'File={}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\ntools:\n  platform:\n    path: '{}'\n",
                work.display(),
                self.base.display(),
                platform.display(),
            ),
        )
        .expect("config");
        // Память о базе, как после её создания раннером: тесты замка начинают не с первого
        // знакомства.
        let base = support::memory::Base::File(&self.base);
        let key = support::memory::ad_hoc_key(&base);
        support::memory::remember_base(
            &work,
            &key,
            base,
            &[support::memory::Set::configuration("main", &sources)],
        );
        Copy {
            root,
            config,
            work,
            platform,
            connection: format!("File={}", self.base.display()),
        }
    }

    /// Файлы замка базы рядом с её каталогом.
    fn lock_files(&self) -> Vec<String> {
        let parent = self.base.parent().expect("base parent");
        let mut names: Vec<String> = fs::read_dir(parent)
            .expect("base parent")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.contains(".lock"))
            .collect();
        names.sort();
        names
    }
}

/// Команда первой копии, которая держит базу. Сброс отпускает заглушку платформы и
/// снимает раннер, даже если тест упал раньше.
struct Holder {
    release: PathBuf,
    runner: RunnerGuard,
}

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = fs::write(&self.release, "");
    }
}

impl Copy {
    fn run(&self, args: &[&str]) -> Output {
        v8_runner_command()
            .arg("--config")
            .arg(&self.config)
            .arg("--infobase")
            .arg(&self.connection)
            .arg("--json-message")
            .args(args)
            .output()
            .expect("run CLI")
    }

    /// Запускает `push`, который держит базу, пока тест не создаст `release`, и
    /// возвращает раннер, когда платформа уже получила работу.
    fn hold_the_base(&self) -> Holder {
        let started = self.root.join("platform-started");
        let release = self.root.join("platform-release");
        write_shell_script(&self.platform, &interruptible_stub(&started, &release));
        let runner = RunnerGuard(
            v8_runner_command()
                .arg("--config")
                .arg(&self.config)
                .arg("--infobase")
                .arg(&self.connection)
                .arg("--json-message")
                .arg("push")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn push"),
        );
        assert!(
            wait_for_file(&started, Duration::from_secs(30)),
            "the holding push never reached the platform"
        );
        Holder { release, runner }
    }

    fn canonical_work(&self) -> String {
        fs::canonicalize(&self.work)
            .expect("canonical work")
            .display()
            .to_string()
    }
}

fn envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no json envelope: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn succeeded(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    envelope(output)
}

/// Отказ занятой базы: код, род, шаг и то, что он называет первую команду.
fn assert_infobase_busy(output: &Output, command: &str, holder: &Copy) {
    let payload = envelope(output);
    assert_eq!(output.status.code(), Some(3), "{payload}");
    assert_eq!(payload["command"], command, "{payload}");
    assert_eq!(payload["error"]["code"], "infobase_busy", "{payload}");
    assert_eq!(payload["error"]["kind"], "workspace", "{payload}");
    assert_eq!(payload["steps"][0]["name"], "infobase lock", "{payload}");
    assert_eq!(payload["steps"][0]["status"], "failed", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("'push'"),
        "names the holding command: {message}"
    );
    assert!(
        message.contains(&holder.canonical_work()),
        "names the holding working copy: {message}"
    );
}

/// Вторая команда на занятой базе не ждёт: отказывает сразу и называет первую. Замок живёт
/// ровно столько, сколько первая команда: после неё вторая проходит.
#[test]
fn a_second_command_on_a_held_base_is_refused_at_once_and_names_the_first() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    let mut holder = first.hold_the_base();

    let started = Instant::now();
    let refused = second.run(&["push"]);
    let waited = started.elapsed();

    assert_infobase_busy(&refused, "push", &first);
    assert!(waited < AT_ONCE, "the refusal waited {waited:?}");

    fs::write(&holder.release, "").expect("release the first push");
    let status = holder.runner.0.wait().expect("first push");
    assert!(status.success(), "the first push finished");
    succeeded(&second.run(&["push"]));
    assert_eq!(stand.lock_files(), Vec::<String>::new());
}

/// После `kill -9` ОС снимает замок, а файлы его остаются; следующая команда из другой
/// рабочей копии их не пугается и уносит с собой.
#[test]
fn a_base_held_by_a_killed_command_lets_the_next_one_in() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    let mut holder = first.hold_the_base();

    holder.runner.0.kill().expect("kill -9 the holding push");
    holder.runner.0.wait().expect("killed push");
    fs::write(&holder.release, "").expect("release the orphaned platform");
    assert_ne!(
        stand.lock_files(),
        Vec::<String>::new(),
        "a killed command cannot remove its lock files"
    );

    succeeded(&second.run(&["push"]));
    assert_eq!(stand.lock_files(), Vec::<String>::new());
}

/// Замок базы берётся после замка `workPath`: когда заняты оба, отказ — про каталог.
#[test]
fn a_busy_work_path_is_refused_before_the_base_lock() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    let _holder = first.hold_the_base();
    hold_workspace_lock(&second.work);

    let refused = second.run(&["push"]);

    let payload = envelope(&refused);
    assert_eq!(payload["error"]["code"], "workspace_busy", "{payload}");
    assert_eq!(payload["steps"][0]["name"], "workspace lock", "{payload}");
}

/// Превью замка базы не берёт: на занятой базе оно проходит.
#[test]
fn a_preview_on_a_held_base_takes_no_base_lock() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    let _holder = first.hold_the_base();

    let preview = succeeded(&second.run(&["push", "--dry-run"]));

    assert_eq!(preview["ok"], true, "{preview}");
}

/// Делает так, что рядом с базой замок не взять не из-за другой команды: каталог рядом с
/// базой закрыт на запись. Под root права не действуют, и тогда имя файла блокировки
/// занято каталогом — открыть его на запись нельзя всё равно.
fn refuse_the_lock_beside(stand: &Stand) -> impl Drop {
    struct Restore(PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        }
    }
    let parent = stand.base.parent().expect("base parent").to_path_buf();
    let probe = parent.join("probe");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).expect("chmod");
    if fs::write(&probe, "").is_ok() {
        fs::remove_file(&probe).expect("remove probe");
        fs::create_dir(parent.join(".ib.v8-runner.infobase.lock.system")).expect("blocker");
    }
    Restore(parent)
}

/// Если замок базы не взять не потому, что её держит другая команда, команда записи
/// отказывает и называет каталог и причину, а команда чтения идёт дальше и говорит об этом.
#[test]
fn a_write_without_the_base_lock_is_refused_and_a_read_goes_on_with_a_warning() {
    let stand = Stand::new();
    let copy = stand.copy("copy");
    let _restore = refuse_the_lock_beside(&stand);
    let parent = fs::canonicalize(stand.base.parent().expect("parent"))
        .expect("canonical parent")
        .display()
        .to_string();

    let refused = copy.run(&["push"]);
    let payload = envelope(&refused);
    assert!(!refused.status.success(), "{payload}");
    assert_eq!(payload["error"]["code"], "runtime_failure", "{payload}");
    assert_eq!(payload["steps"][0]["name"], "infobase lock", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains(&parent), "names the directory: {message}");
    assert!(
        message.contains("denied") || message.contains("directory"),
        "names the reason: {message}"
    );

    let snapshot = copy.root.join("base.dt");
    let dump = succeeded(&copy.run(&[
        "infobase",
        "dump",
        "--output",
        snapshot.to_str().expect("snapshot path"),
    ]));
    assert!(snapshot.is_file(), "{dump}");
    let warnings = dump["warnings"].as_array().expect("warnings");
    assert!(
        warnings.iter().any(|warning| warning
            .as_str()
            .is_some_and(|text| text.contains("infobase lock") && text.contains(&parent))),
        "{dump}"
    );
}

/// Инструмент MCP на занятой базе отказывает сразу, кодом своего словаря, и называет базу
/// и команду, которая её держит.
#[test]
fn an_mcp_tool_on_a_held_base_is_refused_at_once() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    let _holder = first.hold_the_base();

    let started = Instant::now();
    let answer = support::mcp::call_tool(&second.config, "build_project", json!({}));
    let waited = started.elapsed();

    assert!(answer.is_error, "{}", answer.envelope);
    let payload = &answer.envelope;
    assert_eq!(payload["error"]["code"], "runtime_failure", "{payload}");
    assert_eq!(payload["error"]["kind"], "runtime", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    let base = fs::canonicalize(&stand.base)
        .expect("canonical base")
        .display()
        .to_string();
    assert!(message.contains(&base), "names the base: {message}");
    assert!(message.contains("'push'"), "names the holder: {message}");
    assert!(waited < AT_ONCE, "the refusal waited {waited:?}");
}

/// `launch` держит замок базы только своей командой: клиент, которого он запустил, дальше
/// держит базу сам, и следующая команда раннера проходит, пока клиент жив.
#[test]
fn launch_releases_the_base_lock_while_its_client_runs() {
    let stand = Stand::new();
    let copy = stand.copy("copy");
    let other = stand.copy("other");
    let install = copy.root.join("platform");
    write_shell_script(&install.join("bin").join("1cv8"), QUICK_PLATFORM);
    write_shell_script(&install.join("bin").join("1cv8c"), "sleep 30");
    fs::write(
        &copy.config,
        fs::read_to_string(&copy.config).expect("config").replace(
            &copy.platform.display().to_string(),
            &install.display().to_string(),
        ),
    )
    .expect("config with the platform directory");

    let launched = succeeded(&copy.run(&["launch", "thin"]));
    let pid = launched["data"]["pid"].as_u64().expect("client pid");
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|status| status.success())
    };

    let next = other.run(&["push"]);
    let client_was_alive = alive();
    let _ = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status();

    succeeded(&next);
    assert!(
        client_was_alive,
        "the client still ran during the next command"
    );
}
