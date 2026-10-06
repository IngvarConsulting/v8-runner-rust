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

/// Входы, без которых превью отказывает до плана: файл конфигурации для `upload`, снимок
/// базы для `infobase restore` и файл файловой базы для `download` и `infobase dump`,
/// которые до плана проверяют, что база на месте. Содержимое не читается: превью
/// смотрит только, что файл есть.
fn write_preview_inputs(dir: &Path) {
    fs::write(dir.join("main.cf"), "cf").expect("configuration artifact");
    fs::write(dir.join("main.dt"), "dt").expect("infobase snapshot");
    fs::create_dir_all(dir.join("ib")).expect("infobase dir");
    fs::write(dir.join("ib").join("1Cv8.1CD"), "1cd").expect("file infobase");
}

/// Раскладка проекта EDT в `source`: без неё набор исходников формата EDT не проходит
/// проверку настроек.
fn write_edt_layout(source: &Path, name: &str, nature: &str) {
    fs::create_dir_all(source.join("DT-INF")).expect("dt-inf");
    fs::create_dir_all(source.join("src").join("Configuration")).expect("src");
    fs::write(
        source.join(".project"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>{name}</name>\n  <natures>\n    <nature>com._1c.g5.v8.dt.core.{nature}</nature>\n  </natures>\n</projectDescription>\n"
        ),
    )
    .expect("edt project descriptor");
    fs::write(
        source.join("DT-INF").join("PROJECT.PMF"),
        "Manifest-Version: 1.0\nRuntime-Version: 8.3.27\n",
    )
    .expect("edt manifest");
    fs::write(
        source
            .join("src")
            .join("Configuration")
            .join("Configuration.mdo"),
        "<Configuration />\n",
    )
    .expect("edt configuration");
}

/// Образец формата EDT для `check edt`: проверка EDT в проекте Конфигуратора отказывает до
/// плана. Остальное — общий образец, включая поддельный EDT CLI; конфигурация и расширение
/// инструмента получают раскладку проекта EDT.
fn write_edt_project(dir: &Path) {
    let config_path = previews::write_project(dir, true);
    let project = dir.join("project");
    write_edt_layout(
        &project.join("configuration"),
        "main",
        "V8ConfigurationNature",
    );
    write_edt_layout(
        &project.join("exts").join("client-mcp"),
        "client_mcp",
        "V8ExtensionNature",
    );
    let config = fs::read_to_string(&config_path).expect("read config");
    assert!(config.contains("format: DESIGNER\n"), "{config}");
    fs::write(
        &config_path,
        config.replace("format: DESIGNER\n", "format: EDT\n"),
    )
    .expect("write edt config");
}

/// Листья, чьё превью на общем образце Конфигуратора не доходит до плана по природе
/// проекта, а не по нехватке входов.
const NEEDS_THE_EDT_SAMPLE: &[&str] = &["check edt"];

/// Превью каждой команды отвечает той же формой, что и настоящий прогон, поэтому формы
/// проверяются на нём: планировщик уже собрал ответ, а платформа ещё не нужна. Значения
/// при этом свои: `check`, например, называет `status: planned` — исхода, которого не
/// было, оно не выдумывает.
///
/// Перечень выводится из `LEAVES_WITH_PREVIEW`: прогоняется каждый лист оттуда, вызовом из
/// общей таблицы `support::previews`. Лист без строки в таблице роняет проверку, а не
/// выпадает из неё молча.
///
/// Каждое превью обязано дойти до плана, `ok: true`: отказ, напечатанный формой самой
/// команды, сверку формы проходит, но ветку превью этой формы не проверяет (#304). Поэтому
/// образец даёт превью всё, что им нужно, — входные файлы, утилиты, ключи, — а отказ
/// любого листа роняет проверку с его именем.
///
/// Живой прогон `check` здесь не нужен: форму настоящего исхода держат
/// `tests/mcp_stdio.rs::mcp_stdio_tools_answer_in_the_forms_of_their_commands` и
/// `mcp_stdio_the_live_edt_check_answers_in_the_form_of_check`, а ответ прежнего имени
/// `syntax` под новым — `tests/cli_syntax.rs`.
#[test]
fn every_previewable_command_answers_in_the_form_declared_for_it() {
    let designer = temp_workspace();
    previews::write_project(designer.path(), true);
    // Превью `push` без памяти о базе называет отказ `no_memory`, а не план.
    support::memory::remember_sample(designer.path());
    write_preview_inputs(designer.path());
    let edt = temp_workspace();
    write_edt_project(edt.path());

    // Сверка — только с формами самой команды: общая форма отказа тоже объявлена, и отказ до
    // диспетчеризации иначе прошёл бы за форму команды.
    let mut refused = Vec::new();
    for leaf in LEAVES_WITH_PREVIEW {
        let sample = if NEEDS_THE_EDT_SAMPLE.contains(leaf) {
            edt.path()
        } else {
            designer.path()
        };
        let previewed = previews::with_preview(sample)
            .into_iter()
            .find(|row| row.leaf == *leaf)
            .unwrap_or_else(|| panic!("`{leaf}` has a preview but no invocation to check"));
        let mut arguments = previewed.arguments;
        arguments.push("--dry-run".to_owned());
        let (code, payload) = previews::run(sample, &arguments);
        let context = format!("`{}` (leaf `{leaf}`)", arguments.join(" "));
        // Имя команды сверяется до формы: ответ под чужим именем прошёл бы сверку с формой
        // той, чужой команды.
        assert_eq!(
            payload["command"], previewed.command,
            "{context}: {payload}"
        );
        // Отказ называется до сверки формы: отказ общей формой иначе уронил бы проверку
        // сообщением о форме, не сказав, что превью отказало. Отказ роняет проверку и так.
        if payload["ok"] != true || code != 0 {
            refused.push(format!(
                "{context} answered with a refusal (exit code {code}): {}",
                payload["error"]
            ));
            continue;
        }
        assert_data_matches_its_command_form(&payload, &context);
    }
    assert!(
        refused.is_empty(),
        "every preview must answer `ok: true`, these refused instead:\n{}",
        refused.join("\n")
    );

    // `version` превью не имеет и ничего не запускает: его ответ сверяется прямым вызовом
    // на своём образце.
    let dir = temp_workspace();
    let config_path = write_project(dir.path());
    let payload = run(&config_path, &["version"]);
    assert_eq!(payload["command"], "version", "{payload}");
    assert_data_matches_its_command_form(&payload, "`version`");
}

/// Половина сверки, которой не хватало проверке выше: перечень превью назывался руками и
/// держал не все листья (#268). Лист, получивший превью, в перечень не попадал, и сверка
/// форм оставалась зелёной, ничего о нём не сказав. Состав страхуется и в
/// `contract_previews::every_leaf_with_a_preview_is_exercised_here` — намеренно: таблица
/// общая, а сверки у неё разные.
#[test]
fn every_leaf_with_a_preview_is_checked_against_its_form() {
    let mut checked: Vec<&str> = previews::with_preview(Path::new("."))
        .into_iter()
        .map(|row| row.leaf)
        .collect();
    checked.sort_unstable();
    // Лист с несколькими формами ответа стоит в таблице несколькими строками.
    checked.dedup();
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
