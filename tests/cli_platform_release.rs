//! `tools.platform.version: sources`: выпуск платформы по формату исходников без отката на
//! новейшую установку (`INV.PLATFORM.A-MISSING-RELEASE-OF-THE-SOURCES-FORMAT-IS-REFUSED`).
//!
//! Поддельная установка — каталоги версий под `tools.platform.path` с файлами утилит. Под
//! unix утилиты — скрипты, которые пишут свои вызовы в журнал: по нему видно, что платформа
//! не запускалась. Отказ приходит до запуска, поэтому под Windows хватает пустых файлов.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::command_data::assert_data_matches_its_command_form;
use support::{temp_workspace, v8_runner_command};

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

struct Project {
    dir: tempfile::TempDir,
    config: PathBuf,
    sources: PathBuf,
    calls: PathBuf,
}

/// Установка версии `version` с утилитами `utilities` под `root`.
fn install(root: &Path, version: &str, utilities: &[&str], calls: &Path) {
    let bin = root.join(version).join("bin");
    fs::create_dir_all(&bin).expect("bin");
    for utility in utilities {
        let path = bin.join(format!("{utility}{EXE}"));
        #[cfg(unix)]
        support::write_shell_script(
            &path,
            &format!("echo '{utility} {version}' >> '{}'", calls.display()),
        );
        #[cfg(not(unix))]
        {
            let _ = calls;
            fs::write(&path, "fake").expect("utility");
        }
    }
}

/// Проект формата Конфигуратора в формате исходников `format`, установки 8.3.27 и 8.5.4 под
/// `tools.platform.path` и `tools.platform.version: sources`.
fn project(format: &str) -> Project {
    let dir = temp_workspace();
    let root = dir.path();
    let sources = root.join("src");
    fs::create_dir_all(sources.join("Catalogs")).expect("src");
    fs::write(
        sources.join("Configuration.xml"),
        format!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"{format}\">\n</MetaDataObject>\n"
        ),
    )
    .expect("Configuration.xml");
    fs::write(sources.join("Catalogs").join("Items.xml"), "<Catalog/>").expect("catalog");
    // Файловая база есть: перенос сперва проверяет её, а потом ищет платформу.
    fs::create_dir_all(root.join("ib")).expect("ib");
    fs::write(root.join("ib").join("1Cv8.1CD"), "").expect("ib file");
    let platform = root.join("platform");
    let calls = root.join("calls.log");
    install(&platform, "8.3.27.2074", &["1cv8", "ibcmd"], &calls);
    install(&platform, "8.5.4.1306", &["1cv8", "1cv8c", "ibcmd"], &calls);
    let config = root.join("v8project.yaml");
    fs::write(
        &config,
        format!(
            "workPath: work\nformat: DESIGNER\ninfobase:\n  connection: 'File={ib}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntools:\n  platform:\n    path: '{platform}'\n    version: sources\n",
            ib = root.join("ib").display(),
            platform = platform.display(),
        ),
    )
    .expect("config");
    support::memory::remember_base(
        root,
        "origin",
        support::memory::Base::File(&root.join("ib")),
        &[support::memory::Set::configuration("main", &sources)],
    );
    Project {
        dir,
        config,
        sources,
        calls,
    }
}

fn project_command(project: &Project) -> std::process::Command {
    let mut command = v8_runner_command();
    command
        .current_dir(project.dir.path())
        .arg("--config")
        .arg(&project.config)
        .arg("--json-message");
    command
}

fn run(project: &Project, arguments: &[&str]) -> (i32, Value) {
    run_command(&mut project_command(project), arguments)
}

fn run_command(command: &mut std::process::Command, arguments: &[&str]) -> (i32, Value) {
    let output = command.args(arguments).output().expect("run v8-runner");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let payload: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "{error}: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    (output.status.code().expect("exit code"), payload)
}

/// Снимок исходников: имя и содержимое каждого файла.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).expect("read dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push((path.clone(), fs::read(&path).expect("read")));
            }
        }
    }
    files.sort();
    files
}

/// Отказ выпуска: код, род, выход 2 и поле с форматом и описью установок.
fn assert_release_refusal(code: i32, payload: &Value) {
    assert_eq!(code, 2, "{payload}");
    let error = &payload["error"];
    assert_eq!(error["code"], "platform_release_missing", "{payload}");
    assert_eq!(error["kind"], "environment", "{payload}");
    let release = &error["platform_release"];
    assert_eq!(release["source_format"]["version"], "2.21", "{payload}");
    assert_eq!(release["source_format"]["release"], "8.5.1", "{payload}");
    assert!(
        release["source_format"]["file"]
            .as_str()
            .is_some_and(|file| file.ends_with("Configuration.xml")),
        "{payload}"
    );
    let versions: Vec<&str> = release["installations"]
        .as_array()
        .expect("installations")
        .iter()
        .map(|installation| installation["version"].as_str().expect("version"))
        .collect();
    assert_eq!(versions, ["8.5.4.1306", "8.3.27.2074"], "{payload}");
    let message = error["message"].as_str().expect("message");
    for named in [
        "2.21",
        "Configuration.xml",
        "8.5.1",
        "8.5.4.1306",
        "8.3.27.2074",
    ] {
        assert!(message.contains(named), "{named}: {message}");
    }
}

/// Формат 2.21 пишет 8.5.1, а установлены 8.3.27 и 8.5.4: `pull`, `push`, `make`, `check`,
/// `launch`, `download`, `infobase dump` и `status --deep` отвечают одним кодом
/// `platform_release_missing` с описью, платформу не запускают и исходников не трогают.
#[test]
fn every_platform_command_refuses_without_the_release_of_the_sources_format() {
    let project = project("2.21");
    let before = snapshot(&project.sources);

    for arguments in [
        &["pull"][..],
        &["push"],
        &["make", "main", "--output", "out.cf"],
        &["check"],
        &["launch", "thin"],
        &["--dry-run", "push"],
        &["download", "main", "--output", "main.cf"],
        &["infobase", "dump", "--output", "ib.dt"],
        &["status", "--deep"],
    ] {
        let (code, payload) = run(&project, arguments);
        assert_release_refusal(code, &payload);
    }

    assert!(!project.calls.exists(), "the platform was started");
    assert_eq!(
        snapshot(&project.sources),
        before,
        "the sources were touched"
    );
}

/// Запись поколения сделал другой инструмент, а исполнителю утилиты не нужны: поколение
/// спрашивает инструмент записи, и отсутствие выпуска остаётся отказом выпуска, а не
/// «инструмент не ответил» — тот стал бы отказом сверки `non_fast_forward`.
#[test]
fn a_foreign_generation_record_answers_the_release_refusal_not_non_fast_forward() {
    let project = project("2.21");
    let root = project.dir.path();
    let port = support::free_tcp_port();
    let exchange = root.join("exchange");
    fs::create_dir_all(&exchange).expect("exchange");
    let config = fs::read_to_string(&project.config).expect("config");
    let ib = format!("  connection: 'File={}'\n", root.join("ib").display());
    fs::write(
        &project.config,
        config.replace(
            &ib,
            &format!(
                "  user: agent\n  password: secret\n  standalone:\n    gate: 127.0.0.1:{port}\n    exchange:\n      dir: '{}'\n",
                exchange.display()
            ),
        ),
    )
    .expect("standalone config");
    support::memory::remember_base(
        root,
        "origin",
        support::memory::Base::Standalone {
            host: "127.0.0.1",
            port,
        },
        &[support::memory::Set::configuration(
            "main",
            &project.sources,
        )],
    );
    let ledger_path = root.join(".local/infobases/origin/generation.json");
    let mut ledger: Value =
        serde_json::from_str(&fs::read_to_string(&ledger_path).expect("ledger")).expect("json");
    ledger["main"]["tool"] = Value::from("ibcmd");
    ledger["main"]["token"] = Value::from("1".repeat(40));
    fs::write(&ledger_path, ledger.to_string()).expect("ledger");

    let (code, payload) = run(&project, &["push"]);

    assert_release_refusal(code, &payload);
    assert!(!project.calls.exists(), "the platform was started");
}

/// `platforms` при `sources` без выпуска: отказ — в `warnings`, `selected` пуст, установки
/// названы; платформа не запускается.
#[test]
fn platforms_name_the_release_refusal_in_the_warnings() {
    let project = project("2.21");

    let (code, envelope) = run(&project, &["platforms"]);

    assert_eq!(code, 0, "{envelope}");
    assert_data_matches_its_command_form(&envelope, "platforms");
    let data = &envelope["data"];
    assert_eq!(data["requirement"]["version"], "sources", "{envelope}");
    for utility in ["designer", "thin_client", "ibcmd"] {
        assert!(data["selected"][utility].is_null(), "{utility}: {envelope}");
    }
    let warnings: Vec<&str> = envelope["warnings"]
        .as_array()
        .expect("warnings")
        .iter()
        .filter_map(Value::as_str)
        .filter(|warning| warning.contains("8.5.1"))
        .collect();
    assert_eq!(warnings.len(), 1, "{envelope}");
    assert!(warnings[0].contains("2.21"), "{envelope}");
    assert!(!project.calls.exists(), "the platform was started");
}

/// Выпуск формата установлен: команды идут, как без ключа, — `platforms` выбирает его.
#[test]
fn an_installed_release_of_the_sources_format_is_chosen() {
    let project = project("2.20");

    let (code, envelope) = run(&project, &["platforms"]);

    assert_eq!(code, 0, "{envelope}");
    let designer = &envelope["data"]["selected"]["designer"];
    assert_eq!(designer["version"], "8.3.27.2074", "{envelope}");
    assert_eq!(designer["chosen_by"], "source_format", "{envelope}");
}

#[test]
fn an_explicit_bin_directory_counts_for_the_sources_release() {
    let project = project("2.20");
    let root = project.dir.path();
    let platform = root.join("platform");
    let version = "8.3.27.2074";
    let release = platform.join(version);
    let bin = release.join("bin");
    let executable = bin.join(format!("1cv8{EXE}"));
    let other_bin = platform.join("8.5.4.1306").join("bin");
    install(&platform, version, &["1cv8c"], &project.calls);
    let config = fs::read_to_string(&project.config).expect("config");
    let configure = |hint: &Path, requirement: &str| {
        fs::write(
            &project.config,
            config.replace(
                &format!("    path: '{}'\n    version: sources", platform.display()),
                &format!(
                    "    path: '{}'\n    version: {requirement}\n    strict: true",
                    hint.display()
                ),
            ),
        )
        .expect("strict platform config");
    };
    let run = |arguments: &[&str]| {
        run_command(project_command(&project).env("PATH", &other_bin), arguments)
    };
    let preview = ["make", "main", "--output", "out.cf", "--dry-run"];
    let utilities = [
        ("designer", "1cv8"),
        ("thin_client", "1cv8c"),
        ("ibcmd", "ibcmd"),
    ];

    for (hint, requirement, chosen_by) in [
        (bin.as_path(), version, "version"),
        (bin.as_path(), "sources", "source_format"),
        (release.as_path(), "sources", "source_format"),
        (executable.as_path(), "sources", "source_format"),
    ] {
        configure(hint, requirement);
        let (code, envelope) = run(&["platforms"]);
        assert_eq!(code, 0, "{}: {envelope}", hint.display());
        assert_data_matches_its_command_form(&envelope, "platforms");
        let data = &envelope["data"];
        let inventory = data["installations"].as_array().expect("installations");
        assert_eq!(inventory.len(), 1, "{}: {envelope}", hint.display());
        assert_eq!(inventory[0]["version"], version, "{envelope}");
        let inventory_root = inventory[0]["root"].as_str().expect("inventory root");
        assert_eq!(
            fs::canonicalize(inventory_root).expect("listed root"),
            fs::canonicalize(&release).expect("release root"),
            "{envelope}"
        );
        for (component, executable) in utilities {
            assert_eq!(inventory[0]["components"][component], true, "{envelope}");
            let selected = &data["selected"][component];
            assert_eq!(selected["version"], version, "{envelope}");
            assert_eq!(selected["chosen_by"], chosen_by, "{envelope}");
            let expected = bin.join(format!("{executable}{EXE}"));
            let selected_path = selected["path"].as_str().expect("selected path");
            assert_eq!(
                fs::canonicalize(selected_path).expect("selected executable"),
                fs::canonicalize(expected).expect("fixture executable"),
                "{envelope}"
            );
        }

        let (code, envelope) = run(&preview);
        assert_eq!(code, 0, "{}: {envelope}", hint.display());
        assert_eq!(envelope["data"]["provider_dispatched"], false, "{envelope}");
    }

    let source = project.sources.join("Configuration.xml");
    let xml = fs::read_to_string(&source).expect("Configuration.xml");
    let newer_xml = xml.replace("2.20", "2.22");
    fs::write(&source, &newer_xml).expect("format 2.22");
    configure(&bin, "sources");
    let (code, envelope) = run(&preview);
    assert_eq!(code, 2, "{envelope}");
    let error = &envelope["error"];
    assert_eq!(error["code"], "platform_release_missing", "{envelope}");
    assert_eq!(error["kind"], "environment", "{envelope}");
    let missing = &error["platform_release"];
    assert_eq!(missing["source_format"]["release"], "8.5.4", "{envelope}");
    let inventory = missing["installations"].as_array().expect("installations");
    assert_eq!(inventory.len(), 1, "{envelope}");
    assert_eq!(inventory[0]["version"], version, "{envelope}");

    fs::copy(&executable, release.join(format!("1cv8{EXE}"))).expect("parent utility");
    for (_, utility) in utilities {
        fs::remove_file(bin.join(format!("{utility}{EXE}"))).expect("empty bin");
    }
    fs::write(&source, xml).expect("restore format 2.20");
    let (code, envelope) = run(&preview);
    assert_eq!(code, 2, "{envelope}");
    let error = &envelope["error"];
    assert_eq!(error["code"], "platform_release_missing", "{envelope}");
    assert_eq!(
        error["platform_release"]["installations"],
        serde_json::json!([]),
        "{envelope}"
    );

    #[cfg(unix)]
    {
        fs::write(&source, &newer_xml).expect("format 2.22");
        std::os::unix::fs::symlink(other_bin.join("1cv8"), &executable).expect("foreign utility");
        let (code, envelope) = run(&preview);
        assert_eq!(code, 2, "{envelope}");
        assert_eq!(
            envelope["error"]["code"], "platform_release_missing",
            "{envelope}"
        );
        assert_eq!(
            envelope["error"]["platform_release"]["installations"],
            serde_json::json!([]),
            "{envelope}"
        );
    }

    assert!(!project.calls.exists(), "the platform was started");
    assert!(!root.join("out.cf").exists(), "preview created an artifact");
}
