//! Что обещает превью любой команды: ни одного следа в файловой системе и отказ до
//! одобрения плана, если платформы нет.
//!
//! Харнесс подкладывает shell-скрипты вместо утилит платформы, поэтому файл целиком
//! собирается только под unix — как и остальные тесты, использующие тот же `support`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

fn write_project(dir: &Path, with_platform: bool) -> PathBuf {
    let base_path = dir.join("project");
    let work_path = dir.join("work");
    let install_dir = dir.join("platform");
    let extension_source = base_path.join("exts").join("client-mcp");
    fs::create_dir_all(&extension_source).expect("extension dir");
    // Расширение инструмента объявлено намеренно: шаг его подготовки — место, где превью
    // сборки однажды запускало платформу и писало состояние (#252). Без него образец
    // этого класса не видит.
    fs::write(
        extension_source.join("Configuration.xml"),
        "<Configuration><Properties><Name>client_mcp</Name><ConfigurationExtensionPurpose kind=\"Customization\">Customization</ConfigurationExtensionPurpose></Properties></Configuration>",
    )
    .expect("extension marker");
    fs::write(
        extension_source.join("Module.bsl"),
        "procedure Tool() endprocedure",
    )
    .expect("extension module");
    fs::create_dir_all(base_path.join("configuration")).expect("configuration dir");
    fs::write(
        base_path.join("configuration").join("Configuration.xml"),
        "<MetaDataObject/>",
    )
    .expect("configuration marker");
    fs::create_dir_all(&work_path).expect("work dir");
    fs::create_dir_all(install_dir.join("bin")).expect("platform dir");
    if with_platform {
        write_shell_script(&install_dir.join("bin").join("1cv8"), "exit 0");
        write_shell_script(&install_dir.join("bin").join("ibcmd"), "exit 0");
    }

    // Без платформы поиск обязан отказать, а не уйти в PATH или в корни по умолчанию:
    // строгий режим с версией не даёт локатору найти что-то за пределами каталога.
    let strictness = if with_platform {
        ""
    } else {
        "    strict: true\n    version: '8.3.27'\n"
    };
    let config_path = dir.join("v8project.yaml");
    fs::write(
        &config_path,
        format!(
            "workPath: {}\nformat: DESIGNER\ninfobase:\n  connection: 'File={}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: {}\n{strictness}  client_mcp:\n    extension:\n      name: client_mcp\n      source:\n        path: {}\n",
            work_path.display(),
            dir.join("ib").display(),
            install_dir.display(),
            extension_source.display()
        ),
    )
    .expect("write config");
    config_path
}

fn run(config_path: &Path, arguments: &[&str]) -> (i32, Value) {
    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
        ])
        .args(arguments)
        .output()
        .expect("run command");
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "`{}` printed no json envelope: {error}\nstdout: {}\nstderr: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.code().unwrap_or(-1), payload)
}

// Состав таблицы листьев, общий с `src/cli/global_flags.rs`.
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/cli/global_flags_expected.in"
));

/// Каждый лист с превью: минимальный вызов и путь листа. Состав сверяется с общим
/// списком `LEAVES_WITH_PREVIEW`, поэтому новый лист с превью обязан появиться и здесь.
fn with_preview<'a>(artifact: &'a str, snapshot: &'a str) -> Vec<(Vec<&'a str>, &'static str)> {
    vec![
        (vec!["extensions"], "extensions"),
        (vec!["extensions", "list"], "extensions list"),
        (
            vec!["extensions", "info", "--name", "client_mcp"],
            "extensions info",
        ),
        (
            vec![
                "extensions",
                "create",
                "--name",
                "Demo",
                "--name-prefix",
                "Demo",
            ],
            "extensions create",
        ),
        (
            vec!["extensions", "delete", "--name", "client_mcp"],
            "extensions delete",
        ),
        (
            vec![
                "extensions",
                "activate",
                "--name",
                "client_mcp",
                "--active",
                "yes",
            ],
            "extensions activate",
        ),
        (vec!["build"], "push"),
        (vec!["load", "--path", artifact], "upload"),
        (vec!["dump", "--mode", "full"], "pull"),
        (
            vec!["download", "--state", "working", "--output", artifact],
            "download",
        ),
        (vec!["infobase", "create"], "infobase create"),
        (
            vec![
                "infobase",
                "configuration",
                "export",
                "--state",
                "working",
                "--output",
                artifact,
            ],
            "infobase configuration export",
        ),
        (
            vec!["infobase", "dump", "--output", snapshot],
            "infobase dump",
        ),
        (
            vec!["infobase", "restore", "--input", snapshot, "--replace"],
            "infobase restore",
        ),
        (vec!["convert"], "convert"),
        (vec!["make", "--output", artifact], "make"),
        (vec!["check"], "check"),
        (vec!["check", "designer-config"], "check designer-config"),
        (vec!["check", "designer-modules"], "check designer-modules"),
        (vec!["check", "edt"], "check edt"),
        (vec!["launch", "designer"], "launch"),
        (vec!["publish"], "publish"),
    ]
}

/// Подмножество, у которого превью на этом образце доходит до успеха. Только на нём можно
/// требовать нулевого кода и сверять содержимое: остальным нужен свой проект — базу,
/// веб-сервер или формат EDT этот образец не объявляет.
const SUCCEEDS_HERE: &[&str] = &[
    "push",
    "upload",
    "pull",
    "infobase create",
    "make",
    "check",
    "launch",
];

fn previews(artifact: &str) -> Vec<Vec<&str>> {
    with_preview(artifact, artifact)
        .into_iter()
        .filter(|(_, leaf)| SUCCEEDS_HERE.contains(leaf))
        .map(|(mut arguments, _)| {
            arguments.push("--dry-run");
            arguments
        })
        .collect()
}

/// Пути внутри `root`, относительно него, в устойчивом порядке.
fn entries_under(root: &Path, dir: &Path, found: &mut Vec<String>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if let Ok(relative) = path.strip_prefix(root) {
            found.push(relative.display().to_string());
        }
        if path.is_dir() {
            entries_under(root, &path, found);
        }
    }
}

/// Превью не оставляет следов: рабочего каталога после него нет вовсе. Проверяется по
/// отсутствию файлов, а не по отсутствию вызова — так требует
/// `DEC.2026-09-23.A-PREVIEW-LEAVES-NO-TRACE`.
///
/// Прежде здесь допускался журнал действий: правило велело превью оставить строку. Но
/// открытие журнала создаёт рабочий каталог, а превью запускают из песочниц, где запись
/// запрещена вовсе. Запись о вызове несёт конверт на stdout.
/// Половина сверки, которой здесь не хватало: перечень проверяемых превью назывался
/// руками и держал семь команд из двадцати двух. Лист, переведённый в `Preview::Runs`,
/// ускользал молча — проверено мутацией на `tools download`, которое под превью выкладывало
/// файл, оставляя договор зелёным.
#[test]
fn every_leaf_with_a_preview_is_exercised_here() {
    let mut covered: Vec<&str> = with_preview("artifact", "snapshot")
        .into_iter()
        .map(|(_, leaf)| leaf)
        .collect();
    covered.sort_unstable();
    let mut named: Vec<&str> = LEAVES_WITH_PREVIEW.to_vec();
    named.sort_unstable();

    assert_eq!(covered, named);
}

/// Названный путь журнала под превью тоже не исполняется: переменная принимает любой
/// путь, а открытие журнала создаёт родительский каталог — значит путь внутри проекта
/// создал бы то, чего превью создавать не должно. Решение это обещает, и обещание
/// проверяется.
#[test]
fn a_named_action_log_path_is_not_honoured_by_a_preview() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), true);
    let named = dir.path().join("named").join("actions.log");

    let preview = v8_runner_command()
        .env("V8TR_ACTION_LOG_FILE", &named)
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "check",
            "--dry-run",
        ])
        .output()
        .expect("run preview");
    assert_eq!(preview.status.code(), Some(0));
    assert!(
        !named.exists(),
        "превью завело журнал по названному пути: {}",
        fs::read_to_string(&named).unwrap_or_default()
    );

    // Боевой прогон названный путь по-прежнему исполняет: иначе проверка держала бы не
    // отказ превью, а поломку самой переменной.
    let real = v8_runner_command()
        .env("V8TR_ACTION_LOG_FILE", &named)
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "check",
        ])
        .output()
        .expect("run command");
    // Исход боевого прогона здесь не важен: важно, что названный путь он исполняет.
    assert!(
        named.exists(),
        "боевой прогон потерял названный путь журнала: код {:?}",
        real.status.code()
    );
}

/// Ни один лист с превью не создаёт рабочего каталога — ни тот, чьё превью здесь доходит
/// до успеха, ни тот, кому для успеха нужен свой проект. Отказ следов оставлять тоже не
/// вправе.
#[test]
fn no_leaf_with_a_preview_creates_the_work_path() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), true);
    fs::write(dir.path().join("main.cf"), "cf").expect("artifact");
    let artifact = dir.path().join("main.cf").display().to_string();
    let snapshot = dir.path().join("main.dt").display().to_string();
    let work = dir.path().join("work");

    for (arguments, leaf) in with_preview(&artifact, &snapshot) {
        let _ = fs::remove_dir_all(&work);
        let mut arguments = arguments;
        arguments.push("--dry-run");

        let (code, payload) = run(&config_path, &arguments);
        if SUCCEEDS_HERE.contains(&leaf) {
            assert_eq!(code, 0, "`{leaf}` did not preview: {payload}");
        }
        assert!(!work.exists(), "`{leaf}` created the work path");
    }
}

#[test]
fn no_preview_creates_anything_in_the_work_path() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), true);
    fs::write(dir.path().join("main.cf"), "cf").expect("artifact");
    let artifact = dir.path().join("main.cf").display().to_string();
    let work = dir.path().join("work");

    for preview in previews(&artifact) {
        // Каждое превью смотрится на чистом месте: иначе след одного сошёл бы за след
        // другого. Каталог именно удаляется, а не опустошается — превью не должно
        // создавать и его самого.
        let _ = fs::remove_dir_all(&work);

        let (code, payload) = run(&config_path, &preview);
        assert_eq!(
            code,
            0,
            "`{}` did not preview: {payload}",
            preview.join(" ")
        );

        assert!(
            !work.exists(),
            "`{}` created the work path: {:?}",
            preview.join(" "),
            {
                let mut left = Vec::new();
                entries_under(&work, &work, &mut left);
                left.sort();
                left
            }
        );
    }
}

/// Содержимое каждого файла под `root`. Журнал действий больше не исключается: превью
/// его не ведёт, поэтому дописанная строка — такой же след, как всякий другой.
fn contents_under(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut paths = Vec::new();
    entries_under(root, root, &mut paths);
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let bytes = fs::read(root.join(&path)).ok()?;
            Some((path, bytes))
        })
        .collect()
}

/// Предыдущий страж видит созданное на чистом месте. Этот — тронутое: рабочий каталог засевается
/// боевой сборкой, и после каждого превью его содержимое обязано совпадать побайтно.
/// Сравнивается содержимое, а не время правки: открытие базы состояния сдвигает `mtime`,
/// ничего в неё не записав.
#[test]
fn no_preview_changes_what_a_real_build_left_in_the_work_path() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), true);
    fs::write(dir.path().join("main.cf"), "cf").expect("artifact");
    let artifact = dir.path().join("main.cf").display().to_string();
    let work = dir.path().join("work");

    let (code, payload) = run(&config_path, &["build"]);
    assert_eq!(code, 0, "боевая сборка образца не прошла: {payload}");
    // Расширение меняется после засева: иначе переписывать нечего — утечка нашла бы
    // состояние свежим, пропустила подготовку, и страж промолчал бы.
    fs::write(
        dir.path()
            .join("project")
            .join("exts")
            .join("client-mcp")
            .join("Module.bsl"),
        "procedure Tool() // changed after the seed\nendprocedure",
    )
    .expect("modify extension");

    let seeded = contents_under(&work);
    assert!(
        !seeded.is_empty(),
        "боевая сборка ничего не оставила — сравнивать нечего"
    );

    for preview in previews(&artifact) {
        let (code, payload) = run(&config_path, &preview);
        assert_eq!(
            code,
            0,
            "`{}` did not preview: {payload}",
            preview.join(" ")
        );
        // Сравниваются оба снимка целиком: превью, которое снесло бы засеянный файл, из
        // сравнения «только по новому» ускользнуло бы, а снос — ровно то, что делает
        // подготовка расширения из исходников EDT.
        let after = contents_under(&work);
        let mut differs = Vec::new();
        for (path, before) in &seeded {
            match after.iter().find(|(was, _)| was == path) {
                None => differs.push(format!("removed {path}")),
                Some((_, now)) if now != before => differs.push(format!("rewrote {path}")),
                Some(_) => {}
            }
        }
        for (path, _) in &after {
            if !seeded.iter().any(|(was, _)| was == path) {
                differs.push(format!("added {path}"));
            }
        }
        assert!(
            differs.is_empty(),
            "`{}` changed the work path: {differs:?}",
            preview.join(" ")
        );
    }
}

/// Отсутствие платформы отказывает до одобрения плана: превью возвращается после
/// поиска утилиты, и вызывающий узнаёт об этом раньше, чем согласится с планом.
#[test]
fn a_preview_refuses_before_naming_a_plan_when_the_platform_is_missing() {
    let dir = temp_workspace();
    let config_path = write_project(dir.path(), false);
    let artifact = dir.path().join("main.cf").display().to_string();

    for preview in previews(&artifact) {
        let (code, payload) = run(&config_path, &preview);
        assert_ne!(
            code,
            0,
            "`{}` approved a plan without a platform: {payload}",
            preview.join(" ")
        );
        assert_eq!(payload["ok"], false, "{payload}");
        assert!(
            payload["data"]["plan"].is_null(),
            "`{}` named a plan it cannot run: {payload}",
            preview.join(" ")
        );
    }
}
