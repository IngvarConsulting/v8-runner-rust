#![cfg(unix)]

//! Позиционный аргумент `push`, `pull`, `make`, `download` и `convert` — набор исходников.
//! Базу называет `--infobase`; значение, которое набором не является, отвергается до
//! запуска платформы, даже если так зовут объявленную базу или это строка соединения.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
    calls: PathBuf,
    out: PathBuf,
}

/// Проект формата платформы: набор конфигурации `main` и набор расширения `sales`; местный
/// слой объявляет базы `origin` и `test`. Поддельная платформа только записывает вызов.
fn project() -> Project {
    let dir = temp_workspace();
    let root = dir.path().join("project");
    let work = dir.path().join("work");
    let calls = dir.path().join("calls.log");
    let platform = dir.path().join("1cv8");
    for set in ["main", "sales"] {
        fs::create_dir_all(root.join(set)).expect("source set");
    }
    fs::create_dir_all(&work).expect("work");
    // Файловая база `origin` готова: исполнитель выгрузки проверяет её до плана.
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
            "workPath: '{}'\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\n  - name: sales\n    type: EXTENSION\n    path: sales\ntools:\n  platform:\n    path: '{}'\n",
            work.display(),
            platform.display(),
        ),
    )
    .expect("project file");
    fs::write(
        root.join("v8project.local.yaml"),
        format!(
            "infobases:\n  origin:\n    connection: 'File={}'\n  test:\n    connection: 'File={}'\n",
            infobase.display(),
            dir.path().join("ib-test").display(),
        ),
    )
    .expect("local layer");
    let out = dir.path().join("out");
    Project {
        _dir: dir,
        root,
        calls,
        out,
    }
}

fn run(project: &Project, args: &[&str]) -> Output {
    v8_runner_command()
        .current_dir(&project.root)
        .arg("--json-message")
        .args(args)
        .output()
        .expect("run command")
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

/// Команды словаря с позиционным набором; `value` стоит на месте набора.
fn commands_with_a_set<'a>(value: &'a str, cf: &'a str) -> Vec<Vec<&'a str>> {
    vec![
        vec!["push", value],
        vec!["pull", value, "--mode", "full"],
        vec!["make", value, "--output", cf],
        vec!["download", value, "--output", cf],
        vec!["convert", value],
    ]
}

#[test]
fn a_positional_argument_names_a_source_set_and_never_a_base() {
    let project = project();
    let cf = project.out.join("main.cf").display().to_string();
    // Имя объявленной базы и строка соединения: ни то, ни другое набором не является.
    for value in ["test", "File=/tmp/another-ib"] {
        for args in commands_with_a_set(value, &cf) {
            let output = run(&project, &args);
            assert_eq!(output.status.code(), Some(2), "{args:?}");
            let envelope = envelope(&output);
            assert_eq!(envelope["ok"], false, "{args:?}: {envelope}");
            assert_eq!(
                envelope["error"]["kind"], "validation",
                "{args:?}: {envelope}"
            );
            let message = envelope["error"]["message"].as_str().unwrap_or_default();
            assert!(
                message.contains(&format!("unknown source-set '{value}'")),
                "{args:?}: {envelope}"
            );
            assert!(
                !project.calls.exists(),
                "{args:?}: the platform must not be started"
            );
        }
    }
}

#[test]
fn a_positional_source_set_selects_that_set() {
    let project = project();
    let cfe = project.out.join("sales.cfe").display().to_string();

    let push = envelope(&run(&project, &["push", "sales", "--dry-run"]));
    assert_eq!(push["ok"], true, "{push}");
    let planned = push["data"]["steps"]
        .as_array()
        .unwrap_or_else(|| panic!("planned steps: {push}"))
        .iter()
        .map(|step| step["source_set"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(planned, ["sales"], "{push}");

    let pull = envelope(&run(
        &project,
        &["pull", "sales", "--mode", "full", "--dry-run"],
    ));
    assert_eq!(pull["ok"], true, "{pull}");
    assert_eq!(pull["data"]["source_set"], "sales", "{pull}");
    assert_eq!(pull["data"]["extension"], "sales", "{pull}");

    let download = envelope(&run(
        &project,
        &["download", "sales", "--output", &cfe, "--dry-run"],
    ));
    assert_eq!(download["ok"], true, "{download}");
    assert_eq!(
        download["data"]["subject"]["kind"], "extension",
        "{download}"
    );
    assert_eq!(download["data"]["subject"]["name"], "sales", "{download}");

    let make = envelope(&run(
        &project,
        &["make", "sales", "--output", &cfe, "--dry-run"],
    ));
    assert_eq!(make["ok"], true, "{make}");
    assert_eq!(make["data"]["source_set"], "sales", "{make}");

    assert!(!project.calls.exists(), "a preview starts no platform");
}

/// `download` без ключа состояния берёт основную конфигурацию, `--state db` — конфигурацию
/// базы данных; на проводе состояние называется как прежде.
#[test]
fn download_state_db_takes_the_database_configuration() {
    let project = project();
    let cf = project.out.join("main.cf").display().to_string();

    let preview = envelope(&run(
        &project,
        &["download", "--state", "db", "--output", &cf, "--dry-run"],
    ));
    assert_eq!(preview["ok"], true, "{preview}");
    assert_eq!(preview["data"]["state"], "database", "{preview}");
    assert_eq!(preview["data"]["subject"]["kind"], "main", "{preview}");

    let working = envelope(&run(&project, &["download", "--output", &cf, "--dry-run"]));
    assert_eq!(working["ok"], true, "{working}");
    assert_eq!(working["data"]["state"], "working", "{working}");
}

fn calls(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// Набор, который не является конфигурацией или расширением, `download` не берёт: отказ
/// до платформы, а не догадка о предмете.
#[test]
fn download_refuses_a_set_of_external_files() {
    let project = project();
    let config = project.root.join("v8project.yaml");
    let text = fs::read_to_string(&config).expect("project file");
    fs::create_dir_all(project.root.join("epf")).expect("epf");
    fs::write(
        &config,
        text.replace(
            "tools:",
            "  - name: tools\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\ntools:",
        ),
    )
    .expect("project file");
    let cf = project.out.join("main.cf").display().to_string();

    let output = run(&project, &["download", "tools", "--output", &cf]);
    assert_eq!(output.status.code(), Some(2));
    let envelope = envelope(&output);
    assert_eq!(envelope["error"]["kind"], "validation", "{envelope}");
    assert!(
        envelope["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("source-set 'tools' holds external files")),
        "{envelope}"
    );
    assert!(
        calls(&project.calls).is_empty(),
        "the platform must not be started"
    );
}

/// Прежние значения `--state working` и `--state database` принимаются один цикл, но
/// справка их не печатает: в ней только словарь сайта.
#[test]
fn download_accepts_the_hidden_state_values_and_help_hides_them() {
    let project = project();
    let cf = project.out.join("main.cf").display().to_string();

    for value in ["working", "database"] {
        let preview = envelope(&run(
            &project,
            &["download", "--state", value, "--output", &cf, "--dry-run"],
        ));
        assert_eq!(preview["ok"], true, "--state {value}: {preview}");
        assert_eq!(
            preview["data"]["state"], value,
            "--state {value}: {preview}"
        );
    }

    let help = v8_runner_command()
        .args(["download", "--help"])
        .output()
        .expect("run help");
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    // Описание ключа вправе говорить о состояниях словами; скрыты значения в списке.
    let possible_values = help
        .lines()
        .find(|line| line.contains("--state <STATE>"))
        .and_then(|line| line.split_once("[possible values:"))
        .and_then(|(_, values)| values.split_once(']'))
        .map(|(values, _)| values.trim())
        .unwrap_or_else(|| panic!("--state lists its values:\n{help}"));
    assert_eq!(possible_values, "db", "{help}");
}

/// Набор расширения требует `.cfe`: файл `.cf` — отказ до платформы, а не выгрузка
/// основной конфигурации.
#[test]
fn download_of_an_extension_set_into_a_cf_is_refused_before_the_platform() {
    let project = project();
    let cf = project.out.join("sales.cf").display().to_string();

    let output = run(&project, &["download", "sales", "--output", &cf]);
    assert_eq!(output.status.code(), Some(2));
    let envelope = envelope(&output);
    assert_eq!(envelope["error"]["kind"], "validation", "{envelope}");
    let message = envelope["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("must have .cfe suffix"), "{envelope}");
    assert_eq!(
        envelope["data"]["subject"]["kind"], "extension",
        "{envelope}"
    );
    assert_eq!(envelope["data"]["subject"]["name"], "sales", "{envelope}");
    assert!(
        calls(&project.calls).is_empty(),
        "the platform must not be started"
    );
}

/// Настройки не загрузились — набор не разрешён. Ответ не называет предметом основную
/// конфигурацию, когда запрос просил пакет расширения: предмет следует суффиксу, который
/// набор обязан подтвердить.
#[test]
fn download_of_a_set_names_no_false_subject_when_the_settings_fail_to_load() {
    let project = project();
    fs::write(project.root.join("v8project.yaml"), "source-set: [").expect("broken project");
    let cfe = project.out.join("sales.cfe").display().to_string();
    let cf = project.out.join("main.cf").display().to_string();

    let extension = envelope(&run(&project, &["download", "sales", "--output", &cfe]));
    assert_eq!(extension["ok"], false, "{extension}");
    assert_eq!(
        extension["data"]["subject"]["kind"], "extension",
        "{extension}"
    );
    assert_eq!(extension["data"]["subject"]["name"], "sales", "{extension}");
    assert_eq!(extension["data"]["artifact_kind"], "cfe", "{extension}");

    let main = envelope(&run(&project, &["download", "main", "--output", &cf]));
    assert_eq!(main["ok"], false, "{main}");
    assert_eq!(main["data"]["subject"]["kind"], "main", "{main}");
    assert_eq!(main["data"]["artifact_kind"], "cf", "{main}");
    assert!(
        calls(&project.calls).is_empty(),
        "the platform must not be started"
    );
}
