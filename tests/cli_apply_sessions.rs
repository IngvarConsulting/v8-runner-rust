//! `push`/`apply --sessions` и отказы `infobase_sessions` и `infobase_designer_busy` (#211).
//!
//! Поддельный Конфигуратор на `/UpdateDBCfg` без `-SessionTerminate` при файле `sessions` пишет
//! в `/Out` текст отказа, снятый на 8.3.27.2342, и выходит с 1; с ключом — применяет. На
//! `/LoadConfigFromFiles` при файле `designer` отказывает текстом блокировки для
//! конфигурирования. Сеанс файловой базы изображает процесс, который держит блокировку `fcntl`
//! на `1Cv8.1CD` (у Конфигуратора — на `1Cv8.cgr`), как держат их клиенты платформы (замер
//! #435 и #468).
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

const TOKEN: &str = "1111111111111111111111111111111111111111";

/// Текст `/Out`, снятый на 8.3.27.2342 у файловой базы с тонким и толстым клиентом.
const MEASURED_OUT: &str = "Обработка структуры базы данных...\n\nОшибка исключительной блокировки информационной базы.\nАктивны сеансы:\nкомпьютер: w11p-igor, сеанс: 2, начат: 11.11.2026 в 2:49:42, приложение: Тонкий клиент\nкомпьютер: w11p-igor, сеанс: 1, начат: 11.11.2026 в 2:49:41, приложение: Толстый клиент\n";

fn platform(root: &Path) -> String {
    format!(
        r#"printf '%s\n' "$*" >> '{calls}'
out=''
previous=''
for arg in "$@"; do
  if [ "$previous" = '/Out' ]; then out="$arg"; fi
  previous="$arg"
done
case "$*" in
  *'/GetConfigGenerationID'*)
    cat '{token}' > "$out"
    exit 0 ;;
  *'/UpdateDBCfg'*'-SessionTerminate force'*)
    : > "$out"
    exit 0 ;;
  *'/UpdateDBCfg'*)
    if [ -f '{sessions}' ]; then cp '{measured}' "$out"; exit 1; fi ;;
  *'/LoadConfigFromFiles'*)
    if [ -f '{designer}' ]; then
      printf '%s\n' 'Ошибка блокировки информационной базы для конфигурирования.' 'Возможно, информационная база уже открыта Конфигуратором.' > "$out"
      exit 1
    fi ;;
esac
if [ -n "$out" ]; then : > "$out"; fi
exit 0"#,
        calls = root.join("calls.log").display(),
        token = root.join("token").display(),
        sessions = root.join("sessions").display(),
        measured = root.join("measured.out").display(),
        designer = root.join("designer").display(),
    )
}

struct Project {
    dir: tempfile::TempDir,
    config: PathBuf,
}

impl Project {
    fn new(connection: &str) -> Self {
        let dir = temp_workspace();
        let root = dir.path();
        let sources = root.join("sources");
        fs::create_dir_all(&sources).expect("sources");
        fs::write(sources.join("Configuration.xml"), "<Configuration/>\n").expect("source");
        fs::write(sources.join("Module.bsl"), "Procedure A()\nEndProcedure\n").expect("module");
        fs::write(root.join("token"), format!("{TOKEN}\r\n")).expect("token");
        fs::write(root.join("measured.out"), MEASURED_OUT).expect("measured");
        write_shell_script(&root.join("1cv8"), &platform(root));
        let base = root.join("ib");
        fs::create_dir_all(&base).expect("base");
        fs::write(base.join("1Cv8.1CD"), "database").expect("database file");
        let config = root.join("v8project.yaml");
        fs::write(
            &config,
            format!(
                "workPath: work\nformat: DESIGNER\n{designer_leads}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\ntools:\n  platform:\n    path: '{}'\n",
                root.join("1cv8").display(),
                designer_leads = support::DESIGNER_LEADS,
            ),
        )
        .expect("config");
        let connection = connection.replace("{base}", &base.display().to_string());
        fs::write(
            root.join("v8project.local.yaml"),
            format!("infobases:\n  origin:\n    connection: '{connection}'\n"),
        )
        .expect("local layer");
        Self { dir, config }
    }

    fn file() -> Self {
        Self::new("File={base}")
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn base(&self) -> PathBuf {
        self.root().join("ib")
    }

    fn mark(&self, name: &str) {
        fs::write(self.root().join(name), "").expect("mark");
    }

    fn run(&self, args: &[&str]) -> Output {
        v8_runner_command()
            .arg("--config")
            .arg(&self.config)
            .arg("--json-message")
            .args(args)
            .output()
            .expect("run CLI")
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.root().join("calls.log")).unwrap_or_default()
    }

    fn edit(&self) {
        let module = self.root().join("sources").join("Module.bsl");
        let text = fs::read_to_string(&module).expect("module");
        fs::write(&module, format!("{text}// edited\n")).expect("edit");
    }

    /// Процесс, который держит блокировку `fcntl` на файле базы, пока жив.
    fn hold(&self, file: &str, exclusive: bool) -> support::RunnerGuard {
        let path = self.base().join(file);
        if !path.exists() {
            fs::write(&path, "held").expect("held file");
        }
        let ready = self.root().join(format!("{file}.held"));
        let (mode, lock) = if exclusive {
            ("r+b", "LOCK_EX")
        } else {
            ("rb", "LOCK_SH")
        };
        let child = std::process::Command::new("python3")
            .args([
                "-c",
                &format!(
                    "import fcntl, pathlib, sys, time; f=open(sys.argv[1], '{mode}'); fcntl.lockf(f, fcntl.{lock}); pathlib.Path(sys.argv[2]).touch(); time.sleep(60)"
                ),
            ])
            .arg(&path)
            .arg(&ready)
            .spawn()
            .expect("holder");
        let holder = support::RunnerGuard(child);
        assert!(
            support::wait_for_file(&ready, Duration::from_secs(10)),
            "the holder must lock {file}"
        );
        holder
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
    let payload = envelope(output);
    assert!(output.status.success(), "{payload}");
    payload
}

/// Отказ файловой базы называет сеансы из текста платформы, а выход — тот же вызов без
/// ключей: завершать сеансы файловой базы раннер не может.
#[test]
fn a_file_base_held_by_a_session_is_refused_with_the_session_list() {
    let project = Project::file();
    succeeded(&project.run(&["push", "--force"]));
    project.mark("sessions");
    let _client = project.hold("1Cv8.1CD", false);

    let refused = project.run(&["apply", "main"]);
    let payload = envelope(&refused);

    assert_eq!(refused.status.code(), Some(3), "{payload}");
    let error = &payload["error"];
    assert_eq!(error["code"], "infobase_sessions", "{payload}");
    assert_eq!(error["kind"], "workspace", "{payload}");
    let sessions = &error["sessions"];
    assert_eq!(sessions["terminable"], false, "{payload}");
    assert_eq!(sessions["lock"]["state"], "none", "{payload}");
    assert_eq!(sessions["from_this_machine"], false, "{payload}");
    assert!(
        sessions["base"]
            .as_str()
            .unwrap_or_default()
            .starts_with("File="),
        "{payload}"
    );
    let list = sessions["list"].as_array().expect("list");
    assert_eq!(list.len(), 2, "{payload}");
    assert_eq!(list[0]["computer"], "w11p-igor");
    assert_eq!(list[0]["application"], "1CV8C");
    assert_eq!(list[0]["application_title"], "Тонкий клиент");
    assert_eq!(list[0]["started"], "2026-11-11T02:49:42");
    assert_eq!(list[0]["user"], Value::Null);
    assert_eq!(list[0]["id"], "2");
    assert_eq!(list[1]["application"], "1CV8");
    assert_eq!(error["next"]["command"], "apply", "{payload}");
    assert_eq!(error["next"]["source_set"], "main", "{payload}");
    assert!(error["next"].get("keys").is_none(), "{payload}");
}

/// `--sessions force` у файловой базы отказывает так же: ключ платформы ей не помогает, и
/// раннер его не передаёт, блокировку начала сеансов не ставит.
#[test]
fn force_does_not_end_sessions_of_a_file_base() {
    let project = Project::file();
    succeeded(&project.run(&["push", "--force"]));
    project.mark("sessions");
    let _client = project.hold("1Cv8.1CD", false);
    let _ = fs::remove_file(project.root().join("calls.log"));

    let refused = project.run(&["apply", "--sessions", "force"]);
    let payload = envelope(&refused);

    assert_eq!(refused.status.code(), Some(3), "{payload}");
    assert_eq!(payload["error"]["code"], "infobase_sessions", "{payload}");
    assert_eq!(
        payload["error"]["sessions"]["terminable"], false,
        "{payload}"
    );
    assert!(
        !project.calls().contains("-SessionTerminate"),
        "{}",
        project.calls()
    );
    assert!(!project.base().join("1Cv8.cdn").exists());
}

/// Отправка, применение которой не пустили сеансы, сохраняет загрузку, а выход — та же
/// отправка: следующая `push` применит загруженное.
#[test]
fn a_push_held_by_sessions_keeps_the_load_and_names_the_same_push() {
    let project = Project::file();
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    project.mark("sessions");
    let client = project.hold("1Cv8.1CD", false);

    let refused = project.run(&["push"]);
    let payload = envelope(&refused);

    assert_eq!(refused.status.code(), Some(3), "{payload}");
    assert_eq!(payload["error"]["code"], "infobase_sessions", "{payload}");
    assert_eq!(payload["error"]["next"]["command"], "push", "{payload}");
    assert!(
        payload["error"]["next"].get("source_set").is_none(),
        "{payload}"
    );
    assert_eq!(payload["data"]["steps"][0]["applied"], false, "{payload}");

    drop(client);
    fs::remove_file(project.root().join("sessions")).expect("sessions closed");
    let _ = fs::remove_file(project.root().join("calls.log"));
    succeeded(&project.run(&["push"]));
    assert!(
        !project.calls().contains("/LoadConfigFromFiles"),
        "{}",
        project.calls()
    );
    assert!(
        project.calls().contains("/UpdateDBCfg"),
        "{}",
        project.calls()
    );
}

/// Чужой Конфигуратор держит файловую базу: загрузка отказывает раньше применения, а отказ —
/// отдельный, без списка и без `force`.
#[test]
fn a_file_base_open_in_the_designer_refuses_the_load() {
    let project = Project::file();
    succeeded(&project.run(&["push", "--force"]));
    project.edit();
    project.mark("designer");
    let _designer = project.hold("1Cv8.cgr", true);
    let _ = fs::remove_file(project.root().join("calls.log"));

    let refused = project.run(&["push", "main"]);
    let payload = envelope(&refused);

    assert_eq!(refused.status.code(), Some(3), "{payload}");
    let error = &payload["error"];
    assert_eq!(error["code"], "infobase_designer_busy", "{payload}");
    assert_eq!(error["kind"], "workspace", "{payload}");
    assert_eq!(error["sessions"]["list"], Value::Null, "{payload}");
    assert_eq!(error["sessions"]["terminable"], false, "{payload}");
    assert_eq!(error["next"]["command"], "push", "{payload}");
    assert_eq!(error["next"]["source_set"], "main", "{payload}");
    assert!(
        !project.calls().contains("/UpdateDBCfg"),
        "{}",
        project.calls()
    );
}

/// Кластер: `--sessions force` после отказа повторяет применение с `-SessionTerminate force`,
/// а ответ называет завершённые сеансы из списка первого отказа.
#[test]
fn force_on_a_cluster_ends_the_sessions_and_names_them() {
    let project = Project::new("Srvr=server;Ref=main");
    succeeded(&project.run(&["push", "--force"]));
    project.mark("sessions");
    let _ = fs::remove_file(project.root().join("calls.log"));

    let applied = succeeded(&project.run(&["apply", "--sessions", "force"]));

    let sessions = &applied["data"]["sessions"];
    assert_eq!(sessions["lock"]["state"], "unknown", "{applied}");
    assert_eq!(sessions["message_delivered"], false, "{applied}");
    let terminated = sessions["terminated"].as_array().expect("terminated");
    assert_eq!(terminated.len(), 2, "{applied}");
    assert_eq!(terminated[1]["application_title"], "Толстый клиент");
    let calls = project.calls();
    let updates = calls
        .lines()
        .filter(|line| line.contains("/UpdateDBCfg"))
        .collect::<Vec<_>>();
    assert_eq!(updates.len(), 2, "{calls}");
    assert!(updates[1].ends_with("-SessionTerminate force"), "{calls}");
}

/// Сообщение без `force` некому отдать: это ошибка вызова, платформу она не запускает.
#[test]
fn a_sessions_message_without_force_is_refused() {
    let project = Project::file();

    let refused = project.run(&["apply", "--sessions-message", "bye"]);
    let payload = envelope(&refused);

    assert_eq!(refused.status.code(), Some(2), "{payload}");
    assert_eq!(payload["error"]["code"], "invalid_argument", "{payload}");
    assert!(project.calls().is_empty(), "{}", project.calls());
}

/// `push --no-apply` не применяет, и ключа сеансов у него нет; `make` его не знает вовсе.
#[test]
fn push_without_apply_takes_no_sessions_key() {
    let project = Project::file();

    for args in [
        &["push", "--no-apply", "--sessions", "force"][..],
        &["make", "--sessions", "force"][..],
    ] {
        let refused = project.run(args);
        assert_eq!(refused.status.code(), Some(2), "{args:?}");
    }
    assert!(project.calls().is_empty(), "{}", project.calls());
}
