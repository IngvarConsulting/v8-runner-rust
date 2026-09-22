#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script as write_script};

const V8_CONFIGURATION_NATURE: &str = "com._1c.g5.v8.dt.core.V8ConfigurationNature";
const EDT_RUNTIME_VERSION: &str = "8.3.27";

fn write_edt_configuration_source(path: &Path, project_name: &str) {
    fs::create_dir_all(path.join("metadata")).expect("metadata");
    fs::create_dir_all(path.join("DT-INF")).expect("dt-inf");
    fs::create_dir_all(path.join("src").join("Configuration")).expect("src");
    fs::write(
        path.join(".project"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<projectDescription>\n  <name>{project_name}</name>\n  <natures>\n    <nature>{V8_CONFIGURATION_NATURE}</nature>\n  </natures>\n</projectDescription>\n"
        ),
    )
    .expect("project");
    fs::write(
        path.join("DT-INF").join("PROJECT.PMF"),
        format!("Manifest-Version: 1.0\nRuntime-Version: {EDT_RUNTIME_VERSION}\n"),
    )
    .expect("manifest");
    fs::write(
        path.join("metadata").join("Configuration.xml"),
        "<Configuration />",
    )
    .expect("descriptor");
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

fn write_config(
    path: &Path,
    _base_path: &Path,
    work_path: &Path,
    platform_path: &Path,
    format: &str,
    edt_cli_path: Option<&Path>,
) {
    let edt_section = edt_cli_path
        .map(|path| format!("  edt_cli:\n    path: '{}'\n", path.display()))
        .unwrap_or_default();
    let config = format!(
        "workPath: '{}'\nformat: {}\ninfobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\ntools:\n  platform:\n    path: '{}'\n{}",
        work_path.display(),
        format,
        platform_path.display(),
        edt_section
    );
    fs::write(path, config).expect("config");
}

fn setup_project(script_body: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let install_dir = dir.path().join("platform");
    let config_path = base_path.join("v8project.yaml");

    fs::create_dir_all(&base_path).expect("base");
    fs::create_dir_all(&work_path).expect("work");
    write_script(&install_dir.join("bin").join("1cv8"), script_body);
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &install_dir,
        "DESIGNER",
        None,
    );

    (dir, config_path)
}

fn setup_edt_project(script_body: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = temp_workspace();
    let base_path = dir.path().join("project");
    let work_path = dir.path().join("work");
    let install_dir = dir.path().join("platform");
    let edt_cli = dir.path().join("edt").join("1cedtcli");
    let config_path = base_path.join("v8project.yaml");

    fs::create_dir_all(&base_path).expect("base");
    write_edt_configuration_source(&base_path, "main");
    fs::create_dir_all(&work_path).expect("work");
    write_script(&install_dir.join("bin").join("1cv8"), "exit 0");
    write_script(&edt_cli, script_body);
    write_config(
        &config_path,
        &base_path,
        &work_path,
        &install_dir,
        "EDT",
        Some(&edt_cli),
    );

    (dir, config_path)
}

/// Превью доходит до поиска утилиты и возвращается до первой записи: каталога журналов
/// платформы не появляется, а Конфигуратор не запускается — его подставной сценарий
/// отвечает ненулевым кодом, и боевой прогон на нём отказал бы.
#[test]
fn a_preview_of_the_configuration_check_plans_without_running_the_designer() {
    let (dir, config_path) = setup_project("exit 3");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "check",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["data"]["status"], "planned");
    assert_eq!(payload["data"]["provider_dispatched"], false);
    assert_eq!(payload["data"]["check_name"], "designer-config");
    // Кода выхода не наблюдалось: платформа не запускалась.
    assert_eq!(payload["data"]["exit_code"], -1);
    // Журнала не будет, поэтому и путь к нему не называется.
    assert!(payload["data"]["platform_log_path"].is_null(), "{payload}");
    // Предмет назван: и режимы, и найденная утилита.
    let message = payload["data"]["message"].as_str().expect("message");
    assert!(message.contains("/CheckConfig -ThinClient"), "{message}");
    assert!(message.contains("1cv8"), "{message}");
    // Квитанция о выборе исполнителя остаётся — поиск утилиты превью проходит.
    assert!(!payload["data"]["provider"].is_null(), "{payload}");

    assert!(
        !dir.path().join("work/logs/platform").exists(),
        "превью создало каталог журналов платформы"
    );
}

/// Ветка EDT останавливается там же. Квитанции о выборе исполнителя у неё нет — её не
/// имеет и боевой прогон: EDT CLI ищется напрямую.
#[test]
fn a_preview_of_the_edt_check_plans_without_running_the_edt_cli() {
    let (dir, config_path) = setup_edt_project("exit 3");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "check",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["data"]["status"], "planned");
    assert_eq!(payload["data"]["provider_dispatched"], false);
    assert_eq!(payload["data"]["check_name"], "edt");
    assert_eq!(payload["data"]["exit_code"], -1);
    assert!(payload["data"]["provider"].is_null(), "{payload}");
    let message = payload["data"]["message"].as_str().expect("message");
    assert!(message.contains("main"), "{message}");

    assert!(
        !dir.path().join("work/logs/platform").exists(),
        "превью создало каталог журналов платформы"
    );
}

#[test]
fn syntax_designer_config_json_returns_clean_envelope() {
    let (_dir, config_path) = setup_project(
        "out=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf '' > \"$out\"; fi\nprintf 'RAW_STDOUT\\n'\nexit 0",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "syntax",
            "designer-config",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "check");
    assert_eq!(payload["data"]["check_name"], "designer-config");
    assert_eq!(payload["data"]["status"], "clean");
    assert_eq!(payload["data"]["exit_code"], 0);
}

#[test]
fn syntax_text_clean_success_stays_compact() {
    let (_dir, config_path) = setup_project(
        "out=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf '' > \"$out\"; fi\nexit 0",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "syntax",
            "designer-config",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("● Syntax check designer-config completed successfully"));
    assert!(stdout.contains("│   status: clean (exit 0, errors 0, warnings 0, info 0"));
    assert!(!stdout.contains("platform log"));
}

/// Инструмент вышел нулём, а журнал с его замечаниями прочитать не удалось. До
/// 2026-09-17 это давало «завершено с предупреждениями» и код возврата 0, то есть CI
/// зеленел на проверке, чьих замечаний никто не видел. Теперь вердикт неизвестен, и
/// неизвестность названа отдельно, а не сведена к чистоте.
#[test]
fn syntax_with_an_unreadable_log_refuses_instead_of_reporting_clean() {
    let (_dir, config_path) = setup_project(
        "args=\"$*\"\nprintf 'RAW_STDOUT\\n'\nif printf '%s' \"$args\" | grep -F -q -- '/Out'; then\n  exit 0\nfi\nexit 0",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "syntax",
            "designer-config",
        ])
        .output()
        .expect("run command");

    assert!(
        !output.status.success(),
        "an unread verdict must not be reported as a pass"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[warning] log"), "{stdout}");
    assert!(stdout.contains("[diagnostic] platform log -> "), "{stdout}");
    assert!(!stdout.contains("status: clean"), "{stdout}");
}

#[test]
fn syntax_designer_modules_json_returns_structured_validation_failure() {
    let (_dir, config_path) = setup_project(
        "args=\"$*\"\nout=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif printf '%s' \"$args\" | grep -F -q -- '/CheckConfig'; then\n  cat <<'LOG' > \"$out\"\n{CommonModules.TestModule(4,2)}: Ошибка компиляции\n{1}: context\nLOG\n  exit 101\nfi\nexit 0",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "syntax",
            "designer-modules",
            "--server",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "runtime_failure");
    assert_eq!(payload["error"]["kind"], "runtime");
    assert_eq!(payload["data"]["status"], "issues_found");
    assert_eq!(payload["data"]["exit_code"], 101);
    assert_eq!(payload["data"]["summary"]["errors"], 1);
    assert_eq!(payload["data"]["issues"][0]["kind"], "module");
    assert_eq!(
        payload["data"]["issues"][0]["path"],
        "CommonModules.TestModule"
    );
}

/// Прежнее имя без режимов больше не отвергается: требование «хотя бы один режим»
/// принадлежало `/CheckModules`, а её путь исчез. Пустой запрос выполняет профиль по
/// умолчанию, и проверка действительно идёт.
#[test]
fn a_check_without_modes_runs_the_default_profile() {
    let (_dir, config_path) = setup_project(
        "args=\"$*\"\nout=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nprintf '%s\\n' \"$args\" >> \"$(dirname \"$0\")/calls.log\"\n: > \"$out\"\nexit 0",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "check",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success(), "{output:?}");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "check");
    assert_eq!(payload["data"]["check_name"], "designer-config");
    let root = config_path
        .parent()
        .and_then(Path::parent)
        .expect("project root");
    let calls = fs::read_to_string(root.join("platform").join("bin").join("calls.log"))
        .expect("designer calls");
    for flag in [
        "/CheckConfig",
        "-ThinClient",
        "-Server",
        "-UnreferenceProcedures",
        "-HandlersExistence",
        "-EmptyHandlers",
        "-ExtendedModulesCheck",
    ] {
        assert!(calls.contains(flag), "{flag}: {calls}");
    }
}

#[test]
fn syntax_text_output_hides_raw_stdout_and_prints_structured_issue() {
    let (_dir, config_path) = setup_project(
        "args=\"$*\"\nout=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nprintf 'RAW_STDOUT\\n'\nif printf '%s' \"$args\" | grep -F -q -- '/CheckConfig'; then\n  cat <<'LOG' > \"$out\"\nCommonModules.TestModule Warning: потенциальная проблема\nLOG\n  exit 101\nfi\nexit 0",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "syntax",
            "designer-modules",
            "--server",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Syntax check designer-config found issues"));
    assert!(stdout.contains("CommonModules.TestModule"));
    assert!(stdout.contains("[issue] WARNING"));
    assert!(!stdout.contains("RAW_STDOUT"));
}

#[test]
fn syntax_edt_json_returns_structured_edt_issues() {
    let (_dir, config_path) = setup_edt_project(
        "out=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"--file\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then cat <<'LOG' > \"$out\"\nERROR\tCommonModules.Test\t7\t2\tRule\tbad call\nLOG\nfi\nexit 1",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "syntax",
            "edt",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));

    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["data"]["check_name"], "edt");
    assert_eq!(payload["data"]["status"], "issues_found");
    assert_eq!(payload["data"]["summary"]["errors"], 1);
    assert_eq!(payload["data"]["issues"][0]["kind"], "edt");
    assert_eq!(payload["data"]["issues"][0]["path"], "CommonModules.Test");
}

/// Команда одна: режимы `/CheckConfig` живут на ней самой, подкоманда не нужна.
#[test]
fn check_takes_its_modes_without_a_subcommand() {
    let (_dir, config_path) = setup_project(
        "args=\"$*\"\nout=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nprintf '%s\\n' \"$args\" >> \"$(dirname \"$0\")/calls.log\"\n: > \"$out\"\nexit 0",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "check",
            "--server",
            "--incorrect-references",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success(), "{output:?}");
    let root = config_path
        .parent()
        .and_then(Path::parent)
        .expect("project root");
    let calls =
        fs::read_to_string(root.join("platform").join("bin").join("calls.log")).expect("calls");
    assert!(calls.contains("/CheckConfig"), "{calls}");
    assert!(calls.contains("-IncorrectReferences"), "{calls}");
    assert!(calls.contains("-Server"), "{calls}");
    // Назван режим — профиль по умолчанию не подмешивается.
    assert!(!calls.contains("-EmptyHandlers"), "{calls}");
}

/// Ключ, который ветка не исполняет, отвергается, а не игнорируется.
#[test]
fn check_refuses_a_key_the_branch_does_not_execute() {
    let (_dir, config_path) = setup_project("exit 0");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "check",
            "--project",
            "main",
        ])
        .output()
        .expect("run command");

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let message = payload["data"]["message"].as_str().expect("message");
    assert!(message.contains("--project"), "{payload}");
    assert!(message.contains("/CheckConfig"), "{payload}");
}

/// Ключи самой команды рядом с прежним именем не исполняются, поэтому отвергаются.
#[test]
fn check_refuses_its_keys_next_to_a_previous_name() {
    let (_dir, config_path) = setup_project("exit 0");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "check",
            "--server",
            "designer-config",
        ])
        .output()
        .expect("run command");

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert!(
        payload["data"]["message"]
            .as_str()
            .expect("message")
            .contains("cannot be combined with a subcommand"),
        "{payload}"
    );
}

/// Проверка внешних обработок платформой не описана: проект, где других наборов нет,
/// получил бы «чисто», не проверив предмета. Отказ — по предмету, и он не изменится.
#[test]
fn check_refuses_a_project_of_external_subjects_only() {
    let (dir, config_path) = setup_project("exit 0");
    let work_path = dir.path().join("work");
    let install_dir = dir.path().join("platform");
    // Раскладка внешнего набора проверяется раньше: отказ по предмету должен приходить
    // на верном проекте, а не на пустом каталоге.
    let sources = config_path.parent().expect("project dir").join("reports");
    fs::create_dir_all(&sources).expect("sources");
    fs::write(sources.join("Report.xml"), "<ExternalReport/>").expect("descriptor");
    fs::write(
        &config_path,
        format!(
            "workPath: '{}'\nformat: DESIGNER\ninfobase:\n  connection: 'File=/tmp/ib'\nsource-set:\n  - name: reports\n    type: EXTERNAL_REPORTS\n    path: reports\ntools:\n  platform:\n    path: '{}'\n",
            work_path.display(),
            install_dir.display()
        ),
    )
    .expect("config");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "check",
        ])
        .output()
        .expect("run command");

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["error"]["kind"], "capability", "{payload}");
    assert_eq!(payload["error"]["code"], "subject", "{payload}");
}

/// Синоним держится один цикл ровно тем, чем был: у проверки модулей режим обязателен, и
/// профиль по умолчанию сюда не подмешивается.
#[test]
fn a_previous_name_keeps_its_own_rule_about_modes() {
    let (_dir, config_path) = setup_project("exit 0");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "check",
            "designer-modules",
        ])
        .output()
        .expect("run command");

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["command"], "check", "{payload}");
    assert!(
        payload["data"]["message"]
            .as_str()
            .expect("message")
            .contains("requires at least one mode flag"),
        "{payload}"
    );
}

/// Прежнее имя без режимов не получает профиль по умолчанию и тогда, когда режимы названы:
/// проверки конфигурации остаются пустыми.
#[test]
fn a_previous_name_runs_only_the_modes_it_was_given() {
    let (_dir, config_path) = setup_project(
        "args=\"$*\"\nout=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nprintf '%s\\n' \"$args\" >> \"$(dirname \"$0\")/calls.log\"\n: > \"$out\"\nexit 0",
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "check",
            "designer-modules",
            "--server",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success(), "{output:?}");
    let root = config_path
        .parent()
        .and_then(Path::parent)
        .expect("project root");
    let calls =
        fs::read_to_string(root.join("platform").join("bin").join("calls.log")).expect("calls");
    assert!(calls.contains("/CheckConfig"), "{calls}");
    assert!(calls.contains("-Server"), "{calls}");
    assert!(!calls.contains("-EmptyHandlers"), "{calls}");
    assert!(!calls.contains("-ThinClient"), "{calls}");
}
