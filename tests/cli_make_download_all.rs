//! `make` и `download` без набора: пакет каждого набора в каталог `--output` (#364).
//!
//! `make` собирает наборы порядком назначения — основная конфигурация, расширения, внешние
//! файлы, — `download` спрашивает базу о расширениях и выгружает пакеты конфигурации тех
//! наборов, чьё расширение в базе есть. Поддельный Конфигуратор записывает каждый вызов,
//! пишет пакет по доводу `/DumpCfg` и обработку по доводу
//! `/LoadExternalDataProcessorOrReportFromFiles` (и её описание по доводу
//! `/DumpExternalDataProcessorOrReportToFiles`), отвечает списком расширений из файла
//! `installed` и отказывает на расширении, названном в файле `fail`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::PathBuf;
use std::process::Output;

use serde_json::Value;
use support::command_data::assert_data_matches_one_of;
use support::{temp_workspace, v8_runner_command, write_shell_script};

const DESIGNER: &str = r#"root="$(dirname "$0")/.."
out=''
target=''
extension=''
list=''
external=''
want=''
descriptor=''
previous=''
for argument in "$@"; do
  if [ "$previous" = /DumpExternalDataProcessorOrReportToFiles ]; then descriptor="$argument"; fi
  if [ "$want" = binary ]; then external="$argument"; want=''; fi
  if [ "$want" = xml ]; then want=binary; fi
  if [ "$argument" = /LoadExternalDataProcessorOrReportFromFiles ]; then want=xml; fi
  case "$previous" in
    /Out) out="$argument" ;;
    /DumpCfg) target="$argument" ;;
    -Extension) extension="$argument" ;;
  esac
  if [ "$argument" = /DumpDBCfgList ]; then list=1; fi
  previous="$argument"
done
printf '%s\n' "$*" >> "$root/calls"
if [ -n "$out" ]; then : > "$out"; fi
if [ -n "$extension" ] && [ "$extension" = "$(cat "$root/fail" 2>/dev/null)" ]; then exit 17; fi
if [ -n "$list" ]; then
  printf '\357\273\277' > "$out"
  cat "$root/installed" >> "$out"
  exit 0
fi
if [ -n "$target" ]; then printf 'package' > "$target"; fi
if [ -n "$external" ]; then printf 'epf' > "$external"; fi
if [ -n "$descriptor" ]; then
  mkdir -p "$(dirname "$descriptor")"
  printf '<ExternalDataProcessor><Properties><Name>Tool</Name></Properties></ExternalDataProcessor>' > "$descriptor"
fi
exit 0"#;

struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Project {
    /// Наборы `main` (конфигурация), `Sales` и `Gone` (расширения) и `tools` (внешние
    /// обработки — объявлены первыми, чтобы порядок обхода был виден); в базе установлены
    /// `installed`.
    fn new(installed: &[&str]) -> Self {
        let dir = temp_workspace();
        let root = dir.path().join("project");
        for set in ["src/cf", "src/sales", "src/gone", "src/tools"] {
            fs::create_dir_all(root.join(set)).expect("set dir");
        }
        fs::write(
            root.join("src/tools/Tool.xml"),
            "<ExternalDataProcessor><Properties><Name>Tool</Name></Properties></ExternalDataProcessor>",
        )
        .expect("external descriptor");
        fs::create_dir_all(root.join("ib")).expect("infobase");
        fs::write(root.join("ib/1Cv8.1CD"), "database").expect("infobase file");
        fs::write(
            root.join("v8project.yaml"),
            "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: tools\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src/tools\n  - name: Sales\n    type: EXTENSION\n    path: src/sales\n  - name: main\n    type: CONFIGURATION\n    path: src/cf\n  - name: Gone\n    type: EXTENSION\n    path: src/gone\ntools:\n  platform:\n    path: platform\n",
        )
        .expect("project file");
        fs::write(
            root.join("v8project.local.yaml"),
            "infobases:\n  origin:\n    connection: 'File=ib'\n",
        )
        .expect("local layer");
        write_shell_script(&root.join("platform/bin/1cv8"), DESIGNER);
        let names = installed
            .iter()
            .map(|name| format!("{name}\r\n"))
            .collect::<String>();
        fs::write(root.join("platform/installed"), names).expect("installed");
        Self { _dir: dir, root }
    }

    fn fail_on(&self, extension: &str) {
        fs::write(self.root.join("platform/fail"), extension).expect("fail marker");
    }

    fn run(&self, args: &[&str]) -> Output {
        v8_runner_command()
            .current_dir(&self.root)
            .arg("--json-message")
            .args(args)
            .output()
            .expect("run command")
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.root.join("platform/calls")).unwrap_or_default()
    }

    fn out(&self) -> PathBuf {
        self.root.join("out")
    }
}

fn envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "one json document expected ({error}):\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn field<'a>(sets: &'a Value, name: &str) -> Vec<&'a str> {
    sets.as_array()
        .expect("sets")
        .iter()
        .map(|set| set[name].as_str().unwrap_or_default())
        .collect()
}

/// `make` без набора собирает каждый набор порядком назначения — основная конфигурация,
/// расширения в порядке объявления, внешние обработки — в `<каталог>/<SET>.cf`,
/// `<SET>.cfe` и каталог `<SET>`; ответ — форма `make-all`, по форме `make` на набор.
#[test]
fn make_without_a_set_builds_every_set_into_the_directory() {
    let project = Project::new(&[]);
    let output = project.run(&["make", "--output", "out"]);
    let envelope = envelope(&output);
    assert!(output.status.success(), "{envelope}");
    assert_data_matches_one_of(&envelope["data"], "make without a set", &["make-all"]);
    let data = &envelope["data"];
    assert_eq!(data["ok"], true, "{envelope}");
    assert_eq!(
        field(&data["sets"], "source_set"),
        ["main", "Sales", "Gone", "tools"],
        "{envelope}"
    );
    let out = project.out();
    assert_eq!(
        fs::read_to_string(out.join("main.cf")).expect("main.cf"),
        "package"
    );
    assert!(out.join("Sales.cfe").is_file(), "{envelope}");
    assert!(out.join("Gone.cfe").is_file(), "{envelope}");
    assert!(out.join("tools").is_dir(), "{envelope}");
    assert!(
        fs::read_dir(out.join("tools"))
            .expect("tools dir")
            .next()
            .is_some(),
        "the external set publishes its files into its own directory: {envelope}"
    );
}

/// Превью `make` без набора планирует каждый набор и ничего не собирает.
#[test]
fn make_without_a_set_preview_builds_nothing() {
    let project = Project::new(&[]);
    let output = project.run(&["make", "--output", "out", "--dry-run"]);
    let envelope = envelope(&output);
    assert!(output.status.success(), "{envelope}");
    assert_data_matches_one_of(
        &envelope["data"],
        "make preview without a set",
        &["make-all"],
    );
    let data = &envelope["data"];
    assert_eq!(data["provider_dispatched"], false, "{envelope}");
    assert_eq!(
        field(&data["sets"], "source_set"),
        ["main", "Sales", "Gone", "tools"],
        "{envelope}"
    );
    assert!(project.calls().is_empty(), "{}", project.calls());
    assert!(!project.out().exists());
}

/// Путь к файлу без набора — отказ до платформы, и `next` называет ту же команду с набором
/// основной конфигурации и файлом `.cf`.
#[test]
fn a_file_output_without_a_set_is_refused_with_the_set_step() {
    let project = Project::new(&["Sales"]);
    for (command, output) in [("make", "out/release.cf"), ("download", "out/release.cfe")] {
        let answer = project.run(&[command, "--output", output]);
        assert_eq!(answer.status.code(), Some(2), "{command}");
        let envelope = envelope(&answer);
        assert_eq!(envelope["command"], command, "{envelope}");
        assert_eq!(envelope["error"]["kind"], "validation", "{envelope}");
        let next = &envelope["error"]["next"];
        assert_eq!(next["command"], command, "{envelope}");
        assert_eq!(next["source_set"], "main", "{envelope}");
        assert_eq!(next["keys"]["--output"], "out/release.cf", "{envelope}");
    }
    assert!(
        project.calls().is_empty(),
        "the platform must not be started: {}",
        project.calls()
    );
}

/// Отказ набора останавливает обход `make`: наборы после него не собираются, ответ несёт
/// собранное и отказавший набор.
#[test]
fn make_without_a_set_stops_at_the_first_failed_set() {
    let project = Project::new(&[]);
    project.fail_on("Sales");
    let output = project.run(&["make", "--output", "out"]);
    assert!(!output.status.success());
    let envelope = envelope(&output);
    assert_data_matches_one_of(
        &envelope["data"],
        "failed make without a set",
        &["make-all"],
    );
    assert_eq!(envelope["data"]["ok"], false, "{envelope}");
    assert_eq!(
        field(&envelope["data"]["sets"], "source_set"),
        ["main", "Sales"],
        "{envelope}"
    );
    assert!(!project.out().join("Gone.cfe").exists());
    assert!(!project.out().join("tools").exists());
}

/// `download` без набора спрашивает базу о расширениях и выгружает пакеты конфигурации в
/// порядке инвентаря: основная конфигурация, затем расширения проекта, которые в базе есть.
/// Набор расширения, которого в базе нет, назван в `not_installed`; набор внешних файлов в
/// обход не входит.
#[test]
fn download_without_a_set_downloads_the_installed_packages() {
    let project = Project::new(&["sales", "Foreign"]);
    let project_file = fs::read_to_string(project.root.join("v8project.yaml")).expect("file");
    let output = project.run(&["download", "--output", "out"]);
    let envelope = envelope(&output);
    assert!(output.status.success(), "{envelope}");
    assert_data_matches_one_of(
        &envelope["data"],
        "download without a set",
        &["download-all"],
    );
    let data = &envelope["data"];
    assert_eq!(data["ok"], true, "{envelope}");
    assert_eq!(data["provider_dispatched"], true, "{envelope}");
    assert_eq!(data["provider"]["selected"], "designer", "{envelope}");
    assert_eq!(
        data["not_installed"],
        serde_json::json!(["Gone"]),
        "{envelope}"
    );
    let outputs = field(&data["sets"], "output");
    let out = project.out();
    assert_eq!(outputs.len(), 2, "{envelope}");
    assert!(outputs[0].ends_with("out/main.cf"), "{envelope}");
    assert!(outputs[1].ends_with("out/Sales.cfe"), "{envelope}");
    assert!(out.join("main.cf").is_file());
    assert!(out.join("Sales.cfe").is_file());
    assert!(!out.join("Gone.cfe").exists());
    let calls = project.calls();
    assert!(
        calls
            .lines()
            .next()
            .unwrap_or_default()
            .contains("/DumpDBCfgList"),
        "{calls}"
    );
    assert!(!calls.contains("-Extension Gone"), "{calls}");
    assert!(!calls.contains("Foreign"), "{calls}");
    // Расширение базы без набора не объявляется: проектный файл не меняется.
    assert_eq!(
        fs::read_to_string(project.root.join("v8project.yaml")).expect("file"),
        project_file
    );
}

/// Превью `download` без набора платформу не запускает: выгрузку планирует набору основной
/// конфигурации, а наборы расширений называет в `if_installed`.
#[test]
fn download_without_a_set_preview_reads_nothing() {
    let project = Project::new(&["Sales"]);
    let output = project.run(&["download", "--output", "out", "--dry-run"]);
    let envelope = envelope(&output);
    assert!(output.status.success(), "{envelope}");
    assert_data_matches_one_of(
        &envelope["data"],
        "download preview without a set",
        &["download-all"],
    );
    let data = &envelope["data"];
    assert_eq!(data["provider_dispatched"], false, "{envelope}");
    assert_eq!(
        data["if_installed"],
        serde_json::json!(["Sales", "Gone"]),
        "{envelope}"
    );
    let outputs = field(&data["sets"], "output");
    assert_eq!(outputs.len(), 1, "{envelope}");
    assert!(outputs[0].ends_with("out/main.cf"), "{envelope}");
    assert!(project.calls().is_empty(), "{}", project.calls());
    assert!(!project.out().exists());
}

/// Отказ набора останавливает обход `download`: пакеты после него не выгружаются.
#[test]
fn download_without_a_set_stops_at_the_first_failed_set() {
    let project = Project::new(&["Sales", "Gone"]);
    project.fail_on("Sales");
    let output = project.run(&["download", "--output", "out"]);
    assert!(!output.status.success());
    let envelope = envelope(&output);
    assert_data_matches_one_of(
        &envelope["data"],
        "failed download without a set",
        &["download-all"],
    );
    let data = &envelope["data"];
    assert_eq!(data["ok"], false, "{envelope}");
    let outputs = field(&data["sets"], "output");
    assert_eq!(outputs.len(), 2, "{envelope}");
    assert!(outputs[1].ends_with("out/Sales.cfe"), "{envelope}");
    assert!(
        !project.calls().contains("-Extension Gone"),
        "{}",
        project.calls()
    );
}

impl Project {
    fn rewrite(&self, from: &str, to: &str) {
        let path = self.root.join("v8project.yaml");
        let text = fs::read_to_string(&path).expect("project file");
        assert!(text.contains(from), "{from} in {text}");
        fs::write(&path, text.replacen(from, to, 1)).expect("project file");
    }

    fn run_in(&self, dir: &std::path::Path, args: &[&str]) -> Output {
        v8_runner_command()
            .current_dir(dir)
            .arg("--config")
            .arg(self.root.join("v8project.yaml"))
            .arg("--json-message")
            .args(args)
            .output()
            .expect("run command")
    }
}

fn refused(output: &Output) -> Value {
    assert_eq!(output.status.code(), Some(2));
    let envelope = envelope(output);
    assert_eq!(envelope["ok"], false, "{envelope}");
    assert_eq!(envelope["error"]["kind"], "validation", "{envelope}");
    envelope
}

/// Пакет не ложится на каталог набора и `workPath`, внутрь них и вокруг них: публикация
/// каталога внешнего набора поверх `src/tools` заменила бы его исходники. Отказ — до
/// работы, исходники на месте.
#[test]
fn a_package_directory_overlapping_the_sources_is_refused_before_work() {
    let project = Project::new(&["Sales"]);
    for (command, output) in [
        // `src/tools` — каталог набора `tools`: тот же путь.
        ("make", "src"),
        // `src/cf/main.cf` — внутри каталога набора `main`.
        ("make", "src/cf"),
        ("make", "work"),
        ("download", "src/cf"),
        ("download", "work"),
    ] {
        let answer = project.run(&[command, "--output", output]);
        let envelope = refused(&answer);
        let message = envelope["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("overlaps"),
            "{command} {output}: {envelope}"
        );
    }
    assert!(project.root.join("src/tools/Tool.xml").is_file());
    assert!(
        project.calls().is_empty(),
        "the platform must not be started: {}",
        project.calls()
    );
}

/// Каталог внешнего набора называется именем набора и с точкой в имени: `tools.v2` —
/// каталог, а не файл с суффиксом.
#[test]
fn an_external_set_with_a_dot_in_its_name_is_built_into_its_directory() {
    let project = Project::new(&[]);
    project.rewrite("- name: tools\n", "- name: tools.v2\n");
    let output = project.run(&["make", "--output", "out"]);
    let envelope = envelope(&output);
    assert!(output.status.success(), "{envelope}");
    assert!(project.out().join("tools.v2").is_dir(), "{envelope}");
    assert!(
        fs::read_dir(project.out().join("tools.v2"))
            .expect("set dir")
            .next()
            .is_some(),
        "{envelope}"
    );
}

/// Два набора, чьи пакеты на файловой системе без регистра назвались бы одним файлом, и
/// набор с именем устройства Windows — отказ до работы.
#[test]
fn package_names_that_collide_or_name_a_device_are_refused_before_work() {
    for (from, to, expected) in [
        ("- name: Gone\n", "- name: sales\n", "case-insensitive"),
        ("- name: Gone\n", "- name: CON\n", "device name"),
    ] {
        let project = Project::new(&["Sales"]);
        project.rewrite(from, to);
        for command in ["make", "download"] {
            let answer = project.run(&[command, "--output", "out"]);
            let envelope = refused(&answer);
            let message = envelope["error"]["message"].as_str().unwrap_or_default();
            assert!(message.contains(expected), "{command} {to}: {envelope}");
        }
        assert!(project.calls().is_empty(), "{}", project.calls());
    }
}

/// Относительный каталог `make` считает от текущего каталога, `download` — от `basePath`,
/// и ответ называет разрешённый каталог.
#[test]
fn a_relative_directory_resolves_like_the_command_with_a_set() {
    let project = Project::new(&["Sales"]);
    let sub = project.root.join("sub");
    fs::create_dir_all(&sub).expect("sub dir");

    let made = envelope(&project.run_in(&sub, &["make", "--output", "out", "--dry-run"]));
    assert_eq!(made["ok"], true, "{made}");
    let made_dir = made["data"]["output_path"].as_str().unwrap_or_default();
    assert!(made_dir.ends_with("project/sub/out"), "{made}");
    assert!(
        made["data"]["sets"][0]["output_path"]
            .as_str()
            .unwrap_or_default()
            .ends_with("project/sub/out/main.cf"),
        "{made}"
    );

    let downloaded = envelope(&project.run_in(&sub, &["download", "--output", "out", "--dry-run"]));
    assert_eq!(downloaded["ok"], true, "{downloaded}");
    let output = downloaded["data"]["output"].as_str().unwrap_or_default();
    assert!(
        output.ends_with("project/out") && std::path::Path::new(output).is_absolute(),
        "{downloaded}"
    );
}

/// Без набора конфигурации шага к набору нет: отказ на файл без набора не называет `next`.
#[test]
fn without_a_configuration_set_a_file_output_names_no_step() {
    let project = Project::new(&["Sales"]);
    project.rewrite(
        "  - name: main\n    type: CONFIGURATION\n    path: src/cf\n",
        "",
    );
    for command in ["make", "download"] {
        let envelope = refused(&project.run(&[command, "--output", "out/release.cf"]));
        assert!(
            envelope["error"].get("next").is_none(),
            "{command}: {envelope}"
        );
    }
    assert!(project.calls().is_empty(), "{}", project.calls());
}

/// Обход `download` идёт так же в проекте формата EDT: по составу базы и в каталог.
#[test]
fn download_without_a_set_walks_an_edt_project() {
    let project = Project::new(&["Sales"]);
    project.rewrite("format: DESIGNER\n", "format: EDT\n");
    let output = project.run(&["download", "--output", "out"]);
    let envelope = envelope(&output);
    assert!(output.status.success(), "{envelope}");
    assert_eq!(
        envelope["data"]["not_installed"],
        serde_json::json!(["Gone"]),
        "{envelope}"
    );
    assert!(project.out().join("main.cf").is_file());
    assert!(project.out().join("Sales.cfe").is_file());
}

/// Набор расширения, исходники которого называют другое установленное расширение, —
/// отказ `download` до первой выгрузки, как у `pull --all` (#218): по имени набора он взял бы
/// не то расширение.
#[test]
fn download_refuses_a_set_whose_sources_name_another_installed_extension() {
    let project = Project::new(&["Sales", "Other"]);
    fs::write(
        project.root.join("src/sales/Configuration.xml"),
        "<MetaDataObject><Configuration><Properties><Name>Other</Name><ConfigurationExtensionPurpose>AddOn</ConfigurationExtensionPurpose></Properties></Configuration></MetaDataObject>\n",
    )
    .expect("descriptor");
    let envelope = refused(&project.run(&["download", "--output", "out"]));
    let message = envelope["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("#218"), "{envelope}");
    assert!(!project.out().exists());
    assert!(!project.calls().contains("/DumpCfg"), "{}", project.calls());
}
