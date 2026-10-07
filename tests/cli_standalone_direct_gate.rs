//! Прямой шлюз автономного сервера: Конфигуратор идёт в него строкой
//! `Srvr=<host>:<port>;Ref=<name>`, как в кластер, и стоит первым исполнителем; агент по
//! SSH-шлюзу — вторым (#205).
//!
//! Команды Конфигуратора через прямой шлюз замерены вручную (#178, #179,
//! `references/1c/confirmed-runtime-measurements.md`); здесь проверяется путь раннера —
//! кого он выбирает, какую командную строку собирает и где лежат файлы. Поддельный
//! Конфигуратор пишет каждый вызов строкой в журнал и создаёт то, что команда должна
//! оставить. SSH-шлюз объявлен портом, который никто не слушает: обращение к нему уронило
//! бы команду.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use support::{free_tcp_port, temp_workspace, v8_runner_command, write_shell_script};

/// Строка прямого шлюза, как её выводит замер #178: основной порт и имя `--name`.
const DIRECT_GATE: &str = "Srvr=127.0.0.1:1541;Ref=demo";
/// Тот же адрес ключом `/S`, которым раннер отдаёт серверную строку платформе.
const DIRECT_GATE_SWITCH: &str = "127.0.0.1:1541\\demo";

fn platform(root: &Path) -> String {
    format!(
        r#"printf '%s\n' "$*" >> '{calls}'
out=''
target=''
file=''
previous=''
for arg in "$@"; do
  if [ "$previous" = '/Out' ]; then out="$arg"; fi
  if [ "$previous" = '/DumpConfigToFiles' ]; then target="$arg"; fi
  case "$previous" in /DumpDBCfg|/DumpCfg|/DumpIB) file="$arg" ;; esac
  previous="$arg"
done
if [ -n "$target" ]; then
  mkdir -p "$target"
  printf '<Configuration/>\n' > "$target/Configuration.xml"
  printf '<ConfigDumpInfo version="2.17"/>\n' > "$target/ConfigDumpInfo.xml"
fi
if [ -n "$file" ]; then
  mkdir -p "$(dirname "$file")"
  printf 'payload' > "$file"
fi
if [ -n "$out" ]; then : > "$out"; fi
exit 0"#,
        calls = root.join("calls.log").display(),
    )
}

struct Project {
    dir: tempfile::TempDir,
    config: PathBuf,
}

impl Project {
    /// Проект с набором `main`, Конфигуратором на машине раннера и автономным сервером,
    /// объявленным секцией `standalone` (`section` — её тело без отступа базы).
    fn new(connection: &str, section: &str) -> Self {
        let dir = temp_workspace();
        let root = dir.path();
        let sources = root.join("sources");
        fs::create_dir_all(&sources).expect("sources");
        fs::write(sources.join("Configuration.xml"), "<Configuration/>\n").expect("source");
        fs::write(sources.join("Module.bsl"), "Procedure A()\nEndProcedure\n").expect("module");
        write_shell_script(
            &root.join("platform").join("bin").join("1cv8"),
            &platform(root),
        );
        let config = root.join("v8project.yaml");
        fs::write(
            &config,
            format!(
                "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\ntools:\n  platform:\n    path: '{}'\n",
                root.join("platform").display()
            ),
        )
        .expect("config");
        let connection = if connection.is_empty() {
            String::new()
        } else {
            format!("    connection: '{connection}'\n")
        };
        fs::write(
            root.join("v8project.local.yaml"),
            format!(
                "infobases:\n  origin:\n{connection}    user: Admin\n    password: s3cret\n    standalone:{section}\n"
            ),
        )
        .expect("local layer");
        Self { dir, config }
    }

    /// Только прямой шлюз: ни SSH-шлюза, ни канала обмена.
    fn direct_gate_only() -> Self {
        Self::new(DIRECT_GATE, " {}")
    }

    /// Оба шлюза; SSH-шлюз на порту, который никто не слушает.
    fn both_gates() -> Self {
        let section = format!(
            "\n      gate: 127.0.0.1:{}\n      exchange: sftp",
            free_tcp_port()
        );
        Self::new(DIRECT_GATE, &section)
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn work(&self) -> PathBuf {
        self.root().join("work")
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

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.root().join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
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
    assert_eq!(payload["ok"], true, "{payload}");
    payload
}

/// Каждый вызов Конфигуратора идёт в прямой шлюз ключом `/S` с реквизитами базы.
fn assert_every_call_goes_to_the_direct_gate(calls: &[String]) {
    assert!(!calls.is_empty(), "the Designer was not started");
    for call in calls {
        assert!(
            call.contains(&format!("/S {DIRECT_GATE_SWITCH}")) && call.contains("/N Admin"),
            "{call}"
        );
    }
}

/// Лежит ли путь под каталогом — как он записан или в каноническом виде (под macOS
/// временный каталог живёт за ссылкой `/var` → `/private/var`).
fn under(path: &Path, base: &Path) -> bool {
    path.starts_with(base) || fs::canonicalize(base).is_ok_and(|base| path.starts_with(base))
}

/// Пути, которые раннер дал Конфигуратору после ключа, — пути машины раннера.
fn path_after(call: &str, switch: &str) -> PathBuf {
    let words = call.split_whitespace().collect::<Vec<_>>();
    let position = words
        .iter()
        .position(|word| *word == switch)
        .unwrap_or_else(|| panic!("{switch} in {call}"));
    PathBuf::from(words[position + 1])
}

/// Секции `standalone` без SSH-шлюза и без канала обмена достаточно строки прямого
/// шлюза: конфиг принимается без предупреждений, `push` и `pull` исполняет Конфигуратор,
/// SSH-шлюза никто не ищет.
#[test]
fn a_standalone_server_with_only_the_direct_gate_is_served_by_the_designer() {
    let project = Project::direct_gate_only();

    let push = succeeded(&project.run(&["push", "--force"]));
    let pull = succeeded(&project.run(&["pull", "--force"]));

    for payload in [&push, &pull] {
        assert_eq!(
            payload["data"]["provider"]["selected"], "designer",
            "{payload}"
        );
        assert!(
            payload["data"]["provider"].get("skipped").is_none(),
            "{payload}"
        );
        assert!(
            payload["data"]["provider"].get("endpoint").is_none(),
            "{payload}"
        );
        assert!(
            payload["warnings"]
                .as_array()
                .is_none_or(|warnings| warnings.is_empty()),
            "{payload}"
        );
    }
    let calls = project.calls();
    assert_every_call_goes_to_the_direct_gate(&calls);
    for switch in ["/LoadConfigFromFiles", "/UpdateDBCfg", "/DumpConfigToFiles"] {
        assert!(
            calls.iter().any(|call| call.contains(switch)),
            "{switch}: {calls:?}"
        );
    }
}

/// При обоих шлюзах первым стоит Конфигуратор: команда идёт в прямой шлюз, а SSH-шлюз,
/// на порту которого никто не слушает, не тронут.
#[test]
fn the_designer_leads_the_chain_when_both_gates_are_declared() {
    let project = Project::both_gates();

    let payload = succeeded(&project.run(&["pull", "--force"]));

    assert_eq!(
        payload["data"]["provider"]["selected"], "designer",
        "{payload}"
    );
    assert_eq!(
        payload["data"]["provider"]["origin"]["kind"], "default",
        "{payload}"
    );
    assert_every_call_goes_to_the_direct_gate(&project.calls());
}

/// По прямому шлюзу файлы остаются у раннера: Конфигуратор получает пути машины раннера —
/// исходники проекта, каталог выгрузки рядом с ними и журнал `/Out` под `workPath`, — и
/// канал обмена ему не нужен. Журналы, которые называет ответ, лежат под `workPath`, а не
/// на стороне цели.
#[test]
fn the_designer_by_the_direct_gate_keeps_the_files_on_the_runner_side() {
    let project = Project::direct_gate_only();
    let root = project.root().to_path_buf();
    let work = project.work();

    succeeded(&project.run(&["push", "--force"]));
    let pull = succeeded(&project.run(&["pull", "--force"]));

    let calls = project.calls();
    let load = calls
        .iter()
        .find(|call| call.contains("/LoadConfigFromFiles"))
        .unwrap_or_else(|| panic!("{calls:?}"));
    let sources = path_after(load, "/LoadConfigFromFiles");
    assert!(sources.is_absolute() && sources.is_dir(), "{load}");
    let dump = calls
        .iter()
        .find(|call| call.contains("/DumpConfigToFiles"))
        .unwrap_or_else(|| panic!("{calls:?}"));
    let staging = path_after(dump, "/DumpConfigToFiles");
    assert!(under(&staging, &root), "{dump}");
    for call in &calls {
        assert!(under(&path_after(call, "/Out"), &work), "{call}");
    }
    let log = pull["data"]["platform_log_path"]
        .as_str()
        .unwrap_or_else(|| panic!("{pull}"));
    assert!(under(Path::new(log), &work), "{pull}");
    assert!(
        fs::read_to_string(project.root().join("sources").join("Configuration.xml"))
            .expect("published dump")
            .contains("<Configuration/>"),
        "the dump was published into the project sources"
    );
    assert!(project.work().is_dir());
}

/// Операции, которых у SSH-шлюза нет, Конфигуратор исполняет по прямому шлюзу:
/// `download --state db` (`/DumpDBCfg`), `check` (`/CheckConfig`) и снимок (`/DumpIB`).
#[test]
fn the_direct_gate_serves_what_the_ssh_gate_lacks() {
    let project = Project::both_gates();
    let package = project.root().join("dist").join("main.cf");
    let snapshot = project.root().join("dist").join("base.dt");

    let download = succeeded(&project.run(&[
        "download",
        "main",
        "--state",
        "db",
        "--output",
        &package.display().to_string(),
    ]));
    let check = envelope(&project.run(&["check"]));
    let dump = succeeded(&project.run(&[
        "infobase",
        "dump",
        "--output",
        &snapshot.display().to_string(),
    ]));

    for payload in [&download, &check, &dump] {
        assert_eq!(
            payload["data"]["provider"]["selected"], "designer",
            "{payload}"
        );
    }
    assert_eq!(download["data"]["state"], "database", "{download}");
    assert_eq!(fs::read(&package).expect("package"), b"payload");
    assert_eq!(fs::read(&snapshot).expect("snapshot"), b"payload");
    let calls = project.calls();
    assert_every_call_goes_to_the_direct_gate(&calls);
    for switch in ["/DumpDBCfg", "/CheckConfig", "/DumpIB"] {
        assert!(
            calls.iter().any(|call| call.contains(switch)),
            "{switch}: {calls:?}"
        );
    }
}

/// Превью `upload` и подъёма снимка называют Конфигуратор: их у автономного сервера
/// исполняет только он, по прямому шлюзу.
#[test]
fn upload_and_restore_plan_the_designer_by_the_direct_gate() {
    let project = Project::direct_gate_only();
    let artifact = project.root().join("in.cf");
    fs::write(&artifact, "cf").expect("cf");
    let snapshot = project.root().join("in.dt");
    fs::write(&snapshot, "dt").expect("dt");

    let upload = succeeded(&project.run(&[
        "upload",
        "--path",
        &artifact.display().to_string(),
        "--dry-run",
    ]));
    let restore = succeeded(&project.run(&[
        "infobase",
        "restore",
        "--input",
        &snapshot.display().to_string(),
        "--replace",
        "--dry-run",
    ]));

    for payload in [&upload, &restore] {
        assert_eq!(
            payload["data"]["provider"]["selected"], "designer",
            "{payload}"
        );
        assert_eq!(payload["data"]["provider_dispatched"], false, "{payload}");
    }
    assert!(project.calls().is_empty(), "{:?}", project.calls());
}

/// Без SSH-шлюза агенту пути нет: состав расширений, который исполняет только агент,
/// отказывает до платформы и называет ключ, а назначить агента ключом `providers.*`
/// нельзя.
#[test]
fn without_the_ssh_gate_the_agent_is_not_offered() {
    let project = Project::direct_gate_only();

    let extensions = envelope(&project.run(&["extensions", "list"]));
    assert_eq!(extensions["ok"], false, "{extensions}");
    assert_eq!(extensions["error"]["kind"], "validation", "{extensions}");
    assert_eq!(
        extensions["error"]["message"],
        "extensions reaches a standalone server only through agent by the SSH gate — declare infobase.standalone.gate",
        "{extensions}"
    );

    fs::write(
        &project.config,
        fs::read_to_string(&project.config).expect("config") + "providers:\n  pull: agent\n",
    )
    .expect("config");
    let pull = envelope(&project.run(&["pull", "--force"]));
    assert_eq!(pull["error"]["kind"], "validation", "{pull}");
    assert_eq!(
        pull["error"]["message"],
        "config validation failed: providers.pull: 'agent' reaches a standalone server by the SSH gate, which is not declared: declare infobase.standalone.gate, or remove the key",
        "{pull}"
    );
    assert!(project.calls().is_empty(), "{:?}", project.calls());
}

/// `infobase create` автономного сервера остаётся отказом и при прямом шлюзе: базу
/// сервера создают до его запуска, раннер к нему только подключается.
#[test]
fn infobase_create_is_still_refused_by_the_direct_gate() {
    let project = Project::direct_gate_only();

    let payload = envelope(&project.run(&["infobase", "create"]));

    assert_eq!(
        payload["data"]["steps"][0]["status"], "skipped",
        "{payload}"
    );
    assert!(
        payload["data"]["steps"][0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("never created by the runner")),
        "{payload}"
    );
    assert!(project.calls().is_empty(), "{:?}", project.calls());
}
