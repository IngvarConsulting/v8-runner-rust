//! `provider_dispatched` говорит, получил ли исполнитель работу команды
//! (`INV.WIRE.PROVIDER-DISPATCHED-SAYS-WHETHER-AN-EXECUTOR-GOT-WORK`).
//!
//! Проверка парная и боевая, без превью. На одном образце исполнители — заглушки, которые
//! отмечают вызов в журнале и выходят с нулём, и признак обязан совпасть с журналом: `true`,
//! если заглушку запустили, `false`, если работы не было. Исход команды не проверяется —
//! заглушка ничего не производит, и многие команды после неё падают, но работу исполнитель
//! уже получил. На другом образце исполнителей нет вовсе, и `true` не отвечает ни одна
//! строка.
//!
//! Строки сверяются со схемами форм: форма, чья схема объявляет признак, обязана получить
//! здесь строку, в которой исполнитель работу получает. Вне таблицы остаются вызовы, которые
//! на заглушке проверить нельзя: `launch web` открывает адрес системной программой, а не
//! утилитой платформы; `check edt` требует проекта в формате EDT; `extensions info` после
//! работы отвечает общей формой отказа, без признака (#314).
//!
//! Харнесс подкладывает shell-скрипты вместо утилит платформы, поэтому файл целиком
//! собирается только под unix — как и остальные тесты, использующие тот же `support`.
#![cfg(unix)]

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use support::command_data::{assert_data_matches_one_of, form_index, form_schema, slug_list};
use support::{temp_workspace, v8_runner_command, wait_for_file, write_shell_script};

/// Утилиты, которые заглушает образец с исполнителями.
const EXECUTORS: &[&str] = &["1cv8", "ibcmd", "1cedtcli", "webinst"];

/// Журнал, куда заглушка пишет своё имя, когда её запускают.
fn calls(dir: &Path) -> PathBuf {
    dir.join("calls.log")
}

/// Каталог, который дочерний раннер получает вместо `PATH` на образце без исполнителей:
/// кандидаты из `PATH` локатор берёт без сверки версии, и EDT, установленная на машине,
/// нашлась бы там.
fn no_tools(dir: &Path) -> PathBuf {
    dir.join("no-tools")
}

/// Образец: проект в формате конфигуратора с расширением инструмента, файловая база, у
/// которой есть файл данных, веб-публикация и артефакты для `load` и `infobase restore`.
///
/// С исполнителями платформа лежит в каталоге образца заглушками. Тонкий клиент не выходит
/// сразу: `launch` считает клиент, вышедший до конца пробы старта, не стартовавшим.
/// Без исполнителей каталог платформы пуст, поиск строгий с версией — утилиты за его
/// пределами он не ищет. EDT CLI строгого поиска не знает, поэтому ей назначена версия,
/// которой нет ни у одной установки: кандидаты из корней по умолчанию сверяются с ней.
fn write_sample(dir: &Path, with_executors: bool) {
    let project = dir.join("project");
    let extension = project.join("exts").join("client-mcp");
    fs::create_dir_all(project.join("configuration")).expect("configuration dir");
    fs::write(
        project.join("configuration").join("Configuration.xml"),
        "<MetaDataObject/>",
    )
    .expect("configuration marker");
    fs::create_dir_all(&extension).expect("extension dir");
    fs::write(
        extension.join("Configuration.xml"),
        "<Configuration><Properties><Name>client_mcp</Name><ConfigurationExtensionPurpose kind=\"Customization\">Customization</ConfigurationExtensionPurpose></Properties></Configuration>",
    )
    .expect("extension marker");
    fs::write(
        extension.join("Module.bsl"),
        "procedure Tool() endprocedure",
    )
    .expect("extension module");
    fs::create_dir_all(dir.join("ib")).expect("infobase dir");
    fs::write(dir.join("ib").join("1Cv8.1CD"), "db").expect("infobase data file");
    fs::write(dir.join("main.cf"), "cf").expect("configuration artifact");
    fs::write(dir.join("main.dt"), "dt").expect("infobase snapshot");
    fs::create_dir_all(dir.join("web")).expect("web dir");
    fs::create_dir_all(dir.join("work")).expect("work dir");
    fs::create_dir_all(no_tools(dir)).expect("empty PATH dir");

    let bin = dir.join("platform").join("bin");
    fs::create_dir_all(&bin).expect("platform dir");
    let journal = calls(dir);
    let strictness = if with_executors {
        for executor in EXECUTORS {
            write_shell_script(
                &bin.join(executor),
                &format!(
                    "printf '%s\\n' {executor} >> '{}'\nexit 0",
                    journal.display()
                ),
            );
        }
        write_shell_script(
            &bin.join("1cv8c"),
            &format!(
                "printf '%s\\n' 1cv8c >> '{}'\nexec sleep 5",
                journal.display()
            ),
        );
        ""
    } else {
        "    strict: true\n    version: '8.3.27'\n"
    };

    fs::write(
        dir.join("v8project.yaml"),
        format!(
            "workPath: {work}\nformat: DESIGNER\ninfobase:\n  connection: 'File={ib}'\n  web:\n    server: apache24\n    wsdir: demo\n    dir: '{web}'\n    url: http://localhost/demo\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {platform}\n{strictness}  edt_cli:\n    path: {edt}\n    version: '1999.9.9'\n    interactive-mode: false\n  client_mcp:\n    extension:\n      name: client_mcp\n      source:\n        path: {extension}\n",
            work = dir.join("work").display(),
            ib = dir.join("ib").display(),
            web = dir.join("web").display(),
            platform = dir.join("platform").display(),
            edt = bin.join("1cedtcli").display(),
            extension = extension.display(),
        ),
    )
    .expect("write config");
}

/// Строка таблицы: боевой вызов, форма его ответа и получает ли исполнитель работу на
/// образце с заглушками.
struct Dispatching {
    arguments: Vec<String>,
    form: &'static str,
    work: bool,
}

fn given(arguments: &[&str], form: &'static str) -> Dispatching {
    Dispatching {
        arguments: arguments.iter().map(|value| (*value).to_owned()).collect(),
        form,
        work: true,
    }
}

fn idle(arguments: &[&str], form: &'static str) -> Dispatching {
    Dispatching {
        work: false,
        ..given(arguments, form)
    }
}

fn rows(dir: &Path) -> Vec<Dispatching> {
    let path = |name: &str| dir.join(name).display().to_string();
    let (cloned, origin, platform) = (
        path("cloned"),
        format!("File={}", path("ib")),
        path("platform"),
    );
    let fresh = format!("--infobase=File={}", path("fresh"));
    let (made, artifact, exported) = (path("made.cf"), path("main.cf"), path("exported.cf"));
    let (snapshot, dumped) = (path("main.dt"), path("dumped.dt"));
    vec![
        given(&["check"], "check"),
        // `clone` проектного файла не читает: адрес, версию и платформу он называет ключами.
        given(
            &[
                "clone",
                "--project-dir",
                &cloned,
                "--connection",
                &origin,
                "--platform-version",
                "8.3.27",
                "--platform-path",
                &platform,
            ],
            "clone",
        ),
        given(&["convert"], "convert"),
        given(&["extensions", "list"], "extensions-inventory"),
        // Изменение состава отвечает формой изменения, как и настройка свойств.
        given(
            &[
                "extensions",
                "create",
                "--name",
                "Demo",
                "--name-prefix",
                "Demo",
            ],
            "extensions",
        ),
        given(
            &["extensions", "delete", "--name", "client_mcp"],
            "extensions",
        ),
        given(
            &[
                "extensions",
                "activate",
                "--name",
                "client_mcp",
                "--active",
                "yes",
            ],
            "extensions",
        ),
        given(&["extensions", "--installed-name", "Demo"], "extensions"),
        // Без цели работы нет: ни процесса, ни сессии.
        idle(&["extensions"], "extensions"),
        given(&["infobase", "create", &fresh], "infobase-create"),
        // База образца уже заведена, создавать нечего.
        idle(&["infobase", "create"], "infobase-create"),
        given(&["launch", "thin"], "launch"),
        given(&["make", "--output", &made], "make"),
        given(&["publish"], "publish"),
        given(&["dump", "--mode", "full"], "pull"),
        given(&["build"], "push"),
        given(&["load", "--path", &artifact], "upload"),
        // Формы выгрузки несут признак только в превью: боевой ответ его не называет,
        // хотя работу исполнитель получил.
        given(
            &["download", "--state", "working", "--output", &exported],
            "download",
        ),
        given(&["infobase", "dump", "--output", &dumped], "infobase-dump"),
        given(
            &["infobase", "restore", "--input", &snapshot, "--replace"],
            "infobase-restore",
        ),
    ]
}

/// Формы, чья схема объявляет признак, и обязателен ли он в них. Необязателен он у форм
/// выгрузки: боевой ответ его не несёт.
fn forms_carrying_the_flag() -> BTreeMap<String, bool> {
    let index = form_index();
    let mut forms = BTreeMap::new();
    for slugs in index["forms"].as_object().expect("forms map").values() {
        for slug in slug_list(slugs) {
            let schema = form_schema(&slug);
            let declared = schema["properties"].get("provider_dispatched").is_some();
            // Признак, объявленный глубже корня, эта таблица не увидела бы и промолчала.
            assert_eq!(
                declared,
                schema.to_string().contains("\"provider_dispatched\""),
                "{slug} declares provider_dispatched outside its root properties"
            );
            if declared {
                let required = schema["required"]
                    .as_array()
                    .is_some_and(|names| names.iter().any(|name| name == "provider_dispatched"));
                forms.insert(slug, required);
            }
        }
    }
    forms
}

/// Настройки берутся из каталога образца, а не глобальным ключом: `clone` его отвергает.
/// `V8TR_CONFIG` снимается вместе с ключом — чужое окружение увело бы образец в другой
/// проект.
fn run(dir: &Path, arguments: &[String], path: &str) -> Value {
    let output = v8_runner_command()
        .current_dir(dir)
        .env_remove("V8TR_CONFIG")
        .env("PATH", path)
        .arg("--json-message")
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

/// Каждая форма, чья схема объявляет признак, получает строку, где исполнитель работу
/// получает, а строка не называет формы без признака: новая форма с признаком без строки
/// здесь ломает проверку.
#[test]
fn every_form_carrying_the_flag_has_a_row_with_work() {
    let declared: BTreeSet<String> = forms_carrying_the_flag().into_keys().collect();
    let named: BTreeSet<String> = rows(Path::new("."))
        .into_iter()
        .map(|row| row.form.to_owned())
        .collect();
    let with_work: BTreeSet<String> = rows(Path::new("."))
        .into_iter()
        .filter(|row| row.work)
        .map(|row| row.form.to_owned())
        .collect();

    assert_eq!(named, declared);
    assert_eq!(with_work, declared);
}

/// Признак совпадает с журналом заглушек: `true` — заглушку запустили, `false` — нет.
/// Формы выгрузки после работы признака не называют вовсе.
#[test]
fn the_flag_says_whether_a_stub_executor_ran() {
    let forms = forms_carrying_the_flag();
    for index in 0..rows(Path::new(".")).len() {
        // Каждая строка — на свежем образце: состояние, записанное одной командой, лишило
        // бы работы следующую.
        let dir = temp_workspace();
        write_sample(dir.path(), true);
        let row = rows(dir.path()).swap_remove(index);
        let payload = run(dir.path(), &row.arguments, "/usr/bin:/bin");
        let context = format!("`{}`", row.arguments.join(" "));

        assert_data_matches_one_of(&payload["data"], &context, &[row.form]);
        // Клиент `launch` пишет журнал сам, уже после ответа раннера, поэтому запуск
        // заглушки ждётся. Отсутствие вызова ждать нечем: синхронная команда его бы уже
        // записала.
        let ran = if row.work {
            wait_for_file(&calls(dir.path()), Duration::from_secs(10))
        } else {
            calls(dir.path()).exists()
        };
        assert_eq!(
            ran, row.work,
            "{context}: the stub executor ran: {ran}; {payload}"
        );
        let flag = &payload["data"]["provider_dispatched"];
        if forms[row.form] {
            assert_eq!(flag, row.work, "{context}: {payload}");
        } else {
            assert!(
                flag.is_null(),
                "{context}: an apply answer named the flag: {payload}"
            );
        }
    }
}

/// Без исполнителей `true` не отвечает ни одна строка. Строка с работой обязана дойти до
/// поиска исполнителя и не найти его: отказ по другой причине прошёл бы проверку, ничего не
/// сказав о признаке.
#[test]
fn no_form_says_an_executor_got_work_when_there_is_none() {
    for index in 0..rows(Path::new(".")).len() {
        let dir = temp_workspace();
        write_sample(dir.path(), false);
        let row = rows(dir.path()).swap_remove(index);
        let path = no_tools(dir.path()).display().to_string();
        let payload = run(dir.path(), &row.arguments, &path);
        let context = format!("`{}`", row.arguments.join(" "));

        assert_ne!(
            payload["data"]["provider_dispatched"], true,
            "{context} claimed work without an executor: {payload}"
        );
        if row.work {
            let message = payload["error"]["message"].as_str().unwrap_or_default();
            assert!(
                message.contains("was not found"),
                "{context} did not fail for the missing executor: {payload}"
            );
        }
    }
}
