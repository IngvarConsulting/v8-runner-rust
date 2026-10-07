#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use support::{
    hold_workspace_lock, temp_workspace, v8_runner_command, write_shell_script as write_script,
};

const V8_CONFIGURATION_NATURE: &str = "com._1c.g5.v8.dt.core.V8ConfigurationNature";
const EDT_RUNTIME_VERSION: &str = "8.3.27";

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
        "args=\"$*\"\nprintf '%s\\n' \"$args\" >> \"{}\"\ncase \" $args \" in *\" generation-id \"*) exit 0;; esac\n{}\nmkdir -p \"$(printf '%s' \"$args\" | awk '{{print $NF}}')\"\nexit 0",
        calls_log.display(),
        pattern_branch
    );
    write_script(path, &body);
}

fn write_designer_dump_script_for_edt(path: &Path, calls_log: &Path) {
    let body = format!(
        "args=\"$*\"\nout=\"\"\ntarget=\"\"\nprev=\"\"\nfor arg in \"$@\"; do\n  if [ \"$prev\" = \"/Out\" ]; then out=\"$arg\"; fi\n  if [ \"$prev\" = \"/DumpConfigToFiles\" ]; then target=\"$arg\"; fi\n  prev=\"$arg\"\ndone\nif [ -n \"$out\" ]; then printf 'designer log for %s\\n' \"$args\" > \"$out\"; fi\nprintf '%s\\n' \"$args\" >> \"{}\"\nif [ -n \"$target\" ]; then mkdir -p \"$target\"; printf '<Configuration />\\n' > \"$target/Configuration.xml\"; fi\nexit 0",
        calls_log.display()
    );
    write_script(path, &body);
}

fn write_designer_partial_dump_script(path: &Path, captured_list: &Path) {
    let body = format!(
        "list_file=\"\"\ntarget=\"\"\nprevious=\"\"\nfor argument in \"$@\"; do\n  if [ \"$previous\" = \"-listFile\" ]; then list_file=\"$argument\"; fi\n  if [ \"$previous\" = \"/DumpConfigToFiles\" ]; then target=\"$argument\"; fi\n  previous=\"$argument\"\ndone\ncp \"$list_file\" \"{}\"\nif [ -n \"$target\" ]; then mkdir -p \"$target\"; printf '<Configuration />\\n' > \"$target/Configuration.xml\"; fi\nexit 0",
        captured_list.display()
    );
    write_script(path, &body);
}

fn write_edt_import_script(path: &Path, calls_log: &Path) {
    let body = format!(
        r#"args="$*"
printf '%s\n' "$args" >> "{}"
project=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "--project" ]; then project="$arg"; fi
  prev="$arg"
done
mkdir -p "$project/DT-INF" "$project/src/Configuration"
cat > "$project/.project" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<projectDescription>
  <name>BaseProject</name>
  <natures>
    <nature>{}</nature>
  </natures>
</projectDescription>
EOF
printf 'Manifest-Version: 1.0\nRuntime-Version: {}\n' > "$project/DT-INF/PROJECT.PMF"
printf '<Configuration />\n' > "$project/src/Configuration/Configuration.mdo"
printf 'Procedure Test()\nEndProcedure\n' > "$project/src/Configuration/Module.bsl"
exit 0"#,
        calls_log.display(),
        V8_CONFIGURATION_NATURE,
        EDT_RUNTIME_VERSION
    );
    write_script(path, &body);
}

fn assert_native_edt_project(path: &Path) {
    assert!(path.join(".project").exists());
    assert!(path.join("DT-INF").join("PROJECT.PMF").exists());
    assert!(path.join("src/Configuration/Configuration.mdo").exists());
}

fn write_edt_configuration_source(path: &Path, project_name: &str) {
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
    write_config_with_infobase(
        path,
        base_path,
        work_path,
        platform_path,
        "  connection: 'File=ib'\n",
    );
}

fn write_config_with_infobase(
    path: &Path,
    _base_path: &Path,
    work_path: &Path,
    platform_path: &Path,
    infobase_yaml: &str,
) {
    let config = format!(
        "workPath: '{}'\nformat: DESIGNER\nproviders:\n  init: ibcmd\n  build: ibcmd\n  dump: ibcmd\n  infobase.configuration.export: ibcmd\ninfobase:\n{}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/main\ntools:\n  platform:\n    path: '{}'\n",
        work_path.display(),
        infobase_yaml,
        platform_path.display(),
    );

    fs::write(path, config).expect("config");
}

fn setup_project() -> (
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

    fs::create_dir_all(base_path.join("main")).expect("main");
    fs::create_dir_all(&work_path).expect("work");
    fs::write(base_path.join("main").join("old.txt"), "old").expect("old");

    write_ibcmd_script(&binary_path, &calls_log, None);
    write_config(&config_path, &base_path, &work_path, &binary_path);

    (
        dir,
        config_path,
        binary_path,
        work_path,
        base_path,
        calls_log,
    )
}

fn assert_ibcmd_data_path(calls: &str, work_path: &Path) {
    let expected_fragment = format!("infobase --data {}", work_path.join("ibcmd-data").display());
    assert!(
        calls.contains(&expected_fragment),
        "expected isolated IBCMD data path fragment: {expected_fragment}"
    );
}

fn write_designer_config(path: &Path, work_path: &Path, platform_path: &Path) {
    let config = format!(
        "workPath: '{}'\nformat: DESIGNER\nproviders:\n  push: designer\n  pull: designer\n  download: designer\n  infobase.dump: designer\n  infobase.restore: designer\ninfobase:\n  connection: 'File=ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/main\ntools:\n  platform:\n    path: '{}'\n",
        work_path.display(),
        platform_path.display(),
    );

    fs::write(path, config).expect("config");
}

fn write_edt_dump_config(
    path: &Path,
    _base_path: &Path,
    work_path: &Path,
    platform_path: &Path,
    edt_path: &Path,
) {
    let config = format!(
        "workPath: '{}'\nformat: EDT\ninfobase:\n  connection: 'File=ib'\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: project/main\ntools:\n  platform:\n    path: '{}'\n  edt_cli:\n    path: '{}'\n    interactive-mode: false\n",
        work_path.display(),
        platform_path.display(),
        edt_path.display(),
    );

    fs::write(path, config).expect("config");
}

fn setup_edt_project() -> (
    tempfile::TempDir,
    PathBuf,
    PathBuf,
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
    let platform_path = dir.path().join("1cv8");
    let edt_path = dir.path().join("edt").join("1cedtcli");
    let designer_calls = dir.path().join("designer-calls.log");
    let edt_calls = dir.path().join("edt-calls.log");

    fs::create_dir_all(base_path.join("main")).expect("main");
    fs::create_dir_all(&work_path).expect("work");
    write_edt_configuration_source(&base_path.join("main"), "BaseProject");
    fs::write(base_path.join("main").join("old.txt"), "old").expect("old");

    write_designer_dump_script_for_edt(&platform_path, &designer_calls);
    write_edt_import_script(&edt_path, &edt_calls);
    write_edt_dump_config(
        &config_path,
        &base_path,
        &work_path,
        &platform_path,
        &edt_path,
    );

    (
        dir,
        config_path,
        platform_path,
        edt_path,
        work_path,
        base_path,
        designer_calls,
        edt_calls,
    )
}

/// Страж: превью не берёт workspace lock и не ждёт его.
///
/// Корень прежнего поведения: превью шли через ту же границу блокировки, что
/// применение, поэтому «покажи план» упиралось бы в занятое пространство и отказывало
/// `workspace_busy`, а два одновременных превью выстраивались бы в очередь. Здесь
/// блокировка занята заранее чужим владельцем: применение обязано упереться, превью —
/// пройти.
#[test]
fn dry_run_neither_takes_nor_waits_for_the_workspace_lock() {
    let (_dir, config_path, _binary_path, work_path, _base_path, _calls_log) = setup_project();
    hold_workspace_lock(&work_path);

    let preview = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "dump",
            "--force",
            "--source-set",
            "main",
            "--dry-run",
        ])
        .output()
        .expect("run preview");

    assert!(
        preview.status.success(),
        "preview must not wait for a foreign workspace lock: {}{}",
        String::from_utf8_lossy(&preview.stdout),
        String::from_utf8_lossy(&preview.stderr)
    );

    let apply = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "dump",
            "--force",
            "--source-set",
            "main",
        ])
        .output()
        .expect("run apply");

    assert!(
        !apply.status.success(),
        "apply must still honour the workspace lock"
    );
    let envelope: Value = serde_json::from_slice(&apply.stdout).expect("json");
    let message = envelope["error"]["message"].as_str().expect("message");
    assert!(message.contains("cannot start pull"), "{message}");
    assert!(message.contains("workspace"), "{message}");
}

/// `--clean-before-execution` меняет `workPath`, поэтому с превью он отклоняется,
/// а не пропускается молча.
#[test]
fn dry_run_refuses_clean_before_execution_instead_of_skipping_it() {
    let (_dir, config_path, _binary_path, _work_path, _base_path, _calls_log) = setup_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "--clean-before-execution",
            "dump",
            "--force",
            "--source-set",
            "main",
            "--dry-run",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let reported = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        reported.contains("preview must not modify workPath"),
        "{reported}"
    );
}

#[test]
fn dump_dry_run_plans_the_target_without_writing_it() {
    let (_dir, config_path, _binary_path, _work_path, base_path, calls_log) = setup_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "dump",
            "--force",
            "--source-set",
            "main",
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
    assert_eq!(data["source_set"], "main");
    let message = data["message"].as_str().expect("message");
    assert!(message.contains("nothing written"), "{message}");
    assert!(
        message.contains(base_path.join("main").display().to_string().as_str()),
        "{message}"
    );
    assert!(
        !calls_log.exists(),
        "preview must not dispatch the platform"
    );
}

#[test]
fn dump_ibcmd_full_json_success() {
    let (_dir, config_path, _binary_path, work_path, base_path, calls_log) = setup_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "dump",
            "--force",
            "--source-set",
            "main",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains("--force"));
    assert_ibcmd_data_path(&calls, &work_path);
    assert!(base_path.join("main").exists());
}

#[test]
fn dump_edt_full_json_success_updates_designer_mirror_and_edt_target() {
    let (
        _dir,
        config_path,
        _platform_path,
        _edt_path,
        work_path,
        base_path,
        designer_calls,
        edt_calls,
    ) = setup_edt_project();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "dump",
            "--force",
            "--source-set",
            "main",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "pull");
    assert_eq!(
        payload["data"]["target_path"],
        fs::canonicalize(base_path.join("main"))
            .expect("canonical dump target")
            .display()
            .to_string()
    );
    assert_native_edt_project(&base_path.join("main"));
    assert!(!base_path.join("main").join("old.txt").exists());
    // Снимок Конфигуратора лежит под памятью выбранной базы.
    let bases: Vec<_> = fs::read_dir(work_path.join("infobases"))
        .expect("base memory")
        .map(|entry| entry.expect("entry").path())
        .collect();
    let [base] = bases.as_slice() else {
        panic!("one remembered base: {bases:?}");
    };
    let snapshot = base.join("designer/main");
    assert!(snapshot.join("Configuration.xml").exists());

    let designer_calls = fs::read_to_string(designer_calls).expect("designer calls");
    let edt_calls = fs::read_to_string(edt_calls).expect("edt calls");
    assert!(designer_calls.contains(base.join("designer").display().to_string().as_str()));
    assert!(edt_calls.contains(snapshot.display().to_string().as_str()));
}

#[test]
fn dump_text_success_is_compact_and_keeps_output_visible() {
    let (_dir, config_path, _binary_path, _work_path, base_path, _calls_log) =
        setup_project_in_a_repository();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "dump",
            "--force",
            "--source-set",
            "main",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("◌ dump: full"));
    assert!(!stdout.contains("started_at: "));
    assert!(stdout.contains("[ibcmd] exporting configuration files"));
    assert!(
        stdout
            .find("[ibcmd] exporting configuration files")
            .expect("dump detail")
            < stdout
                .find("● Dump completed successfully")
                .expect("summary")
    );
    assert!(stdout.contains("● Dump completed successfully"));
    assert!(stdout.contains("│   source-set: main"));
    assert!(stdout.contains("│   mode: full"));
    assert!(stdout.contains(base_path.join("main").display().to_string().as_str()));
    assert!(!stdout.contains("platform log"));
}

#[test]
fn dump_ibcmd_incremental_json_success() {
    let (_dir, config_path, _binary_path, work_path, base_path, calls_log) = setup_project();
    fs::remove_dir_all(base_path.join("main")).expect("remove target");
    fs::create_dir_all(base_path.join("main")).expect("target");
    fs::write(
        base_path.join("main/ConfigDumpInfo.xml"),
        "<ConfigDumpInfo version=\"2.17\"/>",
    )
    .expect("version file");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "dump",
            "--source-set",
            "main",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(payload["ok"], true);
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains("--sync"));
    assert_ibcmd_data_path(&calls, &work_path);
    assert!(calls.contains(base_path.join("main").display().to_string().as_str()));
}

#[test]
fn dump_ibcmd_partial_json_success_uses_degraded_fallback() {
    let (_dir, config_path, _binary_path, work_path, _base_path, calls_log) =
        setup_project_in_a_repository();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "dump",
            "--source-set",
            "main",
            "--object",
            "Catalog.Items",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let data = &payload["data"];
    assert_eq!(payload["ok"], true);
    assert_eq!(data["mode"], "PARTIAL");
    assert!(data["message"]
        .as_str()
        .expect("message")
        .contains("IBCMD does not support object-scoped partial dump"));
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains("--sync"));
    assert_ibcmd_data_path(&calls, &work_path);
}

#[test]
fn dump_text_warning_shows_degraded_fallback_reason() {
    let (_dir, config_path, _binary_path, _work_path, _base_path, _calls_log) =
        setup_project_in_a_repository();

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "dump",
            "--source-set",
            "main",
            "--object",
            "Catalog.Items",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("▲ Dump completed with warnings"));
    assert!(stdout.contains("[warning] IBCMD does not support object-scoped partial dump"));
}

#[test]
fn dump_ibcmd_partial_failure_keeps_partial_mode_and_warning() {
    let (_dir, config_path, binary_path, _work_path, _base_path, calls_log) =
        setup_project_in_a_repository();
    write_ibcmd_script(&binary_path, &calls_log, Some("--sync"));

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "dump",
            "--source-set",
            "main",
            "--object",
            "Catalog.Items",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    let data = &payload["data"];
    assert_eq!(payload["ok"], false);
    assert_eq!(data["mode"], "PARTIAL");
    assert!(data["message"]
        .as_str()
        .expect("message")
        .contains("IBCMD does not support object-scoped partial dump"));
    assert!(data["message"]
        .as_str()
        .expect("message")
        .contains("dump failed for source-set 'main' with exit code 17"));
}

#[test]
fn dump_designer_partial_json_normalizes_colon_selector_and_reports_both_forms() {
    let (_dir, config_path, binary_path, work_path, _base_path, _calls_log) =
        setup_project_in_a_repository();
    let designer_binary = binary_path.with_file_name("1cv8");
    let captured_list = config_path
        .parent()
        .expect("project directory")
        .join("partial-list.txt");
    write_designer_partial_dump_script(&designer_binary, &captured_list);
    write_designer_config(&config_path, &work_path, &designer_binary);

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "dump",
            "--object",
            "  Catalog:Items  ",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(captured_list).expect("captured selector list"),
        "Catalog.Items\n"
    );
    let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(
        payload["data"]["selectors"][0]["requested"],
        "  Catalog:Items  "
    );
    assert_eq!(
        payload["data"]["selectors"][0]["normalized"],
        "Catalog.Items"
    );
}

#[test]
fn dump_text_failure_shows_error_message() {
    let (_dir, config_path, binary_path, _work_path, _base_path, calls_log) = setup_project();
    write_ibcmd_script(&binary_path, &calls_log, Some("--force"));

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--no-color",
            "dump",
            "--force",
            "--source-set",
            "main",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✖ Dump failed"));
    assert!(stdout.contains("[error]"));
    assert!(stdout.contains("exit code 17"));
}

/// У кластера `ibcmd` в строке `pull` нет (#206): ключ отказывает при проверке настроек и
/// называет исполнителей строки, `ibcmd` не запускается и при полной секции `dbms`.
#[test]
fn a_cluster_infobase_refuses_providers_pull_ibcmd_before_the_platform() {
    let (_dir, config_path, _binary_path, _work_path, _base_path, calls_log) = setup_project();
    write_config_with_infobase(
        &config_path,
        &config_path.parent().expect("dir").join("project"),
        &config_path.parent().expect("dir").join("work"),
        &config_path.parent().expect("dir").join("ibcmd"),
        "  connection: 'Srvr=server;Ref=main'\n  user: Admin\n  password: secret\n  dbms:\n    kind: PostgreSQL\n    server: localhost\n    name: maindb\n    user: postgres\n    password: pg-secret\n"
    );

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "--json-message",
            "dump",
            "--force",
            "--source-set",
            "main",
        ])
        .output()
        .expect("run command");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "does not implement this operation on a cluster infobase; implemented: agent, designer"
        ),
        "{stdout}"
    );
    assert!(!calls_log.exists(), "ibcmd must not run");
}

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

/// Готовит проект, чей каталог исходников лежит в репозитории с одним
/// зафиксированным файлом.
fn setup_project_in_a_repository() -> (
    tempfile::TempDir,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
) {
    let parts = setup_project();
    let base_path = parts.4.clone();
    git(&base_path, &["init", "-q", "-b", "main", "."]);
    git(&base_path, &["config", "user.email", "test@example.com"]);
    git(&base_path, &["config", "user.name", "Test"]);
    git(&base_path, &["add", "-A"]);
    git(&base_path, &["commit", "-qm", "committed sources"]);
    parts
}

/// Проект в репозитории с годным файлом версий в игноре: `pull` идёт по изменившемуся.
fn setup_project_with_a_version_file_in_a_repository() -> (
    tempfile::TempDir,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
) {
    let parts = setup_project();
    let base_path = parts.4.clone();
    fs::write(base_path.join(".gitignore"), "ConfigDumpInfo.xml\n").expect("gitignore");
    fs::write(
        base_path.join("main").join("ConfigDumpInfo.xml"),
        "<ConfigDumpInfo version=\"2.17\"/>",
    )
    .expect("version file");
    git(&base_path, &["init", "-q", "-b", "main", "."]);
    git(&base_path, &["config", "user.email", "test@example.com"]);
    git(&base_path, &["config", "user.name", "Test"]);
    git(&base_path, &["add", "-A"]);
    git(&base_path, &["commit", "-qm", "committed sources"]);
    parts
}

/// Полная выгрузка заменяет каталог исходников целиком, а прежнее содержимое
/// раннер до сих пор удалял последним шагом. Файл вне учёта не вернуть ничем,
/// поэтому команда обязана остановиться и назвать его.
#[test]
fn a_dump_refuses_to_destroy_work_version_control_cannot_give_back() {
    let (_dir, config_path, _binary, _work, base_path, _calls) = setup_project_in_a_repository();
    fs::write(
        base_path.join("main").join("hand-written.xml"),
        "written by hand\n",
    )
    .expect("hand-written");

    // Командная строка просит замену только с согласием (`pull --force`); полная выгрузка,
    // которая спрашивает сначала, — у MCP: согласия ему взять неоткуда.
    let answer = support::mcp::call_tool(&config_path, "dump_config", json!({ "mode": "FULL" }));

    let rendered = answer.envelope.to_string();
    assert!(answer.is_error, "the dump must be refused: {rendered}");
    assert_eq!(
        answer.envelope["error"]["kind"], "validation",
        "a refusal is a validation error, not a runtime one: {rendered}"
    );
    assert!(
        rendered.contains("hand-written.xml"),
        "the refusal must name what would be lost: {rendered}"
    );
    assert!(
        rendered.contains("--force"),
        "the refusal must say how to proceed anyway: {rendered}"
    );
    assert!(
        base_path.join("main").join("hand-written.xml").is_file(),
        "the refusal must happen before anything is replaced"
    );
}

/// Опись версий штатно лежит в игноре, а полная выгрузка пишет её заново: её
/// прежнее содержимое не потеря, и выгрузка идёт без `--force`.
#[test]
fn a_full_dump_replaces_an_ignored_version_file_without_asking() {
    let (_dir, config_path, _binary, _work, base_path, _calls) = setup_project_in_a_repository();
    let version_file = base_path.join("main").join("ConfigDumpInfo.xml");
    fs::write(base_path.join(".gitignore"), "ConfigDumpInfo.xml\n").expect("gitignore");
    git(
        &base_path,
        &[
            "rm",
            "-q",
            "--cached",
            "--ignore-unmatch",
            "main/ConfigDumpInfo.xml",
        ],
    );
    git(&base_path, &["add", ".gitignore"]);
    git(&base_path, &["commit", "-qm", "ignore the version file"]);
    fs::write(&version_file, "<info previous=\"yes\"/>\n").expect("version file");

    // Без согласия: полная выгрузка MCP спрашивает систему контроля версий сначала.
    let answer = support::mcp::call_tool(&config_path, "dump_config", json!({ "mode": "FULL" }));

    assert!(
        !answer.is_error,
        "an ignored version file must not stop a full dump: {}",
        answer.envelope
    );
    // Поддельная платформа описи не пишет: в заменённом каталоге файла с прежним
    // содержимым остаться не должно.
    assert_ne!(
        fs::read_to_string(&version_file).ok().as_deref(),
        Some("<info previous=\"yes\"/>\n"),
        "the previous version file must be replaced, not kept"
    );
}

/// Попросили явно — уничтожаем, как и обещает имя ключа. Резервная копия, о
/// которой не просили и про которую молчат, была бы мусором в чужом каталоге.
#[test]
fn an_explicit_request_replaces_the_directory_and_keeps_nothing() {
    let (_dir, config_path, _binary, _work, base_path, _calls) = setup_project_in_a_repository();
    fs::write(
        base_path.join("main").join("hand-written.xml"),
        "written by hand\n",
    )
    .expect("hand-written");

    let output = v8_runner_command()
        .args([
            "--config",
            &config_path.display().to_string(),
            "pull",
            "main",
            "--force",
        ])
        .output()
        .expect("run dump");

    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "dump must proceed: {rendered}");
    assert!(
        !base_path.join("main").join("hand-written.xml").exists(),
        "the directory was replaced, so the hand-written file is gone"
    );
    assert!(
        kept_backups(&base_path).is_empty(),
        "nothing was asked to be kept: {:?}",
        kept_backups(&base_path)
    );
}

fn kept_backups(base_path: &Path) -> Vec<PathBuf> {
    fs::read_dir(base_path)
        .expect("read base")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".dump-backup"))
        })
        .collect()
}

/// `pull main` с добавленными ключами в форме JSON: выход, конверт и весь вывод.
fn pull_json(config_path: &Path, extra: &[&str]) -> (std::process::Output, Value, String) {
    let config = config_path.display().to_string();
    let mut args = vec![
        "--config",
        config.as_str(),
        "--json-message",
        "pull",
        "main",
    ];
    args.extend_from_slice(extra);
    let output = v8_runner_command().args(&args).output().expect("run pull");
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("json envelope ({error}): {rendered}"));
    (output, envelope, rendered)
}

/// Потери, которые ответ называет полем `losses`, строками.
fn losses_of(envelope: &Value) -> Vec<String> {
    envelope["data"]["losses"]
        .as_array()
        .map(|paths| {
            paths
                .iter()
                .map(|path| path.as_str().expect("loss is a path").to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Каталог вне системы контроля версий: ответа нет, и потерять можно всё. Отказ идёт на
/// тех же правах, что найденное безвозвратное, называет каждый файл и случается до запуска
/// платформы — и у выгрузки поверх каталога, и у замены каталога без согласия.
#[test]
fn a_directory_outside_version_control_is_refused_and_every_file_is_named() {
    let (_dir, config_path, _binary, _work, base_path, calls_log) = setup_project();
    let nested = base_path.join("main").join("Catalogs").join("Hand.xml");
    fs::create_dir_all(nested.parent().expect("parent")).expect("nested dir");
    fs::write(&nested, "written by hand\n").expect("nested file");

    let (output, envelope, rendered) = pull_json(&config_path, &[]);
    assert_eq!(output.status.code(), Some(2), "{rendered}");
    assert_eq!(envelope["error"]["kind"], "validation", "{rendered}");
    let message = envelope["error"]["message"].as_str().expect("message");
    assert!(message.contains("refusing to overwrite"), "{message}");
    assert!(message.contains("version control"), "{message}");
    assert!(message.contains("old.txt"), "{message}");
    assert!(message.contains("Hand.xml"), "{message}");
    assert!(message.contains("pull main --force"), "{message}");

    let answer = support::mcp::call_tool(&config_path, "dump_config", json!({ "mode": "FULL" }));
    let rendered = answer.envelope.to_string();
    assert!(
        answer.is_error,
        "the replacement must be refused: {rendered}"
    );
    assert_eq!(answer.envelope["error"]["kind"], "validation", "{rendered}");
    assert!(rendered.contains("refusing to replace"), "{rendered}");
    assert!(rendered.contains("old.txt"), "{rendered}");

    assert!(
        !calls_log.exists(),
        "the refusal comes before the platform starts: {:?}",
        fs::read_to_string(&calls_log).ok()
    );
    assert!(base_path.join("main").join("old.txt").is_file());
    assert!(nested.is_file());
    assert!(kept_backups(&base_path).is_empty());
}

/// Пустой каталог вне системы контроля версий терять нечего: выгрузка идёт без согласия.
#[test]
fn an_empty_directory_outside_version_control_has_nothing_to_lose() {
    let (_dir, config_path, _binary, _work, base_path, calls_log) = setup_project();
    fs::remove_file(base_path.join("main").join("old.txt")).expect("empty the directory");

    let (output, envelope, rendered) = pull_json(&config_path, &[]);

    assert!(output.status.success(), "{rendered}");
    assert!(losses_of(&envelope).is_empty(), "{rendered}");
    assert!(calls_log.exists(), "the platform must run: {rendered}");
}

/// `pull` кладёт выгрузку поверх каталога: чего в базе нет, остаётся. `pull --force`
/// приводит каталог ровно к базе; зафиксированное потерей не считается, и ответ ничего
/// уничтоженным не называет.
#[test]
fn a_pull_lays_the_dump_over_the_directory_and_force_brings_it_to_the_base() {
    let (_dir, config_path, _binary, _work, base_path, calls_log) =
        setup_project_with_a_version_file_in_a_repository();

    let (output, envelope, rendered) = pull_json(&config_path, &[]);
    assert!(output.status.success(), "{rendered}");
    assert_eq!(envelope["data"]["mode"], "INCREMENTAL", "{rendered}");
    assert!(
        fs::read_to_string(&calls_log)
            .expect("calls")
            .contains("--sync"),
        "{rendered}"
    );
    assert!(
        base_path.join("main").join("old.txt").is_file(),
        "what the base does not have stays in place: {rendered}"
    );
    assert!(losses_of(&envelope).is_empty(), "{rendered}");

    let (output, envelope, rendered) = pull_json(&config_path, &["--force"]);
    assert!(output.status.success(), "{rendered}");
    assert_eq!(envelope["data"]["mode"], "FULL", "{rendered}");
    assert!(
        !base_path.join("main").join("old.txt").exists(),
        "the directory is brought exactly to the base: {rendered}"
    );
    assert!(
        losses_of(&envelope).is_empty(),
        "a committed file is not destroyed: {rendered}"
    );
}

/// Пообъектная перезапись спрашивает сторожа так же, как замена: незафиксированное в
/// каталоге останавливает `pull` до платформы и называется поимённо.
#[test]
fn an_incremental_pull_refuses_over_work_version_control_cannot_give_back() {
    let (_dir, config_path, _binary, _work, base_path, calls_log) =
        setup_project_with_a_version_file_in_a_repository();
    fs::write(
        base_path.join("main").join("hand-written.xml"),
        "written by hand\n",
    )
    .expect("hand-written");

    let (output, envelope, rendered) = pull_json(&config_path, &[]);

    assert_eq!(output.status.code(), Some(2), "{rendered}");
    let message = envelope["error"]["message"].as_str().expect("message");
    assert!(message.contains("refusing to overwrite"), "{message}");
    assert!(message.contains("main/hand-written.xml"), "{message}");
    assert!(
        !message.contains("ran full"),
        "the version file is usable, so the dump is incremental: {message}"
    );
    assert!(
        !calls_log.exists(),
        "the platform must not start: {rendered}"
    );
    assert!(base_path.join("main").join("hand-written.xml").is_file());
}

/// Выборка объектов тоже перезаписывает каталог на месте: незафиксированная правка
/// отслеживаемого файла останавливает `pull --object` до платформы, файл цел.
#[test]
fn a_partial_pull_refuses_over_an_uncommitted_edit() {
    let (_dir, config_path, _binary, _work, base_path, calls_log) =
        setup_project_with_a_version_file_in_a_repository();
    let edited = base_path.join("main").join("old.txt");
    fs::write(&edited, "edited by hand\n").expect("edit a committed file");

    let (output, envelope, rendered) = pull_json(&config_path, &["--object", "Catalog:Items"]);

    assert_eq!(output.status.code(), Some(2), "{rendered}");
    let message = envelope["error"]["message"].as_str().expect("message");
    assert!(message.contains("refusing to overwrite"), "{message}");
    assert!(message.contains("main/old.txt"), "{message}");
    assert!(
        !calls_log.exists(),
        "the platform must not start: {rendered}"
    );
    assert_eq!(
        fs::read_to_string(&edited).expect("the edit survives"),
        "edited by hand\n"
    );
}

/// `--force` называет уничтоженное поимённо: и найденное системой контроля версий, и весь
/// каталог, когда ответа у неё нет.
#[test]
fn force_names_what_it_destroyed() {
    let (_dir, config_path, _binary, _work, base_path, _calls) = setup_project_in_a_repository();
    fs::write(
        base_path.join("main").join("hand-written.xml"),
        "written by hand\n",
    )
    .expect("hand-written");

    let (output, envelope, rendered) = pull_json(&config_path, &["--force"]);
    assert!(output.status.success(), "{rendered}");
    assert_eq!(
        losses_of(&envelope),
        vec!["main/hand-written.xml".to_owned()],
        "{rendered}"
    );
    let message = envelope["data"]["message"].as_str().expect("message");
    assert!(message.contains("main/hand-written.xml"), "{message}");
    assert!(!base_path.join("main").join("hand-written.xml").exists());

    let (_dir, config_path, _binary, _work, base_path, _calls) = setup_project();
    let (output, envelope, rendered) = pull_json(&config_path, &["--force"]);
    assert!(output.status.success(), "{rendered}");
    let losses = losses_of(&envelope);
    assert_eq!(losses.len(), 1, "{rendered}");
    assert!(
        Path::new(&losses[0]).ends_with("main/old.txt"),
        "{rendered}"
    );
    let message = envelope["data"]["message"].as_str().expect("message");
    assert!(message.contains("old.txt"), "{message}");
    assert!(message.contains("version control"), "{message}");
    assert!(!base_path.join("main").join("old.txt").exists());
}

/// Превью перечисляет потери поимённо и ничего не трогает: без согласия — то, на чём
/// выгрузка остановится, с согласием — то, что она уничтожит.
#[test]
fn a_pull_preview_names_the_losses_and_touches_nothing() {
    let (_dir, config_path, _binary, _work, base_path, calls_log) = setup_project();

    for (extra, says) in [
        (&["--dry-run"][..], "would stop"),
        (&["--force", "--dry-run"][..], "would discard"),
    ] {
        let (output, envelope, rendered) = pull_json(&config_path, extra);
        assert!(output.status.success(), "{extra:?}: {rendered}");
        let losses = losses_of(&envelope);
        assert_eq!(losses.len(), 1, "{extra:?}: {rendered}");
        assert!(
            Path::new(&losses[0]).ends_with("main/old.txt"),
            "{extra:?}: {rendered}"
        );
        let message = envelope["data"]["message"].as_str().expect("message");
        assert!(message.contains(says), "{extra:?}: {message}");
        assert!(message.contains("old.txt"), "{extra:?}: {message}");
    }
    assert!(base_path.join("main").join("old.txt").is_file());
    assert!(!calls_log.exists(), "a preview must not start the platform");
}

/// У проекта EDT слияния нет: выгрузка заменяет проект по тем же правилам подтверждения.
/// Вне системы контроля версий без согласия — отказ до платформы, с согласием — замена,
/// которая называет уничтоженное.
#[test]
fn an_edt_project_is_replaced_only_by_the_same_confirmation_rules() {
    let (
        _dir,
        config_path,
        _platform_path,
        _edt_path,
        _work_path,
        base_path,
        designer_calls,
        _edt_calls,
    ) = setup_edt_project();

    let (output, envelope, rendered) = pull_json(&config_path, &[]);
    assert_eq!(output.status.code(), Some(2), "{rendered}");
    let message = envelope["error"]["message"].as_str().expect("message");
    assert!(message.contains("refusing to replace"), "{message}");
    assert!(message.contains("old.txt"), "{message}");
    assert!(
        !designer_calls.exists(),
        "the refusal comes before the platform starts: {rendered}"
    );
    assert!(base_path.join("main").join("old.txt").is_file());

    let (output, envelope, rendered) = pull_json(&config_path, &["--force"]);
    assert!(output.status.success(), "{rendered}");
    assert!(!base_path.join("main").join("old.txt").exists());
    assert!(
        losses_of(&envelope)
            .iter()
            .any(|loss| Path::new(loss).ends_with("main/old.txt")),
        "{rendered}"
    );
}

/// Запускает полную выгрузку набора `main` и отдаёт выход вместе с выводом.
fn pull_main(config_path: &Path, extra: &[&str]) -> (std::process::Output, String) {
    let config = config_path.display().to_string();
    let mut args = vec![
        "--config",
        config.as_str(),
        "pull",
        "--force",
        "--source-set",
        "main",
    ];
    args.extend_from_slice(extra);
    let output = v8_runner_command().args(&args).output().expect("run pull");
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output, rendered)
}

/// Опись версий в индексе — состояние чужой базы, которое `checkout` и `merge`
/// подменяют молча. Выгрузка останавливается до платформы, называет путь и рецепт,
/// а превью отказывает так же, как боевой прогон.
#[test]
fn a_pull_refuses_when_the_version_file_is_tracked_by_git() {
    let (_dir, config_path, _binary, _work, base_path, calls_log) = setup_project();
    fs::write(
        base_path.join("main").join("ConfigDumpInfo.xml"),
        "<ConfigDumpInfo/>\n",
    )
    .expect("version file");
    git(&base_path, &["init", "-q", "-b", "main", "."]);
    git(&base_path, &["config", "user.email", "test@example.com"]);
    git(&base_path, &["config", "user.name", "Test"]);
    git(&base_path, &["add", "-A"]);
    git(&base_path, &["commit", "-qm", "committed sources"]);

    for extra in [&["--dry-run"][..], &[][..]] {
        let (output, rendered) = pull_main(&config_path, extra);
        assert_eq!(
            output.status.code(),
            Some(2),
            "a tracked version file is a validation refusal ({extra:?}): {rendered}"
        );
        assert!(
            rendered.contains("git rm --cached main/ConfigDumpInfo.xml && git commit"),
            "the refusal names the file and carries the recipe ({extra:?}): {rendered}"
        );
    }
    assert!(
        !calls_log.exists(),
        "the platform must not start: {:?}",
        fs::read_to_string(&calls_log).ok()
    );
}

/// Описи в индексе нет — выгрузка идёт как обычно.
#[test]
fn a_pull_proceeds_when_the_version_file_is_not_tracked() {
    let (_dir, config_path, _binary, _work, _base_path, calls_log) =
        setup_project_in_a_repository();

    let (output, rendered) = pull_main(&config_path, &[]);

    assert!(output.status.success(), "pull must proceed: {rendered}");
    assert!(calls_log.exists(), "the platform must run: {rendered}");
}

/// Вне репозитория ответа нет: работа идёт молча, без отказа и без предупреждения.
#[test]
fn a_pull_proceeds_silently_when_tracking_is_unknown() {
    let (_dir, config_path, _binary, _work, base_path, calls_log) = setup_project();
    fs::write(
        base_path.join("main").join("ConfigDumpInfo.xml"),
        "<ConfigDumpInfo/>\n",
    )
    .expect("version file");

    let (output, rendered) = pull_main(&config_path, &[]);

    assert!(output.status.success(), "pull must proceed: {rendered}");
    assert!(
        !rendered.contains("ConfigDumpInfo.xml"),
        "an unknown answer is silent: {rendered}"
    );
    assert!(calls_log.exists(), "the platform must run: {rendered}");
}

/// Команда `v8-runner …` из совета отказа, разобранная на слова так, как её разобрала бы
/// оболочка: совет кавычит значения одинарными кавычками.
fn advised_command(message: &str) -> Vec<String> {
    let start = message
        .find("`v8-runner ")
        .unwrap_or_else(|| panic!("the advice must name an exact command: {message}"))
        + 1;
    let rest = &message[start..];
    let line = &rest[..rest.find('`').expect("closing backtick")];
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut started = false;
    for ch in line.chars() {
        match ch {
            '\'' => {
                quoted = !quoted;
                started = true;
            }
            ' ' if !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            _ => {
                word.push(ch);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    assert_eq!(
        words.first().map(String::as_str),
        Some("v8-runner"),
        "{line}"
    );
    words
}

/// Слова совета, где значения `--config` и `--workdir` приведены к каноническому пути: на
/// macOS временный каталог `/var/…` — ссылка на `/private/var/…`, и раннер называет
/// разрешённый путь. Сравнивают с тоже каноническими ожидаемыми путями.
fn with_canonical_paths(words: &[String]) -> Vec<String> {
    let mut canonical = words.to_vec();
    for index in 1..canonical.len() {
        if matches!(words[index - 1].as_str(), "--config" | "--workdir") {
            canonical[index] = fs::canonicalize(&words[index])
                .unwrap_or_else(|error| panic!("{}: {error}", words[index]))
                .display()
                .to_string();
        }
    }
    canonical
}

/// Выполняет совет буквально — из другого каталога, без `--json-message` и прочего, что
/// было в исходном вызове.
fn run_advice_from_elsewhere(
    advice: &[String],
    elsewhere: &Path,
) -> (std::process::Output, String) {
    fs::create_dir_all(elsewhere).expect("elsewhere");
    let output = v8_runner_command()
        .current_dir(elsewhere)
        .args(&advice[1..])
        .output()
        .expect("run advice");
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output, rendered)
}

/// В проекте EDT отказывают и `pull --object`, и прежний `--mode incremental|partial`.
/// «Тот же вызов с `--force`» упёрся бы во второй отказ (`--object`/`--mode` спорят с
/// `--force`), поэтому совет — точная `pull <SET> --force` с глобальными ключами вызова и
/// прямо названная полная замена. Выполненный буквально из другого каталога, он проходит и
/// заменяет тот же каталог.
#[test]
fn an_edt_refusal_advises_a_full_replacement_that_runs_as_written() {
    for keys in [
        &["--object", "Catalog:Items"][..],
        &["--mode", "incremental"],
        &["--mode", "partial"],
    ] {
        let (dir, config_path, _platform, _edt, _work, base_path, _designer, _edt_calls) =
            setup_edt_project();
        git(&base_path, &["init", "-q", "-b", "main", "."]);
        git(&base_path, &["config", "user.email", "test@example.com"]);
        git(&base_path, &["config", "user.name", "Test"]);
        git(&base_path, &["add", "-A"]);
        git(&base_path, &["commit", "-qm", "committed sources"]);
        let hand_written = base_path.join("main").join("hand-written.xml");
        fs::write(&hand_written, "written by hand\n").expect("hand-written");

        let config = config_path.display().to_string();
        let mut args = vec![
            "--config",
            config.as_str(),
            "--json-message",
            "pull",
            "main",
        ];
        args.extend_from_slice(keys);
        let output = v8_runner_command().args(&args).output().expect("run pull");
        let payload: Value = serde_json::from_slice(&output.stdout).expect("json");
        assert_eq!(output.status.code(), Some(2), "{keys:?}: {payload}");
        let message = payload["error"]["message"].as_str().expect("message");
        assert!(message.contains("hand-written.xml"), "{keys:?}: {message}");
        assert!(
            !message.contains("repeat the same command with `--force` added"),
            "{keys:?}: {message}"
        );
        assert!(
            message.contains("a full dump of source-set 'main' that replaces its whole directory"),
            "{keys:?}: {message}"
        );

        let advice = advised_command(message);
        let canonical_config = fs::canonicalize(&config_path).expect("canonical config");
        assert_eq!(
            with_canonical_paths(&advice)[1..],
            [
                "--config".to_owned(),
                canonical_config.display().to_string(),
                "pull".to_owned(),
                "main".to_owned(),
                "--force".to_owned(),
            ],
            "{keys:?}: {message}"
        );

        let (output, rendered) = run_advice_from_elsewhere(&advice, &dir.path().join("elsewhere"));
        assert!(
            output.status.success(),
            "{keys:?}: the advice must not run into a second refusal: {rendered}"
        );
        assert!(
            !hand_written.exists(),
            "{keys:?}: the advice replaced the same directory"
        );
    }
}

/// MCP по stdio, сервер запущен с `--infobase` и `--workdir`: совет называет команду строки
/// с конфигом абсолютным путём и теми же ключами. Выполненная буквально из другого
/// каталога, она идёт в ту же базу и тот же рабочий каталог, а не в базу по умолчанию.
#[test]
fn an_mcp_refusal_advises_the_command_line_of_the_same_base_and_workdir() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let (dir, config_path, _binary, _work, base_path, calls_log) = setup_project_in_a_repository();
    fs::write(
        config_path.with_file_name("v8project.local.yaml"),
        format!("infobases:\n  staging:\n    connection: 'File={tmp}/staging-ib'\n"),
    )
    .expect("local config");
    let hand_written = base_path.join("main").join("hand-written.xml");
    fs::write(&hand_written, "written by hand\n").expect("hand-written");
    let other_work = dir.path().join("other-work");
    fs::create_dir_all(&other_work).expect("other work");

    let config = config_path.display().to_string();
    let workdir = other_work.display().to_string();
    let answer = support::mcp::call_tool_started_with(
        &[
            "--config",
            config.as_str(),
            "--infobase",
            "staging",
            "--workdir",
            workdir.as_str(),
        ],
        "dump_config",
        json!({ "mode": "FULL" }),
    );
    assert!(answer.is_error, "{}", answer.envelope);
    let message = answer.envelope["error"]["message"]
        .as_str()
        .expect("message")
        .to_owned();
    assert!(message.contains("hand-written.xml"), "{message}");
    assert!(message.contains("from the command line"), "{message}");
    assert!(
        !message.contains("on the machine where the MCP server runs"),
        "stdio runs on the caller's machine: {message}"
    );

    let advice = advised_command(&message);
    let canonical_config = fs::canonicalize(&config_path).expect("canonical config");
    let canonical_work = fs::canonicalize(&other_work).expect("canonical work");
    assert_eq!(
        with_canonical_paths(&advice)[1..],
        [
            "--config".to_owned(),
            canonical_config.display().to_string(),
            "--infobase".to_owned(),
            "staging".to_owned(),
            "--workdir".to_owned(),
            canonical_work.display().to_string(),
            "pull".to_owned(),
            "main".to_owned(),
            "--force".to_owned(),
        ],
        "{message}"
    );

    let (output, rendered) = run_advice_from_elsewhere(&advice, &dir.path().join("elsewhere"));
    assert!(output.status.success(), "{rendered}");
    assert!(
        !hand_written.exists(),
        "the advice replaced the same directory"
    );
    let calls = fs::read_to_string(calls_log).expect("calls");
    assert!(calls.contains("staging-ib"), "the same base: {calls}");
    assert_ibcmd_data_path(&calls, &other_work);
}

/// Значение в строке соединения, которого загрузчик не отвергает: учётные данные строка
/// нести не может (#380), но совет не повторяет и остальное — ни части строки.
const CONNECTION_SECRET: &str = "SECRETPW";

/// Строка соединения из `--infobase` может нести секрет, и совет её не повторяет: просит
/// то же значение `--infobase` словами. Команда строки, отказавшая сторожем EDT.
#[test]
fn a_command_line_advice_never_repeats_the_connection_string() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let (_dir, config_path, _platform, _edt, _work, base_path, _designer, _edt_calls) =
        setup_edt_project();
    git(&base_path, &["init", "-q", "-b", "main", "."]);
    git(&base_path, &["config", "user.email", "test@example.com"]);
    git(&base_path, &["config", "user.name", "Test"]);
    git(&base_path, &["add", "-A"]);
    git(&base_path, &["commit", "-qm", "committed sources"]);
    fs::write(base_path.join("main").join("hand-written.xml"), "mine\n").expect("hand-written");

    let config = config_path.display().to_string();
    let connection = format!("File={tmp}/staging-ib;Locale={CONNECTION_SECRET}");
    let output = v8_runner_command()
        .args([
            "--config",
            config.as_str(),
            "--infobase",
            connection.as_str(),
            "--json-message",
            "pull",
            "main",
            "--object",
            "Catalog:Items",
        ])
        .output()
        .expect("run pull");
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(2), "{rendered}");
    assert!(rendered.contains("hand-written.xml"), "{rendered}");
    assert!(rendered.contains(" pull main --force`"), "{rendered}");
    assert!(
        rendered.contains("with the same `--infobase` value as this command"),
        "{rendered}"
    );
    assert!(!rendered.contains(CONNECTION_SECRET), "{rendered}");
    assert!(!rendered.contains("staging-ib"), "{rendered}");
}

/// Сервер MCP по stdio, запущенный со строкой соединения в `--infobase`: совет отказа не
/// повторяет её, а просит то же значение, с которым запущен сервер.
#[test]
fn an_mcp_advice_never_repeats_the_connection_string_of_the_server() {
    let bases = support::temp_workspace();
    let tmp = bases.path().display().to_string();
    let (_dir, config_path, _binary, _work, base_path, _calls_log) =
        setup_project_in_a_repository();
    fs::write(base_path.join("main").join("hand-written.xml"), "mine\n").expect("hand-written");

    let config = config_path.display().to_string();
    let connection = format!("File={tmp}/staging-ib;Locale={CONNECTION_SECRET}");
    let answer = support::mcp::call_tool_started_with(
        &[
            "--config",
            config.as_str(),
            "--infobase",
            connection.as_str(),
        ],
        "dump_config",
        json!({ "mode": "FULL" }),
    );
    assert!(answer.is_error, "{}", answer.envelope);
    let rendered = answer.envelope.to_string();
    assert!(rendered.contains("hand-written.xml"), "{rendered}");
    assert!(rendered.contains(" pull main --force`"), "{rendered}");
    assert!(
        rendered.contains("with the same `--infobase` value the MCP server was started with"),
        "{rendered}"
    );
    assert!(!rendered.contains(CONNECTION_SECRET), "{rendered}");
    assert!(!rendered.contains("staging-ib"), "{rendered}");
}
