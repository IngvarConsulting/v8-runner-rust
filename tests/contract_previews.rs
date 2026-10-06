//! Что обещает превью любой команды: ни одного следа в файловой системе и отказ до
//! одобрения плана, если платформы нет.
//!
//! Харнесс подкладывает shell-скрипты вместо утилит платформы, поэтому файл целиком
//! собирается только под unix — как и остальные тесты, использующие тот же `support`.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::Path;

use support::previews::{run, with_preview, write_project};
use support::{temp_workspace, v8_runner_command};

// Состав таблицы листьев, общий с `src/cli/global_flags.rs`.
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/cli/global_flags_expected.in"
));

/// Подмножество, у которого превью на этом образце доходит до успеха. Только на нём можно
/// требовать нулевого кода и сверять содержимое: остальным нужен свой проект — базу
/// или формат EDT этот образец не объявляет.
const SUCCEEDS_HERE: &[&str] = &[
    "clone",
    "push",
    "upload",
    "pull",
    "infobase create",
    "make",
    "check",
    "check designer-modules",
    "launch",
    "publish",
];

fn previews(dir: &Path) -> Vec<Vec<String>> {
    with_preview(dir)
        .into_iter()
        .filter(|row| SUCCEEDS_HERE.contains(&row.leaf))
        .map(|row| {
            let mut arguments = row.arguments;
            arguments.push("--dry-run".to_owned());
            arguments
        })
        .collect()
}

/// Память о базе образца перед превью `push`: без неё оно называет отказ `no_memory`
/// (`INV.USE-CASES.A-PUSH-WITHOUT-MEMORY-OF-THE-BASE-IS-REFUSED`). Возвращает содержимое
/// следа после записи памяти — превью его менять не вправе; у прочих листьев — `None`.
fn remember_for(leaf: &str, dir: &Path, trace: &Path) -> Option<Vec<(String, Vec<u8>)>> {
    matches!(leaf, "push" | "build").then(|| {
        support::memory::remember_sample(dir);
        contents_under(trace)
    })
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
    let mut covered: Vec<&str> = with_preview(Path::new("."))
        .into_iter()
        .map(|row| row.leaf)
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
    write_project(dir.path(), true);
    fs::write(dir.path().join("main.cf"), "cf").expect("artifact");

    for previewed in with_preview(dir.path()) {
        let _ = fs::remove_dir_all(&previewed.trace);
        let leaf = previewed.leaf;
        // Превью `push` без памяти о базе называет отказ `no_memory`; память лежит в рабочем
        // каталоге, поэтому у него след — всё, чего не было до превью.
        let remembered = remember_for(leaf, dir.path(), &previewed.trace);
        let mut arguments = previewed.arguments;
        arguments.push("--dry-run".to_owned());

        let (code, payload) = run(dir.path(), &arguments);
        if SUCCEEDS_HERE.contains(&leaf) {
            assert_eq!(code, 0, "`{leaf}` did not preview: {payload}");
        }
        match remembered {
            Some(before) => assert_eq!(
                contents_under(&previewed.trace),
                before,
                "`{leaf}` changed {}",
                previewed.trace.display()
            ),
            None => assert!(
                !previewed.trace.exists(),
                "`{leaf}` left {}",
                previewed.trace.display()
            ),
        }
    }
}

#[test]
fn no_preview_creates_anything_in_the_work_path() {
    let dir = temp_workspace();
    write_project(dir.path(), true);
    fs::write(dir.path().join("main.cf"), "cf").expect("artifact");
    let work = dir.path().join("work");

    for preview in previews(dir.path()) {
        // Каждое превью смотрится на чистом месте: иначе след одного сошёл бы за след
        // другого. Каталог именно удаляется, а не опустошается — превью не должно
        // создавать и его самого.
        let _ = fs::remove_dir_all(&work);
        let remembered = remember_for(
            preview.first().map(String::as_str).unwrap_or_default(),
            dir.path(),
            &work,
        );

        let (code, payload) = run(dir.path(), &preview);
        assert_eq!(
            code,
            0,
            "`{}` did not preview: {payload}",
            preview.join(" ")
        );

        if let Some(before) = remembered {
            assert_eq!(
                contents_under(&work),
                before,
                "`{}` changed the work path",
                preview.join(" ")
            );
            continue;
        }
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
    write_project(dir.path(), true);
    fs::write(dir.path().join("main.cf"), "cf").expect("artifact");
    let work = dir.path().join("work");

    let (code, payload) = run(dir.path(), &["build".to_owned(), "--force".to_owned()]);
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

    for preview in previews(dir.path()) {
        let (code, payload) = run(dir.path(), &preview);
        if preview.first().map(String::as_str) == Some("clone") {
            // Базу засеяла боевая сборка этой рабочей копии, и она за ней в метке: `clone`
            // выгрузил бы её в другой проект, поэтому его превью называет отказ по владельцу
            // (`INV.CLI.A-PREVIEW-NAMES-THE-OWNERSHIP-REFUSAL`). След он оставить не вправе и тут.
            assert_eq!(code, 3, "`{}`: {payload}", preview.join(" "));
            assert_eq!(payload["error"]["code"], "infobase_held", "{payload}");
        } else {
            assert_eq!(
                code,
                0,
                "`{}` did not preview: {payload}",
                preview.join(" ")
            );
        }
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

/// Превью работы исполнителю не даёт, поэтому никакой его ответ — ни план, ни отказ — не
/// говорит `provider_dispatched: true`. Проверяются все листья с превью на обоих образцах,
/// с платформой и без неё; утверждение не зависит от исхода. Листья, чьё превью на образце
/// доходит до плана, обязаны назвать признак прямо: `false`, а не молчание. Остальные
/// могут ответить общей формой отказа без признака: отказ до диспетчеризации утверждения
/// о запуске не несёт вовсе.
#[test]
fn no_preview_claims_that_an_executor_got_work() {
    for with_platform in [true, false] {
        let dir = temp_workspace();
        write_project(dir.path(), with_platform);
        // Память о базе — чтобы превью `push` доходило до плана, а не до отказа `no_memory`.
        support::memory::remember_sample(dir.path());
        // Артефакт на месте, чтобы превью `upload` доходило до плана, а не до отказа.
        fs::write(dir.path().join("main.cf"), "cf").expect("artifact");
        for previewed in with_preview(dir.path()) {
            let leaf = previewed.leaf;
            let mut arguments = previewed.arguments;
            arguments.push("--dry-run".to_owned());
            let (code, payload) = run(dir.path(), &arguments);
            let dispatched = &payload["data"]["provider_dispatched"];
            // С платформой `convert` тоже доходит до плана: путь к EDT CLI в образце явный,
            // и стаб на месте.
            if with_platform && (SUCCEEDS_HERE.contains(&leaf) || leaf == "convert") {
                assert_eq!(code, 0, "`{leaf}` did not preview: {payload}");
                assert_eq!(dispatched, false, "`{leaf}` preview: {payload}");
            } else {
                assert_ne!(
                    dispatched, true,
                    "`{leaf}` (platform present: {with_platform}) claimed work in a preview: {payload}"
                );
            }
        }
    }
}

/// Отсутствие платформы отказывает до одобрения плана: превью возвращается после
/// поиска утилиты, и вызывающий узнаёт об этом раньше, чем согласится с планом.
#[test]
fn a_preview_refuses_before_naming_a_plan_when_the_platform_is_missing() {
    let dir = temp_workspace();
    write_project(dir.path(), false);

    for preview in previews(dir.path()) {
        let (code, payload) = run(dir.path(), &preview);
        assert_ne!(
            code,
            0,
            "`{}` approved a plan without a platform: {payload}",
            preview.join(" ")
        );
        assert_eq!(payload["ok"], false, "{payload}");
        // Отказ не вправе сообщать о запуске, которого не было. Отсутствие поля проходит:
        // часть отказов отвечает общей формой отказа, и утверждения о запуске там нет
        // вовсе. Ложное «да» не проходит — прежде его никто не ловил, и выгрузка отвечала
        // `true`, не найдя утилиты (#267).
        assert_ne!(
            payload["data"]["provider_dispatched"],
            true,
            "`{}` reported a dispatch that did not happen: {payload}",
            preview.join(" ")
        );
        assert!(
            payload["data"]["plan"].is_null(),
            "`{}` named a plan it cannot run: {payload}",
            preview.join(" ")
        );
    }
}
