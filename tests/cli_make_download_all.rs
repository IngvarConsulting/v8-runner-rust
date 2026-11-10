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
if [ "$1" = CREATEINFOBASE ]; then
  db=$(printf '%s' "${2#File=}" | tr -d "'")
  mkdir -p "$db"
  printf 'database' > "$db/1Cv8.1CD"
fi
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
    -listFile) cp "$argument" "$root/partial-list" ;;
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
            format!("workPath: work\nformat: DESIGNER\n{designer_leads}source-set:\n  - name: tools\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src/tools\n  - name: Sales\n    type: EXTENSION\n    path: src/sales\n  - name: main\n    type: CONFIGURATION\n    path: src/cf\n  - name: Gone\n    type: EXTENSION\n    path: src/gone\ntools:\n  platform:\n    path: platform\n", designer_leads = support::DESIGNER_LEADS),
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

/// Обход `make` собирает все наборы в одной базе сборки. Она сохраняется между прогонами;
/// основная конфигурация загружается один раз, расширения ложатся поверх.
#[test]
fn make_without_a_set_builds_every_package_in_one_build_infobase() {
    let project = Project::new(&[]);
    let output = project.run(&["make", "--output", "out"]);
    let envelope = envelope(&output);
    assert!(output.status.success(), "{envelope}");
    let calls = project.calls();
    let lines = calls.lines().collect::<Vec<_>>();
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("CREATEINFOBASE"))
            .count(),
        1,
        "{calls}"
    );
    let configuration_loads = lines
        .iter()
        .filter(|line| line.contains("/LoadConfigFromFiles") && !line.contains("-Extension"))
        .count();
    assert_eq!(configuration_loads, 1, "{calls}");
    for extension in ["Sales", "Gone"] {
        assert!(
            lines
                .iter()
                .any(|line| line.contains("/LoadConfigFromFiles")
                    && line.contains(&format!("-Extension {extension}"))),
            "{calls}"
        );
    }
    let build_base = project.root.join("work/temp/build-infobase");
    assert!(
        lines
            .iter()
            .filter(|line| !line.starts_with("CREATEINFOBASE"))
            .all(|line| line.contains(&build_base.display().to_string())),
        "every call goes to the build infobase: {calls}"
    );
    assert!(
        !calls.contains("File=ib"),
        "the project base is untouched: {calls}"
    );
    assert!(!calls.contains("-updateConfigDumpInfo"), "{calls}");
    assert!(build_base.join("ib/1Cv8.1CD").is_file());
    assert!(!project.root.join("work/temp/throwaway-infobases").exists());
}

#[test]
fn make_reuses_an_edt_build_infobase_until_forced() {
    let project = Project::new(&[]);
    let source = project.root.join("src/cf");
    let module = source.join("src/Configuration/Module.bsl");
    fs::create_dir_all(module.parent().expect("module parent")).expect("source");
    fs::write(&module, "// initial").expect("module");
    fs::write(
        source.join(".project"),
        "<projectDescription><name>Main</name><natures><nature>com._1c.g5.v8.dt.core.V8ConfigurationNature</nature></natures></projectDescription>",
    )
    .expect("EDT project descriptor");
    fs::create_dir(source.join("DT-INF")).expect("manifest directory");
    fs::write(
        source.join("DT-INF/PROJECT.PMF"),
        "Manifest-Version: 1.0\nRuntime-Version: 8.3.27\n",
    )
    .expect("EDT project manifest");
    fs::write(
        source.join("src/Configuration/Configuration.mdo"),
        "<Configuration/>",
    )
    .expect("EDT metadata marker");
    fs::write(
        project.root.join("v8project.yaml"),
        "workPath: work\nformat: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src/cf\ntools:\n  platform:\n    path: platform\n  edt_cli:\n    path: platform/bin/1cedtcli\n",
    )
    .expect("project config");
    write_shell_script(
        &project.root.join("platform/bin/1cedtcli"),
        r#"root="$(dirname "$0")/.."
target=''
source=''
previous=''
for argument in "$@"; do
  if [ "$previous" = --configuration-files ]; then target="$argument"; fi
  if [ "$previous" = --project ]; then source="$argument"; fi
  previous="$argument"
done
test -n "$target" && test -n "$source" || exit 13
printf '%s\n' "$*" >> "$root/edt-calls"
mkdir -p "$target/CommonModules/Demo/Ext"
printf '<Configuration/>' > "$target/Configuration.xml"
printf '<CommonModule/>' > "$target/CommonModules/Demo.xml"
cp "$source/src/Configuration/Module.bsl" "$target/CommonModules/Demo/Ext/Module.bsl"
"#,
    );
    let build_base = project.root.join("work/temp/build-infobase");
    let edt_log = project.root.join("platform/edt-calls");
    let run = |options: &[&str]| {
        let mut args = vec!["make", "main", "--output", "out/main.cf"];
        args.extend_from_slice(options);
        let output = project.run(&args);
        let answer = envelope(&output);
        assert!(output.status.success(), "{answer}");
        answer
    };

    let preview = run(&["--dry-run", "--full"]);
    assert_eq!(preview["data"]["provider_dispatched"], false, "{preview}");
    assert!(
        preview["data"]["execution"]["payload"]["build"].is_null(),
        "{preview}"
    );
    assert!(!build_base.exists());
    assert!(!edt_log.exists());
    assert!(project.calls().is_empty());

    let first = run(&[]);
    assert_eq!(
        first["data"]["execution"]["payload"]["build"]["mode"], "full",
        "{first}"
    );
    let original_calls = project.calls();
    assert_eq!(original_calls.lines().count(), 3, "{original_calls}");
    let original_edt = fs::read_to_string(&edt_log).expect("first EDT export");
    let actual_source = original_edt
        .split_whitespace()
        .skip_while(|arg| *arg != "--project")
        .nth(1)
        .expect("EDT --project argument");
    assert_eq!(
        fs::canonicalize(actual_source).expect("actual source"),
        fs::canonicalize(&source).expect("expected source"),
        "{original_edt}"
    );

    let second = run(&["--clean-before-execution"]);
    assert_eq!(
        second["data"]["execution"]["payload"]["build"]["mode"], "unchanged",
        "{second}"
    );
    assert_eq!(fs::read_to_string(&edt_log).expect("EDT log"), original_edt);
    assert!(build_base.join("ib/1Cv8.1CD").is_file());
    let second_calls = project.calls();
    let second_run = second_calls
        .strip_prefix(&original_calls)
        .expect("second run");
    assert_eq!(second_run.lines().count(), 1, "{second_run}");
    assert!(second_run.contains("/DumpCfg"), "{second_run}");

    fs::write(&module, "// changed").expect("modified module");
    let partial = run(&[]);
    assert_eq!(
        partial["data"]["execution"]["payload"]["build"]["mode"], "partial",
        "{partial}"
    );
    assert_eq!(
        fs::read_to_string(&edt_log)
            .expect("EDT log")
            .lines()
            .count(),
        2
    );
    let modified_calls = project.calls();
    let modified_run = modified_calls
        .strip_prefix(&second_calls)
        .expect("modified run");
    assert_eq!(modified_run.lines().count(), 2, "{modified_run}");
    assert!(
        modified_run.contains("-partial -listFile"),
        "{modified_run}"
    );
    assert!(modified_run.contains("/DumpCfg"), "{modified_run}");
    assert!(
        !modified_run.contains("-updateConfigDumpInfo"),
        "{modified_run}"
    );
    assert!(!modified_run.contains("/UpdateDBCfg"), "{modified_run}");
    let list =
        fs::read_to_string(project.root.join("platform/partial-list")).expect("partial list");
    assert!(list.contains("CommonModules/Demo/Ext/Module.bsl"), "{list}");
    // Описание модуля не менялось: модуль идёт один
    // (`INV.USE-CASES.A-CHANGED-MODULE-LOADS-WITHOUT-ITS-DESCRIPTION`).
    assert!(!list.contains("CommonModules/Demo.xml"), "{list}");

    let sentinel = build_base.join("ib/old-build");
    fs::write(&sentinel, "old build").expect("sentinel");
    let full = run(&["--full"]);
    assert_eq!(
        full["data"]["execution"]["payload"]["build"]["mode"], "full",
        "{full}"
    );
    assert!(
        full["data"]["execution"]["payload"]["build"]["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty()),
        "{full}"
    );
    assert!(!sentinel.exists(), "the old infobase was replaced");
    assert!(build_base.join("ib/1Cv8.1CD").is_file());
    let full_calls = project.calls();
    let forced_run = full_calls
        .strip_prefix(&modified_calls)
        .expect("forced run");
    assert!(forced_run.starts_with("CREATEINFOBASE"), "{forced_run}");
    assert!(forced_run.contains("/LoadConfigFromFiles"), "{forced_run}");
    assert!(!forced_run.contains("-partial"), "{forced_run}");
}

#[test]
fn make_reuses_the_build_infobase_when_the_package_is_inside_the_source_set() {
    let project = Project::new(&[]);
    let source = project.root.join("src/cf");
    fs::write(source.join("Configuration.xml"), "<Configuration/>").expect("descriptor");
    let args = ["make", "main", "--output", "src/cf/release.cf"];
    let output = project.run(&args);
    assert!(output.status.success(), "{}", envelope(&output));
    assert!(source.join("release.cf").is_file());
    let first = project.calls();

    let output = project.run(&args);
    let answer = envelope(&output);
    assert!(output.status.success(), "{answer}");
    assert_eq!(
        answer["data"]["execution"]["payload"]["build"]["mode"], "unchanged",
        "{answer}"
    );
    let second = project.calls();
    let commands = second.strip_prefix(&first).expect("second run");
    assert_eq!(commands.lines().count(), 1, "{commands}");
    assert!(commands.contains("/DumpCfg"), "{commands}");
}

/// `make` базу проекта не выбирает: без местного слоя и без `origin` он собирает пакет, а
/// `--infobase` отвергает.
#[test]
fn make_needs_no_infobase_and_refuses_the_infobase_key() {
    let project = Project::new(&[]);
    fs::remove_file(project.root.join("v8project.local.yaml")).expect("local layer");
    let output = project.run(&["make", "main", "--output", "out/main.cf"]);
    let built = envelope(&output);
    assert!(output.status.success(), "{built}");
    assert!(project.out().join("main.cf").is_file(), "{built}");

    let output = project.run(&[
        "make",
        "main",
        "--output",
        "out/main.cf",
        "--infobase",
        "origin",
    ]);
    assert_eq!(output.status.code(), Some(2));
    let refused = envelope(&output);
    assert_eq!(refused["error"]["kind"], "validation", "{refused}");
}

/// `make` базу проекта не читает и о синониме её секции `infobase:` в проектном файле не
/// предупреждает; `download`, который базу читает, предупреждает.
#[test]
fn make_does_not_warn_about_the_infobase_synonym() {
    let project = Project::new(&[]);
    fs::remove_file(project.root.join("v8project.local.yaml")).expect("local layer");
    let file = project.root.join("v8project.yaml");
    let text = fs::read_to_string(&file).expect("project file");
    fs::write(&file, format!("{text}infobase:\n  connection: 'File=ib'\n")).expect("synonym");

    let made = envelope(&project.run(&["make", "main", "--output", "out/main.cf", "--dry-run"]));
    assert_eq!(made["ok"], true, "{made}");
    assert!(
        made["warnings"].as_array().is_none_or(Vec::is_empty),
        "{made}"
    );
    let downloaded =
        envelope(&project.run(&["download", "main", "--output", "out/main.cf", "--dry-run"]));
    assert!(
        downloaded["warnings"].to_string().contains("infobase"),
        "{downloaded}"
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

/// Внешний набор с голым относительным `--output` (`Deploy`) публикуется в каталог от
/// текущего: родитель пустого пути — текущий каталог, а не ошибка fsync (#443).
#[test]
fn an_external_set_is_published_to_a_bare_relative_output() {
    let project = Project::new(&[]);
    let output = project.run(&["make", "tools", "--output", "Deploy"]);
    let first = envelope(&output);
    assert!(output.status.success(), "{first}");
    let deploy = project.root.join("Deploy");
    assert!(deploy.is_dir(), "{first}");
    assert!(
        fs::read_dir(&deploy).expect("deploy dir").next().is_some(),
        "{first}"
    );

    // Повторная сборка заменяет существующий каталог — путь с резервной копией.
    let again = project.run(&["make", "tools", "--output", "Deploy"]);
    assert!(again.status.success(), "{}", envelope(&again));
    assert!(deploy.is_dir());
    // Ни в каталоге, ни рядом с ним не остаётся служебных меток промежуточных копий.
    for dir in [&deploy, &project.root] {
        let leftovers: Vec<_> = fs::read_dir(dir)
            .expect("listing")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".meta.json") || name.starts_with(".v8-runner"))
            .collect();
        assert!(leftovers.is_empty(), "{}: {leftovers:?}", dir.display());
    }

    // Файл пакета с голым именем — замена файла в текущем каталоге.
    let package = project.run(&["make", "main", "--output", "main.cf"]);
    assert!(package.status.success(), "{}", envelope(&package));
    assert!(project.root.join("main.cf").is_file());
    let replaced = project.run(&["make", "main", "--output", "main.cf"]);
    assert!(replaced.status.success(), "{}", envelope(&replaced));
}

/// Два набора, чьи пакеты (файл или каталог) на файловой системе без регистра назвались бы
/// одним именем, и набор с именем устройства Windows — отказ до работы.
#[test]
fn package_names_that_collide_or_name_a_device_are_refused_before_work() {
    for (from, to, expected, commands) in [
        (
            "- name: Gone\n",
            "- name: sales\n",
            "case-insensitive",
            &["make", "download"][..],
        ),
        (
            "- name: Gone\n",
            "- name: CON\n",
            "device name",
            &["make", "download"][..],
        ),
        // Внешний набор `Sales.cfe` — каталог `out/Sales.cfe`, тот же путь, что пакет
        // расширения `Sales`; `download` внешние наборы не обходит.
        (
            "- name: tools\n",
            "- name: Sales.cfe\n",
            "case-insensitive",
            &["make"][..],
        ),
    ] {
        let project = Project::new(&["Sales"]);
        project.rewrite(from, to);
        for command in commands {
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

    // `..` свёрнут лексически: ответ называет каталог без `..`, ещё не созданный.
    let folded = envelope(&project.run_in(&sub, &["make", "--output", "../out", "--dry-run"]));
    assert_eq!(folded["ok"], true, "{folded}");
    let folded_set = folded["data"]["sets"][0]["output_path"]
        .as_str()
        .unwrap_or_default();
    assert!(
        folded_set.ends_with("project/out/main.cf") && !folded_set.contains(".."),
        "{folded}"
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
