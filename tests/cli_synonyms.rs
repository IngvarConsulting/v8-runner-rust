#![cfg(unix)]

//! Прежние имена команд, ключей и значений принимаются один цикл выпуска и отвечают так же,
//! как запись словаря вместо них: конверт называет новое имя, данные и квитанция исполнителя
//! совпадают. Перечень прежних имён — `src/cli/synonyms.rs`; здесь он не повторяется, а
//! проверяется, что у каждой его строки есть вызов.

mod support;

#[path = "../src/cli/synonyms.rs"]
mod synonyms;

use std::fs;
use std::path::PathBuf;
use std::process::Output;

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};
use synonyms::{Previous, SYNONYMS};

struct Project {
    dir: tempfile::TempDir,
    root: PathBuf,
    calls: PathBuf,
}

/// Проект формата платформы: набор конфигурации `main` и набор расширения `sales`.
/// Поддельная платформа только записывает вызов.
fn project() -> Project {
    let dir = temp_workspace();
    let root = dir.path().join("project");
    let work = dir.path().join("work");
    let calls = dir.path().join("calls.log");
    let platform = dir.path().join("1cv8");
    for set in ["main", "sales"] {
        fs::create_dir_all(root.join(set)).expect("source set");
    }
    fs::write(
        root.join("main/Configuration.xml"),
        "<MetaDataObject><Configuration><Properties><Name>Main</Name></Properties></Configuration></MetaDataObject>\n",
    )
    .expect("main descriptor");
    fs::write(
        root.join("sales/Configuration.xml"),
        "<MetaDataObject><Configuration><Properties><Name>sales</Name><ConfigurationExtensionPurpose>AddOn</ConfigurationExtensionPurpose></Properties></Configuration></MetaDataObject>\n",
    )
    .expect("extension descriptor");
    fs::create_dir_all(&work).expect("work");
    let infobase = dir.path().join("ib");
    fs::create_dir_all(&infobase).expect("infobase");
    fs::write(infobase.join("1Cv8.1CD"), "database").expect("infobase file");
    write_shell_script(
        &platform,
        &format!("printf '%s\\n' \"$*\" >> \"{}\"\nexit 0", calls.display()),
    );
    fs::write(
        root.join("v8project.yaml"),
        format!(
            "workPath: '{}'\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\n  - name: sales\n    type: EXTENSION\n    path: sales\ntools:\n  platform:\n    path: '{}'\n  edt_cli:\n    path: '{}'\n",
            work.display(),
            platform.display(),
            dir.path().join("edt/1cedtcli").display(),
        ),
    )
    .expect("project file");
    fs::write(
        root.join("v8project.local.yaml"),
        format!(
            "infobases:\n  origin:\n    connection: 'File={}'\n",
            infobase.display(),
        ),
    )
    .expect("local layer");
    edt_project(&dir.path().join("edt"));
    let out = dir.path().join("out");
    fs::create_dir_all(&out).expect("out");
    fs::write(out.join("main.cf"), "cf").expect("package");
    fs::write(out.join("settings.xml"), "<Settings/>").expect("settings");
    Project { dir, root, calls }
}

/// Проект формата EDT рядом с основным: ветку проверки выбирает формат проекта, и прежнее
/// `check edt` сравнивается с `check` там, где это одна и та же ветка.
fn edt_project(root: &std::path::Path) {
    let source = root.join("main");
    for dir in ["metadata", "DT-INF", "src/Configuration"] {
        fs::create_dir_all(source.join(dir)).expect("edt layout");
    }
    fs::write(
        source.join(".project"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>main</name>\n  <natures>\n    <nature>com._1c.g5.v8.dt.core.V8ConfigurationNature</nature>\n  </natures>\n</projectDescription>\n",
    )
    .expect("project");
    fs::write(
        source.join("DT-INF/PROJECT.PMF"),
        "Manifest-Version: 1.0\nRuntime-Version: 8.3.27\n",
    )
    .expect("manifest");
    fs::write(
        source.join("metadata/Configuration.xml"),
        "<Configuration />",
    )
    .expect("descriptor");
    fs::write(
        source.join("src/Configuration/Configuration.mdo"),
        "<Configuration />\n",
    )
    .expect("configuration");
    let edt_cli = root.join("1cedtcli");
    write_shell_script(&edt_cli, "exit 3");
    write_shell_script(&root.join("platform/bin/1cv8"), "exit 3");
    fs::write(
        root.join("v8project.yaml"),
        format!(
            "workPath: '{}'\nformat: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\ntools:\n  platform:\n    path: '{}'\n  edt_cli:\n    path: '{}'\n",
            root.join("work").display(),
            root.join("platform").display(),
            edt_cli.display(),
        ),
    )
    .expect("edt project file");
    fs::write(
        root.join("v8project.local.yaml"),
        format!(
            "infobases:\n  origin:\n    connection: 'File={}'\n",
            root.join("ib").display()
        ),
    )
    .expect("edt local layer");
}

impl Project {
    fn path(&self, name: &str) -> String {
        self.dir.path().join(name).display().to_string()
    }
}

fn run(project: &Project, args: &[String]) -> Output {
    v8_runner_command()
        .current_dir(&project.root)
        .arg("--json-message")
        .args(args)
        .output()
        .expect("run command")
}

/// Конверт без длительностей: только они вправе разойтись у двух прогонов.
fn envelope(args: &[String], output: &Output) -> Value {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(object) => {
                object.remove("duration_ms");
                object.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{args:?} printed no json envelope ({error}):\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    strip(&mut value);
    value
}

/// Пары «прежняя запись → запись словаря»; у каждой строки перечня есть своя пара.
fn cases(project: &Project) -> Vec<(Vec<String>, Vec<String>)> {
    let cf = project.path("out/main.cf");
    let cfe = project.path("out/sales.cfe");
    let settings = project.path("out/settings.xml");
    let ib = format!("File={}", project.path("ib"));
    let clone_dir = project.path("clone");
    let edt_config = project.path("edt/v8project.yaml");
    let init_output = project.path("initialized/v8project.yaml");
    let owned = |args: &[&str]| args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    let pairs: Vec<(Vec<&str>, Vec<&str>)> = vec![
        // Команды.
        (vec!["build", "--dry-run"], vec!["push", "--dry-run"]),
        (vec!["dump", "--dry-run"], vec!["pull", "--dry-run"]),
        (
            vec!["load", &cf, "--dry-run"],
            vec!["upload", &cf, "--dry-run"],
        ),
        (
            vec!["syntax", "--thin-client"],
            vec!["check", "--thin-client"],
        ),
        (
            vec!["check", "designer-config", "--thin-client"],
            vec!["check", "--thin-client"],
        ),
        (
            vec!["check", "designer-modules", "--server"],
            vec!["check", "--server"],
        ),
        (
            vec!["check", "edt", "--dry-run", "--config", &edt_config],
            vec!["check", "--dry-run", "--config", &edt_config],
        ),
        (
            vec![
                "bootstrap",
                "--from",
                &ib,
                "--platform-version",
                "8.3.27",
                "--project-dir",
                &clone_dir,
                "--dry-run",
            ],
            vec![
                "clone",
                "--from",
                &ib,
                "--platform-version",
                "8.3.27",
                "--project-dir",
                &clone_dir,
                "--dry-run",
            ],
        ),
        (
            vec!["config", "init", "--output", &init_output],
            vec!["init", "--output", &init_output],
        ),
        // Отказ по глобальному ключу тоже называет лист словаря, а не прежний путь.
        (
            vec!["config", "init", "--dry-run"],
            vec!["init", "--dry-run"],
        ),
        (
            vec![
                "infobase",
                "configuration",
                "export",
                "--output",
                &cf,
                "--dry-run",
            ],
            vec!["download", "--output", &cf, "--dry-run"],
        ),
        // Ключи.
        (
            vec![
                "clone",
                "--connection",
                &ib,
                "--platform-version",
                "8.3.27",
                "--project-dir",
                &clone_dir,
                "--dry-run",
            ],
            vec![
                "clone",
                "--from",
                &ib,
                "--platform-version",
                "8.3.27",
                "--project-dir",
                &clone_dir,
                "--dry-run",
            ],
        ),
        (
            vec!["init", "--connection", &ib, "--output", &init_output],
            vec!["--infobase", &ib, "init", "--output", &init_output],
        ),
        (
            vec!["push", "--full-rebuild", "--dry-run"],
            vec!["push", "--full", "--dry-run"],
        ),
        (
            vec!["push", "--source-set", "sales", "--dry-run"],
            vec!["push", "sales", "--dry-run"],
        ),
        (
            vec!["pull", "--source-set", "sales", "--dry-run"],
            vec!["pull", "sales", "--dry-run"],
        ),
        (
            vec![
                "make",
                "--source-set",
                "sales",
                "--output",
                &cfe,
                "--dry-run",
            ],
            vec!["make", "sales", "--output", &cfe, "--dry-run"],
        ),
        (
            vec!["convert", "--source-set", "sales", "--dry-run"],
            vec!["convert", "sales", "--dry-run"],
        ),
        (
            vec!["pull", "--discard-uncommitted", "--dry-run"],
            vec!["pull", "--force", "--dry-run"],
        ),
        (
            vec!["convert", "--discard-uncommitted", "--dry-run"],
            vec!["convert", "--force", "--dry-run"],
        ),
        (
            vec!["pull", "--mode", "incremental", "--dry-run"],
            vec!["pull", "--dry-run"],
        ),
        (
            vec!["pull", "--mode", "partial", "--dry-run"],
            vec!["pull", "--dry-run"],
        ),
        (
            vec![
                "pull",
                "--mode",
                "partial",
                "--object",
                "Catalog:Items",
                "--dry-run",
            ],
            vec!["pull", "--object", "Catalog:Items", "--dry-run"],
        ),
        (
            vec!["test", "--no-build", "yaxunit", "all"],
            vec!["test", "--no-push", "yaxunit", "all"],
        ),
        (
            vec!["upload", "--path", &cf, "--dry-run"],
            vec!["upload", &cf, "--dry-run"],
        ),
        // Значения.
        (
            vec![
                "upload",
                &cf,
                "--mode",
                "merge",
                "--settings",
                &settings,
                "--dry-run",
            ],
            vec![
                "upload",
                &cf,
                "--mode",
                "combine",
                "--settings",
                &settings,
                "--dry-run",
            ],
        ),
        (
            vec![
                "download",
                "--state",
                "working",
                "--output",
                &cf,
                "--dry-run",
            ],
            vec!["download", "--output", &cf, "--dry-run"],
        ),
        (
            vec![
                "download",
                "--state",
                "database",
                "--output",
                &cf,
                "--dry-run",
            ],
            vec!["download", "--state", "db", "--output", &cf, "--dry-run"],
        ),
    ];
    pairs
        .into_iter()
        .map(|(previous, current)| (owned(&previous), owned(&current)))
        .collect()
}

/// Вызов с прежним именем отвечает тем же конвертом, что запись словаря: та же метка
/// `command`, те же данные и квитанция исполнителя, тот же код выхода
/// (`INV.CLI.A-SYNONYM-ANSWERS-UNDER-THE-NEW-NAME`, `INV.CLI.AN-OLD-KEY-MAPS-TO-ITS-NEW-MEANING-FOR-ONE-CYCLE`).
#[test]
fn every_previous_name_answers_as_its_dictionary_entry() {
    // Журнал платформы и каталог прогона названы моментом и процессом запуска: у двух
    // прогонов имена разные, и только они.
    let launch_names = [
        (
            regex::Regex::new(r"_\d+_\d+_\d+\.log").expect("log name"),
            "_<launch>.log",
        ),
        (
            regex::Regex::new(r"/runs/\d+-\d+-[0-9a-f]+").expect("run name"),
            "/runs/<launch>",
        ),
    ];
    let anonymous = |text: &str, root: &str| {
        launch_names
            .iter()
            .fold(text.replace(root, "<root>"), |text, (pattern, name)| {
                pattern.replace_all(&text, *name).into_owned()
            })
    };
    let count = cases(&project()).len();
    for index in 0..count {
        // Каждой стороне — свой проект: первый прогон не должен готовить почву второму.
        let left = project();
        let right = project();
        let previous = cases(&left).swap_remove(index).0;
        let current = cases(&right).swap_remove(index).1;

        let previous_output = run(&left, &previous);
        let current_output = run(&right, &current);
        let left_root = left.dir.path().display().to_string();
        let right_root = right.dir.path().display().to_string();
        let normalized = |answer: Value, root: &str| anonymous(&answer.to_string(), root);
        let previous_answer = normalized(envelope(&previous, &previous_output), &left_root);
        let current_answer = normalized(envelope(&current, &current_output), &right_root);

        assert_eq!(
            previous_output.status.code(),
            current_output.status.code(),
            "{previous:?} vs {current:?}:\n{previous_answer}\n{current_answer}"
        );
        assert_eq!(
            previous_answer, current_answer,
            "{previous:?} must answer as {current:?}"
        );
        let calls = |project: &Project| fs::read_to_string(&project.calls).unwrap_or_default();
        assert_eq!(
            anonymous(&calls(&left), &left_root),
            anonymous(&calls(&right), &right_root),
            "{previous:?} must ask the platform what {current:?} asks"
        );
    }
}

/// У каждой строки перечня прежних имён есть вызов выше: синоним без проверки ответа не
/// появится.
#[test]
fn every_synonym_of_the_table_has_a_case() {
    let project = project();
    let cases = cases(&project);
    for synonym in SYNONYMS {
        let covered = cases.iter().any(|(previous, _)| {
            let at = |index: usize| previous.get(index).map(String::as_str);
            let scoped = |offset: usize| {
                synonym
                    .command
                    .iter()
                    .enumerate()
                    .all(|(index, name)| at(index + offset) == Some(*name))
            };
            match synonym.previous {
                Previous::Command(name) => scoped(0) && at(synonym.command.len()) == Some(name),
                Previous::Key(name) => {
                    scoped(0) && previous.iter().any(|arg| *arg == format!("--{name}"))
                }
                Previous::Value { key, value } => {
                    scoped(0)
                        && previous
                            .windows(2)
                            .any(|pair| pair[0] == format!("--{key}") && pair[1] == value)
                }
            }
        });
        assert!(
            covered,
            "{:?} under {:?} has no case: add one that compares it with {}",
            synonym.previous, synonym.command, synonym.current
        );
    }
}

/// Отказ до платформы: `invalid_argument`, выход 2, метка `pull`, и текст содержит каждую
/// из названных фраз.
fn assert_refused(project: &Project, args: &[&str], phrases: &[&str]) {
    let args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    let output = run(project, &args);
    let answer = envelope(&args, &output);
    assert_eq!(output.status.code(), Some(2), "{args:?}: {answer}");
    assert_eq!(answer["ok"], false, "{answer}");
    assert_eq!(answer["command"], "pull", "{answer}");
    assert_eq!(answer["error"]["code"], "invalid_argument", "{answer}");
    let message = answer["error"]["message"].as_str().unwrap_or_default();
    for phrase in phrases {
        assert!(
            message.contains(phrase),
            "{args:?} must say {phrase:?}: {answer}"
        );
    }
    assert!(
        !project.calls.exists(),
        "{args:?}: the platform must not be started"
    );
}

/// Что делает замена — так, чтобы последствие было видно без документации.
const FORCE_MEANS: &str =
    "`pull --force` is a full dump that replaces the source tree and discards uncommitted changes";

/// `--mode full` не отображается в `--force`: молчаливое отображение дало бы согласие на
/// уничтожение, о котором не просили. Отказ называет `pull --force` и говорит, что он делает.
#[test]
fn mode_full_is_refused_and_names_pull_force() {
    let project = project();
    let gone = "`--mode full` is gone: use `pull --force`";
    assert_refused(&project, &["pull", "--mode", "full"], &[gone, FORCE_MEANS]);
    assert_refused(
        &project,
        &["dump", "--mode", "full", "--dry-run"],
        &[gone, FORCE_MEANS],
    );
    assert_refused(
        &project,
        &["pull", "main", "--mode", "full", "--force"],
        &[gone, FORCE_MEANS],
    );
}

/// Прежний режим, который спорит с `--force`, не превращается молча в замену каталога:
/// отказ до платформы называет оба выхода (решение владельца 05.10.2026, #191).
#[test]
fn a_mode_that_contradicts_force_is_refused_with_the_choice() {
    let project = project();
    for mode in ["incremental", "partial"] {
        assert_refused(
            &project,
            &["pull", "--mode", mode, "--force", "--dry-run"],
            &[
                &format!("`--mode {mode}` contradicts `--force`"),
                "drop `--mode`",
                "drop `--force`",
                FORCE_MEANS,
            ],
        );
    }
    assert_refused(
        &project,
        &["pull", "--object", "Catalog:Items", "--force", "--dry-run"],
        &[
            "`--object` contradicts `--force`",
            "keep `--object`",
            FORCE_MEANS,
        ],
    );
    assert_refused(
        &project,
        &[
            "pull",
            "--mode",
            "partial",
            "--object",
            "Catalog:Items",
            "--force",
        ],
        &["`--mode partial` contradicts `--force`"],
    );
}

/// Справка одна объясняет последствия: без ключей — инкрементальная выгрузка поверх
/// каталога, `--force` — полная замена с потерей незафиксированного.
#[test]
fn pull_help_says_what_each_form_does() {
    for flag in ["-h", "--help"] {
        let output = v8_runner_command()
            .args(["pull", flag])
            .output()
            .expect("run help");
        assert!(output.status.success());
        let help = String::from_utf8_lossy(&output.stdout);
        for phrase in [
            "Without keys: incremental dump",
            "With --force: full dump that replaces the source tree",
            "uncommitted changes and untracked files there are discarded",
            "cannot be combined with --force",
        ] {
            assert!(help.contains(phrase), "{flag} must say {phrase:?}:\n{help}");
        }
    }
}
