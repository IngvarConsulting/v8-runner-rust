#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script as write_script};

const V8_CONFIGURATION_NATURE: &str = "com._1c.g5.v8.dt.core.V8ConfigurationNature";
const V8_EXTENSION_NATURE: &str = "com._1c.g5.v8.dt.core.V8ExtensionNature";
const EDT_RUNTIME_VERSION: &str = "8.3.27";

/// Прежний глобальный `builder` в тестовых конфигах: `DESIGNER` — умолчания матрицы,
/// `IBCMD` — `ibcmd` всюду, где у операции есть развилка.
fn providers_yaml(builder: &str) -> &'static str {
    if builder == "IBCMD" {
        "providers:\n  init: ibcmd\n  build: ibcmd\n  dump: ibcmd\n  infobase.configuration.export: ibcmd\n"
    } else {
        ""
    }
}

fn write_build_script(path: &Path, fail_pattern: Option<&str>) {
    let pattern_branch = fail_pattern
        .map(|pattern| {
            format!(
                "if printf '%s' \"$args\" | grep -F -q -- '{}'; then exit 17; fi",
                pattern
            )
        })
        .unwrap_or_default();
    let body = format!(
        "args=\"$*\"\nout=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf 'designer log for %s\\n' \"$args\" > \"$out\"; fi\n{}\nexit 0",
        pattern_branch
    );
    write_script(path, &body);
}

fn write_ibcmd_script(path: &Path, calls_log: &Path, fail_pattern: Option<&str>) {
    let pattern_branch = fail_pattern
        .map(|pattern| {
            format!(
                "if printf '%s' \"$args\" | grep -F -q -- '{}'; then exit 17; fi",
                pattern
            )
        })
        .unwrap_or_default();
    let body = format!(
        "args=\"$*\"\nprintf '%s\\n' \"$args\" >> \"{}\"\n{}\nexit 0",
        calls_log.display(),
        pattern_branch
    );
    write_script(path, &body);
}

fn write_edt_script(path: &Path, calls_log: &Path) {
    let body = format!(
        "args=\"$*\"\ntarget=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"--configuration-files\" ]; then target=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$target\" ]; then mkdir -p \"$target\"; printf '<Configuration />\\n' > \"$target/Configuration.xml\"; fi\nprintf '%s\\n' \"$args\" >> \"{}\"\nexit 0",
        calls_log.display()
    );
    write_script(path, &body);
}

fn write_native_edt_project(
    path: &Path,
    project_name: &str,
    nature: &str,
    base_project: Option<&str>,
) {
    fs::create_dir_all(path.join("DT-INF")).expect("dt-inf");
    fs::create_dir_all(path.join("src").join("Configuration")).expect("src");
    fs::write(
        path.join(".project"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>{project_name}</name>\n  <natures>\n    <nature>{nature}</nature>\n  </natures>\n</projectDescription>\n"
        ),
    )
    .expect("project");
    let base_project_line = base_project
        .map(|value| format!("Base-Project: {value}\n"))
        .unwrap_or_default();
    fs::write(
        path.join("DT-INF").join("PROJECT.PMF"),
        format!(
            "{base_project_line}Manifest-Version: 1.0\nRuntime-Version: {EDT_RUNTIME_VERSION}\n"
        ),
    )
    .expect("manifest");
    fs::write(
        path.join("src")
            .join("Configuration")
            .join("Configuration.mdo"),
        "<Configuration />\n",
    )
    .expect("configuration marker");
    fs::write(
        path.join("src").join("Configuration").join("Module.bsl"),
        "Procedure Test()\nEndProcedure\n",
    )
    .expect("module marker");
}

fn write_config(path: &Path, base_path: &Path, work_path: &Path, platform_path: &Path) {
    write_config_with_builder(
        path,
        base_path,
        work_path,
        platform_path,
        "DESIGNER",
        "File=/tmp/ib",
    );
}

fn write_config_with_builder(
    path: &Path,
    base_path: &Path,
    work_path: &Path,
    platform_path: &Path,
    builder: &str,
    connection: &str,
) {
    let infobase = format!("  connection: '{}'\n", connection);
    write_config_with_builder_and_infobase(
        path,
        base_path,
        work_path,
        platform_path,
        builder,
        &infobase,
    );
}

fn write_config_with_builder_and_infobase(
    path: &Path,
    _base_path: &Path,
    work_path: &Path,
    platform_path: &Path,
    builder: &str,
    infobase_yaml: &str,
) {
    let config = format!(
        "workPath: '{}'\nformat: DESIGNER\n{}infobase:\n{}build:\n  partialLoadThreshold: 20\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/main\n  - name: ext\n    type: EXTENSION\n    path: project/ext\ntools:\n  platform:\n    path: '{}'\n",
        work_path.display(),
        providers_yaml(builder),
        infobase_yaml,
        platform_path.display(),
    );

    fs::write(path, config).expect("config");
}

fn write_live_workspace_lock(work_path: &Path, command: &str) {
    let canonical_work = fs::canonicalize(work_path).expect("canonical work");
    let lock_owner = "integration-test-lock-owner";
    let started_at = chrono::Utc::now().to_rfc3339();

    fs::write(
        canonical_work.join(".v8-runner.workspace.lock"),
        serde_json::json!({
            "tool": "v8-runner",
            "pid": std::process::id(),
            "owner_id": lock_owner,
            "created_at": started_at,
        })
        .to_string(),
    )
    .expect("workspace lock");
    fs::write(
        canonical_work.join(".v8-runner.workspace.lock.json"),
        serde_json::json!({
            "pid": std::process::id(),
            "lock_owner": lock_owner,
            "command": command,
            "started_at": started_at,
            "canonical_work_path": canonical_work,
        })
        .to_string(),
    )
    .expect("workspace lock sidecar");
}

fn setup_project() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let config_path = dir.path().join("v8project.yaml");
    let binary_path = dir.path().join("1cv8");

    fs::create_dir_all(base_path.join("main").join("Catalogs.Items")).expect("main");
    fs::create_dir_all(base_path.join("ext").join("CommonModules")).expect("ext");
    fs::create_dir_all(&work_path).expect("work");
    fs::write(
        base_path
            .join("main")
            .join("Catalogs.Items")
            .join("ObjectModule.bsl"),
        "procedure Test() endprocedure",
    )
    .expect("main bsl");
    fs::write(
        base_path
            .join("main")
            .join("Catalogs.Items")
            .join("ObjectModule.xml"),
        "<MetaDataObject />",
    )
    .expect("main xml");
    fs::write(
        base_path
            .join("ext")
            .join("CommonModules")
            .join("Module.bsl"),
        "procedure Test() endprocedure",
    )
    .expect("ext bsl");

    write_build_script(&binary_path, None);
    write_config(&config_path, &base_path, &work_path, &binary_path);

    (dir, config_path, binary_path, work_path)
}

fn setup_ibcmd_project() -> (
    tempfile::TempDir,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let config_path = dir.path().join("v8project.yaml");
    let binary_path = dir.path().join("ibcmd");
    let calls_log = dir.path().join("calls.log");

    fs::create_dir_all(base_path.join("main").join("Catalogs.Items")).expect("main");
    fs::create_dir_all(base_path.join("ext").join("CommonModules")).expect("ext");
    fs::create_dir_all(&work_path).expect("work");
    fs::write(
        base_path
            .join("main")
            .join("Catalogs.Items")
            .join("ObjectModule.bsl"),
        "procedure Test() endprocedure",
    )
    .expect("main bsl");
    fs::write(
        base_path
            .join("main")
            .join("Catalogs.Items")
            .join("ObjectModule.xml"),
        "<MetaDataObject />",
    )
    .expect("main xml");
    fs::write(
        base_path
            .join("ext")
            .join("CommonModules")
            .join("Module.bsl"),
        "procedure Test() endprocedure",
    )
    .expect("ext bsl");

    write_ibcmd_script(&binary_path, &calls_log, None);
    write_config_with_builder(
        &config_path,
        &base_path,
        &work_path,
        &binary_path,
        "IBCMD",
        "File=/tmp/ib",
    );

    (
        dir,
        config_path,
        binary_path,
        work_path,
        base_path,
        calls_log,
    )
}

fn setup_edt_ibcmd_project() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let config_path = dir.path().join("v8project.yaml");
    let ibcmd_path = dir.path().join("ibcmd");
    let edt_cli_path = dir.path().join("edt").join("1cedtcli");
    let ibcmd_calls_log = dir.path().join("ibcmd-calls.log");
    let edt_calls_log = dir.path().join("edt-calls.log");

    fs::create_dir_all(base_path.join("configuration").join("Catalogs.Items")).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_native_edt_project(
        &base_path.join("configuration"),
        "configuration",
        V8_CONFIGURATION_NATURE,
        None,
    );
    fs::write(
        base_path
            .join("configuration")
            .join("Catalogs.Items")
            .join("ObjectModule.bsl"),
        "procedure Test() endprocedure",
    )
    .expect("bsl");

    write_ibcmd_script(&ibcmd_path, &ibcmd_calls_log, None);
    write_edt_script(&edt_cli_path, &edt_calls_log);

    let config = format!(
        "workPath: '{}'\nformat: EDT\nproviders:\n  init: ibcmd\n  build: ibcmd\n  dump: ibcmd\n  infobase.configuration.export: ibcmd\ninfobase:\n  connection: 'File=/tmp/ib'\nbuild:\n  partialLoadThreshold: 20\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: '{}'\n  edt_cli:\n    path: '{}'\n",
        work_path.display(),
        ibcmd_path.display(),
        edt_cli_path.display(),
    );
    fs::write(&config_path, config).expect("config");

    (dir, config_path, ibcmd_calls_log, edt_calls_log)
}

fn setup_edt_extension_project() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let config_path = dir.path().join("v8project.yaml");
    let platform_path = dir.path().join("platform").join("bin").join("1cv8");
    let edt_cli_path = dir.path().join("edt").join("1cedtcli");
    let edt_calls_log = dir.path().join("edt-calls.log");

    fs::create_dir_all(base_path.join("configuration").join("Catalogs.Items")).expect("base");
    fs::create_dir_all(base_path.join("exts").join("client-mcp")).expect("ext");
    fs::create_dir_all(&work_path).expect("work");
    write_native_edt_project(
        &base_path.join("configuration"),
        "configuration",
        V8_CONFIGURATION_NATURE,
        None,
    );
    fs::write(
        base_path
            .join("configuration")
            .join("Catalogs.Items")
            .join("ObjectModule.bsl"),
        "procedure Test() endprocedure",
    )
    .expect("configuration bsl");
    fs::write(
        base_path
            .join("configuration")
            .join("Catalogs.Items")
            .join("ObjectModule.xml"),
        "<MetaDataObject />",
    )
    .expect("configuration xml");
    write_native_edt_project(
        &base_path.join("exts").join("client-mcp"),
        "client_mcp",
        V8_EXTENSION_NATURE,
        Some("configuration"),
    );
    fs::write(
        base_path.join("exts").join("client-mcp").join("Module.bsl"),
        "procedure Test() endprocedure",
    )
    .expect("extension bsl");

    write_build_script(&platform_path, None);
    write_edt_script(&edt_cli_path, &edt_calls_log);

    let config = format!(
        "workPath: '{}'\nformat: EDT\ninfobase:\n  connection: 'File=/tmp/ib'\nbuild:\n  partialLoadThreshold: 20\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: project/configuration\n  - name: client_mcp\n    type: EXTENSION\n    path: project/exts/client-mcp\ntools:\n  platform:\n    path: '{}'\n  edt_cli:\n    path: '{}'\n",
        work_path.display(),
        platform_path.display(),
        edt_cli_path.display(),
    );
    fs::write(&config_path, config).expect("config");

    (dir, config_path, work_path)
}

/// Всё, что лежит в рабочем каталоге после превью. Пусто оно быть обязано целиком:
/// `DEC.2026-09-23.A-PREVIEW-LEAVES-NO-TRACE` не оставляет превью и журнала.
fn left_in_work_path(work_path: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) {
        let Ok(read) = fs::read_dir(dir) else {
            return;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if let Ok(relative) = path.strip_prefix(root) {
                found.push(relative.display().to_string());
            }
            if path.is_dir() {
                walk(root, &path, found);
            }
        }
    }

    let mut found = Vec::new();
    walk(work_path, work_path, &mut found);
    found.sort();
    found
}

const V8_EXTERNAL_OBJECTS_NATURE: &str = "com._1c.g5.v8.dt.core.V8ExternalObjectsNature";

fn write_edt_external_project(path: &Path, name: &str) {
    fs::create_dir_all(path.join("DT-INF")).expect("dt-inf");
    fs::create_dir_all(path.join("src")).expect("src");
    fs::write(
        path.join(".project"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>{name}</name>\n  <natures>\n    <nature>{V8_EXTERNAL_OBJECTS_NATURE}</nature>\n  </natures>\n</projectDescription>\n"
        ),
    )
    .expect("project");
    fs::write(
        path.join("DT-INF").join("PROJECT.PMF"),
        format!(
            "Base-Project: BaseProject\nManifest-Version: 1.0\nRuntime-Version: {EDT_RUNTIME_VERSION}\n"
        ),
    )
    .expect("manifest");
    fs::write(
        path.join("src").join("root.xml"),
        format!(
            "<ExternalDataProcessor><Properties><Name>{name}</Name></Properties></ExternalDataProcessor>\n"
        ),
    )
    .expect("descriptor");
}

/// Превью сборки EDT ищет обе утилиты. Шаг обещает выгрузку и следующую за ней загрузку
/// в базу, поэтому одобрить его, не зная, чем грузить, значит одобрить невыполнимое
/// (`INV.CLI.PREVIEW-RETURNS-AFTER-TOOL-LOOKUP`). Проверяются оба исполнителя загрузки:
/// пропажа поиска у одного из них прошла бы молча.
#[test]
fn a_planned_edt_build_refuses_when_the_utility_that_would_load_it_is_missing() {
    for (providers, missing) in [("", "1cv8"), ("providers:\n  build: ibcmd\n", "ibcmd")] {
        let dir = temp_workspace();
        let base_path = dir.path().join("project");
        let work_path = dir.path().join("work");
        let config_path = dir.path().join("v8project.yaml");
        let edt_cli_path = dir.path().join("edt").join("1cedtcli");
        let edt_calls_log = dir.path().join("edt-calls.log");
        // Платформы нет: каталог пуст, а строгий режим с версией не даёт локатору уйти
        // в PATH или в корни по умолчанию.
        let empty_platform = dir.path().join("empty-platform");

        fs::create_dir_all(base_path.join("configuration")).expect("base");
        fs::create_dir_all(&work_path).expect("work");
        fs::create_dir_all(empty_platform.join("bin")).expect("empty platform");
        write_native_edt_project(
            &base_path.join("configuration"),
            "configuration",
            V8_CONFIGURATION_NATURE,
            None,
        );
        write_edt_script(&edt_cli_path, &edt_calls_log);
        fs::write(
            &config_path,
            format!(
                "workPath: '{}'\nformat: EDT\n{providers}infobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: '{}'\n    strict: true\n    version: '8.3.27'\n  edt_cli:\n    path: '{}'\n",
                work_path.display(),
                empty_platform.display(),
                edt_cli_path.display()
            ),
        )
        .expect("config");

        let output = v8_runner_command()
            .args([
                "--config",
                &config_path.display().to_string(),
                "--json-message",
                "build",
                "--dry-run",
            ])
            .output()
            .expect("run command");

        let reported = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.status.success(),
            "превью одобрило план, которому нечем грузить `{missing}`: {reported}"
        );

        // Отказ обязан прийти от шага, а не откуда-нибудь ещё: имя утилиты лежит и в
        // квитанции выбора исполнителя, и она печатается при любом исходе.
        let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
        let step = payload["data"]["steps"]
            .as_array()
            .and_then(|steps| steps.last())
            .cloned()
            .unwrap_or_else(|| panic!("шаг отказа пропал из ответа: {payload}"));
        assert_eq!(step["ok"], false, "{step}");
        assert_eq!(step["mode"], "edt_export", "{step}");
        assert!(
            step["message"].as_str().expect("message").contains(missing),
            "{step}"
        );
    }
}

/// Превью сборки EDT не грузит файлы конфигуратора в базу. Этот путь доходит и тогда,
/// когда этап EDT пропущен: каталог файлов уже есть, а состояние Конфигуратора устарело —
/// так бывает после оборванного боевого прогона. Запуск идёт против базы, поэтому
/// нарушение здесь дороже прочих.
#[test]
fn a_planned_edt_build_does_not_load_the_generated_designer_files() {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let config_path = dir.path().join("v8project.yaml");
    let platform_path = dir.path().join("platform").join("bin").join("1cv8");
    let edt_cli_path = dir.path().join("edt").join("1cedtcli");
    let edt_calls_log = dir.path().join("edt-calls.log");
    let v8_calls_log = dir.path().join("v8-calls.log");

    fs::create_dir_all(base_path.join("configuration")).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_native_edt_project(
        &base_path.join("configuration"),
        "configuration",
        V8_CONFIGURATION_NATURE,
        None,
    );
    fs::write(
        base_path
            .join("configuration")
            .join("src")
            .join("Configuration")
            .join("Module.bsl"),
        "procedure Test() endprocedure",
    )
    .expect("configuration bsl");
    write_edt_script(&edt_cli_path, &edt_calls_log);
    write_script(
        &platform_path,
        &format!(
            "args=\"$*\"\nout=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf 'designer log\\n' > \"$out\"; fi\nprintf '%s\\n' \"$args\" >> '{}'\nexit 0",
            v8_calls_log.display()
        ),
    );
    fs::write(
        &config_path,
        format!(
            "workPath: '{}'\nformat: EDT\ninfobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: '{}'\n  edt_cli:\n    path: '{}'\n",
            work_path.display(),
            platform_path.display(),
            edt_cli_path.display()
        ),
    )
    .expect("config");

    let seed = v8_runner_command()
        .args(["--config", &config_path.display().to_string(), "build"])
        .output()
        .expect("seed build");
    assert!(
        seed.status.success(),
        "{}",
        String::from_utf8_lossy(&seed.stderr)
    );

    // Состояние Конфигуратора теряется — так выглядит база после оборванного прогона.
    // Этап EDT при этом остаётся пройденным, и загрузка планируется заново.
    let designer_state = work_path
        .join("hash-storages")
        .join("designer-configuration.redb");
    fs::remove_file(&designer_state).expect("drop designer state");
    fs::remove_file(&v8_calls_log).expect("drop calls log");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(
        !v8_calls_log.exists(),
        "превью запустило Конфигуратор против базы: {}",
        fs::read_to_string(&v8_calls_log).unwrap_or_default()
    );
    assert!(
        !designer_state.exists(),
        "превью зафиксировало состояние обнаружения изменений"
    );

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let step = payload["data"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["mode"] == "full")
        .cloned()
        .unwrap_or_else(|| panic!("шаг загрузки пропал из ответа: {payload}"));
    assert_eq!(step["ok"], true, "{step}");
    assert!(
        step["message"]
            .as_str()
            .expect("message")
            .contains("planned"),
        "{step}"
    );
}

/// Превью сборки EDT не экспортирует внешние артефакты. Экспорт запускает EDT CLI,
/// пересоздаёт каталог в рабочем каталоге и фиксирует состояние обнаружения изменений —
/// превью не делает ничего из этого (`INV.CLI.PREVIEW-DISPATCHES-NOTHING`).
#[test]
fn a_planned_edt_build_does_not_export_the_external_artifacts() {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let config_path = dir.path().join("v8project.yaml");
    let platform_path = dir.path().join("platform").join("bin").join("1cv8");
    let edt_cli_path = dir.path().join("edt").join("1cedtcli");
    let edt_calls_log = dir.path().join("edt-calls.log");

    fs::create_dir_all(base_path.join("configuration")).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_native_edt_project(
        &base_path.join("configuration"),
        "configuration",
        V8_CONFIGURATION_NATURE,
        None,
    );
    write_edt_external_project(
        &base_path.join("processors").join("processor-a"),
        "ProcessorA",
    );
    write_build_script(&platform_path, None);
    write_edt_script(&edt_cli_path, &edt_calls_log);

    fs::write(
        &config_path,
        format!(
            "workPath: '{}'\nformat: EDT\ninfobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: project/configuration\n  - name: processors\n    type: EXTERNAL_DATA_PROCESSORS\n    path: project/processors\ntools:\n  platform:\n    path: '{}'\n  edt_cli:\n    path: '{}'\n",
            work_path.display(),
            platform_path.display(),
            edt_cli_path.display()
        ),
    )
    .expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    // Правило проверяется по отсутствию файлов, а не по отсутствию вызова, и проверяется
    // первым: иначе утечка свалила бы тест на исходе прогона, не дойдя до предмета.
    assert!(
        !edt_calls_log.exists(),
        "превью запустило EDT CLI: {}",
        fs::read_to_string(&edt_calls_log).unwrap_or_default()
    );
    // Подметается весь рабочий каталог, а не три имени: перечень пропустил бы и журнал
    // платформы, и staging, и всякий новый каталог.
    let left = left_in_work_path(&work_path);
    assert!(
        left.is_empty(),
        "превью оставило в рабочем каталоге: {left:?}"
    );

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["provider_dispatched"], false, "{payload}");
    let step = payload["data"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["source_set"] == "processors")
        .cloned()
        .unwrap_or_else(|| panic!("шаг внешнего набора пропал из ответа: {payload}"));
    assert_eq!(step["ok"], true, "{step}");
    assert_eq!(step["mode"], "edt_export", "{step}");
    let message = step["message"].as_str().expect("message");
    assert!(message.contains("planned"), "{message}");
    // Предмет назван: найденная утилита попадает в сообщение.
    assert!(
        message.contains(&edt_cli_path.display().to_string()),
        "{message}"
    );
}

#[test]
fn build_dry_run_plans_every_source_set_without_dispatching_designer() {
    let (dir, config_path, binary_path, work_path) = setup_project();
    let marker = dir.path().join("designer-ran.marker");
    // A fake that records the fact of being run at all.
    write_script(
        &binary_path,
        &format!("printf 'ran' > '{}'\nexit 0", marker.display()),
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
            "--full-rebuild",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let data = &payload["data"];
    assert_eq!(data["provider_dispatched"], false);
    assert_eq!(data["ok"], true);
    let steps = data["steps"].as_array().expect("steps");
    assert!(!steps.is_empty());
    for step in steps {
        assert_eq!(step["ok"], true);
        let message = step["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("planned") || step["mode"] == "skipped",
            "{step}"
        );
    }
    assert!(!marker.exists(), "preview must not dispatch Designer");
    // A planned build commits no change-detection state either. The state lives in
    // `workPath/hash-storages/<key>.redb`; the directory this once named never existed,
    // so the promise went unchecked and a real leak survived it (#252).
    assert!(
        !work_path.join("hash-storages").exists(),
        "preview committed change-detection state"
    );
}

#[test]
fn build_json_failure_returns_step_payload() {
    let (_dir, config_path, binary_path, _work_path) = setup_project();
    write_build_script(&binary_path, Some("/UpdateDBCfg -Extension ext"));

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
            "--full-rebuild",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "push");
    assert_eq!(payload["error"]["code"], "platform_failure");
    assert_eq!(payload["error"]["kind"], "platform");
    assert_eq!(payload["data"]["ok"], false);
    assert_eq!(payload["data"]["steps"][0]["source_set"], "main");
    assert_eq!(payload["data"]["steps"][0]["ok"], true);
    assert_eq!(payload["data"]["steps"][1]["source_set"], "ext");
    assert_eq!(payload["data"]["steps"][1]["ok"], false);
    assert!(payload["data"]["steps"][1]["message"]
        .as_str()
        .expect("message")
        .contains("exit code 17"));
}

#[test]
fn build_ibcmd_json_failure_reports_operation_target_and_exit_code() {
    let (_dir, config_path, binary_path, _work_path, _base_path, calls_log) = setup_ibcmd_project();
    write_ibcmd_script(&binary_path, &calls_log, Some("config apply"));

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
            "--full-rebuild",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "platform_failure");
    assert!(payload["data"]["steps"][0]["message"]
        .as_str()
        .expect("message")
        .contains("apply failed for source-set 'main' with exit code 17"));
}

#[test]
fn build_text_failure_does_not_print_success_footer() {
    let (_dir, config_path, binary_path, _work_path) = setup_project();
    write_build_script(&binary_path, Some("/UpdateDBCfg -Extension ext"));

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "build",
            "--full-rebuild",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Build failed"));
    assert!(!stdout.contains("Build completed successfully"));
}

#[test]
fn build_text_stdout_includes_action_logs() {
    let (_dir, config_path, _binary_path, _work_path) = setup_project();

    let output = v8_runner_command()
        .args([
            "--no-color",
            "--config",
            &config_path.display().to_string(),
            "build",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("● main:"));
    assert!(stdout.contains("│   Изменения: найдено"));
    assert!(stdout.contains("изменено"));
    assert!(stdout.contains("[Конфигуратор] Загрузка изменений в базу"));
    assert!(stdout.contains("│   ✓ partial load"));
    assert_eq!(stdout.matches("● main").count(), 1);
    assert!(stdout.contains("main"));
    assert!(stdout.contains("Build completed successfully"));
}

#[test]
fn build_text_highlights_timeline_detail_prefixes() {
    let (_dir, config_path, _binary_path, _work_path) = setup_project();

    // Вывод теста перенаправлен, а цвет включается только там, где его увидят.
    // `FORCE_COLOR` — тот же способ, которым его включает CI.
    let output = v8_runner_command()
        .env("FORCE_COLOR", "1")
        .args(["--config", &config_path.display().to_string(), "build"])
        .output()
        .expect("run designer build");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\x1b[1mmain\x1b[0m:"));
    assert!(stdout.contains("\x1b[1;34mИзменения\x1b[0m:"));
    assert!(stdout.contains("\x1b[1;34m[Конфигуратор]\x1b[0m"));
    assert!(stdout.contains("\x1b[1;32m✓\x1b[0m partial load"));

    let (_dir, config_path, _ibcmd_calls_log, _edt_calls_log) = setup_edt_ibcmd_project();
    let output = v8_runner_command()
        .env("FORCE_COLOR", "1")
        .args(["--config", &config_path.display().to_string(), "build"])
        .output()
        .expect("run edt build");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\x1b[1;34m[EDT]\x1b[0m"));
    assert!(stdout.contains("\x1b[1;34m[ibcmd]\x1b[0m"));
}

#[test]
fn build_text_workspace_lock_conflict_prints_single_error() {
    let (_dir, config_path, _binary_path, work_path) = setup_project();
    write_live_workspace_lock(&work_path, "build");

    let output = v8_runner_command()
        .args([
            "--no-color",
            "--config",
            &config_path.display().to_string(),
            "build",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let error_prefix = "runtime error: cannot start push";
    let combined = format!("{stdout}{stderr}");

    assert_eq!(
        combined.matches(error_prefix).count(),
        1,
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("ERROR: runtime error: cannot start push"),
        "stderr:\n{stderr}"
    );
    assert!(
        !stdout.contains(error_prefix),
        "stdout should not contain duplicate error log:\n{stdout}"
    );
}

#[test]
fn build_text_no_changes_collapses_per_source_set_noise() {
    let (_dir, config_path, _binary_path, _work_path) = setup_project();

    let first = v8_runner_command()
        .args([
            "--no-color",
            "--config",
            &config_path.display().to_string(),
            "build",
        ])
        .output()
        .expect("first build");
    assert!(first.status.success());

    let second = v8_runner_command()
        .args([
            "--no-color",
            "--config",
            &config_path.display().to_string(),
            "build",
        ])
        .output()
        .expect("second build");

    assert!(second.status.success());
    let stdout = String::from_utf8_lossy(&second.stdout);

    assert!(
        stdout.contains("Build completed: no changes"),
        "stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("changes - no changes"),
        "stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("skipped - no changes"),
        "stdout:\n{stdout}"
    );
}

#[test]
fn build_source_set_json_limits_steps_to_requested_source_set() {
    let (_dir, config_path, _binary_path, _work_path) = setup_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
            "--source-set",
            "ext",
            "--full-rebuild",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let steps = payload["data"]["steps"].as_array().expect("steps");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0]["source_set"], "ext");
    assert_eq!(steps[0]["mode"], "full");
}

#[test]
fn build_source_set_json_rejects_unknown_source_set() {
    let (_dir, config_path, _binary_path, _work_path) = setup_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
            "--source-set",
            "missing",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "push");
    assert_eq!(payload["error"]["kind"], "validation");
    assert_eq!(payload["error"]["message"], "unknown source-set 'missing'");
    assert_eq!(payload["data"]["steps"].as_array().expect("steps").len(), 0);
}

#[test]
fn build_edt_text_interleaves_export_stage_after_edt_log() {
    let (_dir, config_path, ibcmd_calls_log, edt_calls_log) = setup_edt_ibcmd_project();

    let output = v8_runner_command()
        .args([
            "--no-color",
            "--config",
            &config_path.display().to_string(),
            "build",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    let source_set_stage_index = lines
        .iter()
        .position(|line| *line == "● configuration:")
        .expect("source-set timeline stage");
    assert!(lines
        .iter()
        .skip(source_set_stage_index + 1)
        .any(|line| line.contains("[EDT] Конвертация в файлы конфигуратора")));
    assert!(stdout.contains("│   ✓ completed"));
    assert!(stdout.contains("[ibcmd] Загрузка в базу"));
    assert!(stdout.contains("[ibcmd] Применение изменений"));
    assert!(stdout.contains("│   ✓ full load selected by partial-load rules"));
    assert_eq!(stdout.matches("● configuration").count(), 1);

    let ibcmd_calls = fs::read_to_string(ibcmd_calls_log).expect("ibcmd calls");
    let edt_calls = fs::read_to_string(edt_calls_log).expect("edt calls");
    assert!(edt_calls.contains("export --project-name configuration"));
    assert!(ibcmd_calls.contains("config import"));
    assert!(ibcmd_calls.contains("config apply"));
}

#[test]
fn build_text_groups_tool_extension_stages_under_single_build_node() {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let config_path = dir.path().join("v8project.yaml");
    let platform_path = dir.path().join("platform").join("bin").join("1cv8");
    let edt_cli_path = dir.path().join("edt").join("1cedtcli");
    let edt_calls_log = dir.path().join("edt-calls.log");
    let tool_source = base_path.join("tools").join("client-mcp");

    fs::create_dir_all(&work_path).expect("work");
    write_native_edt_project(
        &base_path.join("configuration"),
        "configuration",
        V8_CONFIGURATION_NATURE,
        None,
    );
    write_native_edt_project(
        &tool_source,
        "client-mcp-project",
        V8_EXTENSION_NATURE,
        Some("configuration"),
    );
    write_build_script(&platform_path, None);
    write_edt_script(&edt_cli_path, &edt_calls_log);

    let config = format!(
        "workPath: '{}'\nformat: EDT\ninfobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n  - name: configuration\n    type: CONFIGURATION\n    path: project/configuration\ntools:\n  platform:\n    path: '{}'\n  edt_cli:\n    path: '{}'\n  client_mcp:\n    extension:\n      name: client_mcp\n      source:\n        path: '{}'\n        format: EDT\n",
        work_path.display(),
        platform_path.display(),
        edt_cli_path.display(),
        tool_source.display(),
    );
    fs::write(&config_path, config).expect("config");

    let output = v8_runner_command()
        .args([
            "--no-color",
            "--config",
            &config_path.display().to_string(),
            "build",
            "--full-rebuild",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches("◌ tool:client_mcp:").count(), 1);
    assert!(stdout.contains("[EDT] Экспорт расширения client_mcp"));
    assert!(stdout.contains("[Конфигуратор] Загрузка расширения client_mcp"));
    assert!(stdout.contains("[Конфигуратор] Применение расширения client_mcp"));
    assert!(stdout.contains("│   ✓ prepared extension 'client_mcp' from sources"));
    assert!(!stdout.contains("tool extension"));
    assert!(!stdout.contains("build: tool extension"));
}

#[test]
fn build_json_writes_action_log_file_without_polluting_stdout() {
    let (_dir, config_path, _binary_path, work_path) = setup_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let _payload: Value = serde_json::from_slice(&output.stdout).expect("json");

    let action_log = work_path.join("logs").join("mcp").join("actions.log");
    let contents = fs::read_to_string(action_log).expect("action log");
    assert!(contents.contains("main:"));
    assert!(contents.contains("Изменения: найдено"));
    assert!(contents.contains("[Конфигуратор] Загрузка изменений в базу"));
    assert!(contents.contains("✓ partial load"));
}

/// Проект с объявленным расширением клиентского MCP. Подставная утилита отмечает каждый
/// свой вызов, поэтому запуск платформы виден по файлу, а не по косвенным признакам.
fn setup_client_mcp_extension_project() -> (
    tempfile::TempDir,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
) {
    let (dir, config_path, binary_path, work_path) = setup_project();
    let tool_source = dir.path().join("project").join("exts").join("client-mcp");
    let calls_log = dir.path().join("calls.log");

    fs::create_dir_all(&tool_source).expect("tool source");
    fs::write(
        tool_source.join("Module.bsl"),
        "procedure Tool() endprocedure",
    )
    .expect("tool bsl");
    fs::write(
        tool_source.join("Configuration.xml"),
        "<Configuration><Properties><Name>client_mcp</Name><ConfigurationExtensionPurpose kind=\"Customization\">Customization</ConfigurationExtensionPurpose></Properties></Configuration>",
    )
    .expect("tool descriptor");
    // Сценарий и журнал платформы пишет, как остальные образцы, и отмечает свой вызов.
    write_script(
        &binary_path,
        &format!(
            "args=\"$*\"\nout=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf 'designer log for %s\\n' \"$args\" > \"$out\"; fi\nprintf '%s\\n' \"$args\" >> '{}'\nexit 0",
            calls_log.display()
        ),
    );
    write_client_mcp_config(&config_path, &work_path, &binary_path, &tool_source, "");

    (
        dir,
        config_path,
        binary_path,
        work_path,
        tool_source,
        calls_log,
    )
}

fn write_client_mcp_config(
    path: &Path,
    work_path: &Path,
    platform_path: &Path,
    tool_source: &Path,
    platform_extra: &str,
) {
    fs::write(
        path,
        format!(
            "workPath: '{}'\nformat: DESIGNER\ninfobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/main\ntools:\n  platform:\n    path: '{}'\n{platform_extra}  client_mcp:\n    extension:\n      name: client_mcp\n      source:\n        path: '{}'\n",
            work_path.display(),
            platform_path.display(),
            tool_source.display()
        ),
    )
    .expect("config");
}

/// Превью сборки не готовит расширение клиентского MCP. Подготовка запускает платформу
/// против базы и фиксирует состояние обнаружения изменений, а превью не делает ни того,
/// ни другого (`INV.CLI.PREVIEW-DISPATCHES-NOTHING`). Поиск утилиты превью при этом
/// проходит — иначе оно одобрило бы план, который боевой прогон выполнить не может.
#[test]
fn a_planned_build_does_not_prepare_the_client_mcp_extension() {
    let (_dir, config_path, binary_path, work_path, _tool_source, calls_log) =
        setup_client_mcp_extension_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["provider_dispatched"], false, "{payload}");

    assert!(
        !calls_log.exists(),
        "превью запустило платформу: {}",
        fs::read_to_string(&calls_log).unwrap_or_default()
    );
    // Состояние обнаружения изменений проверяется по отсутствию файла, а не по времени
    // правки: открытие базы состояния сдвигает mtime, ничего в неё не записав.
    assert!(
        !work_path
            .join("hash-storages")
            .join("tool-client_mcp-source.redb")
            .exists(),
        "превью зафиксировало состояние обнаружения изменений"
    );

    let step = payload["data"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["source_set"] == "tool:client_mcp")
        .cloned()
        .unwrap_or_else(|| panic!("шаг расширения пропал из ответа: {payload}"));
    assert_eq!(step["ok"], true, "{step}");
    let message = step["message"].as_str().expect("message");
    assert!(message.starts_with("would prepare"), "{message}");
    // Предмет назван: найденная утилита попадает в сообщение.
    assert!(
        message.contains(&binary_path.display().to_string()),
        "{message}"
    );
}

/// Отказ превью приходит и тогда, когда искать утилиту больше некому: наборы исходников
/// не изменились и пропускаются без поиска, а изменилось одно расширение. Поиск
/// обязателен (`INV.CLI.PREVIEW-RETURNS-AFTER-TOOL-LOOKUP`) — иначе превью одобрит план,
/// который боевой прогон выполнить не может.
#[test]
fn a_planned_build_refuses_when_only_the_extension_still_needs_a_missing_platform() {
    let (dir, config_path, _binary_path, work_path, tool_source, _calls_log) =
        setup_client_mcp_extension_project();

    // Боевой прогон запоминает состояние и наборов, и расширения.
    let seed = v8_runner_command()
        .args(["--config", &config_path.display().to_string(), "build"])
        .output()
        .expect("seed build");
    assert!(
        seed.status.success(),
        "{}",
        String::from_utf8_lossy(&seed.stderr)
    );

    // Меняется только расширение: наборы исходников останутся без изменений и пропустятся.
    fs::write(
        tool_source.join("Module.bsl"),
        "procedure Tool() // changed\nendprocedure",
    )
    .expect("modify extension");

    // Платформы больше нет. Строгий режим с версией не даёт локатору уйти в PATH или в
    // корни по умолчанию: отказать обязан поиск.
    let empty = dir.path().join("empty-platform");
    fs::create_dir_all(empty.join("bin")).expect("empty platform");
    write_client_mcp_config(
        &config_path,
        &work_path,
        &empty,
        &tool_source,
        "    strict: true\n    version: '8.3.27'\n",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    let reported = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "превью одобрило план без платформы: {reported}"
    );
    assert!(reported.contains("1cv8"), "{reported}");
}

#[test]
fn build_json_edt_extension_uses_full_load_and_writes_platform_log() {
    let (_dir, config_path, work_path) = setup_edt_extension_project();

    let first = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("prime build");
    assert!(first.status.success());

    fs::write(
        config_path
            .parent()
            .expect("config dir")
            .join("project")
            .join("exts")
            .join("client-mcp")
            .join("Module.bsl"),
        "procedure Test()\n  // changed after snapshot\nendprocedure",
    )
    .expect("modify extension");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "build",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["data"]["ok"], true);

    let platform_log = work_path
        .join("logs")
        .join("platform")
        .join("build-01-client_mcp-load.log");
    let contents = fs::read_to_string(platform_log).expect("platform log");
    assert!(contents.contains("/LoadConfigFromFiles"));
    assert!(contents.contains("-Extension client_mcp"));
    assert!(!contents.contains("-partial"));
}

#[test]
fn build_ibcmd_full_rebuild_invokes_import_and_apply() {
    let (_dir, config_path, _binary_path, _work_path, _base_path, calls_log) =
        setup_ibcmd_project();

    let output = v8_runner_command()
        .args([
            "--no-color",
            "--config",
            &config_path.display().to_string(),
            "build",
            "--full-rebuild",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("◌ main:"), "{stdout}");
    assert!(stdout.contains("[ibcmd] Загрузка в базу"));
    assert!(stdout.contains("[ibcmd] Применение изменений"));
    assert_eq!(stdout.matches("◌ main").count(), 1);
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains("config import"));
    assert!(calls.contains("config apply"));
}

#[test]
fn build_ibcmd_passes_credentials_to_import_and_apply() {
    let (dir, config_path, binary_path, work_path, _base_path, calls_log) = setup_ibcmd_project();
    let config = format!(
        "workPath: '{}'\nformat: DESIGNER\nproviders:\n  init: ibcmd\n  build: ibcmd\n  dump: ibcmd\n  infobase.configuration.export: ibcmd\ninfobase:\n  connection: 'File=/tmp/ib'\n  user: Admin\n  password: secret\nbuild:\n  partialLoadThreshold: 20\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/main\ntools:\n  platform:\n    path: '{}'\n",
        work_path.display(),
        binary_path.display(),
    );
    fs::write(&config_path, config).expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "build",
            "--full-rebuild",
        ])
        .current_dir(dir.path())
        .output()
        .expect("run command");

    assert!(output.status.success());
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(
        calls.contains("infobase --db-path /tmp/ib config import --user Admin --password secret")
    );
    assert!(
        calls.contains("infobase --db-path /tmp/ib config apply --user Admin --password secret")
    );
    assert!(calls.contains("--user Admin"));
    assert!(calls.contains("--password secret"));
}

#[test]
fn build_ibcmd_partial_uses_relative_positional_args_and_base_dir() {
    let (_dir, config_path, _binary_path, _work_path, base_path, calls_log) = setup_ibcmd_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "build",
            "--full-rebuild",
        ])
        .output()
        .expect("run command");
    assert!(output.status.success());

    let changed_file = base_path
        .join("main")
        .join("Catalogs.Items")
        .join("ObjectModule.bsl");
    fs::write(&changed_file, "procedure Test() // changed endprocedure").expect("change");

    let output = v8_runner_command()
        .args(["--config", &config_path.display().to_string(), "build"])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains("config import files"));
    assert!(calls.contains("--partial"));
    assert!(calls.contains("--base-dir "));
    assert!(calls.contains("Catalogs.Items/ObjectModule.bsl"));
}

#[test]
fn build_ibcmd_server_connection_fails_at_config_load() {
    let (dir, config_path, binary_path, _work_path, _base_path, _calls_log) = setup_ibcmd_project();
    write_config_with_builder(
        &config_path,
        &dir.path().join("project"),
        &dir.path().join("work"),
        &binary_path,
        "IBCMD",
        "Srvr=server;Ref=main",
    );

    let output = v8_runner_command()
        .args(["--config", &config_path.display().to_string(), "build"])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn build_ibcmd_server_connection_passes_dbms_and_infobase_credentials() {
    let (dir, config_path, binary_path, _work_path, _base_path, calls_log) = setup_ibcmd_project();
    write_config_with_builder_and_infobase(
        &config_path,
        &dir.path().join("project"),
        &dir.path().join("work"),
        &binary_path,
        "IBCMD",
        "  connection: 'Srvr=server;Ref=main'\n  user: Admin\n  password: secret\n  dbms:\n    kind: PostgreSQL\n    server: localhost\n    name: maindb\n    user: postgres\n    password: pg-secret\n",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "build",
            "--full-rebuild",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains("--dbms PostgreSQL --database-server localhost --database-name maindb"));
    assert!(calls.contains("--user Admin --password secret"));
    assert!(calls.contains("--database-user postgres --database-password pg-secret"));
    assert!(calls.contains("config import"));
    assert!(calls.contains("config apply"));
}

#[test]
fn build_ibcmd_accepts_raw_f_connection() {
    let (dir, config_path, binary_path, _work_path, _base_path, calls_log) = setup_ibcmd_project();
    write_config_with_builder(
        &config_path,
        &dir.path().join("project"),
        &dir.path().join("work"),
        &binary_path,
        "IBCMD",
        "/F /tmp/ib",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "build",
            "--full-rebuild",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains("--db-path /tmp/ib"));
    assert!(calls.contains("config apply"));
}
