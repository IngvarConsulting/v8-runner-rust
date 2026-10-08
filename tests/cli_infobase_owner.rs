//! Метка владельца файловой базы: базу, с которой ведут разработку, держит одна рабочая
//! копия.
//!
//! Каждая копия — свой проект со своим местным слоем, а база у них одна. Метка лежит рядом
//! с каталогом базы; команда записи на базе другой живой копии идёт с предупреждением и метку
//! не трогает, команда чтения проходит молча.
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
    /// Каталог файловой базы, к которой подключены копии.
    base: PathBuf,
}

struct Copy {
    root: PathBuf,
    config: PathBuf,
}

impl Stand {
    fn new() -> Self {
        let dir = temp_workspace();
        let base = dir.path().join("bases").join("ib");
        fs::create_dir_all(&base).expect("infobase dir");
        fs::write(base.join("1Cv8.1CD"), "database").expect("infobase file");
        Self { dir, base }
    }

    /// Рабочая копия `name`, которая объявляет базу стенда как `origin` своего местного слоя.
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
                "workPath: work\nformat: DESIGNER\n{designer_leads}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: sources\ntools:\n  platform:\n    path: '{}'\n",
                platform.display(),
 designer_leads = support::DESIGNER_LEADS,
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

/// Предупреждение о базе другой копии в ответе `payload` команды `command`: оно называет
/// копию-владельца, метку, как освободить базу, и выходы к своей базе.
fn another_copy_warning(payload: &Value, command: &str, owner: &Copy, stand: &Stand) -> String {
    assert_eq!(payload["command"], command, "{payload}");
    let warning = warnings(payload)
        .into_iter()
        .find(|warning| warning.contains("of another working copy"))
        .unwrap_or_else(|| panic!("warns about a base of another copy: {payload}"));
    assert!(
        warning.contains(&owner.canonical_root()),
        "names the owning copy: {warning}"
    );
    assert!(
        warning.contains(&stand.marker().display().to_string())
            || warning.contains(
                &fs::canonicalize(stand.marker())
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
            ),
        "names where the marker lies: {warning}"
    );
    assert!(
        warning.contains("changes that working copy's infobase"),
        "says the command changes the base of another copy: {warning}"
    );
    // Три выхода к своей базе: копия этой базы, база из эталонного образа и голая база.
    for way in [
        "init --infobase <connection string>",
        "infobase create --from upstream",
        "infobase restore --input <reference>.dt --create",
        "`v8-runner infobase create`",
    ] {
        assert!(warning.contains(way), "{way}: {warning}");
    }
    assert!(
        warning.contains("To free the infobase: remove the infobase from v8project.local.yaml of")
            || warning.contains("To free the infobase: delete the record of"),
        "says how to free the base: {warning}"
    );
    warning
}

/// Команда записи на базе другой копии прошла и предупредила.
fn assert_warned(output: &Output, command: &str, owner: &Copy, stand: &Stand) -> String {
    another_copy_warning(&succeeded(output), command, owner, stand)
}

/// Команда записи на базе другой живой копии идёт и предупреждает, чья это база; метка не
/// меняется — владельцем остаётся прежняя копия, — а сам владелец работает дальше без
/// предупреждения.
#[test]
fn a_write_on_a_base_of_another_copy_runs_with_a_warning_and_names_the_owner() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");

    succeeded(&first.run(&["push"]));
    assert_eq!(stand.owners(), [first.canonical_root()]);
    let marker = stand.marker_text();

    let pushed = second.run(&["push"]);

    let payload = succeeded(&pushed);
    assert!(payload["error"].is_null(), "{payload}");
    assert_warned(&pushed, "push", &first, &stand);
    assert_eq!(stand.marker_text(), marker, "the write leaves the marker");
    let again = succeeded(&first.run(&["push"]));
    assert!(
        !warnings(&again)
            .iter()
            .any(|warning| warning.contains("of another working copy")),
        "the owner hears no warning: {again}"
    );
    assert_eq!(
        stand.marker_text(),
        marker,
        "the owner is not written again"
    );
}

/// `pull --all` пишет в проект и в каталоги наборов, а базу берёт как любая выгрузка: на
/// базе другой копии он идёт, его ответ — удача или отказ самой выгрузки — несёт
/// предупреждение, а метка не меняется.
#[test]
fn pull_all_on_a_base_of_another_copy_warns_and_names_the_owner() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    succeeded(&first.run(&["push"]));
    let marker = stand.marker_text();

    let pulled = envelope(&second.run(&["pull", "--all"]));

    another_copy_warning(&pulled, "pull", &first, &stand);
    assert_eq!(stand.marker_text(), marker, "the write leaves the marker");
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
/// один, второй либо получает отказ занятой базы, либо идёт после него с предупреждением о
/// базе другой копии.
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

        let owners = stand.owners();
        assert_eq!(
            owners.len(),
            1,
            "exactly one copy holds the base: {owners:?}"
        );
        for (copy, output) in copies.iter().zip(&outputs) {
            let payload = envelope(output);
            if copy.canonical_root() == owners[0] {
                assert!(output.status.success(), "the owner's push: {payload}");
            } else if output.status.success() {
                let owner = copies
                    .iter()
                    .find(|copy| copy.canonical_root() == owners[0])
                    .expect("the owner is one of the copies");
                another_copy_warning(&payload, "push", owner, &stand);
            } else {
                assert_eq!(
                    payload["error"]["code"], "infobase_busy",
                    "the other copy is refused only by the busy base: {payload}"
                );
            }
        }
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
    // `--full` отказал бы `no_memory`, а выход каталога — перезапись `--force`.
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

/// Местный слой владельца, который нельзя прочитать, делает его живым: команда записи другой
/// копии его не сменяет, а предупреждает о его базе.
#[test]
fn an_unreadable_local_layer_of_the_owner_keeps_it_alive() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    succeeded(&first.run(&["push"]));
    fs::write(first.root.join("v8project.local.yaml"), "infobases: [\n").expect("break layer");

    let pushed = second.run(&["push"]);

    let warning = assert_warned(&pushed, "push", &first, &stand);
    assert!(
        warning.contains("cannot be read"),
        "says the owner's layer is unreadable: {warning}"
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
        message.contains("version 2"),
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
    // Метка лежит рядом с каталогом базы, а не в нём, и называет машину и каталог проекта.
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
    assert_eq!(marker["version"], 2, "{marker}");
    let owner = &marker["owners"][0];
    assert!(
        owner["machine"]
            .as_str()
            .is_some_and(|machine| !machine.is_empty()),
        "{marker}"
    );
    let again = succeeded(&copy.run(&["push"]));
    assert!(
        !warnings(&again)
            .iter()
            .any(|warning| warning.contains("now held")),
        "the owner hears it once: {again}"
    );
}

/// Строка соединения в `--infobase` владельцем не становится — ни на базе без метки, ни на
/// базе ушедшего владельца, — а на базе другой копии идёт с тем же предупреждением.
#[test]
fn a_connection_string_never_owns_and_warns_on_a_base_of_another_copy() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    let connection = format!("File={}", stand.base.display());

    succeeded(&second.run(&["--infobase", &connection, "push", "--force"]));
    assert_eq!(stand.marker_text(), None, "an ad hoc base is not recorded");

    succeeded(&first.run(&["push"]));
    let marker = stand.marker_text();
    let pushed = second.run(&["--infobase", &connection, "push", "--force"]);
    assert_warned(&pushed, "push", &first, &stand);
    assert_eq!(stand.marker_text(), marker, "the write leaves the marker");

    fs::remove_dir_all(&first.root).expect("remove the first copy");
    succeeded(&second.run(&["--infobase", &connection, "push", "--force"]));
    assert_eq!(
        stand.marker_text(),
        marker,
        "a gone owner is not replaced by an ad hoc base"
    );
}

/// Превью команды записи на базе другой копии предупреждает так же, как прогон, и ничего не
/// пишет; на базе без метки превью её не заводит.
#[test]
fn a_preview_names_the_warning_on_a_base_of_another_copy_and_writes_nothing() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");

    succeeded(&first.run(&["push", "--dry-run"]));
    assert_eq!(stand.marker_text(), None, "a preview takes no base");

    succeeded(&first.run(&["push"]));
    let marker = stand.marker_text();
    let preview = second.run(&["push", "--dry-run"]);

    let previewed = assert_warned(&preview, "push", &first, &stand);
    assert_eq!(stand.marker_text(), marker);
    let run = assert_warned(&second.run(&["push"]), "push", &first, &stand);
    assert_eq!(previewed, run, "the preview warns as the run does");

    // Превью восстановления возвращается раньше замков, но предупреждает и оно.
    let input = second.root.join("base.dt");
    fs::write(&input, "dt").expect("dt");
    let preview = second.run(&[
        "infobase",
        "restore",
        "--input",
        input.to_str().expect("input path"),
        "--replace",
        "--dry-run",
    ]);
    assert_warned(&preview, "infobase.restore", &first, &stand);
    assert_eq!(stand.marker_text(), marker);
}

/// Инструмент MCP на базе другой копии идёт так же, как командная строка, и метку не
/// меняет. Предупреждение в ответ инструмента пока не попадает — только в журнал сервера
/// (#404, `INV.MCP.A-BOUNDARY-NOTE-REACHES-THE-TOOL-ANSWER`).
#[test]
fn an_mcp_tool_on_a_base_of_another_copy_runs_like_the_cli() {
    let stand = Stand::new();
    let first = stand.copy("first");
    let second = stand.copy("second");
    succeeded(&first.run(&["push"]));
    let marker = stand.marker_text();
    assert_warned(&second.run(&["push"]), "push", &first, &stand);

    let answer = support::mcp::call_tool(&second.config, "build_project", json!({}));

    assert!(!answer.is_error, "{}", answer.envelope);
    assert_eq!(stand.marker_text(), marker);
}

/// Владелец с другой машины всегда живой: раннер его не сменяет и метку не трогает, а
/// предупреждение называет его каталог и машину.
#[test]
fn an_owner_on_another_machine_is_never_replaced() {
    let stand = Stand::new();
    let copy = stand.copy("copy");
    let foreign = json!({
        "version": 2,
        "owners": [{
            "machine": "a".repeat(64),
            "host": "build-agent",
            "project": "/srv/elsewhere",
            "since": "2026-10-01T00:00:00Z"
        }]
    })
    .to_string();
    fs::write(stand.marker(), &foreign).expect("marker");

    let pushed = succeeded(&copy.run(&["push"]));

    let warning = warnings(&pushed)
        .into_iter()
        .find(|warning| warning.contains("of another working copy"))
        .unwrap_or_else(|| panic!("warns: {pushed}"));
    assert!(warning.contains("/srv/elsewhere"), "{warning}");
    assert!(warning.contains("'build-agent'"), "{warning}");
    assert_eq!(stand.marker_text().as_deref(), Some(foreign.as_str()));
}

/// Метку версии 1 раннер читает: лишние поля её записей он пропускает, запись в базу её
/// владельца идёт с предупреждением, а метка остаётся как была.
#[test]
fn a_marker_of_version_one_is_still_read() {
    let stand = Stand::new();
    let copy = stand.copy("copy");
    let legacy = json!({
        "version": 1,
        "owners": [{
            "machine": "a".repeat(64),
            "host": "build-agent",
            "project": "/srv/elsewhere",
            "shared": true,
            "since": "2026-10-01T00:00:00Z"
        }]
    })
    .to_string();
    fs::write(stand.marker(), &legacy).expect("marker");

    let pushed = succeeded(&copy.run(&["push"]));

    assert!(
        warnings(&pushed)
            .iter()
            .any(|warning| warning.contains("of another working copy")
                && warning.contains("/srv/elsewhere")),
        "{pushed}"
    );
    assert_eq!(stand.marker_text().as_deref(), Some(legacy.as_str()));
    let snapshot = copy.root.join("base.dt");
    let dump = succeeded(&copy.run(&[
        "infobase",
        "dump",
        "--output",
        snapshot.to_str().expect("snapshot path"),
    ]));
    assert!(warnings(&dump).is_empty(), "a read understands it: {dump}");
}

/// Сначала чья база, затем память: копия с памятью о другой базе на базе другой рабочей
/// копии получает отказ чужой памяти, и он уже несёт предупреждение о базе другой копии.
#[test]
fn ownership_is_warned_before_foreign_memory() {
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

    let refused = envelope(&copied.run(&["push"]));

    assert_eq!(refused["error"]["code"], "no_memory", "{refused}");
    another_copy_warning(&refused, "push", &holder, &stand);
    assert_eq!(stand.owners(), [holder.canonical_root()]);
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

    let pushed = second.run(&["push"]);

    let warning = assert_warned(&pushed, "push", &first, &stand);
    assert!(warning.contains("cannot be parsed"), "{warning}");
    let stdout = String::from_utf8_lossy(&pushed.stdout);
    let stderr = String::from_utf8_lossy(&pushed.stderr);
    assert!(!stdout.contains(SECRET), "{stdout}");
    assert!(!stderr.contains(SECRET), "{stderr}");
    let answer = support::mcp::call_tool(&second.config, "build_project", json!({}));
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
    let warning = assert_warned(&output, "push", &first, &stand);
    assert!(warning.contains("cannot be read"), "{warning}");
}

/// Поддельный Конфигуратор с поколением базы: `/GetConfigGenerationID` отвечает токеном из
/// файла стенда, а каждая загрузка сдвигает его — так меняет базу запись любой копии.
fn generation_platform(stand: &Stand) -> String {
    format!(
        r#"out=''; previous=''
for arg in "$@"; do
  if [ "$previous" = '/Out' ]; then out="$arg"; fi
  previous="$arg"
done
case "$*" in
  *'/GetConfigGenerationID'*)
    cat '{token}' > "$out"
    exit 0 ;;
  *'/LoadConfigFromFiles'*)
    n=$(cat '{counter}'); n=$((n + 1)); printf '%s\n' "$n" > '{counter}'
    printf '%040d\n' "$n" > '{token}' ;;
esac
if [ -n "$out" ]; then : > "$out"; fi
exit 0"#,
        token = stand.dir.path().join("token").display(),
        counter = stand.dir.path().join("counter").display(),
    )
}

/// Запись другой копии не затирается молча: владелец, чья память о базе отстала от неё, на
/// следующей отправке получает отказ «база ушла вперёд» (#215), а метка остаётся за ним.
#[test]
fn the_owner_notices_a_write_of_another_copy_by_the_generation() {
    let stand = Stand::new();
    fs::write(stand.dir.path().join("counter"), "1\n").expect("counter");
    fs::write(stand.dir.path().join("token"), format!("{:040}\n", 1)).expect("token");
    let first = stand.copy("first");
    let second = stand.copy("second");
    for copy in [&first, &second] {
        write_shell_script(&copy.root.join("1cv8"), &generation_platform(&stand));
    }

    succeeded(&first.run(&["push", "--force"]));
    assert_eq!(stand.owners(), [first.canonical_root()]);
    fs::write(second.root.join("sources").join("Module.bsl"), "second").expect("edit");
    assert_warned(&second.run(&["push", "--force"]), "push", &first, &stand);
    assert_eq!(stand.owners(), [first.canonical_root()]);

    fs::write(first.root.join("sources").join("Module.bsl"), "first").expect("edit");
    let refused = envelope(&first.run(&["push"]));

    assert_eq!(refused["error"]["code"], "non_fast_forward", "{refused}");
    assert_eq!(
        refused["error"]["base_generation"],
        format!("{:040}", 3),
        "{refused}"
    );
    assert_eq!(
        refused["error"]["local_generation"],
        format!("{:040}", 2),
        "{refused}"
    );
    assert_eq!(stand.owners(), [first.canonical_root()]);
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
