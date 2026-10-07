//! Метка владельца файловой базы: базу, с которой ведут разработку, держит одна рабочая
//! копия.
//!
//! Каждая копия — свой проект со своим местным слоем, а база у них одна. Метка лежит рядом
//! с каталогом базы; команда записи на базе другой живой копии отказывает `infobase_held`,
//! команда чтения проходит и метку не трогает.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use serde_json::{json, Value};
use support::{temp_workspace, v8_runner_command, write_shell_script};

/// Поддельная платформа, которая сразу отвечает успехом и пишет снимок для `/DumpIB`.
const QUICK_PLATFORM: &str = r#"previous=''
for argument in "$@"; do
  case "$previous" in
    /DumpIB) printf 'payload' > "$argument" ;;
  esac
  previous="$argument"
done
exit 0"#;

/// Имя файла метки рядом с каталогом базы `ib`.
const MARKER_NAME: &str = ".ib.v8-runner.owners.json";

struct Stand {
    dir: tempfile::TempDir,
    /// Каталог файловой базы, общей для копий.
    base: PathBuf,
}

struct Copy {
    root: PathBuf,
    config: PathBuf,
}

impl Stand {
    fn new() -> Self {
        let dir = temp_workspace();
        let base = dir.path().join("shared").join("ib");
        fs::create_dir_all(&base).expect("infobase dir");
        fs::write(base.join("1Cv8.1CD"), "database").expect("infobase file");
        Self { dir, base }
    }

    /// Рабочая копия `name`, которая объявляет общую базу как `origin` своего местного слоя.
    fn copy(&self, name: &str) -> Copy {
        let copy = Copy::at(self.dir.path().join(name));
        copy.declare(&format!("File={}", self.base.display()));
        copy
    }

    fn marker(&self) -> PathBuf {
        self.base.parent().expect("base parent").join(MARKER_NAME)
    }

    fn marker_text(&self) -> Option<String> {
        fs::read_to_string(self.marker()).ok()
    }

    fn owners(&self) -> Vec<String> {
        owners_of(&self.marker())
    }
}

impl Copy {
    /// Проект в `root` без объявленной базы: её объявляет [`Copy::declare`].
    fn at(root: PathBuf) -> Self {
        let sources = root.join("sources");
        fs::create_dir_all(&sources).expect("sources");
        fs::write(sources.join("Configuration.xml"), "<Configuration/>\n").expect("source");
        let platform = root.join("1cv8");
        write_shell_script(&platform, QUICK_PLATFORM);
        let config = root.join("v8project.yaml");
        fs::write(
            &config,
            format!(
                "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\ntools:\n  platform:\n    path: '{}'\n",
                platform.display(),
            ),
        )
        .expect("config");
        Self { root, config }
    }

    /// Объявляет `origin` местного слоя строкой соединения `connection`.
    fn declare(&self, connection: &str) {
        fs::write(
            self.root.join("v8project.local.yaml"),
            format!("infobases:\n  origin:\n    connection: '{connection}'\n"),
        )
        .expect("local layer");
        if let Some(base) = connection.strip_prefix("File=") {
            self.remember(&self.root.join(base));
        }
    }

    /// Память копии о базе, как после её создания раннером: тесты владельца начинают не с
    /// первого знакомства.
    fn remember(&self, base: &Path) {
        support::memory::remember_base(
            &self.root.join("work"),
            "origin",
            support::memory::Base::File(base),
            &[support::memory::Set::configuration(
                "main",
                &self.root.join("sources"),
            )],
        );
    }

    /// Объявляет `origin` местного слоя общей базой стенда: `shared` — согласие этой копии
    /// делить её.
    fn declare_shared(&self, stand: &Stand, shared: bool) {
        fs::write(
            self.root.join("v8project.local.yaml"),
            format!(
                "infobases:\n  origin:\n    connection: 'File={}'\n    shared: {shared}\n",
                stand.base.display()
            ),
        )
        .expect("local layer");
        self.remember(&stand.base);
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

    fn canonical_root(&self) -> String {
        fs::canonicalize(&self.root)
            .expect("canonical root")
            .display()
            .to_string()
    }
}

fn owners_of(marker: &Path) -> Vec<String> {
    let text = fs::read_to_string(marker).expect("marker");
    let marker: Value = serde_json::from_str(&text).expect("marker json");
    marker["owners"]
        .as_array()
        .expect("owners")
        .iter()
        .map(|owner| owner["project"].as_str().expect("project").to_owned())
        .collect()
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

fn warnings(payload: &Value) -> Vec<String> {
    payload["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .map(|warning| warning.as_str().expect("warning").to_owned())
        .collect()
}

/// Отказ на базе другой копии: код, род, шаг, следующий шаг и то, что он называет.
fn assert_infobase_held(output: &Output, command: &str, owner: &Copy, stand: &Stand) -> String {
    let payload = envelope(output);
    assert_eq!(output.status.code(), Some(3), "{payload}");
    assert_eq!(payload["command"], command, "{payload}");
    assert_eq!(payload["error"]["code"], "infobase_held", "{payload}");
    assert_eq!(payload["error"]["kind"], "workspace", "{payload}");
    assert_eq!(
        payload["error"]["next"]["command"], "infobase create",
        "{payload}"
    );
    assert_eq!(payload["steps"][0]["name"], "infobase owner", "{payload}");
    assert_eq!(payload["steps"][0]["status"], "failed", "{payload}");
    let message = payload["error"]["message"]
        .as_str()
        .expect("message")
        .to_owned();
    assert!(
        message.contains(&owner.canonical_root()),
        "names the owning copy: {message}"
    );
    assert!(
        message.contains(&stand.marker().display().to_string())
            || message.contains(
                &fs::canonicalize(stand.marker())
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
            ),
        "names where the marker lies: {message}"
    );
    assert!(
        message.contains("v8project.local.yaml"),
        "says how to free the base: {message}"
    );
    assert!(
        message.contains("infobase create --from") && message.contains("shared: true"),
        "names the other ways out: {message}"
    );
    let shared_way = message
        .split("a shared infobase")
        .nth(1)
        .and_then(|rest| rest.split(". ").next())
        .expect("names a shared infobase as a way out");
    assert!(
        !shared_way.contains("not available yet"),
        "a shared infobase is available: {message}"
    );
    message
}

/// Команда записи на базе другой живой копии отказывает и называет владельца; метка не
/// меняется, а сам владелец работает дальше.
#[test]
fn a_write_on_a_base_of_another_copy_is_refused_and_names_the_owner() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");

    succeeded(&first.run(&["push"]));
    assert_eq!(stand.owners(), [first.canonical_root()]);
    let marker = stand.marker_text();

    let refused = second.run(&["push"]);

    assert_infobase_held(&refused, "push", &first, &stand);
    assert_eq!(stand.marker_text(), marker, "a refusal leaves the marker");
    succeeded(&first.run(&["push"]));
    assert_eq!(
        stand.marker_text(),
        marker,
        "the owner is not written again"
    );
}

/// `pull --all` пишет в проект и в каталоги наборов, а базу берёт как любая выгрузка: на
/// базе другой копии он отказывает до чтения состава и называет владельца.
#[test]
fn pull_all_on_a_base_of_another_copy_is_refused_and_names_the_owner() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    succeeded(&first.run(&["push"]));
    let marker = stand.marker_text();
    let project = fs::read_to_string(&second.config).expect("project file");

    let refused = second.run(&["pull", "--all"]);

    assert_infobase_held(&refused, "pull", &first, &stand);
    assert_eq!(stand.marker_text(), marker, "a refusal leaves the marker");
    assert_eq!(
        fs::read_to_string(&second.config).expect("project file"),
        project,
        "nothing is declared"
    );
}

/// Команда чтения на базе другой копии проходит и метку не трогает.
#[test]
fn a_read_on_a_base_of_another_copy_passes_and_leaves_the_marker() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    succeeded(&first.run(&["push"]));
    let marker = stand.marker_text();

    let snapshot = second.root.join("base.dt");
    succeeded(&second.run(&[
        "infobase",
        "dump",
        "--output",
        snapshot.to_str().expect("snapshot path"),
    ]));

    assert!(snapshot.is_file());
    assert_eq!(stand.marker_text(), marker);
}

/// Чтение базы без метки владельцем не делает: метка не появляется.
#[test]
fn a_read_of_a_base_without_a_marker_makes_no_owner() {
    let stand = Stand::new();
    let copy = stand.copy("copy");

    let snapshot = copy.root.join("base.dt");
    succeeded(&copy.run(&[
        "infobase",
        "dump",
        "--output",
        snapshot.to_str().expect("snapshot path"),
    ]));

    assert_eq!(stand.marker_text(), None);
}

/// Процессы одной машины, начавшие одновременно на базе без метки: владельцем становится
/// один, второй получает отказ — занятой базы или базы другой копии.
#[test]
fn processes_racing_for_a_base_without_a_marker_leave_one_owner() {
    for _ in 0..3 {
        let stand = Stand::new();
        let copies = [stand.copy("first"), stand.copy("second")];
        let runners: Vec<_> = copies
            .iter()
            .map(|copy| {
                v8_runner_command()
                    .arg("--config")
                    .arg(&copy.config)
                    .arg("--json-message")
                    .arg("push")
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .expect("spawn push")
            })
            .collect();
        let outputs: Vec<Output> = runners
            .into_iter()
            .map(|runner| runner.wait_with_output().expect("push"))
            .collect();

        let winners: Vec<&Copy> = copies
            .iter()
            .zip(&outputs)
            .filter(|(_, output)| output.status.success())
            .map(|(copy, _)| copy)
            .collect();
        assert_eq!(winners.len(), 1, "exactly one copy wins the base");
        assert_eq!(stand.owners(), [winners[0].canonical_root()]);
        let loser = outputs
            .iter()
            .find(|output| !output.status.success())
            .expect("the other copy is refused");
        let code = envelope(loser)["error"]["code"].clone();
        assert!(
            code == "infobase_busy" || code == "infobase_held",
            "the loser is refused by the base: {code}"
        );
    }
}

/// Проект, скопированный целиком вместе с `build/` и меткой в нём, не оставляет живого
/// владельца: прежний проект объявляет свою базу, а не копию, — пути сравниваются в
/// каталоге владельца.
#[test]
fn a_project_copied_whole_leaves_no_live_owner() {
    let dir = temp_workspace();
    let original = Copy::at(dir.path().join("original"));
    original.declare("File=build/ib");
    let base = original.root.join("build").join("ib");
    fs::create_dir_all(&base).expect("base");
    fs::write(base.join("1Cv8.1CD"), "database").expect("infobase file");
    succeeded(&original.run(&["push"]));
    let original_marker = original.root.join("build").join(MARKER_NAME);
    assert_eq!(owners_of(&original_marker), [original.canonical_root()]);

    let copied_root = dir.path().join("copied");
    let status = std::process::Command::new("cp")
        .arg("-r")
        .arg(&original.root)
        .arg(&copied_root)
        .status()
        .expect("cp -r");
    assert!(status.success());
    let copied = Copy {
        config: copied_root.join("v8project.yaml"),
        root: copied_root,
    };
    let copied_marker = copied.root.join("build").join(MARKER_NAME);
    assert_eq!(owners_of(&copied_marker), [original.canonical_root()]);

    // Память в скопированном `work/` описывает прежнюю базу, то есть памяти о новой нет:
    // `--full` отказал бы `no_memory`, а выход каталога — перезапись `--force`. Отказ по
    // владельцу шёл бы раньше и выхода не дал бы.
    // Владельца сменяет граница команды, раньше проверки памяти: смену называет уже отказ.
    let refused = envelope(&copied.run(&["push", "main", "--full"]));
    assert_eq!(refused["error"]["code"], "no_memory", "{refused}");
    assert!(
        warnings(&refused)
            .iter()
            .any(|warning| warning.contains(&original.canonical_root())),
        "names the replaced owner: {refused}"
    );
    assert_eq!(owners_of(&copied_marker), [copied.canonical_root()]);
    succeeded(&copied.run(&["push", "main", "--force"]));

    assert_eq!(owners_of(&copied_marker), [copied.canonical_root()]);
    assert_eq!(owners_of(&original_marker), [original.canonical_root()]);
}

/// Местный слой владельца, который нельзя прочитать, делает его живым и несогласным.
#[test]
fn an_unreadable_local_layer_of_the_owner_keeps_it_alive() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    // Обе копии согласны делить базу: нечитаемый слой владельца — несогласие, и общая база
    // команду записи всё равно не пропускает.
    first.declare_shared(&stand, true);
    second.declare_shared(&stand, true);
    succeeded(&first.run(&["push"]));
    fs::write(first.root.join("v8project.local.yaml"), "infobases: [\n").expect("break layer");

    let refused = second.run(&["push"]);

    let message = assert_infobase_held(&refused, "push", &first, &stand);
    assert!(
        message.contains("cannot be read"),
        "says the owner's layer is unreadable: {message}"
    );
    assert!(
        message.contains("not sharing it"),
        "an unreadable layer does not consent: {message}"
    );
    assert_eq!(stand.owners(), [first.canonical_root()]);
}

/// Метка незнакомой версии не переписывается: команда записи отказывает и называет обе
/// версии, команда чтения идёт дальше и говорит об этом.
#[test]
fn a_marker_of_an_unknown_version_stops_a_write() {
    let stand = Stand::new();
    let copy = stand.copy("copy");
    let foreign = "{\"version\": 99, \"owners\": [], \"future\": true}\n";
    fs::write(stand.marker(), foreign).expect("marker");

    let refused = copy.run(&["push"]);

    let payload = envelope(&refused);
    assert!(!refused.status.success(), "{payload}");
    assert_eq!(payload["error"]["code"], "runtime_failure", "{payload}");
    assert_eq!(payload["steps"][0]["name"], "infobase owner", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("99"),
        "names the marker version: {message}"
    );
    assert!(
        message.contains("version 1"),
        "names the version this runner knows: {message}"
    );
    assert_eq!(stand.marker_text().as_deref(), Some(foreign));

    let snapshot = copy.root.join("base.dt");
    let dump = succeeded(&copy.run(&[
        "infobase",
        "dump",
        "--output",
        snapshot.to_str().expect("snapshot path"),
    ]));
    assert!(
        warnings(&dump).iter().any(|warning| warning.contains("99")),
        "{dump}"
    );
    assert_eq!(stand.marker_text().as_deref(), Some(foreign));
}

/// Метку, которую нельзя прочитать, команда записи не переписывает: отказ называет каталог
/// и причину; команда чтения идёт дальше и говорит об этом.
#[test]
fn an_unreadable_marker_stops_a_write_and_a_read_goes_on() {
    let stand = Stand::new();
    let copy = stand.copy("copy");
    fs::write(stand.marker(), "not json").expect("marker");
    let parent = fs::canonicalize(stand.base.parent().expect("parent"))
        .expect("canonical parent")
        .display()
        .to_string();

    let refused = copy.run(&["push"]);

    let payload = envelope(&refused);
    assert!(!refused.status.success(), "{payload}");
    assert_eq!(payload["error"]["code"], "runtime_failure", "{payload}");
    assert_eq!(payload["steps"][0]["name"], "infobase owner", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains(&parent), "names the directory: {message}");
    assert_eq!(stand.marker_text().as_deref(), Some("not json"));

    let snapshot = copy.root.join("base.dt");
    let dump = succeeded(&copy.run(&[
        "infobase",
        "dump",
        "--output",
        snapshot.to_str().expect("snapshot path"),
    ]));
    assert!(
        warnings(&dump)
            .iter()
            .any(|warning| warning.contains("owner marker") && warning.contains(&parent)),
        "{dump}"
    );
}

/// Ушедшего владельца — каталог исчез или больше не объявляет базу — команда записи
/// сменяет сама и говорит об этом.
#[test]
fn a_gone_owner_is_replaced_and_named() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    let third = stand.copy("third");

    succeeded(&first.run(&["push"]));
    let first_root = first.canonical_root();
    fs::remove_dir_all(&first.root).expect("remove the first copy");
    let pushed = succeeded(&second.run(&["push"]));
    assert_eq!(stand.owners(), [second.canonical_root()]);
    assert!(
        warnings(&pushed)
            .iter()
            .any(|warning| warning.contains(&first_root)),
        "names the gone owner: {pushed}"
    );

    second.declare(&format!(
        "File={}",
        stand.dir.path().join("other").join("ib").display()
    ));
    let pushed = succeeded(&third.run(&["push"]));
    assert_eq!(stand.owners(), [third.canonical_root()]);
    assert!(
        warnings(&pushed)
            .iter()
            .any(|warning| warning.contains(&second.canonical_root())),
        "names the owner that no longer declares the base: {pushed}"
    );
}

/// Копия, которая первой записывается в метку существующей базы без метки, говорит, что
/// база теперь за ней.
#[test]
fn a_base_without_a_marker_is_taken_and_the_answer_says_so() {
    let stand = Stand::new();
    let copy = stand.copy("copy");

    let pushed = succeeded(&copy.run(&["push"]));

    assert_eq!(stand.owners(), [copy.canonical_root()]);
    assert!(
        warnings(&pushed)
            .iter()
            .any(|warning| warning.contains("now held by this working copy")),
        "{pushed}"
    );
    // Метка лежит рядом с каталогом базы, а не в нём, и называет машину, каталог проекта и
    // согласие делить базу.
    let mut inside: Vec<String> = fs::read_dir(&stand.base)
        .expect("base dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    inside.sort();
    assert_eq!(inside, ["1Cv8.1CD"], "the base directory carries no marker");
    let marker: Value = serde_json::from_str(&stand.marker_text().expect("marker")).expect("json");
    assert_eq!(marker["version"], 1, "{marker}");
    let owner = &marker["owners"][0];
    assert!(
        owner["machine"]
            .as_str()
            .is_some_and(|machine| !machine.is_empty()),
        "{marker}"
    );
    assert_eq!(owner["shared"], false, "{marker}");
    let again = succeeded(&copy.run(&["push"]));
    assert!(
        !warnings(&again)
            .iter()
            .any(|warning| warning.contains("now held")),
        "the owner hears it once: {again}"
    );
}

/// Строка соединения в `--infobase` подчиняется владельцу, но им не становится — ни на
/// базе без метки, ни на базе ушедшего владельца.
#[test]
fn a_connection_string_obeys_the_owner_and_never_owns() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    let connection = format!("File={}", stand.base.display());

    succeeded(&second.run(&["--infobase", &connection, "push", "--force"]));
    assert_eq!(stand.marker_text(), None, "an ad hoc base is not recorded");

    succeeded(&first.run(&["push"]));
    let refused = second.run(&["--infobase", &connection, "push", "--force"]);
    assert_infobase_held(&refused, "push", &first, &stand);

    let marker = stand.marker_text();
    fs::remove_dir_all(&first.root).expect("remove the first copy");
    succeeded(&second.run(&["--infobase", &connection, "push", "--force"]));
    assert_eq!(
        stand.marker_text(),
        marker,
        "a gone owner is not replaced by an ad hoc base"
    );
}

/// Превью команды записи на базе другой копии отказывает так же, как прогон, и ничего не
/// пишет; на базе без метки превью её не заводит.
#[test]
fn a_preview_names_the_ownership_refusal_and_writes_nothing() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");

    succeeded(&first.run(&["push", "--dry-run"]));
    assert_eq!(stand.marker_text(), None, "a preview takes no base");

    succeeded(&first.run(&["push"]));
    let marker = stand.marker_text();
    let refused = second.run(&["push", "--dry-run"]);

    assert_infobase_held(&refused, "push", &first, &stand);
    assert_eq!(stand.marker_text(), marker);

    // Превью восстановления возвращается раньше замков, но отказ по владельцу называет и оно.
    let input = second.root.join("base.dt");
    fs::write(&input, "dt").expect("dt");
    let refused = second.run(&[
        "infobase",
        "restore",
        "--input",
        input.to_str().expect("input path"),
        "--replace",
        "--dry-run",
    ]);
    assert_infobase_held(&refused, "infobase.restore", &first, &stand);
    assert_eq!(stand.marker_text(), marker);
}

/// Инструмент MCP на базе другой копии отказывает так же, как командная строка: кодом
/// своего словаря, тем же текстом и тем же следующим шагом.
#[test]
fn an_mcp_tool_on_a_base_of_another_copy_is_refused_like_the_cli() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    succeeded(&first.run(&["push"]));
    let marker = stand.marker_text();
    let cli = envelope(&second.run(&["push"]));

    let answer = support::mcp::call_tool(&second.config, "build_project", json!({}));

    assert!(answer.is_error, "{}", answer.envelope);
    let payload = &answer.envelope;
    assert_eq!(payload["error"]["code"], "runtime_failure", "{payload}");
    assert_eq!(payload["error"]["kind"], "runtime", "{payload}");
    assert_eq!(
        payload["error"]["next"]["command"], "infobase create",
        "{payload}"
    );
    assert_eq!(
        payload["error"]["message"], cli["error"]["message"],
        "the same refusal as the command line"
    );
    assert_eq!(stand.marker_text(), marker);
}

/// Владелец с другой машины всегда живой: раннер его не сменяет, а отказ называет его
/// машину и говорит, что освобождают такую базу удалением записи из метки.
#[test]
fn an_owner_on_another_machine_is_never_replaced() {
    let stand = Stand::new();
    let copy = stand.copy("copy");
    let foreign = json!({
        "version": 1,
        "owners": [{
            "machine": "a".repeat(64),
            "host": "build-agent",
            "project": "/srv/elsewhere",
            "shared": false,
            "since": "2026-10-01T00:00:00Z"
        }]
    })
    .to_string();
    fs::write(stand.marker(), &foreign).expect("marker");

    let refused = copy.run(&["push"]);

    let payload = envelope(&refused);
    assert_eq!(payload["error"]["code"], "infobase_held", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("/srv/elsewhere"), "{message}");
    assert!(message.contains("build-agent"), "{message}");
    assert!(
        message.contains("delete its record"),
        "says how to free a remote base: {message}"
    );
    assert_eq!(stand.marker_text().as_deref(), Some(foreign.as_str()));
}

/// Сначала чья база, затем память: копия с памятью о другой базе на базе другой рабочей
/// копии получает отказ по владельцу, а не отказ чужой памяти.
#[test]
fn ownership_is_refused_before_foreign_memory() {
    let stand = Stand::new();
    let holder = stand.copy("holder");
    succeeded(&holder.run(&["push"]));

    let original = Copy::at(stand.dir.path().join("original"));
    original.declare("File=build/ib");
    let own_base = original.root.join("build").join("ib");
    fs::create_dir_all(&own_base).expect("own base");
    fs::write(own_base.join("1Cv8.1CD"), "database").expect("infobase file");
    succeeded(&original.run(&["push"]));
    let copied_root = stand.dir.path().join("copied");
    let status = std::process::Command::new("cp")
        .arg("-r")
        .arg(&original.root)
        .arg(&copied_root)
        .status()
        .expect("cp -r");
    assert!(status.success());
    let copied = Copy {
        config: copied_root.join("v8project.yaml"),
        root: copied_root,
    };
    // Память копии описывает базу `original/build/ib`, а слой направлен на базу держателя.
    copied.declare(&format!("File={}", stand.base.display()));

    let refused = copied.run(&["push"]);

    assert_infobase_held(&refused, "push", &holder, &stand);
}

/// Местный слой владельца, который не разобрать, называется без своего текста: в нём бывают
/// пароли, и ни командная строка, ни MCP их не повторяют.
#[test]
fn a_secret_in_the_unparsable_layer_of_the_owner_never_reaches_the_answer() {
    const SECRET: &str = "TOPSECRET1";
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    succeeded(&first.run(&["push"]));
    fs::write(
        first.root.join("v8project.local.yaml"),
        format!(
            "infobases:\n  origin:\n    connection: 'File={};Pwd={SECRET}'\n    password: {SECRET}\n  [broken\n",
            stand.base.display()
        ),
    )
    .expect("break layer");

    let refused = second.run(&["push"]);

    let message = assert_infobase_held(&refused, "push", &first, &stand);
    assert!(message.contains("cannot be parsed"), "{message}");
    let stdout = String::from_utf8_lossy(&refused.stdout);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!stdout.contains(SECRET), "{stdout}");
    assert!(!stderr.contains(SECRET), "{stderr}");
    let answer = support::mcp::call_tool(&second.config, "build_project", json!({}));
    assert!(answer.is_error, "{}", answer.envelope);
    assert!(
        !answer.envelope.to_string().contains(SECRET),
        "{}",
        answer.envelope
    );
}

/// Проектный файл владельца, который не обычный файл (FIFO), раннер не открывает: команда не
/// повисает, а владелец считается живым.
#[test]
fn a_fifo_in_place_of_the_owner_project_file_does_not_hang_the_command() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    succeeded(&first.run(&["push"]));
    let project_file = first.root.join("v8project.yaml");
    fs::remove_file(&project_file).expect("remove project file");
    let status = std::process::Command::new("mkfifo")
        .arg(&project_file)
        .status()
        .expect("mkfifo");
    assert!(status.success());

    let mut runner = v8_runner_command()
        .arg("--config")
        .arg(&second.config)
        .arg("--json-message")
        .arg("push")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn push");
    let finished = support::wait_until(
        std::time::Duration::from_secs(30),
        std::time::Duration::from_millis(50),
        || runner.try_wait().ok().flatten().is_some(),
    );
    if !finished {
        let _ = runner.kill();
    }
    let output = runner.wait_with_output().expect("push");

    assert!(finished, "the command hung on the owner's FIFO");
    let message = assert_infobase_held(&output, "push", &first, &stand);
    assert!(message.contains("cannot be read"), "{message}");
}

/// Записанное в метке согласие каждой копии.
fn consents_of(marker: &Path) -> Vec<(String, bool)> {
    let text = fs::read_to_string(marker).expect("marker");
    let marker: Value = serde_json::from_str(&text).expect("marker json");
    marker["owners"]
        .as_array()
        .expect("owners")
        .iter()
        .map(|owner| {
            (
                owner["project"].as_str().expect("project").to_owned(),
                owner["shared"].as_bool().expect("shared"),
            )
        })
        .collect()
}

/// Общая база с согласием всех держателей: команды записи каждой копии идут, и метка
/// называет обе копии с их согласием.
#[test]
fn a_base_shared_by_every_holder_takes_writes_of_each() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    first.declare_shared(&stand, true);
    second.declare_shared(&stand, true);

    succeeded(&first.run(&["push"]));
    succeeded(&second.run(&["push"]));
    succeeded(&first.run(&["push"]));

    let mut consents = consents_of(&stand.marker());
    consents.sort();
    let mut expected = vec![
        (first.canonical_root(), true),
        (second.canonical_root(), true),
    ];
    expected.sort();
    assert_eq!(consents, expected);
}

/// Без согласия одного из держателей команда записи отказывает и называет его — и тогда,
/// когда несогласна сама эта копия.
#[test]
fn a_base_without_consent_of_one_holder_is_refused_and_names_it() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    succeeded(&first.run(&["push"]));
    let marker = stand.marker_text();

    second.declare_shared(&stand, true);
    let refused = second.run(&["push"]);
    let message = assert_infobase_held(&refused, "push", &first, &stand);
    assert!(
        message.contains(&format!(
            "'{}' on this machine, which does not share it",
            first.canonical_root()
        )),
        "names the holder without consent: {message}"
    );
    assert_eq!(stand.marker_text(), marker, "a refusal records no new copy");

    first.declare_shared(&stand, true);
    second.declare_shared(&stand, false);
    let refused = second.run(&["push"]);
    let message = assert_infobase_held(&refused, "push", &first, &stand);
    assert!(
        message.contains("this working copy does not share it"),
        "names this copy as the one without consent: {message}"
    );
}

/// Согласие копий этой машины читается из их местных слоёв в момент команды: отзыв одной
/// копией останавливает команды записи обеих сразу, без её следующей команды, и отказы
/// называют друг друга.
#[test]
fn a_consent_withdrawn_on_this_machine_stops_every_copy_at_once() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    first.declare_shared(&stand, true);
    second.declare_shared(&stand, true);
    succeeded(&first.run(&["push"]));
    succeeded(&second.run(&["push"]));

    first.declare_shared(&stand, false);

    let refused = second.run(&["push"]);
    assert_infobase_held(&refused, "push", &first, &stand);
    let refused = first.run(&["push"]);
    assert_infobase_held(&refused, "push", &second, &stand);
}

/// Копия с другой машины сообщает согласие меткой: с её записанным согласием команда
/// записи идёт, после отзыва в метке — отказывает и называет её.
#[test]
fn a_remote_copy_consents_through_the_marker() {
    let stand = Stand::new();
    let copy = stand.copy("copy");
    copy.declare_shared(&stand, true);
    let remote = |shared: bool| {
        json!({
            "version": 1,
            "owners": [{
                "machine": "a".repeat(64),
                "host": "build-agent",
                "project": "/srv/elsewhere",
                "shared": shared,
                "since": "2026-10-01T00:00:00Z"
            }]
        })
        .to_string()
    };
    fs::write(stand.marker(), remote(true)).expect("marker");

    succeeded(&copy.run(&["push"]));
    assert_eq!(
        consents_of(&stand.marker()),
        [
            ("/srv/elsewhere".to_owned(), true),
            (copy.canonical_root(), true)
        ],
        "the copy joins the remote one with its consent"
    );

    let mut withdrawn: Value =
        serde_json::from_str(&stand.marker_text().expect("marker")).expect("json");
    for owner in withdrawn["owners"].as_array_mut().expect("owners") {
        if owner["project"] == "/srv/elsewhere" {
            owner["shared"] = Value::Bool(false);
        }
    }
    fs::write(stand.marker(), withdrawn.to_string()).expect("marker");

    let refused = copy.run(&["push"]);
    let payload = envelope(&refused);
    assert_eq!(payload["error"]["code"], "infobase_held", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("'/srv/elsewhere' on machine 'build-agent', which does not share it"),
        "{message}"
    );
}

/// Отказ по владельцу на общей базе ведёт к своей базе (`infobase create`), а выгрузку не
/// предлагает: ни следующим шагом, ни текстом.
#[test]
fn a_held_refusal_on_a_shared_base_leads_to_an_own_base() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    first.declare_shared(&stand, true);
    second.declare_shared(&stand, true);
    succeeded(&first.run(&["push"]));
    succeeded(&second.run(&["push"]));
    first.declare_shared(&stand, false);

    for refused in [second.run(&["push"]), second.run(&["push", "--dry-run"])] {
        let payload = envelope(&refused);
        assert_eq!(payload["error"]["code"], "infobase_held", "{payload}");
        assert_eq!(
            payload["error"]["next"]["command"], "infobase create",
            "{payload}"
        );
        let message = payload["error"]["message"].as_str().expect("message");
        assert!(!message.contains("pull"), "{message}");
    }
}

/// Согласие копии — у каждой её секции, которая объявляет базу: вторая секция той же базы
/// без `shared: true` отзывает согласие копии, и команды записи обеих копий отказывают.
#[test]
fn a_second_section_of_the_base_without_consent_withdraws_it() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    first.declare_shared(&stand, true);
    second.declare_shared(&stand, true);
    succeeded(&first.run(&["push"]));
    succeeded(&second.run(&["push"]));

    fs::write(
        first.root.join("v8project.local.yaml"),
        format!(
            "infobases:\n  origin:\n    connection: 'File={base}'\n    shared: true\n  alt:\n    connection: 'File={base}'\n",
            base = stand.base.display()
        ),
    )
    .expect("local layer");

    let refused = first.run(&["push"]);
    let message = assert_infobase_held(&refused, "push", &second, &stand);
    assert!(
        message.contains("this working copy does not share it"),
        "{message}"
    );
    assert!(
        consents_of(&stand.marker()).contains(&(first.canonical_root(), false)),
        "the refused copy records its withdrawal"
    );
    let refused = second.run(&["push"]);
    assert_infobase_held(&refused, "push", &first, &stand);
}

/// Строка соединения в `--infobase` согласия не даёт: на общей базе она отказывает и
/// просит имя базы.
#[test]
fn a_connection_string_on_a_shared_base_is_refused() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    first.declare_shared(&stand, true);
    succeeded(&first.run(&["push"]));
    let marker = stand.marker_text();

    let connection = format!("File={}", stand.base.display());
    let refused = second.run(&["--infobase", &connection, "push", "--force"]);

    let message = assert_infobase_held(&refused, "push", &first, &stand);
    assert!(
        message.contains("a connection string does not share an infobase — pass its name"),
        "{message}"
    );
    assert_eq!(stand.marker_text(), marker);
}

/// На общей базе отказ первого знакомства и отказ «база ушла вперёд» предлагают выгрузку
/// следующим шагом, называют `push --force` текстом, говорят, что базу меняют и другие копии,
/// и называют остальных владельцев — и тогда, когда эта копия взяла базу без метки.
#[test]
fn a_refusal_on_a_shared_base_offers_pull_first_and_names_push_force() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    first.declare_shared(&stand, true);
    second.declare_shared(&stand, true);
    succeeded(&first.run(&["push"]));
    fs::remove_dir_all(second.root.join("work")).expect("forget the base");

    let refused = second.run(&["push"]);
    let payload = envelope(&refused);

    assert_eq!(refused.status.code(), Some(3), "{payload}");
    assert_eq!(payload["error"]["code"], "no_memory", "{payload}");
    assert_eq!(payload["error"]["next"]["command"], "pull", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("push --force`"), "{message}");
    assert!(message.contains("shared"), "{message}");
    assert!(message.contains(&first.canonical_root()), "{message}");

    // База ушла вперёд записанного поколения той же копии.
    write_shell_script(
        &second.root.join("1cv8"),
        &format!(
            "out=''; previous=''\nfor a in \"$@\"; do [ \"$previous\" = /Out ] && out=\"$a\"; previous=\"$a\"; done\ncase \"$*\" in *GetConfigGenerationID*) printf '{}\\n' > \"$out\" ;; esac\nexit 0",
            "2".repeat(40)
        ),
    );
    succeeded(&second.run(&["push", "--force"]));
    let ledger = second
        .root
        .join("work")
        .join("infobases")
        .join("origin")
        .join("generation.json");
    let mut records: Value =
        serde_json::from_str(&fs::read_to_string(&ledger).expect("ledger")).expect("json");
    records["main"] = json!({
        "token": "1".repeat(40),
        "tool": "designer",
        "after": "build",
        "recorded_at": "2026-10-06T00:00:00Z",
        "identity": records["main"]["identity"],
    });
    fs::write(&ledger, records.to_string()).expect("ledger");
    fs::write(second.root.join("sources").join("Module.bsl"), "edited").expect("edit");

    let payload = envelope(&second.run(&["push"]));

    assert_eq!(payload["error"]["code"], "non_fast_forward", "{payload}");
    assert_eq!(payload["error"]["next"]["command"], "pull", "{payload}");
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("push main --force`"), "{message}");
    assert!(message.contains(&first.canonical_root()), "{message}");
}

/// `infobase create` записывает созданную файловую базу в метку за своей копией.
#[test]
fn infobase_create_records_the_created_base_for_this_copy() {
    let dir = temp_workspace();
    let copy = Copy::at(dir.path().join("own"));
    write_shell_script(
        &copy.root.join("1cv8"),
        "if [ \"$1\" = \"CREATEINFOBASE\" ]; then path=${2#File=\\'}; path=${path%\\'}; mkdir -p \"$path\" && : > \"$path/1Cv8.1CD\"; fi\nexit 0",
    );
    let base = dir.path().join("bases").join("ib");
    fs::write(
        copy.root.join("v8project.local.yaml"),
        format!(
            "infobases:\n  origin:\n    connection: 'File={}'\n",
            base.display()
        ),
    )
    .expect("local layer");

    succeeded(&copy.run(&["infobase", "create"]));

    assert!(base.join("1Cv8.1CD").is_file());
    assert_eq!(
        owners_of(&base.parent().expect("parent").join(MARKER_NAME)),
        [copy.canonical_root()]
    );
}
