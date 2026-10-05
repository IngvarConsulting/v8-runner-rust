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
use support::{previews, temp_workspace, v8_runner_command, write_shell_script};

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

// Состав таблицы листьев, общий с `src/cli/global_flags.rs`.
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/cli/global_flags_expected.in"
));

/// Превью каждой команды отвечает той же формой, что и настоящий прогон, поэтому формы
/// проверяются на нём: планировщик уже собрал ответ, а платформа ещё не нужна. Значения
/// при этом свои: `check`, например, называет `status: planned` — исхода, которого не
/// было, оно не выдумывает.
///
/// Перечень выводится из `LEAVES_WITH_PREVIEW`: прогоняется каждый лист оттуда, вызовом из
/// общей таблицы `support::previews`. Лист без строки в таблице роняет проверку, а не
/// выпадает из неё молча.
#[test]
fn every_previewable_command_answers_in_the_form_declared_for_it() {
    let dir = temp_workspace();
    previews::write_project(dir.path(), true);
    let rows = previews::with_preview(dir.path());

    // Сверка — только с формами самой команды: общая форма отказа тоже объявлена, и отказ до
    // диспетчеризации иначе прошёл бы за форму команды. Отказ, напечатанный формой самой
    // команды, проверку проходит: форма у него та же.
    for leaf in LEAVES_WITH_PREVIEW {
        let previewed = rows
            .iter()
            .find(|row| row.leaf == *leaf)
            .unwrap_or_else(|| panic!("`{leaf}` has a preview but no invocation to check"));
        let mut arguments = previewed.arguments.clone();
        arguments.push("--dry-run".to_owned());
        let (_code, payload) = previews::run(dir.path(), &arguments);
        let context = format!("`{}` (leaf `{leaf}`)", arguments.join(" "));
        // Имя команды сверяется до формы: ответ под чужим именем прошёл бы сверку с формой
        // той, чужой команды.
        assert_eq!(
            payload["command"], previewed.command,
            "{context}: {payload}"
        );
        assert_data_matches_its_command_form(&payload, &context);
    }

    // `version` превью не имеет и ничего не запускает: его ответ сверяется прямым вызовом
    // на своём образце.
    let dir = temp_workspace();
    let config_path = write_project(dir.path());
    let payload = run(&config_path, &["version"]);
    assert_eq!(payload["command"], "version", "{payload}");
    assert_data_matches_its_command_form(&payload, "`version`");
}

/// Половина сверки, которой не хватало проверке выше: перечень превью назывался руками и
/// держал шестнадцать вызовов при двадцати трёх листьях (#268). Лист, получивший превью,
/// в перечень не попадал, и сверка форм оставалась зелёной, ничего о нём не сказав.
#[test]
fn every_leaf_with_a_preview_is_checked_against_its_form() {
    let mut checked: Vec<&str> = previews::with_preview(Path::new("."))
        .into_iter()
        .map(|row| row.leaf)
        .collect();
    checked.sort_unstable();
    let mut named: Vec<&str> = LEAVES_WITH_PREVIEW.to_vec();
    named.sort_unstable();

    assert_eq!(checked, named);
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
