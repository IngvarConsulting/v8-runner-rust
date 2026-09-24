//! Форма поля `data` закреплена по команде схемами в `docs/schemas/command-data/`.
//!
//! Конверт описывает оболочку ответа и про `data` не говорит ничего. Предмет команды
//! лежит именно там, поэтому без этой проверки самая большая часть ответа не удерживается
//! ничем: поле можно переименовать или убрать, и падать будет у потребителя.
//!
//! Проверка гоняет живые команды и сверяет их `data` с формой той команды, которую они
//! назвали в конверте. Списки полей в схемах закрыты, так что новое поле — тоже слом.
//!
//! Харнесс подкладывает shell-скрипты вместо утилит платформы, поэтому файл целиком
//! собирается только под unix — как и остальные тесты, использующие тот же `support`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::command_data::{
    assert_data_matches_its_command_form, assert_data_matches_one_of, form_index, repo_root,
    slug_list,
};
use support::{temp_workspace, v8_runner_command, write_shell_script};

fn write_project(dir: &Path) -> PathBuf {
    let base_path = dir.join("project");
    let work_path = dir.join("work");
    let install_dir = dir.join("platform");
    fs::create_dir_all(base_path.join("configuration")).expect("configuration dir");
    fs::write(
        base_path.join("configuration").join("Configuration.xml"),
        "<MetaDataObject/>",
    )
    .expect("configuration marker");
    fs::create_dir_all(&work_path).expect("work dir");
    write_shell_script(&install_dir.join("bin").join("1cv8"), "exit 0");
    write_shell_script(&install_dir.join("bin").join("ibcmd"), "exit 0");

    let config_path = dir.join("v8project.yaml");
    fs::write(
        &config_path,
        format!(
            "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: 'File={}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {}\n",
            work_path.display(),
            dir.join("ib").display(),
            install_dir.display()
        ),
    )
    .expect("write config");
    config_path
}

fn run(config_path: &Path, arguments: &[&str]) -> Value {
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
        ])
        .args(arguments)
        .output()
        .expect("run command");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "`{}` printed no json envelope: {error}\nstdout: {}\nstderr: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// Превью каждой команды отвечает той же формой, что и настоящий прогон, поэтому формы
/// проверяются на нём: планировщик уже собрал ответ, а платформа ещё не нужна. Значения
/// при этом свои: `check`, например, называет `status: planned` — исхода, которого не
/// было, оно не выдумывает.
#[test]
fn every_previewable_command_answers_in_the_form_declared_for_it() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path());
    let artifact = dir.path().join("main.cf");
    let snapshot = dir.path().join("main.dt");
    let artifact_argument = artifact.display().to_string();
    let snapshot_argument = snapshot.display().to_string();

    let previews: Vec<Vec<&str>> = vec![
        vec!["version"],
        vec!["build", "--dry-run"],
        vec!["dump", "--mode", "full", "--dry-run"],
        vec!["convert", "--dry-run"],
        vec!["make", "--output", &artifact_argument, "--dry-run"],
        vec!["load", "--path", &artifact_argument, "--dry-run"],
        vec!["infobase", "create", "--dry-run"],
        vec!["launch", "designer", "--dry-run"],
        vec!["extensions", "list", "--dry-run"],
        vec![
            "extensions",
            "create",
            "--name",
            "Demo",
            "--name-prefix",
            "Demo",
            "--dry-run",
        ],
        vec!["syntax", "designer-config", "--thin-client"],
        vec!["check", "--dry-run"],
        vec![
            "infobase",
            "configuration",
            "export",
            "--state",
            "working",
            "--output",
            &artifact_argument,
            "--dry-run",
        ],
        vec![
            "infobase",
            "dump",
            "--output",
            &snapshot_argument,
            "--dry-run",
        ],
        vec![
            "infobase",
            "restore",
            "--input",
            &snapshot_argument,
            "--dry-run",
        ],
    ];

    // Сверка — только с формами самой команды: общая форма отказа тоже объявлена, и отказ до
    // диспетчеризации иначе прошёл бы за форму команды. Отказ, напечатанный формой самой
    // команды, проверку проходит: форма у него та же.
    for preview in previews {
        let payload = run(&config_path, &preview);
        assert_data_matches_its_command_form(&payload, &format!("`{}`", preview.join(" ")));
    }

    // `clone` проектного файла не читает и глобальный ключ настроек отвергает, поэтому
    // идёт своим вызовом. Перечень выше остаётся названным руками (#268) — эта строка
    // закрывает не перечень, а конкретную дыру.
    let cloned = dir.path().join("cloned");
    let output = v8_runner_command()
        .args([
            "--json-message",
            "clone",
            "--project-dir",
            &cloned.display().to_string(),
            "--connection",
            &format!("File={}", dir.path().join("ib").display()),
            "--platform-version",
            "8.3.27",
            "--platform-path",
            &dir.path().join("platform").display().to_string(),
            "--dry-run",
        ])
        .output()
        .expect("run command");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true, "`clone --dry-run` refused: {payload}");
    assert_data_matches_its_command_form(&payload, "`clone --dry-run`");
}

/// Отказ до диспетчеризации печатает общую форму, и она тоже часть обещания: клиент
/// разбирает `data` до того, как узнал, отказали ему или нет.
#[test]
fn a_refusal_before_dispatch_answers_in_the_shared_form() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path());

    let payload = run(&config_path, &["extensions", "info", "--name", ""]);
    assert_eq!(payload["ok"], false, "an empty name must be refused");
    assert_data_matches_one_of(&payload["data"], "a refusal before dispatch", &["refusal"]);
}

/// Форма объявлена для каждой команды, которая её печатает. Без этой проверки новая
/// команда молча отвечала бы `data`, который никем не закреплён.
#[test]
fn every_declared_form_has_its_artefact_on_disk() {
    let index = form_index();
    for slug in slug_list(&index["shared"]) {
        let path = repo_root().join(format!("docs/schemas/command-data/{slug}.schema.json"));
        assert!(path.is_file(), "shared form {slug} has no artefact");
    }
    let forms = index["forms"].as_object().expect("forms map");
    assert!(!forms.is_empty(), "the index declares no form at all");
    for (command, slugs) in forms {
        let slugs = slugs.as_array().expect("slugs array");
        assert!(!slugs.is_empty(), "command `{command}` declares no form");
        for slug in slugs {
            let slug = slug.as_str().expect("slug is a string");
            let path = repo_root().join(format!("docs/schemas/command-data/{slug}.schema.json"));
            assert!(path.is_file(), "form {slug} of `{command}` has no artefact");
        }
    }
}
