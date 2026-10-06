//! `pull --all`: наборы по составу базы.
//!
//! Команда спрашивает базу, какие расширения в ней установлены, для каждого расширения без
//! набора объявляет набор `src/ext/<Name>` в `v8project.yaml` и выгружает его туда; наборы,
//! которые уже есть, выгружаются как обычным `pull`. Поддельная платформа отвечает списком
//! имён из файла и выгружает в названный каталог описание конфигурации и файл версий.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::PathBuf;
use std::process::Output;

use serde_json::{json, Value};
use support::command_data::assert_data_matches_one_of;
use support::{commit_sources, temp_workspace, v8_runner_command, write_shell_script};

/// Пакетный Конфигуратор: `/DumpDBCfgList -AllExtensions` пишет в `/Out` имена из файла
/// `installed` по одному на строку, с BOM, как замер #187; `/DumpConfigToFiles` создаёт
/// каталог выгрузки с описанием и файлом версий. Каждый вызов дописывается в `calls`.
const DESIGNER: &str = r#"out=''
target=''
list=''
previous=''
for argument in "$@"; do
  case "$previous" in
    /Out) out="$argument" ;;
    /DumpConfigToFiles) target="$argument" ;;
  esac
  if [ "$argument" = "/DumpDBCfgList" ]; then list=1; fi
  previous="$argument"
done
printf '%s\n' "$*" >> "$(dirname "$0")/../../calls"
if [ -n "$list" ]; then
  printf '\357\273\277' > "$out"
  cat "$(dirname "$0")/../../installed" >> "$out"
  exit 0
fi
if [ -n "$target" ]; then
  mkdir -p "$target"
  printf '<Configuration/>\n' > "$target/Configuration.xml"
  printf '<ConfigDumpInfo version="2.17"/>\n' > "$target/ConfigDumpInfo.xml"
fi
if [ -n "$out" ]; then : > "$out"; fi
exit 0"#;

/// `ibcmd`: `config extension list` печатает блоки «ключ : значение» из файла `records`,
/// `config export` создаёт каталог выгрузки — последний довод.
const IBCMD: &str = r#"printf '%s\n' "$*" >> "$(dirname "$0")/../../calls"
case "$*" in
  *"config extension list"*) cat "$(dirname "$0")/../../records"; exit 0 ;;
esac
for argument in "$@"; do target="$argument"; done
mkdir -p "$target"
printf '<Configuration/>\n' > "$target/Configuration.xml"
printf '<ConfigDumpInfo version="2.17"/>\n' > "$target/ConfigDumpInfo.xml"
exit 0"#;

/// Проектный файл с комментариями: `pull --all` дописывает наборы, не трогая остального.
const PROJECT: &str = "# yaml-language-server: $schema=https://example.invalid/v8project.schema.json
# Проект с расширением, объявленным не по соглашению.
workPath: work
format: DESIGNER
source-set:
  # Основная конфигурация.
  - name: main
    type: CONFIGURATION
    path: src/cf
  - name: Old   # расширение со своим каталогом
    type: EXTENSION
    path: exts/old
  - name: Gone
    type: EXTENSION
    path: exts/gone
tools:
  platform:
    path: platform
";

struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Project {
    /// Проект под гитом: наборы `main` и `Old` выгружены прежде и зафиксированы, файлы версий
    /// в игноре. `Gone` объявлен, но в базе его нет. В базе установлены `installed`.
    fn new(installed: &[&str]) -> Self {
        let dir = temp_workspace();
        let root = dir.path().join("project");
        for (set, path) in [("main", "src/cf"), ("Old", "exts/old")] {
            let set_dir = root.join(path);
            fs::create_dir_all(&set_dir).expect("set dir");
            fs::write(
                set_dir.join("Configuration.xml"),
                format!("<Configuration name='{set}'/>\n"),
            )
            .expect("descriptor");
            fs::write(
                set_dir.join("ConfigDumpInfo.xml"),
                "<ConfigDumpInfo version=\"2.17\"/>\n",
            )
            .expect("version file");
        }
        fs::create_dir_all(root.join("ib")).expect("infobase");
        fs::write(root.join("ib").join("1Cv8.1CD"), "database").expect("infobase file");
        fs::write(root.join(".gitignore"), "ConfigDumpInfo.xml\nwork/\nib/\nplatform/\ncalls\ninstalled\nrecords\nv8project.local.yaml\n.dump-*.lock*\n")
            .expect("gitignore");
        fs::write(root.join("v8project.yaml"), PROJECT).expect("project file");
        fs::write(
            root.join("v8project.local.yaml"),
            "infobases:\n  origin:\n    connection: 'File=ib'\n",
        )
        .expect("local layer");
        write_shell_script(&root.join("platform").join("bin").join("1cv8"), DESIGNER);
        write_shell_script(&root.join("platform").join("bin").join("ibcmd"), IBCMD);
        let project = Self { _dir: dir, root };
        project.install(installed);
        commit_sources(&project.root);
        project
    }

    /// Состав расширений базы: имена для Конфигуратора и записи для `ibcmd`.
    fn install(&self, installed: &[&str]) {
        let names = installed
            .iter()
            .map(|name| format!("{name}\r\n"))
            .collect::<String>();
        fs::write(self.root.join("installed"), names).expect("installed");
        let records = installed
            .iter()
            .map(|name| {
                format!(
                    "name                         : \"{name}\"\nversion                      : \nactive                       : no\npurpose                      : customization\nsafe-mode                    : yes\nsecurity-profile-name        : \nunsafe-action-protection     : yes\nused-in-distributed-infobase : no\nscope                        : infobase\nhash-sum                     : \"{name}-hash\"\n\n"
                )
            })
            .collect::<String>();
        fs::write(self.root.join("records"), records).expect("records");
    }

    fn project_file(&self) -> PathBuf {
        self.root.join("v8project.yaml")
    }

    fn project_text(&self) -> String {
        fs::read_to_string(self.project_file()).expect("project file")
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.root.join("calls")).unwrap_or_default()
    }

    fn pull(&self, args: &[&str]) -> (Output, Value) {
        let output = v8_runner_command()
            .arg("--config")
            .arg(self.project_file())
            .arg("--json-message")
            .arg("pull")
            .args(args)
            .output()
            .expect("run pull");
        let envelope = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "no json envelope ({error}): stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output, envelope)
    }
}

fn pulled_sets(envelope: &Value) -> Vec<String> {
    envelope["data"]["sets"]
        .as_array()
        .unwrap_or_else(|| panic!("sets: {envelope}"))
        .iter()
        .map(|set| set["source_set"].as_str().expect("set name").to_owned())
        .collect()
}

fn assert_succeeded(output: &Output, envelope: &Value) {
    assert!(
        output.status.success(),
        "{envelope}\nstderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(envelope["ok"], true, "{envelope}");
    assert_eq!(envelope["command"], "pull", "{envelope}");
    assert_data_matches_one_of(&envelope["data"], "pull --all", &["pull-all"]);
}

/// Расширение без набора объявлено в `v8project.yaml` под `src/ext/<Name>` и выгружено
/// туда; повторный `pull --all` его уже знает и ничего не объявляет.
#[test]
fn an_extension_without_a_set_is_declared_and_pulled() {
    let project = Project::new(&["Old", "Новое", "Second"]);

    let (output, envelope) = project.pull(&["--all"]);

    assert_succeeded(&output, &envelope);
    assert_eq!(
        envelope["data"]["declared"],
        json!([
            {"name": "Second", "type": "EXTENSION", "path": "src/ext/Second"},
            {"name": "Новое", "type": "EXTENSION", "path": "src/ext/Новое"},
        ]),
        "{envelope}"
    );
    assert_eq!(
        pulled_sets(&envelope),
        ["main", "Old", "Second", "Новое"],
        "declared sets first in their order, then the new ones: {envelope}"
    );
    assert_eq!(envelope["data"]["not_installed"], json!(["Gone"]), "{envelope}");
    for name in ["Second", "Новое"] {
        assert!(
            project
                .root
                .join("src/ext")
                .join(name)
                .join("Configuration.xml")
                .is_file(),
            "{name} is pulled into its directory: {envelope}"
        );
    }
    let calls = project.calls();
    assert!(calls.contains("/DumpDBCfgList -AllExtensions"), "{calls}");
    assert!(calls.contains("-Extension Новое"), "{calls}");
    assert!(!calls.contains("-Extension Gone"), "{calls}");

    let text = project.project_text();
    assert!(
        text.starts_with(&PROJECT[..PROJECT.find("tools:").expect("tools")]),
        "the existing text stays as it was: {text}"
    );
    assert!(
        text.contains("# Основная конфигурация.") && text.contains("# расширение со своим каталогом"),
        "comments are kept: {text}"
    );
    assert!(text.ends_with("tools:\n  platform:\n    path: platform\n"), "{text}");
    assert!(text.contains("  - name: 'Новое'\n    type: EXTENSION\n    path: 'src/ext/Новое'\n"), "{text}");

    // Объявленный набор раннер знает: следующий `pull --all` его выгружает и не объявляет.
    commit_sources(&project.root);
    let (output, envelope) = project.pull(&["--all"]);
    assert_succeeded(&output, &envelope);
    assert_eq!(envelope["data"]["declared"], json!([]), "{envelope}");
    assert_eq!(
        pulled_sets(&envelope),
        ["main", "Old", "Second", "Новое"],
        "{envelope}"
    );
    assert_eq!(project.project_text(), text, "nothing is declared twice");
}

/// Набор, который уже есть, выгружается в свой каталог как обычным `pull`, а его запись в
/// `v8project.yaml` не меняется, хоть каталог и не по соглашению `src/ext/<Name>`.
#[test]
fn an_existing_set_is_left_as_declared() {
    let project = Project::new(&["Old"]);

    let (output, envelope) = project.pull(&["--all"]);

    assert_succeeded(&output, &envelope);
    assert_eq!(envelope["data"]["declared"], json!([]), "{envelope}");
    assert_eq!(project.project_text(), PROJECT);
    assert!(!project.root.join("src/ext").exists(), "{envelope}");
    let old = envelope["data"]["sets"]
        .as_array()
        .expect("sets")
        .iter()
        .find(|set| set["source_set"] == "Old")
        .unwrap_or_else(|| panic!("Old is pulled: {envelope}"))
        .clone();
    assert_eq!(old["extension"], "Old", "{envelope}");
    assert_eq!(old["mode"], "INCREMENTAL", "{envelope}");
    assert!(
        old["target_path"]
            .as_str()
            .expect("target")
            .ends_with("exts/old"),
        "{envelope}"
    );
}

/// Превью платформу не запускает, поэтому состава базы не знает: `declared` у него `null`,
/// а наборы проекта названы превью их выгрузки. Ничего не пишется.
#[test]
fn a_pull_all_preview_reads_nothing_and_writes_nothing() {
    let project = Project::new(&["Old", "Новое"]);

    let (output, envelope) = project.pull(&["--all", "--dry-run"]);

    assert_succeeded(&output, &envelope);
    assert_eq!(envelope["data"]["declared"], Value::Null, "{envelope}");
    assert_eq!(envelope["data"]["provider_dispatched"], false, "{envelope}");
    assert_eq!(
        pulled_sets(&envelope),
        ["main", "Old", "Gone"],
        "{envelope}"
    );
    let message = envelope["data"]["message"].as_str().expect("message");
    assert!(message.contains("src/ext/<Name>"), "{message}");
    assert_eq!(project.calls(), "", "the platform is not started");
    assert_eq!(project.project_text(), PROJECT);
    assert!(!project.root.join("src/ext").exists());
    assert!(!project.root.join("work").exists(), "a preview leaves no trace");
}

/// Список у `ibcmd` — блоки «ключ : значение»; выключенное расширение объявляется так же.
#[test]
fn ibcmd_lists_the_installed_extensions() {
    let project = Project::new(&["Old", "Gone", "Fresh"]);
    let text = PROJECT.replace("format: DESIGNER\n", "format: DESIGNER\nproviders:\n  pull: ibcmd\n");
    fs::write(project.project_file(), &text).expect("project file");
    commit_sources(&project.root);

    let (output, envelope) = project.pull(&["--all"]);

    assert_succeeded(&output, &envelope);
    assert_eq!(
        envelope["data"]["declared"],
        json!([{"name": "Fresh", "type": "EXTENSION", "path": "src/ext/Fresh"}]),
        "{envelope}"
    );
    assert!(envelope["data"].get("not_installed").is_none(), "{envelope}");
    assert!(project.calls().contains("config extension list"), "{}", project.calls());
    assert!(project.root.join("src/ext/Fresh/Configuration.xml").is_file());
}

/// `--all` называет все наборы сразу: рядом с набором, расширением или выборкой объектов
/// он отказывает до запуска платформы.
#[test]
fn all_refuses_a_narrower_scope() {
    let project = Project::new(&["Old"]);

    for args in [
        &["--all", "main"][..],
        &["--all", "--extension", "Old"][..],
        &["--all", "--object", "Catalog:Items"][..],
    ] {
        let (output, envelope) = project.pull(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {envelope}");
        assert_eq!(envelope["ok"], false, "{args:?}: {envelope}");
    }
    assert_eq!(project.calls(), "");
}

/// Имя установленного расширения уже занято набором другого назначения: объявить второй
/// набор с тем же именем нельзя, и команда отказывает до выгрузки, ничего не записав.
#[test]
fn an_extension_named_like_another_set_is_refused_before_any_dump() {
    let project = Project::new(&["main"]);

    let (output, envelope) = project.pull(&["--all"]);

    assert!(!output.status.success(), "{envelope}");
    let message = envelope["error"]["message"].as_str().expect("message");
    assert!(message.contains("'main'"), "{message}");
    assert!(!project.calls().contains("/DumpConfigToFiles"), "{}", project.calls());
    assert_eq!(project.project_text(), PROJECT);
}
