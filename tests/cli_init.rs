#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script as write_script};

const V8_CONFIGURATION_NATURE: &str = "com._1c.g5.v8.dt.core.V8ConfigurationNature";
const V8_EXTENSION_NATURE: &str = "com._1c.g5.v8.dt.core.V8ExtensionNature";
const EDT_RUNTIME_VERSION: &str = "8.3.27";

/// Прежний глобальный `builder` в тестовых конфигах: `DESIGNER` — Конфигуратор первым,
/// `IBCMD` — `ibcmd` у создания базы: тесты здесь гоняют только его, а у кластера `ibcmd`
/// остался только в этой строке.
fn providers_yaml(builder: &str) -> &'static str {
    if builder == "IBCMD" {
        "providers:\n  init: ibcmd\n"
    } else {
        support::DESIGNER_LEADS
    }
}

fn write_native_edt_project(
    path: &Path,
    project_name: &str,
    nature: &str,
    base_project: Option<&str>,
) {
    fs::create_dir_all(path.join("metadata")).expect("metadata");
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

fn write_edt_configuration_source(path: &Path, project_name: &str) {
    write_native_edt_project(path, project_name, V8_CONFIGURATION_NATURE, None);
    fs::write(
        path.join("metadata").join("Configuration.xml"),
        "<Configuration />",
    )
    .expect("descriptor");
}

fn write_edt_extension_source(path: &Path, project_name: &str) {
    write_native_edt_project(path, project_name, V8_EXTENSION_NATURE, Some("main"));
    fs::write(
        path.join("metadata").join("Configuration.xml"),
        "<Configuration><ConfigurationExtensionPurpose>Extension</ConfigurationExtensionPurpose></Configuration>",
    )
    .expect("descriptor");
}

fn setup_designer_init_project() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    setup_designer_init_project_with_body(
        "if [ \"$1\" = \"CREATEINFOBASE\" ]; then mkdir -p \"$ib_path\" && : > \"$ib_path/1Cv8.1CD\"; fi\nexit 0",
    )
}

fn setup_designer_init_project_with_body(
    script_body: &str,
) -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let config_path = base_path.join("v8project.yaml");
    let v8_path = dir.path().join("1cv8");
    let infobase_path = dir.path().join("ib");

    fs::create_dir_all(base_path.join("main")).expect("main");
    fs::write(
        base_path.join("main").join("Configuration.xml"),
        "<Configuration/>\n",
    )
    .expect("main source");
    fs::create_dir_all(&work_path).expect("work");
    write_script(
        &v8_path,
        &format!(
            "printf '%s\\n' \"$*\" >> '{}'\n{}",
            dir.path().join("1cv8.calls.log").display(),
            script_body.replace("$ib_path", &infobase_path.display().to_string())
        ),
    );

    let config = format!(
        "workPath: '{}'\nformat: DESIGNER\ninfobase:\n  connection: 'File={}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\ntools:\n  platform:\n    path: '{}'\n",
        work_path.display(),
        infobase_path.display(),
        v8_path.display(),
    );
    fs::write(&config_path, config).expect("config");

    (dir, config_path, work_path, infobase_path)
}

fn setup_edt_init_project(
    format: &str,
    builder: &str,
    connection: &str,
) -> (
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
    let config_path = base_path.join("v8project.yaml");
    let platform_path = dir
        .path()
        .join(if builder == "IBCMD" { "ibcmd" } else { "1cv8" });
    let edt_path = dir.path().join("1cedtcli");
    let edt_calls_log = dir.path().join("edt.calls.log");
    let infobase_path = dir.path().join("ib");
    let resolved_connection = if connection == "__AUTO_FILE__" {
        format!("File={}", infobase_path.display())
    } else {
        connection.to_owned()
    };

    if format == "EDT" {
        write_edt_configuration_source(&base_path.join("main"), "main");
        write_edt_extension_source(&base_path.join("ext"), "ext");
    } else {
        fs::create_dir_all(base_path.join("main")).expect("main");
        fs::create_dir_all(base_path.join("ext")).expect("ext");
    }
    fs::create_dir_all(&work_path).expect("work");
    let platform_body = if builder == "IBCMD" {
        "if [ \"$1\" = \"infobase\" ]; then\n  shift\n  command=\"\"\n  path=\"\"\n  while [ \"$#\" -gt 0 ]; do\n    case \"$1\" in\n      create) command=create ;;\n      --db-path|--database-path) shift; path=$1 ;;\n      --db-path=*|--database-path=*) path=${1#*=} ;;\n    esac\n    shift\n  done\n  if [ \"$command\" = \"create\" ]; then\n    mkdir -p \"$path\" && : > \"$path/1Cv8.1CD\"\n  fi\nfi\nexit 0"
            .to_owned()
    } else {
        "if [ \"$1\" = \"CREATEINFOBASE\" ]; then\n  path=\"$2\"\n  path=${path#File=\\'}\n  path=${path%\\'}\n  mkdir -p \"$path\" && : > \"$path/1Cv8.1CD\"\nfi\nexit 0"
            .to_owned()
    };
    write_script(&platform_path, &platform_body);
    write_script(
        &edt_path,
        &format!(
            "args=\"$*\"\ntarget=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"--configuration-files\" ]; then target=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$target\" ]; then\n  mkdir -p \"$target\"\n  printf '<Configuration />\\n' > \"$target/Configuration.xml\"\n  printf '%s\\n' \"$args\" >> \"{}\"\n  exit 0\nfi\nprintf '%s\\n' \"$args\" >> \"{}\"\nexit 0",
            dir.path().join("edt.export.log").display(),
            edt_calls_log.display()
        ),
    );

    let config = format!(
        "workPath: '{}'\nformat: {}\n{}infobase:\n  connection: '{}'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\n  - name: ext\n    type: EXTENSION\n    path: ext\ntools:\n  platform:\n    path: '{}'\n  edt_cli:\n    path: '{}'\n",
        work_path.display(),
        format,
        providers_yaml(builder),
        resolved_connection,
        platform_path.display(),
        edt_path.display(),
    );
    fs::write(&config_path, config).expect("config");

    (
        dir,
        config_path,
        work_path,
        base_path,
        platform_path,
        edt_calls_log,
    )
}

/// Проект на базе в кластере: `script_body` — поддельный Конфигуратор, `dbms` — секция
/// `infobase.dbms`, `local` — местный слой. Вызовы платформы пишутся в журнал.
fn setup_cluster_init_project(
    script_body: &str,
    dbms: &str,
    local: &str,
) -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let config_path = base_path.join("v8project.yaml");
    let v8_path = dir.path().join("1cv8");
    let calls_log = dir.path().join("1cv8.calls.log");

    fs::create_dir_all(base_path.join("main")).expect("main");
    fs::write(
        base_path.join("main").join("Configuration.xml"),
        "<Configuration/>\n",
    )
    .expect("main source");
    fs::create_dir_all(&work_path).expect("work");
    write_script(
        &v8_path,
        &format!(
            "printf '%s\\n' \"$@\" >> '{}'\n{}\n",
            calls_log.display(),
            script_body
        ),
    );

    let config = format!(
        "workPath: '{}'\nformat: DESIGNER\ninfobase:\n  connection: 'Srvr=cluster:1541;Ref=demo'\n  user: Admin\n{dbms}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: main\ntools:\n  platform:\n    path: '{}'\n",
        work_path.display(),
        v8_path.display(),
    );
    fs::write(&config_path, config).expect("config");
    if !local.is_empty() {
        fs::write(config_path.with_file_name("v8project.local.yaml"), local).expect("local layer");
    }

    (dir, config_path, work_path, calls_log)
}

/// Секция СУБД со всеми реквизитами создания базы в кластере.
const FULL_DBMS: &str = "  dbms:\n    kind: PostgreSQL\n    server: db\n    name: demo_db\n    user: postgres\n    password: pg-s3cret\n    locale: ru\n";

/// Администратор кластера в местном слое.
const CLUSTER_ADMIN: &str =
    "infobases:\n  origin:\n    cluster:\n      user: cadm\n      password: c-s3cret\n";

fn run_infobase_create(config_path: &Path, extra: &[&str]) -> std::process::Output {
    v8_runner_command()
        .arg("--config")
        .arg(config_path)
        .args(["--json-message", "infobase", "create"])
        .args(extra)
        .output()
        .expect("run command")
}

fn json_of(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "json: {error}; stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn init_dry_run_plans_the_infobase_without_creating_it() {
    let (_dir, config_path, work_path, infobase_path) = setup_designer_init_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "infobase",
            "create",
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
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("json envelope");
    let data = &envelope["data"];
    assert_eq!(data["provider_dispatched"], false);
    let infobase_step = data["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["target"] == "infobase")
        .expect("infobase step");
    assert_eq!(infobase_step["status"], "planned");
    let message = infobase_step["message"].as_str().expect("message");
    assert!(
        message.contains("would create a file infobase"),
        "{message}"
    );
    assert!(
        message.contains(infobase_path.display().to_string().as_str()),
        "{message}"
    );
    // Nothing the step would create may exist afterwards.
    assert!(!infobase_path.join("1Cv8.1CD").exists());
    assert!(!work_path.join("edt-workspace").exists());
}

/// Без `ibcmd` файловую базу собирает запасной Конфигуратор: `CREATEINFOBASE`, затем
/// загрузка основной конфигурации без записи файла версий и обновление базы данных.
#[test]
fn init_designer_creates_infobase_and_skips_edt_workspace() {
    let (dir, config_path, work_path, infobase_path) = setup_designer_init_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "infobase",
            "create",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    assert!(infobase_path.join("1Cv8.1CD").exists());
    assert!(!work_path.join("edt-workspace").exists());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("infobase: create"));
    assert!(!stdout.contains("edt_workspace: import"));
    assert!(!stdout.contains("format=DESIGNER"));
    let calls = fs::read_to_string(dir.path().join("1cv8.calls.log")).expect("calls");
    let calls: Vec<_> = calls.lines().collect();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(calls[0].starts_with("CREATEINFOBASE File="), "{calls:?}");
    // Раннер называет набор каноническим путём: на macOS `/var` — ссылка на `/private/var`.
    let main = fs::canonicalize(config_path.with_file_name("main"))
        .expect("canonical main")
        .display()
        .to_string();
    assert!(
        calls[1].ends_with(&format!("/LoadConfigFromFiles {main}")),
        "{calls:?}"
    );
    assert!(calls[2].ends_with("/UpdateDBCfg"), "{calls:?}");
}

#[test]
fn init_designer_non_zero_create_exit_stays_fatal_even_when_marker_appears() {
    let (_dir, config_path, _work_path, _infobase_path) = setup_designer_init_project_with_body(
        "if [ \"$1\" = \"CREATEINFOBASE\" ]; then mkdir -p \"$ib_path\" && : > \"$ib_path/1Cv8.1CD\"; fi\nprintf 'designer create failed\\n' >&2\nexit 1",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "infobase",
            "create",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["steps"][0]["status"], "failed");
    assert!(payload["data"]["steps"][0]["message"]
        .as_str()
        .expect("message")
        .contains("designer create failed"));
}

/// У проекта EDT рабочая область заводится до базы: исходники для неё переводятся оттуда.
/// Отказ базы идёт в ленте после импорта и не отменяет его.
#[test]
fn init_text_reports_the_edt_import_before_the_infobase_failure() {
    let (_dir, config_path, _work_path, _base_path, platform_path, edt_calls_log) =
        setup_edt_init_project("EDT", "DESIGNER", "__AUTO_FILE__");
    write_script(
        &platform_path,
        "printf 'designer create failed\\n' >&2\nexit 1",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "infobase",
            "create",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let failed_step = stdout
        .find("✗ infobase: create")
        .expect("live failed infobase status");
    let edt_import = stdout
        .find("importing source-set project")
        .expect("continued edt import");
    let final_summary = stdout.find("Init failed").expect("final summary");
    assert!(edt_import < failed_step);
    assert!(failed_step < final_summary);
    assert!(stdout.contains("✓ edt_workspace: import"));
    assert!(edt_calls_log.exists());
}

#[test]
fn init_ibcmd_creates_infobase_and_imports_edt_projects_in_order() {
    let (_dir, config_path, work_path, _base_path, _platform_path, edt_calls_log) =
        setup_edt_init_project("DESIGNER", "IBCMD", "__AUTO_FILE__");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "infobase",
            "create",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let config = fs::read_to_string(&config_path).expect("config");
    let connection_line = config
        .lines()
        .find(|line| line.trim_start().starts_with("connection:"))
        .expect("connection line");
    let infobase_dir = connection_line
        .split("File=")
        .nth(1)
        .expect("file path")
        .trim_matches('\'');
    assert!(Path::new(infobase_dir).join("1Cv8.1CD").exists());
    assert!(!work_path.join("edt-workspace").exists());
    assert!(!edt_calls_log.exists());
}

/// Неудачное создание файловой базы `ibcmd` — отказ по коду выхода: второго вопроса к
/// базе нет, иначе база, оставленная неудачным импортом, назвалась бы «уже была».
#[test]
fn a_failed_ibcmd_create_of_a_file_base_is_fatal_and_asks_nothing_more() {
    let (dir, config_path, _work_path, _base_path, platform_path, _edt_calls_log) =
        setup_edt_init_project("DESIGNER", "IBCMD", "__AUTO_FILE__");
    let calls_log = dir.path().join("ibcmd.calls.log");
    write_script(
        &platform_path,
        &format!(
            "printf '%s\\n' \"$*\" >> '{}'\nif printf '%s' \"$*\" | grep -F -q -- 'generation-id'; then exit 0; fi\nif [ \"$1\" = \"infobase\" ]; then printf 'already exists\\n' >&2; exit 17; fi\nexit 0",
            calls_log.display()
        ),
    );

    let output = run_infobase_create(&config_path, &[]);

    assert!(!output.status.success());
    let payload = json_of(&output);
    assert_eq!(payload["data"]["steps"][0]["status"], "failed");
    let message = payload["data"]["steps"][0]["message"]
        .as_str()
        .expect("message");
    assert!(message.contains("exit code 17"), "{message}");
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(!calls.contains("generation-id"), "{calls}");
}

/// Файловую базу `ibcmd` создаёт сразу с основной конфигурацией из исходников, и память
/// знает собранный набор: первая отправка его не грузит, а расширение досылает целиком.
#[test]
fn a_file_base_is_created_by_ibcmd_with_the_main_configuration_and_remembers_it() {
    let (dir, config_path, _work_path, base_path, platform_path, _edt_calls_log) =
        setup_edt_init_project("DESIGNER", "IBCMD", "__AUTO_FILE__");
    fs::write(
        base_path.join("main").join("Configuration.xml"),
        "<Configuration/>\n",
    )
    .expect("main source");
    fs::write(
        base_path.join("ext").join("Configuration.xml"),
        "<Configuration/>\n",
    )
    .expect("extension source");
    let calls_log = dir.path().join("ibcmd.calls.log");
    let body = fs::read_to_string(&platform_path).expect("platform");
    write_script(
        &platform_path,
        &format!(
            "printf '%s\\n' \"$*\" >> '{}'\n{}",
            calls_log.display(),
            body.lines()
                .skip_while(|line| line.starts_with("#!"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );

    let output = run_infobase_create(&config_path, &[]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let payload = json_of(&output);
    assert_eq!(payload["data"]["steps"][0]["status"], "ok");
    let calls = fs::read_to_string(&calls_log).expect("calls");
    let create = calls
        .lines()
        .find(|line| line.contains(" create"))
        .expect("create call");
    assert!(
        create.ends_with(&format!(
            "create --import={} --apply --force",
            fs::canonicalize(base_path.join("main"))
                .expect("canonical main")
                .display()
        )),
        "{create}"
    );

    let push = v8_runner_command()
        .arg("--config")
        .arg(&config_path)
        .args(["--json-message", "push", "--dry-run"])
        .output()
        .expect("push preview");
    assert!(
        push.status.success(),
        "{}",
        String::from_utf8_lossy(&push.stdout)
    );
    let push = json_of(&push);
    let steps = push["data"]["steps"].as_array().expect("steps");
    let mode = |name: &str| {
        steps
            .iter()
            .find(|step| step["source_set"] == name)
            .map(|step| step["mode"].clone())
            .unwrap_or_else(|| panic!("{name} step in {push}"))
    };
    assert_eq!(mode("main"), "skipped", "{push}");
    assert_eq!(mode("ext"), "full", "{push}");
}

/// Существующая файловая база — отказ, и превью называет его тем же отказом, ничего не
/// запуская.
#[test]
fn an_existing_file_base_is_refused_and_the_preview_names_it() {
    let (dir, config_path, _work_path, infobase_path) = setup_designer_init_project();
    fs::create_dir_all(&infobase_path).expect("infobase");
    fs::write(infobase_path.join("1Cv8.1CD"), "database").expect("infobase file");

    for extra in [&["--dry-run"][..], &[][..]] {
        let output = run_infobase_create(&config_path, extra);

        assert!(!output.status.success(), "{extra:?}");
        let payload = json_of(&output);
        assert_eq!(payload["error"]["kind"], "validation", "{payload}");
        assert_eq!(payload["data"]["steps"][0]["status"], "failed");
        let message = payload["error"]["message"].as_str().expect("message");
        assert!(message.contains("already exists"), "{message}");
    }
    assert!(!dir.path().join("1cv8.calls.log").exists());
    assert_eq!(
        fs::read_to_string(infobase_path.join("1Cv8.1CD")).expect("infobase"),
        "database"
    );
}

/// Автономный сервер получает отказ рода подбора с рецептом создания базы на его машине.
#[test]
fn a_standalone_target_is_refused_with_the_recipe() {
    let (dir, config_path, _work_path, _infobase_path) = setup_designer_init_project();
    let config = fs::read_to_string(&config_path).expect("config");
    let connection = config
        .lines()
        .find(|line| line.trim_start().starts_with("connection:"))
        .expect("connection")
        .to_owned();
    fs::write(
        &config_path,
        config.replace(
            &connection,
            "  user: gate\n  password: gate-secret\n  standalone:\n    gate: 127.0.0.1:1543\n    exchange: sftp",
        ),
    )
    .expect("standalone config");

    for extra in [&["--dry-run"][..], &[][..]] {
        let output = run_infobase_create(&config_path, extra);

        assert!(!output.status.success(), "{extra:?}");
        let payload = json_of(&output);
        assert_eq!(payload["error"]["kind"], "capability", "{payload}");
        assert_eq!(payload["error"]["code"], "target", "{payload}");
        let message = payload["error"]["message"].as_str().expect("message");
        assert!(
            message.contains("ibcmd server config init")
                && message.contains("ibcmd infobase create"),
            "{message}"
        );
    }
    assert!(!dir.path().join("1cv8.calls.log").exists());
}

/// Проект EDT: сначала рабочая область, затем основной набор переводится в XML тем же
/// переводом, что у `push`, и `ibcmd` собирает базу из этого каталога; память знает
/// собранный набор, и первая отправка его не переводит и не грузит.
#[test]
fn a_file_base_of_an_edt_project_is_assembled_by_ibcmd_from_its_sources() {
    let (dir, config_path, work_path, base_path, platform_path, edt_calls_log) =
        setup_edt_init_project("EDT", "IBCMD", "__AUTO_FILE__");
    let ibcmd_calls = platform_path.with_file_name("ibcmd.calls.log");
    let body = fs::read_to_string(&platform_path).expect("platform");
    write_script(
        &platform_path,
        &format!(
            "printf '%s\\n' \"$*\" >> '{}'\n{}",
            ibcmd_calls.display(),
            body.lines()
                .skip_while(|line| line.starts_with("#!"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "infobase",
            "create",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let config = fs::read_to_string(&config_path).expect("config");
    let connection_line = config
        .lines()
        .find(|line| line.trim_start().starts_with("connection:"))
        .expect("connection line");
    let infobase_dir = connection_line
        .split("File=")
        .nth(1)
        .expect("file path")
        .trim_matches('\'');
    assert!(Path::new(infobase_dir).join("1Cv8.1CD").exists());
    assert!(work_path.join("edt-workspace").exists());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("importing source-set project 'main'"));
    assert!(stdout.contains("importing source-set project 'ext'"));
    assert!(stdout.contains("imported EDT projects: main, ext"));
    let calls = fs::read_to_string(edt_calls_log).expect("calls");
    let lines: Vec<_> = calls.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains(&base_path.join("main").display().to_string()));
    assert!(lines[1].contains(&base_path.join("ext").display().to_string()));
    let exports = fs::read_to_string(dir.path().join("edt.export.log")).expect("edt export");
    let exports: Vec<_> = exports.lines().collect();
    assert_eq!(exports.len(), 1, "{exports:?}");
    assert!(exports[0].contains("--project-name main"), "{exports:?}");
    let xml = exports[0]
        .split("--configuration-files ")
        .nth(1)
        .expect("export target");
    assert!(Path::new(xml).join("Configuration.xml").is_file());
    let create = fs::read_to_string(&ibcmd_calls).expect("ibcmd calls");
    assert!(
        create.contains(&format!(" create --import={xml} --apply --force")),
        "{create}"
    );

    let push = v8_runner_command()
        .arg("--config")
        .arg(&config_path)
        .args(["--json-message", "push", "--dry-run"])
        .output()
        .expect("push preview");
    let push: Value = serde_json::from_slice(&push.stdout).expect("push json");
    let steps = push["data"]["steps"].as_array().expect("steps");
    let main = steps
        .iter()
        .find(|step| step["source_set"] == "main")
        .unwrap_or_else(|| panic!("main in {push}"));
    assert_eq!(main["mode"], "skipped", "{push}");
}

#[test]
fn init_edt_imports_projects_in_configuration_then_extension_order() {
    let (_dir, config_path, work_path, base_path, _platform_path, edt_calls_log) =
        setup_edt_init_project("EDT", "DESIGNER", "__AUTO_FILE__");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "infobase",
            "create",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let config = fs::read_to_string(&config_path).expect("config");
    let connection_line = config
        .lines()
        .find(|line| line.trim_start().starts_with("connection:"))
        .expect("connection line");
    let infobase_dir = connection_line
        .split("File=")
        .nth(1)
        .expect("file path")
        .trim_matches('\'');
    assert!(Path::new(infobase_dir).join("1Cv8.1CD").exists());
    assert!(work_path.join("edt-workspace").exists());
    let calls = fs::read_to_string(edt_calls_log).expect("calls");
    let lines: Vec<_> = calls.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains(&base_path.join("main").display().to_string()));
    assert!(lines[1].contains(&base_path.join("ext").display().to_string()));
}

/// База в кластере без реквизитов СУБД — отказ шага базы до запуска платформы; шаг
/// рабочей области идёт своим чередом.
#[test]
fn init_non_file_connection_keeps_running_workspace_step_and_returns_payload() {
    let (_dir, config_path, work_path, base_path, _platform_path, edt_calls_log) =
        setup_edt_init_project("EDT", "DESIGNER", "Srvr=demo;Ref=test");

    let output = run_infobase_create(&config_path, &[]);

    assert!(!output.status.success());
    let payload = json_of(&output);
    assert_eq!(payload["command"], "infobase create");
    assert_eq!(payload["data"]["steps"][1]["status"], "failed");
    assert!(payload["data"]["steps"][1]["message"]
        .as_str()
        .expect("message")
        .contains("infobase.dbms.kind"));
    assert_eq!(payload["data"]["steps"][0]["status"], "ok");
    assert!(work_path.join("edt-workspace").exists());
    let calls = fs::read_to_string(edt_calls_log).expect("calls");
    let lines: Vec<_> = calls.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains(&base_path.join("main").display().to_string()));
}

#[test]
fn init_skips_existing_workspace() {
    let (_dir, config_path, work_path, _base_path, _platform_path, edt_calls_log) =
        setup_edt_init_project("EDT", "DESIGNER", "__AUTO_FILE__");
    fs::create_dir_all(work_path.join("edt-workspace")).expect("workspace");
    fs::write(
        work_path.join("edt-workspace").join(".v8tr-initialized"),
        "ok\n",
    )
    .expect("marker");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "infobase",
            "create",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["steps"][0]["status"], "skipped");
    assert!(!edt_calls_log.exists());
}

#[test]
fn init_retries_edt_import_when_previous_run_left_incomplete_workspace() {
    let (_dir, config_path, work_path, base_path, _platform_path, edt_calls_log) =
        setup_edt_init_project("EDT", "DESIGNER", "__AUTO_FILE__");
    let edt_path = work_path.parent().expect("parent").join("1cedtcli");
    // Перевод в XML поддельный EDT делает, импорт проекта в рабочую область — нет.
    let original = fs::read_to_string(&edt_path).expect("edt script");
    let original: String = original
        .lines()
        .skip_while(|line| line.starts_with("#!"))
        .collect::<Vec<_>>()
        .join("\n");
    write_script(
        &edt_path,
        &format!(
            "case \"$*\" in *\"-command import\"*) printf '%s\\n' \"$*\" >> \"{}\"; exit 1;; esac\n{original}",
            edt_calls_log.display()
        ),
    );

    let first = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "infobase",
            "create",
        ])
        .output()
        .expect("first run");

    assert!(!first.status.success());
    let first_payload: Value = serde_json::from_slice(&first.stdout).expect("json");
    assert_eq!(first_payload["command"], "infobase create");
    // Без рабочей области исходники EDT не перевести: база не создаётся.
    assert_eq!(first_payload["data"]["steps"][0]["status"], "failed");
    assert_eq!(first_payload["data"]["steps"][1]["status"], "failed");
    assert!(first_payload["data"]["steps"][1]["message"]
        .as_str()
        .expect("message")
        .contains("the workspace was not initialized"));
    assert!(work_path.join("edt-workspace").exists());
    assert!(!work_path
        .join("edt-workspace")
        .join(".v8tr-initialized")
        .exists());
    let first_calls = fs::read_to_string(&edt_calls_log).expect("calls");
    let first_lines: Vec<_> = first_calls.lines().collect();
    assert_eq!(first_lines.len(), 1);
    assert!(first_lines[0].contains(&base_path.join("main").display().to_string()));

    write_script(&edt_path, &original);

    let second = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "infobase",
            "create",
        ])
        .output()
        .expect("second run");

    // Повтор доимпортирует рабочую область и создаёт базу.
    assert!(second.status.success());
    let payload: Value = serde_json::from_slice(&second.stdout).expect("json");
    assert_eq!(payload["data"]["steps"][0]["status"], "ok");
    assert_eq!(payload["data"]["steps"][1]["status"], "ok");
    assert!(work_path
        .join("edt-workspace")
        .join(".v8tr-initialized")
        .exists());
    let calls = fs::read_to_string(edt_calls_log).expect("calls");
    let lines: Vec<_> = calls.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[1].contains(&base_path.join("main").display().to_string()));
    assert!(lines[2].contains(&base_path.join("ext").display().to_string()));
}

#[test]
fn init_rejects_workspace_path_that_is_not_a_directory() {
    let (_dir, config_path, work_path, _base_path, _platform_path, _edt_calls_log) =
        setup_edt_init_project("EDT", "DESIGNER", "__AUTO_FILE__");
    fs::write(work_path.join("edt-workspace"), "not a dir\n").expect("workspace file");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "infobase",
            "create",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["data"]["steps"][0]["status"], "failed");
    assert!(payload["data"]["steps"][0]["message"]
        .as_str()
        .expect("message")
        .contains("is not a directory"));
}

/// Базу в кластере Конфигуратор создаёт одной командой `CREATEINFOBASE` с клиент-серверной
/// строкой: адрес из подключения, реквизиты СУБД из `dbms`, администратор кластера из
/// `cluster`. Пароли в ответ не попадают.
#[test]
fn a_cluster_base_is_created_by_the_designer_with_the_client_server_string() {
    let (_dir, config_path, _work_path, calls_log) =
        setup_cluster_init_project("exit 0", FULL_DBMS, CLUSTER_ADMIN);

    let output = run_infobase_create(&config_path, &[]);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    let payload = json_of(&output);
    assert_eq!(payload["data"]["provider"]["selected"], "designer");
    let step = &payload["data"]["steps"][0];
    assert_eq!(step["status"], "ok", "{payload}");
    assert!(
        step["message"]
            .as_str()
            .expect("message")
            .contains("the first push loads every source-set in full"),
        "{payload}"
    );
    let calls: Vec<String> = fs::read_to_string(&calls_log)
        .expect("calls")
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        calls,
        [
            "CREATEINFOBASE",
            "Srvr=cluster:1541;Ref=demo;DBMS=PostgreSQL;DBSrvr=db;DB=demo_db;DBUID=postgres;DBPwd=pg-s3cret;CrSQLDB=Y;Locale=ru;SchJobDn=Y;SUsr=cadm;SPwd=c-s3cret",
            "/DisableStartupDialogs",
        ]
    );
    for secret in ["pg-s3cret", "c-s3cret"] {
        assert!(!stdout.contains(secret), "{secret} in {stdout}");
    }

    // Базу Конфигуратор создал пустой: память знает только, что база есть, и первая
    // отправка грузит набор целиком без отказа первого знакомства.
    let push = v8_runner_command()
        .arg("--config")
        .arg(&config_path)
        .args(["--json-message", "push", "--dry-run"])
        .output()
        .expect("push preview");
    let push = json_of(&push);
    assert_eq!(push["ok"], true, "{push}");
    assert_eq!(push["data"]["steps"][0]["source_set"], "main", "{push}");
    assert_eq!(push["data"]["steps"][0]["mode"], "full", "{push}");
}

/// Без обязательного реквизита `dbms` — вида СУБД, сервера, имени базы данных, `locale` —
/// отказ до запуска платформы называет ключ; без `locale` — и почему: платформа оставила бы в
/// СУБД брошенную базу данных.
#[test]
fn a_cluster_base_without_a_required_dbms_field_is_refused_before_the_platform_starts() {
    for (field, line) in [
        ("kind", "    kind: PostgreSQL\n"),
        ("server", "    server: db\n"),
        ("name", "    name: demo_db\n"),
        ("locale", "    locale: ru\n"),
    ] {
        let dbms = FULL_DBMS.replace(line, "");
        let (_dir, config_path, _work_path, calls_log) =
            setup_cluster_init_project("exit 0", &dbms, CLUSTER_ADMIN);

        for extra in [&["--dry-run"][..], &[][..]] {
            let output = run_infobase_create(&config_path, extra);

            assert!(!output.status.success(), "{field} {extra:?}");
            let payload = json_of(&output);
            assert_eq!(payload["error"]["kind"], "validation", "{payload}");
            let message = payload["error"]["message"].as_str().expect("message");
            assert!(
                message.contains(&format!("infobase.dbms.{field} is not declared")),
                "{message}"
            );
            if field == "locale" {
                assert!(message.contains("abandoned database"), "{message}");
            }
        }
        assert!(!calls_log.exists(), "{field}");
    }
}

/// Превью базы в кластере называет цель и утилиту и честно говорит, что «уже есть» до
/// создания не наблюдается; платформа не запускается.
#[test]
fn a_cluster_preview_names_the_target_and_starts_nothing() {
    let (_dir, config_path, _work_path, calls_log) =
        setup_cluster_init_project("exit 0", FULL_DBMS, CLUSTER_ADMIN);

    let output = run_infobase_create(&config_path, &["--dry-run"]);

    assert!(output.status.success());
    let payload = json_of(&output);
    let step = &payload["data"]["steps"][0];
    assert_eq!(step["status"], "planned");
    let message = step["message"].as_str().expect("message");
    assert!(
        message.contains("server infobase 'demo' on 'cluster:1541'")
            && message.contains("the database 'demo_db' on 'db'")
            && message.contains("not observable")
            && message.contains("silently takes an existing database")
            && message.contains("even one holding another infobase")
            && message.contains("abandoned in the DBMS"),
        "{message}"
    );
    assert!(!calls_log.exists());
}

/// Кластер с заполненным списком администраторов: без `SUsr` платформа отказывает. Раннер
/// причину по её прозе не угадывает, но без объявленного администратора кластера отказ
/// называет этот уровень и его ключи.
#[test]
fn a_cluster_with_administrators_refuses_naming_the_cluster_administrator_level() {
    let (_dir, config_path, _work_path, calls_log) = setup_cluster_init_project(
        "if printf '%s' \"$*\" | grep -F -q 'SUsr='; then exit 0; fi\nexit 1",
        FULL_DBMS,
        "",
    );

    let output = run_infobase_create(&config_path, &[]);

    assert!(!output.status.success());
    let payload = json_of(&output);
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("cluster administrator level")
            && message.contains("infobase.cluster.user")
            && message.contains("infobase.cluster.password"),
        "{message}"
    );
    assert!(!fs::read_to_string(calls_log)
        .expect("calls")
        .contains("SUsr="));
}

/// Отказ создания в кластере повторяет вывод платформы, а она бывает и со всей строкой
/// соединения: пароли СУБД и администратора кластера в ответ не попадают.
#[test]
fn a_failed_cluster_create_never_echoes_the_passwords() {
    let (_dir, config_path, _work_path, _calls_log) = setup_cluster_init_project(
        "printf 'Создание информационной базы (\"%s\") не выполнено\\n' \"$2\"\nprintf '%s\\n' \"$2\" >&2\nexit 1",
        FULL_DBMS,
        CLUSTER_ADMIN,
    );

    let output = run_infobase_create(&config_path, &[]);

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let payload = json_of(&output);
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(message.contains("exit code 1"), "{message}");
    assert!(message.contains("DBPwd=***"), "{message}");
    for secret in ["pg-s3cret", "c-s3cret"] {
        assert!(!stdout.contains(secret), "{secret} in {stdout}");
        assert!(!stderr.contains(secret), "{secret} in {stderr}");
    }
}

/// Строка из `infobases.<имя>.connection` местного слоя бывает с `Usr=`/`Pwd=`: ни
/// превью, ни ответ живого прогона её не печатают — базу называет `describe_target`, а
/// строка создания берёт из неё только `Srvr` и `Ref` (INV.CLI.SECRETS-NEVER-REACH-THE-OUTPUT).
#[test]
fn server_infobase_create_never_echoes_the_connection_string_credentials() {
    for (dry_run, expected) in [(true, "planned"), (false, "ok")] {
        let (_dir, config_path, _work_path, calls_log) = setup_cluster_init_project(
            "exit 0",
            FULL_DBMS,
            // An address the project file does not declare: seeing it in the message proves the
            // credential-bearing string from the local layer is the one the command used.
            "infobases:\n  origin:\n    connection: 'Srvr=cluster-local:1641;Ref=demo_local;Usr=ConnUser;Pwd=conn-s3cret'\n",
        );
        let output = run_infobase_create(&config_path, if dry_run { &["--dry-run"] } else { &[] });

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "stdout={stdout} stderr={stderr}");
        let payload = json_of(&output);
        let step = &payload["data"]["steps"][0];
        assert_eq!(step["status"], expected, "{stdout}");
        let message = step["message"].as_str().expect("message");
        assert!(
            message.contains("server infobase 'demo_local' on 'cluster-local:1641' as 'Admin'"),
            "{message}"
        );
        for leaked in ["conn-s3cret", "Pwd=", "Usr=", "ConnUser"] {
            assert!(!stdout.contains(leaked), "{expected}: {leaked} in {stdout}");
            assert!(!stderr.contains(leaked), "{expected}: {leaked} in {stderr}");
        }
        // The preview dispatches nothing; a live run reaches the Designer with Srvr and Ref only.
        assert_eq!(calls_log.exists(), !dry_run, "{expected}");
        if !dry_run {
            let calls = fs::read_to_string(&calls_log).expect("calls");
            assert!(
                calls.contains("Srvr=cluster-local:1641;Ref=demo_local;DBMS=PostgreSQL")
                    && !calls.contains("conn-s3cret"),
                "{calls}"
            );
        }
    }
}

/// Неудачное создание `ibcmd`, после которого файл базы всё же появился, называет
/// оставленный каталог и оба выхода: удалить и создать заново или загрузить поверх.
#[test]
fn a_failed_ibcmd_create_that_left_a_base_names_the_directory_and_the_ways_out() {
    let (dir, config_path, _work_path, base_path, platform_path, _edt_calls_log) =
        setup_edt_init_project("DESIGNER", "IBCMD", "__AUTO_FILE__");
    fs::write(
        base_path.join("main").join("Configuration.xml"),
        "<Configuration/>\n",
    )
    .expect("main source");
    let body = fs::read_to_string(&platform_path).expect("platform");
    write_script(
        &platform_path,
        &body
            .replace("\nexit 0", "\nexit 255")
            .replace("#!/bin/sh\n", ""),
    );

    let output = run_infobase_create(&config_path, &[]);

    assert!(!output.status.success());
    let payload = json_of(&output);
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(
        message.contains(&format!(
            "the directory '{}' is left with a partly created infobase",
            dir.path().join("ib").display()
        )) && message.contains("remove the directory and run infobase create again")
            && message.contains("push --force"),
        "{message}"
    );
}
